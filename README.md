# Muivly

A native live wallpaper engine for Windows, built for machines that struggle with the alternatives.

[![CI](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml/badge.svg)](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> **Status: early development.** Muivly plays a video wallpaper on every
> monitor, decoded on the GPU, stops decoding entirely when nothing is
> visible, and has a settings window — a library, playlists, and a different
> wallpaper per monitor — that lives in the system tray. Memory use is still
> well above where it needs to be. There is no release to download yet — star
> or watch the repo if you want to know when there is.

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
- **Unplugged is a different budget.** On battery Muivly drops to a lower frame
  rate, and under Windows' battery saver it stops moving altogether — the last
  frame stays on screen. Both are settings; both are on by default, because the
  machines this is for are the ones where it matters.
- **The settings window is a separate process.** It uses a WebView, which costs
  RAM. Muivly makes that a cost you pay only while the settings window is open,
  never while the wallpaper is running.

## Download

Releases will appear on the [Releases page](https://github.com/heraklessii/Muivly/releases)
once there is something worth downloading. Nothing there yet.

Planned artifacts, per release:

| File | What it is |
|---|---|
| `Muivly-x.y.z-setup.exe` | Installer. Per user, no administrator prompt. |
| `Muivly-x.y.z-portable.zip` | Unzip and run. No installer, no registry writes. |

Once the first release is out it will also be installable with winget:

```bash
winget install heraklessii.Muivly
```

Windows 10 (1903+) or Windows 11, 64-bit. A GPU with hardware video decode —
essentially anything from 2014 onward, including integrated graphics.

HEVC and AV1 need Microsoft's free codec extensions from the Store; Windows
does not ship either. Muivly says so by name when a file needs one, rather
than reporting that the file would not open.

## Trying it

Two processes: `muivly-core.exe` is the engine and `muivly-ui.exe` is the
settings window. The UI can start the engine for you, or run the engine
directly and point it at a video file:

```bash
muivly-core "C:\path\to\wallpaper.mp4"
```

Ctrl+C stops it and restores your own wallpaper. Without a file it shows a
placeholder gradient instead.

Measured on a two-monitor hybrid laptop (integrated + discrete GPU, so two
decoders — the worst case), 1080p video at 30 fps:

| | CPU (one core) |
|---|---|
| Desktop visible | 13.7% |
| Every monitor covered | **0.4%** |

Memory is the honest weak spot. The worst case — a 4K clip on both GPUs, so
two decoders — measured on that same laptop:

| | Working set | Private bytes | Threads |
|---|---|---|---|
| Before | 610 MB | 790 MB | 80–82 |
| Now | **540 MB** | **706 MB** | 79–81 |

The 85 MB came from telling Media Foundation not to build up the queue of
decoded frames it keeps for smooth playback of a film — a wallpaper has
nothing to seek through. Asking the decoder for a smaller pool of output
samples was also tried and does nothing: those knobs set a floor, and the
floor that matters is the codec's own reference-frame requirement.

What is left is almost entirely those two decoders' picture buffers, and the
threads are Media Foundation's shared work queue. Both need a different shape,
not a smaller number.

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

The engine:

```bash
cargo build --release
```

It lands in `target/release/muivly-core.exe`, about 200 KB, with exactly one
dependency: the `windows` crate (Win32 API bindings).

The settings window (needs Node 24+; it is a separate Cargo workspace, so the
engine build above never pulls Tauri in):

```bash
cd wallpaper-ui && npm install && npx tauri build --no-bundle
```

## Architecture

Two processes that talk over a named pipe:

```
wallpaper-core/   Rust native binary. The engine. Keeps running — and keeps its
                  memory profile — whether or not the UI is open.
  caps/           GPU/system probe. Runs once at startup.
  decoder/        Media Foundation + D3D11VA hardware decode.
  compositor/     WorkerW injection, multi-monitor shared D3D11 texture.
  power/          Fullscreen/occlusion detection, render throttle and pause,
                  and the battery policy.
  audio/          WASAPI playback, and standing down for other applications.
  ipc/            Named pipe server.

wallpaper-ui/     Tauri v2 + React. The settings panel, and nothing else.
                  Wallpapers are never rendered here. Closing the window
                  hides it to the tray; the engine is untouched either way.
```

Design notes and the reasoning behind each choice live in [`docs/`](docs/) —
`decisions.md` in particular records what was considered and rejected.

## Roadmap

- [x] Hardware capability detection
- [x] WorkerW injection, one D3D11 device per adapter, per-monitor surfaces
- [x] Occlusion detection — decoding and rendering stop when nothing is visible
- [x] Media Foundation decoder — hardware H.264/HEVC/VP9/AV1, zero-copy
- [x] Settings UI — Tauri v2, closes to the system tray
- [x] Per-monitor wallpaper assignment
- [x] Local library and playlists
- [x] Frame pacing that holds — high-resolution timer, woken by the video's own
      frame times rather than a grid of ours
- [x] Decoding on its own thread, so a slow read never lands in a frame
- [x] Still images and animated GIFs
- [x] Sound, off by default, muted the moment nothing is visible
- [x] Brightness, saturation and blur
- [x] Start with Windows — the engine only, which restores its own last session
- [x] Tray menu: next wallpaper, pause, mute, quit
- [x] Import from Wallpaper Engine's workshop folders
- [x] Discover — free wallpapers from motionbgs.com, downloaded into the library
- [x] Live CPU and memory readout, measured by the engine itself
- [x] Battery-aware: a lower frame rate unplugged, frozen under battery saver
- [x] Ducking — the soundtrack steps back while another application is audible
- [x] Per-monitor fit, grade and frame rate; one wallpaper spanned across screens
- [x] Crossfade between wallpapers, and playback speed
- [x] Global shortcuts, and "set as wallpaper" in Explorer's right-click menu
- [x] Survives a monitor being plugged in, a resolution change, sleep, and an
      Explorer restart
- [x] `.muivly` packages — a wallpaper and its credit in one file
- [ ] Bring memory down further (a 4K clip on two GPUs is still ~700 MB)
- [ ] First release
- [ ] Measured RAM/CPU comparison against the alternatives

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The short version: this project has a
few hard rules about how video gets on screen, and a change that breaks them
will not be merged no matter how well it works — the rules are the product.

## Privacy

Muivly does not phone home. No telemetry, no analytics, no update pings you did
not ask for, no account.

Installing it writes to two places in your own registry, both optional and both
switchable from the settings window: the start-up entry, and the "Muivly duvar
kağıdı yap" item in Explorer's right-click menu. Nothing is written outside
`HKEY_CURRENT_USER` and `%APPDATA%\Muivly`, which is why the installer never
asks for administrator rights.

It goes online in exactly one place: the **Discover** view, which lists free
wallpapers from [motionbgs.com](https://motionbgs.com). Opening that view
fetches a page; pressing download fetches a file into
`%APPDATA%\Muivly\wallpapers`. Nothing is sent but the request itself — no
identifier, no history, no account — and the engine refuses to talk to any host
other than that one. Close the view and Muivly is offline again. Everything
else works with no connection at all.

## License

[Apache-2.0](LICENSE).
