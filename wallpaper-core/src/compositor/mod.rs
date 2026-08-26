//! Puts pixels on the desktop background.

mod clock;
mod diag;
mod notify;
mod render;
mod shader;
mod stats;
mod window;
mod workerw;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, SystemParametersInfoW, TranslateMessage, MSG, PM_REMOVE,
    SPIF_SENDCHANGE, SPI_SETDESKWALLPAPER,
};

use crate::audio::Audio;
use crate::caps::GpuProfile;
use crate::ipc::{Command, MonitorState, Status};
use crate::power::battery::{PowerPolicy, PowerState, PowerWatch};
use crate::session::{self, Session};
use render::Renderer;
use window::Surface;

pub use diag::dump as dump_window_tree;
pub use render::{Fit, Overrides, Rect, Visual};

static RUNNING: AtomicBool = AtomicBool::new(true);

/// Ask the render loop to finish the current frame and return.
pub fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
}

/// Whether the wallpaper is allowed to make noise, and how loudly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    pub enabled: bool,
    /// 0.0 to 1.0. Independent of the Windows volume, which stays the way
    /// the user set it for everything else.
    pub volume: f32,
    /// Whether to stand down while another application is making sound.
    pub duck: bool,
}

impl Default for Sound {
    fn default() -> Self {
        // Silent until asked. Half volume so the first time it is switched on
        // it is audible without being a shock. Ducking on, because the first
        // time wallpaper sound talks over a video call is the last time
        // anybody leaves it switched on.
        Self {
            enabled: false,
            volume: 0.5,
            duck: true,
        }
    }
}

/// What one monitor is playing, and where it is in a playlist.
struct Playback {
    /// Empty means this monitor keeps the Windows wallpaper. One entry is a
    /// fixed wallpaper. More than one is a playlist.
    items: Vec<PathBuf>,
    index: usize,
    enabled: bool,
    /// When the current item started, for interval-based advancing.
    started: Instant,
    /// The decoder's loop count when the current item started, for
    /// advance-when-the-clip-ends.
    loops_at_start: u32,
}

impl Playback {
    fn current(&self) -> Option<&PathBuf> {
        self.items.get(self.index)
    }

    fn blank() -> Self {
        Self {
            items: Vec::new(),
            index: 0,
            enabled: true,
            started: Instant::now(),
            loops_at_start: 0,
        }
    }
}

/// Everything the user has chosen that is not "which file on which screen".
///
/// One struct rather than a dozen locals, because all of it is written by
/// commands, read by the render loop and saved to the session file — three
/// places that were drifting apart every time a setting was added.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub fps: u32,
    pub fit: Fit,
    /// Zero means "advance when the clip ends" rather than on a clock.
    pub interval_secs: u64,
    pub visual: Visual,
    pub sound: Sound,
    pub power: PowerPolicy,
    /// Playback rate, 1.0 being the speed the file was authored at.
    pub speed: f32,
    /// How long one wallpaper takes to replace another. Zero cuts.
    pub fade: Duration,
    /// One wallpaper stretched across every screen.
    pub span: bool,
    /// The three desktop-wide shortcuts.
    pub hotkeys: bool,
    /// Frozen by the user: the last frame stays on screen and nothing moves.
    /// Not saved — coming back from a restart already frozen would look
    /// exactly like a broken engine.
    pub frozen: bool,
    /// Settings a monitor keeps for itself, keyed by device name.
    pub overrides: HashMap<String, Overrides>,
}

impl Settings {
    fn new(profile: &GpuProfile) -> Self {
        Self {
            fps: profile.rec.target_fps.max(1),
            fit: Fit::default(),
            interval_secs: 0,
            visual: Visual::default(),
            sound: Sound::default(),
            power: PowerPolicy::default(),
            speed: 1.0,
            fade: Duration::from_millis(400),
            span: false,
            hotkeys: true,
            frozen: false,
            overrides: HashMap::new(),
        }
    }

