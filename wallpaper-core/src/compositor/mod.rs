//! Puts pixels on the desktop background.

mod accent;
mod clock;
mod diag;
mod notify;
mod order;
mod render;
mod rules;
mod scenes;
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

use crate::audio::{Audio, Meter, Spectrum};
use crate::caps::GpuProfile;
use crate::ipc::{Command, MonitorState, Status};
use crate::power::apps::AppWatch;
use crate::power::battery::{PowerPolicy, PowerState, PowerWatch};
use crate::power::idle;
use crate::power::load::LoadWatch as PowerLoad;
use crate::session::{self, Session};
use render::Renderer;
use window::Surface;

pub use diag::dump as dump_window_tree;
pub use render::{Drive, Fit, Motion, Overrides, Rect, Visual};
// `Trigger` names the field of a `Rule`; nothing outside this module builds
// one by hand yet, and it is re-exported so that stays possible.
#[allow(unused_imports)]
pub use rules::{parse as parse_rules, written_form as write_rules, Rule, Trigger};
pub use scenes::{
    parse as parse_scene, valid_name as valid_scene_name, written_form as write_scene, Scene,
};

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
    /// Which item of `items` each step plays. `0..n` while shuffle is off,
    /// a permutation of it while shuffle is on — see `compositor::order`.
    /// Kept beside the list rather than applied to it so that turning
    /// shuffle off puts the playlist back in the order the user wrote.
    order: Vec<usize>,
    /// Where in `order` playback has reached, not where in `items`.
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
        self.items.get(*self.order.get(self.index)?)
    }

    /// Which item of the list is on screen, for the UI and for a shuffle
    /// that has to know what not to start the next pass with.
    fn current_index(&self) -> usize {
        self.order.get(self.index).copied().unwrap_or(0)
    }

    /// Point this monitor at a new list, from the top.
    fn play(&mut self, items: Vec<PathBuf>, shuffle: bool) {
        self.order = if shuffle {
            order::shuffled(items.len(), None, order::seed())
        } else {
            order::straight(items.len())
        };
        self.items = items;
        self.index = 0;
        self.started = Instant::now();
        self.loops_at_start = 0;
    }

    /// Draw the order again, or put it back in the order it was written.
    /// The item on screen keeps playing; only what comes after it changes.
    fn reorder(&mut self, shuffle: bool) {
        let playing = self.current_index();
        self.order = if shuffle {
            order::shuffled(self.items.len(), None, order::seed())
        } else {
            order::straight(self.items.len())
        };
        // Carried over so the wallpaper on screen is not restarted by a
        // setting that only decides what plays next.
        self.index = self
            .order
            .iter()
            .position(|item| *item == playing)
            .unwrap_or(0);
    }

    /// Move to the next item, drawing a new order at the end of a pass.
    ///
    /// Split out from `advance` so it can be tested: everything else that
    /// function does needs a GPU, and this is the part with the arithmetic
    /// in it.
    fn step(&mut self, shuffle: bool) {
        if self.order.is_empty() {
            return;
        }

        let finished = self.current_index();
        self.index += 1;
        if self.index >= self.order.len() {
            // The end of one pass through the list. Shuffled, that is where
            // the next order is drawn — told which item just played, so the
            // new pass does not open with the one just seen.
            self.index = 0;
            if shuffle {
                self.order = order::shuffled(self.items.len(), Some(finished), order::seed());
            }
        }
        // The loop count this item is measured against is the caller's to
        // set: it has the decoders, and the next clip may already be open on
        // another monitor with a count of its own. See `advance`.
        self.started = Instant::now();
    }

    fn blank() -> Self {
        Self {
            items: Vec::new(),
            order: Vec::new(),
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
    /// Whether a playlist plays in a drawn order rather than the one it was
    /// written in. See `compositor::order`.
    pub shuffle: bool,
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
    /// How long the desktop must stay out of sight before the decoders are
    /// handed back. Zero never hands them back. See `Renderer::set_idle`.
    pub hibernate_secs: u64,
    /// How far the wallpaper answers to sound and to the cursor.
    pub motion: Motion,
    /// A memory budget in megabytes, which becomes a cap on the frame size
    /// a decoder is asked for. Zero leaves the tier's own cap in place.
    pub memory_mb: u32,
    /// How long the machine may sit untouched before the wallpaper stands
    /// still. Zero never does. See `power::idle`.
    pub idle_secs: u64,
    /// The frame rate to fall to while the machine is busy with something
    /// else. Zero keeps one rate whatever else is running.
    pub busy_fps: u32,
    /// Whether to stop moving when Windows has been told to show fewer
    /// animations. On by default: somebody who turned that off said so about
    /// their whole desktop, and this is the largest moving thing on it.
    pub reduce_motion: bool,
    /// How far a photograph drifts on its own, 0-1. Zero leaves it still.
    pub drift: f32,
    /// Whether the Windows accent colour follows the wallpaper.
    pub accent: bool,
    /// Applications that freeze the wallpaper while they are in front.
    pub apps: Vec<String>,
    /// Wallpapers that change themselves, by clock or by theme.
    pub rules: Vec<Rule>,
    /// Named arrangements of wallpapers across the screens.
    pub scenes: Vec<Scene>,
    /// Where each shader file's own settings are set, keyed by path and then
    /// by the name the file declared.
    pub shader_params: HashMap<PathBuf, HashMap<String, f32>>,
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
            // Off: a playlist plays in the order the user put it in until
            // they say otherwise, and an order nobody asked to be scrambled
            // reads as a fault rather than as a feature.
            shuffle: false,
            visual: Visual::default(),
            sound: Sound::default(),
            power: PowerPolicy::default(),
            speed: 1.0,
            fade: Duration::from_millis(400),
            span: false,
            hotkeys: true,
            // Twenty seconds is past an alt-tab and well inside a loading
            // screen, so the common case is that a game gets the memory back
            // and the user never sees the reopen.
            hibernate_secs: 20,
            motion: Motion::default(),
            memory_mb: 0,
            // Five minutes is past a coffee and well short of a lunch. The
            // desk being empty is the one case where the wallpaper costs a
            // full frame rate for nobody, and coverage never catches it.
            idle_secs: 300,
            // Ten frames a second while something else needs the machine.
            // Still moving, at a third of the cost, and the machines this
            // project is for are the ones where the third matters.
            busy_fps: 10,
            reduce_motion: true,
            drift: 0.0,
            accent: false,
            apps: Vec::new(),
            rules: Vec::new(),
            scenes: Vec::new(),
            shader_params: HashMap::new(),
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
        if let Some(shuffle) = session.shuffle {
            settings.shuffle = shuffle;
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
        if let Some(secs) = session.hibernate_secs {
            settings.hibernate_secs = secs;
        }
        if let Some(motion) = session.motion {
            settings.motion = motion;
        }
        if let Some(mb) = session.memory_mb {
            settings.memory_mb = mb;
        }
        if let Some(secs) = session.idle_secs {
            settings.idle_secs = secs;
        }
        if let Some(fps) = session.busy_fps {
            settings.busy_fps = fps;
        }
        if let Some(on) = session.reduce_motion {
            settings.reduce_motion = on;
        }
        if let Some(drift) = session.drift {
            settings.drift = drift;
        }
        if let Some(accent) = session.accent {
            settings.accent = accent;
        }
        settings.apps = session.apps.clone();
        settings.rules = session.rules.clone();
        settings.scenes = session.scenes.clone();
        settings.shader_params = session.shader_params.clone();
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
    /// The frame size cap this machine's tier chose, before the user's own
    /// memory budget is applied on top of it.
    tier_scale: (u32, u32),
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
            tier_scale: profile.rec.max_scale,
        })
    }

    /// Push every setting into the renderers. Called after a build and after
    /// any change that all of them share.
    fn apply(&mut self, settings: &Settings) {
        let span = settings.span.then_some(self.desktop);
        let max_scale = crate::caps::capped(self.tier_scale, settings.memory_mb);
        for renderer in &mut self.renderers {
            renderer.set_max_scale(max_scale);
            renderer.set_motion(settings.motion);
            renderer.set_fit(settings.fit);
            renderer.set_visual(settings.visual);
            renderer.set_speed(settings.speed);
            renderer.set_fade(settings.fade);
            renderer.set_span(span);
            renderer.set_drift(settings.drift);
            for (path, values) in &settings.shader_params {
                renderer.set_shader_params(path, values.clone());
            }
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

            let mut state = Playback {
                enabled: saved.map(|(_, enabled, _)| *enabled).unwrap_or(true),
                ..Playback::blank()
            };
            state.play(
                match saved {
                    Some((_, _, items)) => items.clone(),
                    None => initial.iter().cloned().collect(),
                },
                settings.shuffle,
            );
            playback.insert(monitor.device_name.clone(), state);
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
            hibernating: false,
            power: power.state(),
            ducking: false,
            away: false,
            busy: false,
            load: 0.0,
            stats: &stats,
            error: None,
            playback: &playback,
            shaders: Vec::new(),
        },
    );

    // How often to check whether a covered desktop has become visible again.
    // Half a second is imperceptible to a user alt-tabbing out of a game, and
    // it is 15 wakeups saved for every one spent at 30 fps.
    const OCCLUDED_POLL: Duration = Duration::from_millis(500);

    // The longest the loop will ever idle, whatever the wallpaper says it
    // needs. This is what keeps the UI feeling attached to the engine.
    const MAX_WAIT: Duration = Duration::from_millis(250);

    // How often the automation rules are consulted. See where it is used.
    const RULES_POLL: Duration = Duration::from_secs(10);

    // How far the wallpaper's average colour has to move before the accent
    // colour follows it. A video's average wanders by a point or two every
    // frame, and following that would be a registry write per frame for a
    // colour nobody could see change.
    const ACCENT_STEP: u8 = 12;

    let start = Instant::now();
    let mut was_paused = false;
    let mut last_error: Option<String> = None;
    // When the desktop was last seen, and whether the decoders have already
    // been handed back because it has been out of sight since.
    let mut hidden_since: Option<Instant> = None;
    let mut hibernating = false;
    // The output meter, opened only once something asks for it. A user with
    // the effect switched off never touches the audio stack at all.
    let mut meter: Option<Meter> = None;
    let mut drive = Drive::default();
    let mut apps = AppWatch::default();
    // How busy the machine is, and whether the wallpaper is standing down
    // for it. Sampled once a second; see `power::load`.
    let mut load = PowerLoad::default();
    let mut busy = false;
    // The loopback capture, opened only while something on screen reads the
    // sound split into bands. Almost always `None`.
    let mut spectrum: Option<Spectrum> = None;
    // The colour last put on the desktop's chrome, so the registry is written
    // when the wallpaper changes rather than on every pass.
    let mut accented: Option<[u8; 3]> = None;
    // Whether the wallpaper is standing still because nobody is at the
    // machine, and whether Windows has asked for less motion.
    let mut away = false;
    let mut still_wanted = false;

    // An engine that was killed rather than closed leaves the user's own
    // accent colours overwritten. Whatever the setting says now, the backup
    // on disk is somebody's colour scheme waiting to be handed back.
    if !settings.accent && accent::is_applied() {
        println!("accent: restoring the colours from a previous run");
        accent::restore();
    }
    // Which rule is on screen, and when the rules were last consulted.
    let mut ruled: Option<Vec<PathBuf>> = None;
    let mut rules_checked: Option<Instant> = None;

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
                        entry.play(items.clone(), settings.shuffle);

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
                    advance(
                        &mut stage.renderers,
                        &mut playback,
                        &monitor,
                        settings.shuffle,
                    )?;
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

                Command::SetShuffle(on) => {
                    persist = true;
                    settings.shuffle = on;
                    // What is on screen stays there; only what comes after
                    // it changes. Redrawing the order is not a reason to
                    // interrupt a wallpaper the user is looking at.
                    for state in playback.values_mut() {
                        state.reorder(on);
                    }
                    println!("shuffle: {}", if on { "drawn" } else { "as written" });
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
                            entry.play(source.clone(), settings.shuffle);
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

                Command::SetMotion(next) => {
                    persist = true;
                    settings.motion = next;
                    stage.apply(&settings);
                    println!(
                        "motion: reactive {:.2}, parallax {:.2}",
                        next.reactive, next.parallax
                    );
                }

                Command::SetMemory(mb) => {
                    persist = true;
                    settings.memory_mb = mb;
                    // Applied through the stage, which reopens whatever is
                    // playing at the new size. The restart is visible, and
                    // it is the honest cost of changing the budget.
                    stage.apply(&settings);
                    let scale = crate::caps::capped(stage.tier_scale, mb);
                    println!(
                        "memory: {} -> frames capped at {}x{}",
                        if mb == 0 {
                            "no budget".to_string()
                        } else {
                            format!("{mb} MB")
                        },
                        scale.0,
                        scale.1
                    );
                }

                Command::SetApps(names) => {
                    persist = true;
                    settings.apps = names;
                    println!(
                        "apps: {}",
                        if settings.apps.is_empty() {
                            "none".to_string()
                        } else {
                            settings.apps.join(", ")
                        }
                    );
                }

                Command::SetRules(next) => {
                    persist = true;
                    settings.rules = next;
                    // Checked on the next pass rather than here: the rule
                    // that applies right now is the same question the loop
                    // asks anyway, and asking it twice is two places to get
                    // it wrong.
                    ruled = None;
                    rules_checked = None;
                    println!("rules: {}", settings.rules.len());
                }

                Command::SetHibernate(secs) => {
                    persist = true;
                    settings.hibernate_secs = secs;
                    println!(
                        "hibernate: {}",
                        if secs == 0 {
                            "never".to_string()
                        } else {
                            format!("after {secs}s out of sight")
                        }
                    );
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

                Command::SetIdle(secs) => {
                    persist = true;
                    settings.idle_secs = secs;
                    println!(
                        "idle: {}",
                        if secs == 0 {
                            "never stands still".to_string()
                        } else {
                            format!("still after {secs}s untouched")
                        }
                    );
                }

                Command::SetBusyFps(fps) => {
                    persist = true;
                    settings.busy_fps = fps;
                    println!(
                        "busy: {}",
                        if fps == 0 {
                            "one rate whatever else runs".to_string()
                        } else {
                            format!("{fps} fps while the machine is busy")
                        }
                    );
                }

                Command::SetReduceMotion(on) => {
                    persist = true;
                    settings.reduce_motion = on;
                    println!(
                        "reduce motion: {}",
                        if on { "respected" } else { "ignored" }
                    );
                }

                Command::SetDrift(drift) => {
                    persist = true;
                    settings.drift = drift;
                    stage.apply(&settings);
                    println!("drift: {:.0}%", drift * 100.0);
                }

                Command::SetAccent(on) => {
                    persist = true;
                    settings.accent = on;
                    // Switching it off puts the user's own colours back at
                    // once rather than at the next restart.
                    if on {
                        accented = None;
                    } else {
                        accent::restore();
                        accented = None;
                    }
                    println!(
                        "accent: {}",
                        if on { "follows the wallpaper" } else { "off" }
                    );
                }

                Command::SetShaderParams { path, values } => {
                    persist = true;
                    settings
                        .shader_params
                        .entry(path.clone())
                        .or_default()
                        .extend(values.iter().map(|(k, v)| (k.clone(), *v)));
                    for renderer in &mut stage.renderers {
                        renderer.set_shader_params(&path, values.clone());
                    }
                    println!("shader: {} setting(s) for {}", values.len(), path.display());
                }

                Command::SaveScene(name) => {
                    let scene = Scene {
                        name: name.clone(),
                        monitors: playback
                            .iter()
                            .map(|(monitor, state)| (monitor.clone(), state.items.clone()))
                            .collect(),
                    };
                    if scenes::store(&mut settings.scenes, scene) {
                        persist = true;
                        println!("scene: saved {name}");
                    } else {
                        last_error = Some(format!("cannot save the scene {name}"));
                    }
                }

                Command::LoadScene(name) => {
                    let wanted =
                        scenes::find(&settings.scenes, &name).map(|scene| scene.monitors.clone());
                    match wanted {
                        Some(monitors) => {
                            persist = true;
                            for (monitor, items) in monitors {
                                let entry = playback
                                    .entry(monitor.clone())
                                    .or_insert_with(Playback::blank);
                                entry.play(items, settings.shuffle);
                                let current = entry.current().cloned();
                                apply(&mut stage.renderers, &monitor, current.as_ref())?;
                            }
                            last_error = None;
                            println!("scene: {name}");
                        }
                        None => last_error = Some(format!("no scene called {name}")),
                    }
                }

                Command::DeleteScene(name) => {
                    if scenes::remove(&mut settings.scenes, &name) {
                        persist = true;
                        println!("scene: removed {name}");
                    }
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
                advance(
                    &mut stage.renderers,
                    &mut playback,
                    &monitor,
                    settings.shuffle,
                )?;
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
                shuffle: Some(settings.shuffle),
                visual: Some(settings.visual),
                sound: Some(settings.sound),
                power: Some(settings.power),
                speed: Some(settings.speed),
                fade: Some(settings.fade),
                span: Some(settings.span),
                hotkeys: Some(settings.hotkeys),
                hibernate_secs: Some(settings.hibernate_secs),
                motion: Some(settings.motion),
                memory_mb: Some(settings.memory_mb),
                idle_secs: Some(settings.idle_secs),
                busy_fps: Some(settings.busy_fps),
                reduce_motion: Some(settings.reduce_motion),
                drift: Some(settings.drift),
                accent: Some(settings.accent),
                apps: settings.apps.clone(),
                rules: settings.rules.clone(),
                scenes: settings.scenes.clone(),
                shader_params: settings.shader_params.clone(),
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
        let playlists = playback
            .values()
            .any(|state| state.enabled && state.items.len() > 1);

        // Asked for only when something could actually advance on it. The
        // answer is a map and a `PathBuf` clone per open wallpaper, built on
        // every pass of the loop — thirty times a second while a wallpaper is
        // playing. For the user with one fixed wallpaper, or a playlist on a
        // clock, every one of those was a heap allocation for a number
        // nothing goes on to read.
        let loops: HashMap<PathBuf, u32> = if playlists && settings.interval_secs == 0 {
            stage
                .renderers
                .iter()
                .flat_map(|r| r.loop_counts())
                .collect()
        } else {
            HashMap::new()
        };

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
            advance(
                &mut stage.renderers,
                &mut playback,
                &monitor,
                settings.shuffle,
            )?;
            changed = true;
        }

        // Wallpapers that change themselves. Ten seconds is well inside a
        // minute, which is the finest a rule can be set to, and it is 600
        // registry reads an hour rather than 200,000.
        let rules_due = !settings.rules.is_empty()
            && rules_checked.is_none_or(|last| last.elapsed() >= RULES_POLL);
        if rules_due {
            rules_checked = Some(Instant::now());
            let now = rules::now_minutes();
            let dark = rules::dark_theme();

            if let Some(rule) = rules::choose(&settings.rules, now, dark) {
                if ruled.as_ref() != Some(&rule.items) {
                    ruled = Some(rule.items.clone());
                    changed = true;
                    // Not persisted. What is on screen because of a rule is
                    // the rule's to decide, and the rules themselves are
                    // saved — so a restart works this out again within the
                    // first ten seconds rather than restoring a stale
                    // answer and then correcting itself.
                    println!("rule: {} item(s) now showing", rule.items.len());

                    let screens: Vec<String> = playback.keys().cloned().collect();
                    for name in screens {
                        let entry = playback.entry(name.clone()).or_insert_with(Playback::blank);
                        entry.play(rule.items.clone(), settings.shuffle);
                        let current = entry.current().cloned();
                        apply(&mut stage.renderers, &name, current.as_ref())?;
                    }
                }
            }
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
        // An application the user named is in front. Treated exactly like a
        // hand-frozen wallpaper — the last frame stays, nothing is decoded
        // — because that is what the user asked for when they named it.
        let app_freeze = apps.matches(&settings.apps);

        // Nobody has touched the machine for long enough that the desktop is
        // being looked at by no one. Coverage cannot catch this — nothing is
        // covering anything — and it is the one case where a visible
        // wallpaper costs a full frame rate for an empty chair.
        let now_away = idle::away(settings.idle_secs);
        if now_away != away {
            away = now_away;
            changed = true;
            println!(
                "{}",
                if away {
                    "still: nobody at the machine"
                } else {
                    "moving: somebody came back"
                }
            );
        }

        // Windows has been told to show fewer animations. Somebody who chose
        // that chose it for their whole desktop.
        let now_still = settings.reduce_motion && !idle::animations_wanted();
        if now_still != still_wanted {
            still_wanted = now_still;
            changed = true;
            println!(
                "{}",
                if still_wanted {
                    "still: Windows is set to reduce motion"
                } else {
                    "moving: Windows allows animation again"
                }
            );
        }

        let frozen = settings.frozen
            || app_freeze
            || away
            || still_wanted
            || settings.power.should_freeze(power_state);

        // How busy the rest of the machine is. Read even while frozen, so
        // the wallpaper is already at the right rate when it starts again.
        let (now_busy, busy_changed) = load.poll();
        if busy_changed {
            busy = now_busy;
            changed = true;
            println!(
                "load: {} ({:.0}% of the machine)",
                if busy {
                    "standing down"
                } else {
                    "back to the usual rate"
                },
                load.percent()
            );
        }

        // A shader that draws the sound needs the sound itself, not a level.
        // The capture is opened for as long as one is on screen and given
        // back the moment it leaves — see `audio::spectrum`.
        let wants_bands = !frozen && stage.renderers.iter().any(|r| r.wants_bands());
        if wants_bands {
            if spectrum.is_none() {
                match Spectrum::new() {
                    Ok(opened) => {
                        println!("spectrum: listening for a shader that draws it");
                        spectrum = Some(opened)
                    }
                    // A wallpaper that cannot hear is one with flat bars, not
                    // one that stops.
                    Err(e) => eprintln!("spectrum: {}", e.message()),
                }
            }
            if let Some(capture) = &mut spectrum {
                drive.bands = capture.read();
                // The output device changed underneath us: the old endpoint
                // stays open and silent forever otherwise.
                if capture.stale() {
                    spectrum = None;
                }
            }
        } else if spectrum.is_some() {
            spectrum = None;
            drive.bands = [0.0; crate::audio::BANDS];
        }

        // A shader is handed the cursor directly and may use `iMouse`
        // whether or not the parallax effect is on, so the cursor is read
        // for one even when nothing else wants it. One call a frame, and
        // only while a shader is on a screen.
        let shader_on_screen = !frozen && stage.renderers.iter().any(|r| r.has_shader());
        if shader_on_screen || wants_bands {
            if shader_on_screen {
                drive.cursor = cursor_position(stage.desktop);
            }
            // A shader reads the drive directly, so the numbers are pushed
            // even where the fit-window effects are switched off.
            for renderer in &mut stage.renderers {
                renderer.set_drive(drive);
            }
        }

        // What the wallpaper is answering to this frame. Measured only when
        // something asked for it, and not at all while nothing is moving.
        if settings.motion != Motion::default() && !frozen {
            if settings.motion.reactive > 0.0 {
                if meter.is_none() {
                    match Meter::new() {
                        Ok(opened) => meter = Some(opened),
                        // No meter is a wallpaper that does not pulse, not a
                        // wallpaper that stops.
                        Err(e) => eprintln!("meter: {}", e.message()),
                    }
                }
                drive.level = meter.as_mut().map(Meter::read).unwrap_or(0.0);
            } else {
                drive.level = 0.0;
            }

            // Left where the block above put it when a shader is on screen:
            // that one reads the cursor for `iMouse`, which is not the same
            // question as whether the parallax effect is switched on.
            drive.cursor = if settings.motion.parallax > 0.0 {
                cursor_position(stage.desktop)
            } else if shader_on_screen {
                drive.cursor
            } else {
                (0.0, 0.0)
            };

            for renderer in &mut stage.renderers {
                renderer.set_drive(drive);
            }
        } else if meter.is_some() {
            // Switched off, or frozen: give the endpoint back rather than
            // holding a COM object for an effect nobody is watching. The
            // bands are left alone — they belong to the capture above, which
            // is a different reading with a different owner.
            meter = None;
            drive.level = 0.0;
            if !shader_on_screen {
                drive.cursor = (0.0, 0.0);
            }
            for renderer in &mut stage.renderers {
                renderer.set_drive(drive);
            }
        }

        // A frozen wallpaper has to be put on screen before it can stand
        // still on it. Every reason to freeze can be true before the first
        // frame was ever drawn — Windows set to reduce motion is true from
        // the moment the engine starts — and a freeze that skipped that
        // frame left the desktop on the Windows wallpaper for as long as the
        // reason lasted, with nothing anywhere saying why.
        let owed = stage.renderers.iter().any(|r| r.owes_a_frame());

        let elapsed = start.elapsed();
        let mut live = 0;
        let mut presented = 0;
        if !frozen || owed {
            for renderer in &mut stage.renderers {
                let pass = renderer.draw(elapsed)?;
                live += pass.live;
                presented += pass.presented;
            }
        }

        // Worth saying out loud: "still" and "paused" both look like nothing
        // is happening, and this is the line that separates a wallpaper
        // standing still on screen from one that never got there.
        // Only ever once per owed frame: presenting it is what stops it
        // being owed.
        if frozen && owed && presented > 0 {
            println!("still: drew the frame it will hold");
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

        // Nothing has been visible for long enough that the memory is worth
        // more to the machine than the decoders are to us. This is the one
        // saving that shows up in Task Manager while a game is running: the
        // render loop already costs nothing when covered, but the picture
        // buffers stay allocated for as long as a decoder is open.
        if paused {
            hidden_since.get_or_insert_with(Instant::now);
        } else {
            hidden_since = None;
        }

        // Handing the decoders back before the owed frame has been drawn
        // would leave the screen showing the wallpaper before last, with no
        // decoder left to correct it — a wallpaper changed while the engine
        // is frozen would simply never appear.
        let want_idle = settings.hibernate_secs > 0
            && paused
            && !owed
            && hidden_since.is_some_and(|since| {
                since.elapsed() >= Duration::from_secs(settings.hibernate_secs)
            });

        if want_idle != hibernating {
            hibernating = want_idle;
            changed = true;
            for renderer in &mut stage.renderers {
                renderer.set_idle(hibernating);
            }

            if hibernating {
                println!("hibernating: decoders released");
            } else {
                println!("waking: decoders reopened");
                // Every clip restarts, so the loop counts a playlist was
                // measuring against are gone with them. Left alone, a
                // playlist would sit on the same item until the new count
                // climbed past the old one.
                for state in playback.values_mut() {
                    state.loops_at_start = 0;
                    state.started = Instant::now();
                }
            }
        }

        // The desktop's chrome, taking its colour from the picture behind it.
        //
        // Only when the wallpaper on the primary screen changed: this reads
        // pixels back from the GPU and writes to the registry, and neither
        // belongs on a per-frame path. `presented` gates it because the
        // colour is read out of a frame, and there is no frame to read
        // before the first one has been drawn.
        if settings.accent && presented > 0 && !hibernating {
            let showing = stage
                .primary
                .as_ref()
                .and_then(|name| playback.get(name.as_str()))
                .and_then(|state| state.current().cloned());

            if let (Some(monitor), Some(_)) = (stage.primary.clone(), showing) {
                let sampled = stage
                    .renderers
                    .iter()
                    .find(|r| r.has_monitor(&monitor))
                    .and_then(|r| r.dominant_colour(&monitor));

                // A colour close enough to the last one is the same colour:
                // a video's average wanders by a point or two every frame,
                // and following that would be a registry write per frame.
                if let Some(colour) = sampled {
                    let moved = accented.is_none_or(|last| {
                        last.iter()
                            .zip(colour)
                            .any(|(a, b)| a.abs_diff(b) > ACCENT_STEP)
                    });
                    if moved {
                        accented = Some(colour);
                        accent::apply(colour);
                        changed = true;
                    }
                }
            }
        }

        stats.drew(presented);
        // Resting is not the same as paused: a frozen wallpaper, a covered
        // one and one waiting for its owner to come back all cost nothing,
        // and all three are what the user is being told about.
        stats.rested(frozen || live == 0);
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
            // Hibernating means giving the memory back, and an audio reader
            // is a decoder like any other. It is muted anyway.
            .filter(|_| !hibernating)
            .and_then(|name| playback.get(name.as_str()))
            .filter(|state| state.enabled)
            .and_then(|state| state.current())
            // A photo has no soundtrack and neither does a shader, and
            // starting a thread to discover that is a thread for nothing.
            .filter(|path| !crate::decoder::is_still(path) && !crate::decoder::is_shader(path))
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
                    hibernating,
                    power: power_state,
                    ducking,
                    away,
                    busy,
                    load: load.percent(),
                    stats: &stats,
                    error: last_error.as_deref(),
                    playback: &playback,
                    shaders: stage
                        .renderers
                        .iter()
                        .flat_map(|renderer| renderer.declared_params())
                        .collect(),
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
        // ...and then whatever the rest of the machine is doing. The lower
        // of the two wins: unplugged *and* busy is not a reason to speed up.
        let fps = crate::power::load::effective_fps(fps, settings.busy_fps, busy);
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

    // The user's own accent colours, handed back. Leaving a desktop tinted
    // by a wallpaper that is no longer running would be Muivly changing
    // something and not changing it back.
    if accent::is_applied() {
        accent::restore();
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

/// Where the cursor is on the desktop, -1 to 1 on each axis.
///
/// Against the whole desktop rather than one monitor, so a wallpaper spanning
/// two screens leans the same way on both — and so the picture on the left
/// screen is already at the end of its travel when the cursor is over the
/// right one, which is what depth looks like.
fn cursor_position(desktop: Rect) -> (f32, f32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    if desktop.width == 0 || desktop.height == 0 {
        return (0.0, 0.0);
    }

    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return (0.0, 0.0);
    }

    let across = |value: i32, origin: i32, size: u32| {
        ((value - origin) as f32 / size as f32 * 2.0 - 1.0).clamp(-1.0, 1.0)
    };

    (
        across(point.x, desktop.x, desktop.width),
        across(point.y, desktop.y, desktop.height),
    )
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
    shuffle: bool,
) -> windows::core::Result<()> {
    let Some(state) = playback.get_mut(monitor) else {
        return Ok(());
    };
    if state.order.is_empty() {
        return Ok(());
    }
    state.step(shuffle);

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
    /// Whether the decoders have been handed back while nobody is looking.
    hibernating: bool,
    power: PowerState,
    /// Whether the soundtrack is standing down for another application.
    ducking: bool,
    /// Whether the wallpaper is standing still because nobody is there.
    away: bool,
    /// Whether it is standing down for a busy machine, and how busy.
    busy: bool,
    load: f32,
    stats: &'a stats::Stats,
    error: Option<&'a str>,
    playback: &'a HashMap<String, Playback>,
    /// What every shader on screen declares it wants sliders for.
    shaders: Vec<(PathBuf, Vec<(crate::decoder::ShaderParam, f32)>)>,
}

fn publish(status: &Arc<Mutex<Status>>, report: &Report) {
    let settings = report.settings;
    let mut status = status.lock().expect("status mutex poisoned");
    status.fps = settings.fps;
    status.fit = settings.fit.name().to_string();
    status.interval_secs = settings.interval_secs;
    status.shuffle = settings.shuffle;
    status.paused = report.paused;
    status.hibernating = report.hibernating;
    status.hibernate_secs = settings.hibernate_secs;
    status.reactive = settings.motion.reactive;
    status.parallax = settings.motion.parallax;
    status.memory_mb = settings.memory_mb;
    status.idle_secs = settings.idle_secs;
    status.away = report.away;
    status.busy_fps = settings.busy_fps;
    status.busy = report.busy;
    status.load = report.load;
    status.reduce_motion = settings.reduce_motion;
    status.drift = settings.drift;
    status.accent = settings.accent;
    status.uptime_secs = report.stats.uptime.as_secs();
    status.resting_secs = report.stats.resting.as_secs();
    status.apps = settings.apps.join("|");
    status.rules = rules::written_form(&settings.rules);
    status.scenes = settings
        .scenes
        .iter()
        .map(scenes::written_form)
        .collect::<Vec<_>>();
    status.shaders = report
        .shaders
        .iter()
        .map(|(path, params)| {
            let fields: Vec<String> = params
                .iter()
                .map(|(param, value)| {
                    format!(
                        "{},{},{},{},{},{}",
                        param.name, param.min, param.max, param.default, value, param.label
                    )
                })
                .collect();
            format!("{}|{}", path.display(), fields.join("|"))
        })
        .collect();
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
            // Where in the list, not where in the play order: the UI shows
            // the list the user wrote and marks what is on screen now.
            index: state.current_index(),
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

    fn list(count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|i| PathBuf::from(format!("{i}.mp4")))
            .collect()
    }

    /// Everything the list holds, in the order this playback would show it,
    /// over `steps` advances.
    fn played(state: &mut Playback, shuffle: bool, steps: usize) -> Vec<PathBuf> {
        let mut seen = vec![state.current().cloned().unwrap()];
        for _ in 0..steps {
            state.step(shuffle);
            seen.push(state.current().cloned().unwrap());
        }
        seen
    }

    #[test]
    fn a_list_plays_in_the_order_it_was_written() {
        let mut state = Playback::blank();
        state.play(list(3), false);
        let mut wanted = list(3);
        wanted.push(PathBuf::from("0.mp4"));
        assert_eq!(played(&mut state, false, 3), wanted);
    }

    /// The property shuffle is for: over one pass every wallpaper appears
    /// exactly once. A shuffle that picked at random each time would fail
    /// this most of the time, which is the whole reason it is a bag.
    #[test]
    fn a_shuffled_pass_shows_every_wallpaper_once() {
        for _ in 0..40 {
            let mut state = Playback::blank();
            state.play(list(5), true);

            let mut seen: Vec<PathBuf> = played(&mut state, true, 4);
            seen.sort();
            assert_eq!(seen, list(5));
        }
    }

    /// The join between two passes. Without the guard in `order::shuffled`
    /// this is where the same wallpaper shows up twice in a row.
    #[test]
    fn a_wallpaper_never_follows_itself_across_a_pass() {
        for _ in 0..60 {
            let mut state = Playback::blank();
            state.play(list(4), true);
            let seen = played(&mut state, true, 12);
            for pair in seen.windows(2) {
                assert_ne!(pair[0], pair[1], "in {seen:?}");
            }
        }
    }

    /// Turning the setting on or off decides what plays *next*. Restarting
    /// the wallpaper the user is looking at would make it a visible edit to
    /// the desktop rather than a preference.
    #[test]
    fn changing_the_setting_leaves_the_wallpaper_on_screen_alone() {
        for shuffle in [true, false] {
            let mut state = Playback::blank();
            state.play(list(6), !shuffle);
            state.step(!shuffle);
            state.step(!shuffle);

            let showing = state.current().cloned();
            state.reorder(shuffle);
            assert_eq!(state.current().cloned(), showing);
        }
    }

    /// A single wallpaper is not a playlist, and neither is an empty screen.
    /// Both used to be arithmetic on a length of one or zero.
    #[test]
    fn a_short_list_survives_being_shuffled() {
        let mut state = Playback::blank();
        state.play(list(1), true);
        state.step(true);
        assert_eq!(state.current(), Some(&PathBuf::from("0.mp4")));

        let mut empty = Playback::blank();
        empty.play(Vec::new(), true);
        empty.step(true);
        assert_eq!(empty.current(), None);
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
