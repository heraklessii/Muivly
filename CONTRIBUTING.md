# Contributing to Muivly

Thanks for looking. Muivly is early — the parts that exist are small enough
that a newcomer can read all of them in an afternoon.

## The rules that are not up for negotiation

Muivly exists because Wallpaper Engine is heavy on old machines. Every one of
these rules is that goal, written down. A pull request that breaks one will be
declined even if it works and even if it is faster in some other way. If you
think a rule is wrong, open an issue and argue the case there first — that is a
real conversation, and `docs/decisions.md` exists to record it when a rule
changes.

1. **Video is decoded on the GPU.** D3D11VA via Media Foundation. No CPU decode
   fallback. If a codec has no hardware decoder on the user's machine, we show
   a static first frame and say so in the UI — we do not quietly burn a core.
2. **Zero-copy.** A decoded frame stays a D3D11 texture. Do not add a path that
   maps it to system memory to get it on screen.
3. **One decode per adapter, never one per monitor.** The same video on several
   monitors is decoded once and the texture shared. The only split is across
   physical GPUs, because cross-adapter sharing costs a system-memory copy.
4. **Nothing visible means nothing running — including memory.** Fullscreen
   app in front, or a hidden desktop, must drop CPU and GPU use to
   approximately zero. Not "reduced" — stopped. And once it has been out of
   sight for a while the decoders themselves are handed back, because a
   picture buffer held for a picture nobody can see is the most expensive
   thing in this process. Do not add state that has to survive that.
5. **No wallpaper rendering in `wallpaper-ui`.** The WebView is for settings.
   Its memory must never be part of the running wallpaper's cost.
6. **No telemetry, and no network the user did not ask for.** No analytics, no
   crash reporting that sends data, no update pings. The one place Muivly
   talks to the internet is the Discover view: it fetches a page from
   motionbgs.com when you open it and downloads a file when you press
   download, and it will not talk to any other host. Nothing runs in the
   background, and nothing identifying is ever sent.
7. **Images may decode on the CPU; video may not.** Rule 1 is about video,
   where a software decoder burns a core for as long as it is on screen. No
   GPU decodes a PNG, and a still image decodes once and then costs nothing
   at all. A shader is lighter still: no decoder, no picture buffers, no
   codec threads. Neither is an exception to rule 1 — both are the shape
   rule 1 is trying to get video closer to.

   Rule 1 covers writing video too. `optimize/` is the only place Muivly
   encodes, and it is hardware decode into hardware encode on one D3D11
   device. When a machine has no hardware encoder the job fails and says so;
   it does not fall back to the CPU.
8. **On battery, less.** The engine watches the power source and lowers its
   frame rate unplugged; under Windows' battery saver it stops drawing
   altogether. Do not add a code path that keeps working at full rate while
   the machine is asked to conserve — the wallpaper is the first thing that
   should give way, not the last.
9. **Nothing is installed the user cannot switch off.** Muivly writes to the
   registry in exactly three places, all under `HKEY_CURRENT_USER` and all
   with a switch in the settings window: the start-up entry, the Explorer
   context-menu item, and the accent colour. Nothing goes in
   `HKEY_LOCAL_MACHINE`, nothing needs administrator rights, and nothing is
   written that the app cannot remove.

   The accent colour is the one that overwrites something the user already
   had, so it carries an extra obligation: the previous values are saved to
   a file before the first write and put back when the setting goes off, when
   the engine quits, or on the next start if the engine was killed before it
   could. A feature that changes a user's settings and cannot change them
   back does not ship.
10. **Nobody at the machine is nobody watching.** Coverage detection answers
    whether the wallpaper *can* be seen. It never fires for a visible desktop
    with an empty chair in front of it, which is a full frame rate spent on
    no one — so the engine also watches how long it has been since the
    keyboard or the mouse was touched, and stands still. It reads Windows'
    own idle counter; it does not install an input hook, and it never sees a
    keystroke. Do not replace that with anything that does.

## Adding a dependency

Weigh it first, and say what you weighed in the PR: how much does it add to the
binary, how much RAM does it pull in at runtime, and can the standard library
or an existing dependency do the job. `wallpaper-core` currently has exactly
one dependency (`windows`) and that is a feature, not an accident.

## Setup

You need [Rust](https://rustup.rs/) (stable) and the MSVC toolchain: Visual
Studio Build Tools with the "Desktop development with C++" workload and the
Windows SDK.

```bash
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

```bash
cargo build
```

The settings window is a separate Cargo workspace and needs Node 24+:

```bash
cd wallpaper-ui && npm install && npx tauri build --no-bundle
```

Build the UI with `tauri build`, not `cargo build`: a plain cargo build marks
it as a development build, and the window then tries to load the Vite dev
server instead of the bundled files.

## Before you open a pull request

```bash
cargo fmt --all
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo test
```

CI runs all three on Windows, for both crates — `wallpaper-ui/src-tauri` has
its own workspace and its own run of the same commands. It will not merge with
warnings.

## Testing GPU code

Most of this project talks to hardware, and hardware differs. If you touch
`caps/`, `decoder/` or `compositor/`, include the output of `muivly-core --caps`
for the machine you tested on. Two configurations are especially easy to get
wrong and easy to miss if you do not have one:

- **A hybrid laptop** (integrated + discrete GPU). Monitors can be attached to
  different adapters, and which adapter owns which output decides where we
  render.
- **Mixed DPI scaling.** Windows hands out DPI-virtualized display coordinates
  by default; a 2560x1440 panel at 125% scaling reports 2048x1152 unless you
  ask for real pixels.

## Code conventions

- Comments and identifiers in English. Commit messages: English or Turkish,
  short and specific.
- One folder per module under `wallpaper-core/src/`.
- Explain *why*, not *what*, in comments — especially where a Win32 API
  behaves in a way the name does not suggest. Those comments are the most
  valuable thing in this codebase.

## Reporting bugs

Include `muivly-core --caps` output, your Windows version, and what you
expected to happen. Video playback issues: say what container and codec, since
hardware support varies by both.
