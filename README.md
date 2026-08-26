# Muivly

A native live wallpaper engine for Windows, built for machines that struggle with the alternatives.

[![CI](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml/badge.svg)](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> **Status: early development.** Muivly renders a placeholder wallpaper on
> every monitor and stops rendering when nothing is visible. It cannot play
> video yet — the decoder is the next piece. There is no release to download.
> Star or watch the repo if you want to know when there is.

## Why

Wallpaper Engine is good software, but on an old laptop with an integrated GPU
and 8 GB of RAM it is a tax you pay all day, every day. Muivly targets exactly
those machines. The goal is not "it runs" — it is "it runs and you cannot tell
it is running."

That goal drives every design decision in this repo:

- **Video is decoded on the GPU, always.** Media Foundation with D3D11VA. There
  is no CPU decode fallback, deliberately: it would blow the CPU budget on the
  machines this is for.
- **Zero-copy.** A decoded frame stays on the GPU as a D3D11 texture. It never
  round-trips through system memory to get on screen.
- **One decode, not one per monitor.** The same video on three screens is
  decoded once and the texture is shared. (Screens on *different* GPUs get one
  decode per GPU — sharing across adapters would break zero-copy.)
- **Zero work when nothing is visible.** Fullscreen app in front? Desktop
  hidden? Rendering stops, not "slows down".
- **The settings window is a separate process.** It uses a WebView, which costs
  RAM. Muivly makes that a cost you pay only while the settings window is open,
  never while the wallpaper is running.

## Download

Releases will appear on the [Releases page](https://github.com/heraklessii/Muivly/releases)
once there is something worth downloading. Nothing there yet.

Planned artifacts, per release:

| File | What it is |
|---|---|
| `Muivly-x.y.z-setup.exe` | Installer. Adds a start-up entry, associates wallpaper files. |
| `Muivly-x.y.z-portable.zip` | Unzip and run. No installer, no registry writes. |

Windows 10 (1903+) or Windows 11, 64-bit. A GPU with hardware video decode —
essentially anything from 2014 onward, including integrated graphics.

## Check your hardware

Reports what Muivly detected about your machine and what settings it would
pick:

```bash
muivly-core --caps
```

```
system: 15654 MB RAM, power: AC
adapter: AMD Radeon(TM) Graphics [Integrated] 1002:15bf vram=419 MB decode=h264+hevc+hevc10+vp9+av1
  output: \\.\DISPLAY5 2560x1440 @180Hz (primary)
adapter: NVIDIA GeForce RTX 4050 Laptop GPU [Discrete] 10de:28a1 vram=5920 MB decode=h264+hevc+hevc10+vp9+av1
  output: \\.\DISPLAY1 1920x1080 @144Hz
tier: Mid -> 30 fps, max 2560x1440, distinct videos: false
reason: integrated GPU
```

Muivly picks defaults from this: an integrated GPU gets 30 fps, a discrete one
60, and the frame rate never exceeds your monitor's refresh rate or goes above
30 on battery. These are defaults, not limits — the settings UI will let you
override them.

If you are filing a bug, paste this output into the issue.

## Building from source

You need [Rust](https://rustup.rs/) and the MSVC toolchain (Visual Studio Build
Tools with the "Desktop development with C++" workload and the Windows SDK).

```bash
git clone https://github.com/heraklessii/Muivly.git
```

```bash
cargo build --release
```

The binary lands in `target/release/muivly-core.exe`. It is about 120 KB and
has exactly one dependency: the `windows` crate (Win32 API bindings).

## Architecture

Two processes that talk over a named pipe:

```
wallpaper-core/   Rust native binary. The engine. Keeps running — and keeps its
                  memory profile — whether or not the UI is open.
  caps/           GPU/system probe. Runs once at startup.
  decoder/        Media Foundation + D3D11VA hardware decode.
  compositor/     WorkerW injection, multi-monitor shared D3D11 texture.
  power/          Fullscreen/occlusion detection, render throttle and pause.
  ipc/            Named pipe server.

wallpaper-ui/     Tauri v2 + React. The settings panel, and nothing else.
                  Wallpapers are never rendered here.
```

Design notes and the reasoning behind each choice live in [`docs/`](docs/) —
`decisions.md` in particular records what was considered and rejected.

## Roadmap

- [x] Hardware capability detection
- [x] WorkerW injection, one D3D11 device per adapter, per-monitor surfaces
- [x] Occlusion detection — rendering stops when nothing is visible
- [ ] Media Foundation decoder
- [ ] Shared texture across monitors on one adapter
- [ ] Settings UI
- [ ] First release
- [ ] Measured RAM/CPU comparison against the alternatives

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The short version: this project has a
few hard rules about how video gets on screen, and a change that breaks them
will not be merged no matter how well it works — the rules are the product.

## Privacy

Muivly does not phone home. No telemetry, no analytics, no update pings you did
not ask for, no account. It reads the video files you point it at and nothing
else.

## License

[Apache-2.0](LICENSE).