    /// Whatever the last session chose, over the defaults this machine
    /// would otherwise get.
    fn restored(profile: &GpuProfile, session: Option<&Session>) -> Self {
        let mut settings = Self::new(profile);
        let Some(session) = session else {
            return settings;
        };

        if let Some(fps) = session.fps {
            settings.fps = fps.max(1);
        }
        if let Some(fit) = session.fit {
            settings.fit = fit;
        }
        if let Some(interval) = session.interval_secs {
            settings.interval_secs = interval;
        }
        if let Some(visual) = session.visual {
            settings.visual = visual;
        }
        // Sound is off until asked for. A desktop background that makes noise
        // nobody asked for is a bug, not a feature — but a user who did ask
        // last time is not asked again.
        if let Some(sound) = session.sound {
            settings.sound = sound;
        }
        if let Some(power) = session.power {
            settings.power = power;
        }
        if let Some(speed) = session.speed {
            settings.speed = speed;
        }
        if let Some(fade) = session.fade {
            settings.fade = fade;
        }
        if let Some(span) = session.span {
            settings.span = span;
        }
        if let Some(hotkeys) = session.hotkeys {
            settings.hotkeys = hotkeys;
        }
        settings.overrides = session.overrides.clone();

        settings
    }
}

/// The renderers, and what they were built for.
///
/// Kept together because a display change invalidates all of it at once: the
/// swap chains are sized to screens that may be gone, the desktop bounding
/// box has moved, and the primary monitor may be a different one.
struct Stage {
    renderers: Vec<Renderer>,
    /// The bounding box of every monitor, which is what a spanned wallpaper
    /// is cut out of.
    desktop: Rect,
    /// Which screen the soundtrack follows. One clip on two monitors is one
    /// song; two clips would be two at once, which is nobody's idea of a
    /// wallpaper.
    primary: Option<String>,
    /// What the displays looked like when this was built, so a broadcast
    /// that changed nothing does not cost a rebuild.
    layout: Vec<(String, i32, i32, u32, u32)>,
}

impl Stage {
    fn build(profile: &GpuProfile, parent: HWND) -> windows::core::Result<Self> {
        // One renderer per adapter. On a hybrid laptop with monitors on both
        // GPUs this is two devices — the alternative, one device feeding a
        // cross-adapter shared texture, copies every frame through system
        // memory. See docs/decisions.md.
        let mut renderers = Vec::new();
        for adapter in &profile.adapters {
            let mut surfaces = Vec::new();
            for monitor in &adapter.outputs {
                surfaces.push(Surface::create(parent, monitor)?);
            }

            let renderer = Renderer::new(
                adapter.luid,
                adapter.decode,
                surfaces,
                profile.rec.max_scale,
            )?;
            println!(
                "surface: {} monitor(s) on {}",
                renderer.monitor_count(),
                adapter.name
            );
            renderers.push(renderer);
        }

        Ok(Self {
            renderers,
            desktop: desktop_bounds(profile),
            primary: primary_monitor(profile),
            layout: layout_of(profile),
        })
    }

    /// Push every setting into the renderers. Called after a build and after
    /// any change that all of them share.
    fn apply(&mut self, settings: &Settings) {
        let span = settings.span.then_some(self.desktop);
        for renderer in &mut self.renderers {
            renderer.set_fit(settings.fit);
            renderer.set_visual(settings.visual);
            renderer.set_speed(settings.speed);
            renderer.set_fade(settings.fade);
            renderer.set_span(span);
            for (monitor, overrides) in &settings.overrides {
                renderer.set_overrides(monitor, *overrides);
            }
        }
    }
}

