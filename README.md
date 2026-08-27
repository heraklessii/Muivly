# Muivly

A native live wallpaper engine for Windows, built for machines that struggle with the alternatives.

[![CI](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml/badge.svg)](https://github.com/heraklessii/Muivly/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**[heraklessii.github.io/Muivly](https://heraklessii.github.io/Muivly)** — the
same case, on one page.

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
  hidden? Rendering stops, not "slows down" — and after twenty seconds the
  decoders are handed back too, so the memory goes with the CPU time. The
  last frame stays on screen; the wallpaper comes back when the desktop does.
- **Nobody at the machine is nobody watching.** A visible desktop with an
  empty chair in front of it costs the full frame rate and nothing catches
  it: nothing is covering anything. So after five minutes without a keypress
  the picture stands still, and comes back on the first one. Muivly reads
  Windows' own idle counter — it installs no input hook and never sees a
  keystroke.
- **Out of the way when the machine is busy.** The wallpaper does not get
  more expensive when you start a build; everything else does. Above 80% of
  the machine it drops to 10 frames a second until things quieten down.
- **Unplugged is a different budget.** On battery Muivly drops to a lower frame
  rate, and under Windows' battery saver it stops moving altogether — the last
  frame stays on screen. Both are settings; both are on by default, because the
  machines this is for are the ones where it matters.
- **The settings window is a separate process.** It uses a WebView, which costs
  RAM. Muivly makes that a cost you pay only while the settings window is open,
  never while the wallpaper is running.

- **A shader is a wallpaper.** Drop in a `.hlsl` file with one
  `mainImage(float2 uv)` function and it plays like anything else — with no
  decoder, no picture buffers and no codec threads behind it. It is the
  lightest thing Muivly can put on a desktop, by a wide margin. There are
  examples in [examples/shaders](examples/shaders).

  A file can declare its own sliders, which appear in the settings window:

  ```hlsl
  // param speed 0.1 3.0 1.0 How fast it moves
  ```

  Shadertoy shaders work too. Save one as `.glsl` or `.frag` and Muivly
  translates it on the way in — line for line, so a compile error still
  points at the line you wrote. Shaders that sample a texture channel
  (`iChannel0`) cannot work and say so rather than failing obscurely.

  A shader can also listen: `iLevel` is how loud the machine is and
  `iBand(0..7)` splits that into bands, which is what
  [`spectrum.hlsl`](examples/shaders/spectrum.hlsl) draws. The capture that
  feeds it is opened only while a shader that reads it is on screen, has no
  thread of its own, and is drained on a frame the engine was drawing anyway.

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

Both of those shapes now exist, and **Lighten** is where they land. The size
of a picture buffer is the codec's reference-frame count times the size of the
frame, and neither belongs to the engine at playback time — but both belong to
whoever writes the file. So Lighten rewrites a clip once, on the GPU, at the
size the desktop actually is *and* with a single reference frame instead of
the four or more an encoder picks by default. A 4K loop on a 1080p laptop
stops decoding four times the pixels that screen can show, permanently, with
no scaler in the playback path to pay for it. The library flags a clip that is
bigger than your largest screen rather than waiting for you to suspect it.

There is also a memory budget in the settings for people who would rather set
a number than convert a file.

None of these numbers are yours. To get yours:

```bash
muivly-core --benchmark "C:\path\to\wallpaper.mp4"
```

That plays the wallpaper for thirty seconds and prints what it cost on your
machine — the real engine, the real desktop, no separate measuring path.

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
- [x] Settings UI — Tauri v2, closes to the system tray; four levels from
      "barely touch this machine" to "as good as it gets", with every dial
      still there behind them
- [x] Per-monitor wallpaper assignment
- [x] Local library and playlists, shuffled or in the order you wrote them
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
- [x] Stands still when nobody has touched the machine, and stands down when
      the machine is busy with something else
- [x] Lighten also cuts the encoder to one reference frame, which is the other
      half of what a decoder's memory is made of
- [x] Shaders with their own sliders, Shadertoy `.glsl` import, and eight
      bands of the sound for shaders that draw it
- [x] Scenes — a named arrangement of wallpapers across the screens
- [x] A slow drift for still photographs, with no decoder behind it
- [x] Optional: the Windows accent colour follows the wallpaper, and gives
      your own colours back when you switch it off
- [x] `--benchmark` — the README's table, measured on your machine
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

Installing it writes to three places in your own registry, all optional and all
switchable from the settings window: the start-up entry, the "Muivly duvar
kağıdı yap" item in Explorer's right-click menu, and — only if you switch it on
— the accent colour. Nothing is written outside `HKEY_CURRENT_USER` and
`%APPDATA%\Muivly`, which is why the installer never asks for administrator
rights.

The accent colour is the only one that overwrites something you already had, so
it is the only one that comes with a promise: your previous colours are saved
before the first write and put back when you switch it off, when the engine
quits, or on the next start if the engine was killed before it could.

The idle detection reads the counter Windows already keeps for the whole
session. There is no input hook, and no keystroke is ever seen by Muivly. The
audio bands a shader can read come from a loopback capture of what your own
machine is playing, opened only while such a shader is on screen and closed
when it leaves; nothing is recorded and nothing leaves the machine.

It goes online in exactly one place: the **Discover** view, which lists free
wallpapers from [motionbgs.com](https://motionbgs.com). Opening that view
fetches a page; pressing download fetches a file into
`%APPDATA%\Muivly\wallpapers`. Nothing is sent but the request itself — no
identifier, no history, no account — and the engine refuses to talk to any host
other than that one. Close the view and Muivly is offline again. Everything
else works with no connection at all.

## License

[Apache-2.0](LICENSE).