/// Create a surface for every monitor and render until stopped.
pub fn run(
    profile: &GpuProfile,
    initial: Option<PathBuf>,
    commands: Receiver<Command>,
    status: Arc<Mutex<Status>>,
) -> windows::core::Result<()> {
    let mut profile = profile.clone();
    let mut parent = find_parent()?;

    // The window that hears about display changes, sleep and hotkeys. None
    // of those reach a child of WorkerW; see compositor::notify.
    let mut notifier = notify::Notifier::create()?;

    // A path on the command line wins; otherwise the engine picks up where it
    // left off. Without this, autostart would bring up an engine with nothing
    // on screen and no way to know what had been there.
    let restored = if initial.is_some() {
        None
    } else {
        crate::session::load()
    };

    let mut settings = Settings::restored(&profile, restored.as_ref());
    notifier.set_hotkeys(settings.hotkeys);

    let mut playback: HashMap<String, Playback> = HashMap::new();
    for adapter in &profile.adapters {
        for monitor in &adapter.outputs {
            let saved = restored.as_ref().and_then(|session| {
                session
                    .monitors
                    .iter()
                    .find(|(name, _, _)| *name == monitor.device_name)
            });

            playback.insert(
                monitor.device_name.clone(),
                Playback {
                    items: match saved {
                        Some((_, _, items)) => items.clone(),
                        None => initial.iter().cloned().collect(),
                    },
                    enabled: saved.map(|(_, enabled, _)| *enabled).unwrap_or(true),
                    ..Playback::blank()
                },
            );
        }
    }

    let mut stage = Stage::build(&profile, parent)?;
    stage.apply(&settings);
    for (name, state) in &playback {
        apply(&mut stage.renderers, name, state.current())?;
    }

    let mut audio: Option<Audio> = None;
    let mut stats = stats::Stats::default();
    let mut power = PowerWatch::new();

    println!("rendering at {} fps - Ctrl+C to stop", settings.fps);
    publish(
        &status,
        &Report {
            settings: &settings,
            paused: false,
            power: power.state(),
            ducking: false,
            stats: &stats,
            error: None,
            playback: &playback,
        },
    );

    // How often to check whether a covered desktop has become visible again.
    // Half a second is imperceptible to a user alt-tabbing out of a game, and
    // it is 15 wakeups saved for every one spent at 30 fps.
    const OCCLUDED_POLL: Duration = Duration::from_millis(500);

    // The longest the loop will ever idle, whatever the wallpaper says it
    // needs. This is what keeps the UI feeling attached to the engine.
    const MAX_WAIT: Duration = Duration::from_millis(250);

    let start = Instant::now();
    let mut was_paused = false;
    let mut last_error: Option<String> = None;

    // Waiting the remainder of a frame with `thread::sleep` is accurate only
    // to the 15.6 ms system timer tick, which at 30 fps means every other
    // frame lands a tick late. That is the stutter; see compositor::clock.
    let pacer = clock::Pacer::new();
    // The earliest the next pass may start. Kept on a fixed grid rather than
    // measured from the end of each pass, so a slow frame does not push every
    // frame after it further out.
    let mut next_tick = Instant::now();

    while RUNNING.load(Ordering::Relaxed) {
        pump_messages();

        // Apply whatever the UI asked for since the last frame. Draining
        // rather than handling one keeps a burst of clicks from taking a
        // frame each.
        let mut changed = false;
        // Set by the commands worth remembering across a restart. Pausing,
        // stats and playlist position are not among them.
        let mut persist = false;
        for command in commands.try_iter() {
            changed = true;
            match command {
                Command::SetPlaylist { monitor, items } => {
                    persist = true;

                    // A spanned wallpaper is one picture, so choosing it on
                    // any screen chooses it on all of them. Otherwise each
                    // monitor would be showing its slice of a different
                    // video, which is not a thing anyone wants to look at.
                    let screens: Vec<String> = if settings.span {
                        playback.keys().cloned().collect()
                    } else {
                        vec![monitor.clone()]
                    };

                    for name in screens {
                        let entry = playback.entry(name.clone()).or_insert_with(Playback::blank);
                        entry.items = items.clone();
                        entry.index = 0;
                        entry.started = Instant::now();
                        entry.loops_at_start = 0;

                        let current = entry.current().cloned();
                        match &current {
                            Some(path) => println!("{name}: {}", path.display()),
                            None => println!("{name}: cleared"),
                        }
                        apply(&mut stage.renderers, &name, current.as_ref())?;
                    }

                    // A new choice is the user acting on whatever the last
                    // message said, so it stops being worth showing.
                    last_error = None;
                }

                Command::SetEnabled { monitor, enabled } => {
                    persist = true;
                    if let Some(entry) = playback.get_mut(&monitor) {
                        entry.enabled = enabled;
                    }
                    for renderer in &mut stage.renderers {
                        if renderer.has_monitor(&monitor) {
                            renderer.set_enabled(&monitor, enabled)?;
                        }
                    }
                    println!("{monitor}: {}", if enabled { "on" } else { "off" });
                }

                Command::Next { monitor } => {
                    persist = true;
                    advance(&mut stage.renderers, &mut playback, &monitor)?;
                }

                Command::Fps(n) => {
                    settings.fps = n.max(1);
                    println!("fps: {n}");
                    persist = true;
                }

                Command::SetFit(next) => {
                    settings.fit = next;
                    stage.apply(&settings);
                    persist = true;
                    println!("fit: {}", next.name());
                }

                Command::Interval(secs) => {
                    persist = true;
                    settings.interval_secs = secs;
                    println!("interval: {secs}s");
                }

                Command::SetVisual(next) => {
                    persist = true;
                    settings.visual = next;
                    stage.apply(&settings);
                    println!(
                        "visual: brightness={:.2} saturation={:.2} blur={:.2}",
                        next.brightness, next.saturation, next.blur
                    );
                }

                Command::SetSound(next) => {
                    persist = true;
                    settings.sound = next;
                    if let Some(playing) = &audio {
                        playing.set_volume(next.volume);
                        playing.set_duck(next.duck);
                    }
                    println!(
                        "sound: {} at {:.0}%{}",
                        if next.enabled { "on" } else { "off" },
                        next.volume * 100.0,
                        if next.duck { ", ducking" } else { "" }
                    );
                }

                Command::SetPower(policy) => {
                    persist = true;
                    settings.power = policy;
                    println!(
                        "power: battery {} fps, saver {}",
                        if policy.battery_fps == 0 {
                            "unchanged".to_string()
                        } else {
                            policy.battery_fps.to_string()
                        },
                        if policy.pause_on_saver {
                            "freezes"
                        } else {
                            "ignored"
                        }
                    );
                }

                Command::SetSpeed(speed) => {
                    persist = true;
                    settings.speed = speed;
                    stage.apply(&settings);
                    println!("speed: {speed:.2}x");
                }

                Command::SetFade(fade) => {
                    persist = true;
                    settings.fade = fade;
                    stage.apply(&settings);
                    println!("fade: {} ms", fade.as_millis());
                }

                Command::SetSpan(span) => {
                    persist = true;
                    settings.span = span;
                    stage.apply(&settings);

                    // Turning it on makes every screen show what the primary
                    // one was showing — the slices have to come from one
                    // picture for spanning to mean anything.
                    if span {
                        let source = stage
                            .primary
                            .as_ref()
                            .and_then(|name| playback.get(name.as_str()))
                            .map(|state| state.items.clone())
                            .unwrap_or_default();

                        let screens: Vec<String> = playback.keys().cloned().collect();
                        for name in screens {
                            let entry =
                                playback.entry(name.clone()).or_insert_with(Playback::blank);
                            entry.items = source.clone();
                            entry.index = 0;
                            let current = entry.current().cloned();
                            apply(&mut stage.renderers, &name, current.as_ref())?;
                        }
                    }
                    println!("span: {}", if span { "on" } else { "off" });
                }

                Command::SetHotkeys(on) => {
                    persist = true;
                    settings.hotkeys = on;
                    notifier.set_hotkeys(on);
                    println!("hotkeys: {}", if on { "on" } else { "off" });
                }

                Command::SetOverrides {
                    monitor,
                    overrides: next,
                } => {
                    persist = true;
                    if next == Overrides::default() {
                        settings.overrides.remove(&monitor);
                    } else {
                        settings.overrides.insert(monitor.clone(), next);
                    }
                    // Cleared through the renderer either way: removing the
                    // entry from the map is not what puts the monitor back on
                    // the desktop's settings.
                    for renderer in &mut stage.renderers {
                        renderer.set_overrides(&monitor, next);
                    }
                    println!("{monitor}: own settings {:?}", next);
                }

                Command::Freeze(frozen) => {
                    settings.frozen = frozen;
                    println!("{}", if frozen { "frozen" } else { "running" });
                }

                Command::Quit => stop(),
            }
        }

        // Hotkeys and the machine's own news. Both arrive through the hidden
        // window rather than the pipe, and both are read once a pass.
        let signals = notifier.take();
        if signals.any() {
            changed = true;
        }
        if signals.next {
            let screens: Vec<String> = playback.keys().cloned().collect();
            for monitor in screens {
                advance(&mut stage.renderers, &mut playback, &monitor)?;
            }
        }
        if signals.pause {
            settings.frozen = !settings.frozen;
            println!(
                "{}",
                if settings.frozen {
                    "frozen (hotkey)"
                } else {
                    "running (hotkey)"
                }
            );
        }
        if signals.mute {
            settings.sound.enabled = !settings.sound.enabled;
            persist = true;
        }

        // A monitor arrived, left or changed shape, or the machine woke up
        // with who-knows-what attached. Everything on screen is sized to a
        // layout that may no longer exist, so it is all rebuilt — including
        // the capability probe, because the new monitor may be on the other
        // GPU.
        if signals.display_changed || signals.resumed {
            let next = crate::caps::probe();
            let parent_gone = !workerw::is_alive(parent);

            if layout_of(&next) != stage.layout || parent_gone {
                println!("displays changed: rebuilding");

                // Dropped before the new one is built: two sets of surfaces
                // parented to the same WorkerW would both be on screen, and
                // the old ones are sized to monitors that may be gone.
                drop(stage);
                if parent_gone {
                    // Explorer restarted and took WorkerW with it.
                    parent = find_parent()?;
                }

                profile = next;
                stage = Stage::build(&profile, parent)?;
                stage.apply(&settings);

                // A monitor that was not there before starts empty; one that
                // came back keeps what it had.
                for adapter in &profile.adapters {
                    for monitor in &adapter.outputs {
                        playback
                            .entry(monitor.device_name.clone())
                            .or_insert_with(Playback::blank);
                    }
                }
                for (name, state) in &playback {
                    apply(&mut stage.renderers, name, state.current())?;
                }
                for (name, state) in &playback {
                    for renderer in &mut stage.renderers {
                        if renderer.has_monitor(name) {
                            renderer.set_enabled(name, state.enabled)?;
                        }
                    }
                }
            }
        }

        if persist {
            session::save(&Session {
                fps: Some(settings.fps),
                fit: Some(settings.fit),
                interval_secs: Some(settings.interval_secs),
                visual: Some(settings.visual),
                sound: Some(settings.sound),
                power: Some(settings.power),
                speed: Some(settings.speed),
                fade: Some(settings.fade),
                span: Some(settings.span),
                hotkeys: Some(settings.hotkeys),
                overrides: settings.overrides.clone(),
                monitors: playback
                    .iter()
                    .map(|(name, state)| (name.clone(), state.enabled, state.items.clone()))
                    .collect(),
            });
        }

        // Playlists advance either on a clock or when the clip ends. Both are
        // checked here rather than in the renderer, which has no idea what a
        // playlist is.
        let loops: HashMap<PathBuf, u32> = stage
            .renderers
            .iter()
            .flat_map(|r| r.loop_counts())
            .collect::<HashMap<_, _>>();

        let due: Vec<String> = playback
            .iter()
            .filter(|(_, state)| state.enabled && state.items.len() > 1)
            .filter(|(_, state)| {
                if settings.interval_secs > 0 {
                    state.started.elapsed().as_secs() >= settings.interval_secs
                } else {
                    state
                        .current()
                        .and_then(|path| loops.get(path))
                        .is_some_and(|count| *count > state.loops_at_start)
                }
            })
            .map(|(name, _)| name.clone())
            .collect();

        for monitor in due {
            advance(&mut stage.renderers, &mut playback, &monitor)?;
            changed = true;
        }

        // What the machine is running on, and what that costs the wallpaper.
        let (power_state, power_changed) = power.poll();
        if power_changed {
            changed = true;
            println!(
                "power: {}{}",
                if power_state.on_battery {
                    "battery"
                } else {
                    "AC"
                },
                if power_state.saver { ", saver on" } else { "" }
            );
        }

        // Frozen means the last frame stays where it is: no decode, no draw,
        // no flip. The surfaces stay up, so what the user sees is the
        // wallpaper they chose, standing still.
        let frozen = settings.frozen || settings.power.should_freeze(power_state);

        let elapsed = start.elapsed();
        let mut live = 0;
        let mut presented = 0;
        if !frozen {
            for renderer in &mut stage.renderers {
                let pass = renderer.draw(elapsed)?;
                live += pass.live;
                presented += pass.presented;
            }
        }

        // A file that would not open does not stop anything; it is a message
        // the user needs, and the only place they will look is the UI.
        for renderer in &mut stage.renderers {
            for message in renderer.take_errors() {
                last_error = Some(message);
                changed = true;
            }
        }

        // Every monitor is covered — by a fullscreen game, a maximised
        // window, or a locked screen. Rendering into it would be work no one
        // can see, which is precisely what this project refuses to spend.
        let paused = frozen || live == 0;
        if paused != was_paused {
            println!(
                "{}",
                if paused {
                    "paused: nothing to draw"
                } else {
                    "resumed: desktop visible again"
                }
            );
            was_paused = paused;
            changed = true;
        }

        stats.drew(presented);
        if stats.sample() {
            changed = true;
        }

        // The soundtrack follows the primary monitor, starts when sound is
        // switched on, and goes quiet the moment nothing is visible — a
        // wallpaper hidden behind a fullscreen game should not be heard
        // either.
        let wanted_track = stage
            .primary
            .as_ref()
            .filter(|_| settings.sound.enabled)
            .and_then(|name| playback.get(name.as_str()))
            .filter(|state| state.enabled)
            .and_then(|state| state.current())
            // A photo has no soundtrack, and starting a thread to discover
            // that is a thread for nothing.
            .filter(|path| !crate::decoder::is_still(path))
            .cloned();

        if audio.as_ref().map(|a| a.path()) != wanted_track.as_deref() {
            // Dropping the old one stops it; there is no crossfade to get
            // wrong because two wallpapers never overlap.
            audio = wanted_track
                .as_deref()
                .map(|path| Audio::play(path, settings.sound.volume, paused, settings.sound.duck));
            changed = true;
        }
        let ducking = match &audio {
            Some(playing) => {
                playing.set_muted(paused);
                playing.is_ducking()
            }
            None => false,
        };

        if changed {
            publish(
                &status,
                &Report {
                    settings: &settings,
                    paused,
                    power: power_state,
                    ducking,
                    stats: &stats,
                    error: last_error.as_deref(),
                    playback: &playback,
                },
            );
        }

        // Two limits decide when to wake up, and the later of them wins.
        //
        // The first is the target fps: never more often than the tier allows,
        // and never more often than the power source allows either. The
        // second is the video itself — the moment its next decoded frame is
        // due. Waking on a grid of our own instead means a 24 fps clip has
        // some frames held for one tick and some for two, which is exactly
        // what a viewer reads as stutter even though no frame was lost.
        let fps = settings.power.effective_fps(settings.fps, power_state);
        let frame_time = Duration::from_secs_f64(1.0 / fps as f64);
        let budget = if paused { OCCLUDED_POLL } else { frame_time };
        let now = Instant::now();
        next_tick += budget;
        if next_tick < now {
            // Fell behind by more than a whole frame — a suspend, a slow
            // decode, a display change. Start the grid again from here rather
            // than trying to catch up on frames nobody saw.
            next_tick = now + budget;
        }

        let deadline = if frozen {
            // Nothing is moving, so the only reason to wake is to hear from
            // the UI or the keyboard.
            now + OCCLUDED_POLL
        } else {
            stage
                .renderers
                .iter()
                .filter_map(|renderer| renderer.time_to_next())
                .min()
                .map_or(next_tick, |due| next_tick.max(now + due))
        };

        // However long the wallpaper says it can wait, the loop still has to
        // notice a command from the UI. A still image asks for an hour; the
        // user asking for a different one has to be answered sooner than
        // that, and a quarter of a second of idling costs nothing.
        let deadline = deadline.min(now + MAX_WAIT);

        pacer.wait_until(deadline);
    }

    // Dropping the renderers destroys the windows. Windows does not repaint
    // the wallpaper underneath on its own, so ask it to.
    drop(stage);
    restore_desktop();

    Ok(())
}

/// Where the wallpaper goes: the window Explorer paints the desktop into.
fn find_parent() -> windows::core::Result<HWND> {
    let target = workerw::find().ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "no WorkerW and no Progman: is Explorer running?",
        )
    })?;
    println!("parent: {}", target.how);
    Ok(target.hwnd)
}

/// The smallest rectangle containing every monitor.
///
/// Not the same as the primary monitor's size, and not the same as any one
/// screen: a wallpaper spanning three displays is cut out of this.
fn desktop_bounds(profile: &GpuProfile) -> Rect {
    let monitors: Vec<_> = profile
        .adapters
        .iter()
        .flat_map(|adapter| &adapter.outputs)
        .collect();

    let Some(first) = monitors.first() else {
        return Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
    };

    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x + first.width as i32;
    let mut bottom = first.y + first.height as i32;

    for monitor in &monitors[1..] {
        left = left.min(monitor.x);
        top = top.min(monitor.y);
        right = right.max(monitor.x + monitor.width as i32);
        bottom = bottom.max(monitor.y + monitor.height as i32);
    }

    Rect {
        x: left,
        y: top,
        width: (right - left).max(0) as u32,
        height: (bottom - top).max(0) as u32,
    }
}

fn primary_monitor(profile: &GpuProfile) -> Option<String> {
    profile
        .adapters
        .iter()
        .flat_map(|adapter| &adapter.outputs)
        .find(|monitor| monitor.primary)
        .or_else(|| {
            profile
                .adapters
                .iter()
                .flat_map(|adapter| &adapter.outputs)
                .next()
        })
        .map(|monitor| monitor.device_name.clone())
}

/// The displays as a comparable list, so a broadcast that changed nothing can
/// be told from one that changed everything. Sorted, because the enumeration
/// order is the driver's business and it is not promised to be stable.
fn layout_of(profile: &GpuProfile) -> Vec<(String, i32, i32, u32, u32)> {
    let mut layout: Vec<_> = profile
        .adapters
        .iter()
        .flat_map(|adapter| &adapter.outputs)
        .map(|monitor| {
            (
                monitor.device_name.clone(),
                monitor.x,
                monitor.y,
                monitor.width,
                monitor.height,
            )
        })
        .collect();
    layout.sort();
    layout
}

/// Move one monitor to the next item in its playlist.
fn advance(
    renderers: &mut [Renderer],
    playback: &mut HashMap<String, Playback>,
    monitor: &str,
) -> windows::core::Result<()> {
    let Some(state) = playback.get_mut(monitor) else {
        return Ok(());
    };
    if state.items.is_empty() {
        return Ok(());
    }

    state.index = (state.index + 1) % state.items.len();
    state.started = Instant::now();

    let current = state.current().cloned();
    // The next clip may already be open on another monitor, in which case it
    // has looped a few times already; its count is the baseline, not zero.
    state.loops_at_start = current
        .as_ref()
        .and_then(|path| {
            renderers
                .iter()
                .flat_map(|r| r.loop_counts())
                .find(|(open, _)| open == path)
                .map(|(_, count)| count)
        })
        .unwrap_or(0);

    apply(renderers, monitor, current.as_ref())
}

/// Route a monitor's wallpaper to whichever adapter drives it.
fn apply(
    renderers: &mut [Renderer],
    monitor: &str,
    video: Option<&PathBuf>,
) -> windows::core::Result<()> {
    for renderer in renderers.iter_mut() {
        if renderer.has_monitor(monitor) {
            renderer.set_video(monitor, video.map(|p| p.as_path()))?;
        }
    }
    Ok(())
}

/// Everything the UI is told in one pass.
///
/// A struct rather than a dozen arguments: the list only grows, and a caller
/// that has to count positional booleans will eventually get two of them the
/// wrong way round.
struct Report<'a> {
    settings: &'a Settings,
    paused: bool,
    power: PowerState,
    /// Whether the soundtrack is standing down for another application.
    ducking: bool,
    stats: &'a stats::Stats,
    error: Option<&'a str>,
    playback: &'a HashMap<String, Playback>,
}

fn publish(status: &Arc<Mutex<Status>>, report: &Report) {
    let settings = report.settings;
    let mut status = status.lock().expect("status mutex poisoned");
    status.fps = settings.fps;
    status.fit = settings.fit.name().to_string();
    status.interval_secs = settings.interval_secs;
    status.paused = report.paused;
    status.frozen = settings.frozen;
    status.brightness = settings.visual.brightness;
    status.saturation = settings.visual.saturation;
    status.blur = settings.visual.blur;
    status.sound = settings.sound.enabled;
    status.volume = settings.sound.volume;
    status.duck = settings.sound.duck;
    status.ducking = report.ducking;
    status.speed = settings.speed;
    status.fade_ms = settings.fade.as_millis() as u64;
    status.span = settings.span;
    status.hotkeys = settings.hotkeys;
    status.battery_fps = settings.power.battery_fps;
    status.pause_on_saver = settings.power.pause_on_saver;
    status.on_battery = report.power.on_battery;
    status.saver = report.power.saver;
    status.battery_percent = report.power.percent;
    status.cpu = report.stats.cpu;
    status.ram_mb = report.stats.ram_mb;
    status.real_fps = report.stats.fps;
    status.error = report.error.map(str::to_string);
    status.monitors = report
        .playback
        .iter()
        .map(|(name, state)| MonitorState {
            device_name: name.clone(),
            enabled: state.enabled,
            index: state.index,
            items: state.items.clone(),
            overrides: settings.overrides.get(name).copied().unwrap_or_default(),
        })
        .collect();
    // A map has no order; the UI shows these as a list and a list that
    // reshuffles itself between polls is unusable.
    status
        .monitors
        .sort_by(|a, b| a.device_name.cmp(&b.device_name));
}

fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, Some(HWND::default()), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Re-apply the user's own wallpaper, so quitting leaves the desktop as it
/// was found.
fn restore_desktop() {
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            None,
            // No UPDATEINIFILE: we are not changing the user's setting, only
            // asking the shell to redraw what it already has.
            SPIF_SENDCHANGE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{
        AdapterClass, AdapterInfo, DecodeCaps, MonitorInfo, Recommendation, SystemInfo, Tier,
    };

    fn monitor(name: &str, x: i32, y: i32, width: u32, height: u32) -> MonitorInfo {
        MonitorInfo {
            device_name: name.to_string(),
            x,
            y,
            width,
            height,
            refresh_hz: 60,
            primary: x == 0 && y == 0,
        }
    }

    /// A profile that exists only to carry a monitor layout. Nothing here
    /// touches hardware, which is the point: the geometry these tests cover
    /// is the part that can be wrong on a machine nobody has.
    fn profile_with(outputs: Vec<MonitorInfo>) -> GpuProfile {
        GpuProfile {
            adapters: vec![AdapterInfo {
                luid: 1,
                vendor_id: 0,
                device_id: 0,
                name: "test".to_string(),
                dedicated_vram: 0,
                shared_mem: 0,
                class: AdapterClass::Integrated,
                feature_level: 0xB000,
                decode: DecodeCaps::default(),
                outputs,
            }],
            system: SystemInfo {
                total_ram_mb: 8192,
                on_battery: false,
            },
            rec: Recommendation {
                tier: Tier::Mid,
                target_fps: 30,
                max_scale: (1920, 1080),
                allow_distinct_videos: true,
                reason: String::new(),
            },
        }
    }

    #[test]
    fn one_monitor_is_its_own_desktop() {
        let profile = profile_with(vec![monitor("A", 0, 0, 1920, 1080)]);
        assert_eq!(
            desktop_bounds(&profile),
            Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn two_side_by_side_span_both() {
        let profile = profile_with(vec![
            monitor("A", 0, 0, 1920, 1080),
            monitor("B", 1920, 0, 1920, 1080),
        ]);
        assert_eq!(desktop_bounds(&profile).width, 3840);
        assert_eq!(desktop_bounds(&profile).height, 1080);
    }

    /// The case that catches sloppy bounds maths: a screen to the *left* of
    /// the primary one has a negative x, and a width computed as
    /// "rightmost + its width" would be far too small.
    #[test]
    fn a_monitor_left_of_the_primary_still_counts() {
        let profile = profile_with(vec![
            monitor("A", 0, 0, 1920, 1080),
            monitor("B", -1280, 0, 1280, 1024),
        ]);
        let bounds = desktop_bounds(&profile);
        assert_eq!(bounds.x, -1280);
        assert_eq!(bounds.width, 3200);
        assert_eq!(bounds.height, 1080);
    }

    #[test]
    fn a_stacked_pair_is_taller_than_either() {
        let profile = profile_with(vec![
            monitor("A", 0, 0, 1920, 1080),
            monitor("B", 0, -1080, 1920, 1080),
        ]);
        let bounds = desktop_bounds(&profile);
        assert_eq!(bounds.y, -1080);
        assert_eq!(bounds.height, 2160);
    }

    #[test]
    fn no_monitors_is_an_empty_desktop() {
        let bounds = desktop_bounds(&profile_with(Vec::new()));
        assert_eq!(bounds.width, 0);
        assert_eq!(bounds.height, 0);
    }

    #[test]
    fn the_layout_ignores_enumeration_order() {
        let one = profile_with(vec![
            monitor("A", 0, 0, 1920, 1080),
            monitor("B", 1920, 0, 1920, 1080),
        ]);
        let other = profile_with(vec![
            monitor("B", 1920, 0, 1920, 1080),
            monitor("A", 0, 0, 1920, 1080),
        ]);
        assert_eq!(layout_of(&one), layout_of(&other));
    }

    #[test]
    fn a_resolution_change_is_a_layout_change() {
        let before = profile_with(vec![monitor("A", 0, 0, 1920, 1080)]);
        let after = profile_with(vec![monitor("A", 0, 0, 2560, 1440)]);
        assert_ne!(layout_of(&before), layout_of(&after));
    }
}
