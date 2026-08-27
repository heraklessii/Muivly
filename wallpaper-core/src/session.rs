//! What the engine remembers between runs.
//!
//! The engine is otherwise told everything by the UI, which was fine while
//! the UI was always the thing that started it. Autostart changes that: at
//! logon the engine comes up alone, and a wallpaper engine that starts with
//! no wallpaper is not one anybody would leave switched on.
//!
//! So the engine keeps its own small file. Not the library — that stays the
//! frontend's, in `state.json`, and is none of the engine's business. Only
//! what is on screen right now and the settings that shape it.
//!
//! The format is the same one-line-per-fact shape as the IPC protocol, for
//! the same reason: the whole file is a dozen lines and a serialisation
//! library would cost more than it saves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::compositor::{Fit, Motion, Overrides, Rule, Scene, Sound, Visual};
use crate::power::battery::PowerPolicy;

/// Everything worth restoring.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub fps: Option<u32>,
    pub fit: Option<Fit>,
    pub interval_secs: Option<u64>,
    /// Whether a playlist plays in a drawn order rather than as written.
    pub shuffle: Option<bool>,
    pub visual: Option<Visual>,
    pub sound: Option<Sound>,
    pub power: Option<PowerPolicy>,
    pub speed: Option<f32>,
    pub fade: Option<Duration>,
    pub span: Option<bool>,
    pub hotkeys: Option<bool>,
    /// How long out of sight before the decoders are handed back.
    pub hibernate_secs: Option<u64>,
    /// How far the wallpaper answers to sound and to the cursor.
    pub motion: Option<Motion>,
    /// A memory budget in megabytes; 0 or absent is none.
    pub memory_mb: Option<u32>,
    /// How long the machine may sit untouched before the wallpaper stands
    /// still.
    pub idle_secs: Option<u64>,
    /// The frame rate while the machine is busy with something else.
    pub busy_fps: Option<u32>,
    /// Whether Windows' reduce-motion setting is honoured.
    pub reduce_motion: Option<bool>,
    /// How far a photograph drifts on its own.
    pub drift: Option<f32>,
    /// Whether the Windows accent colour follows the wallpaper.
    pub accent: Option<bool>,
    /// Applications that freeze the wallpaper while they are in front.
    pub apps: Vec<String>,
    /// Wallpapers that change themselves.
    pub rules: Vec<Rule>,
    /// Named arrangements of wallpapers across the screens.
    pub scenes: Vec<Scene>,
    /// Where each shader file's own settings are set.
    pub shader_params: HashMap<PathBuf, HashMap<String, f32>>,
    /// Settings a monitor keeps for itself, keyed by device name.
    pub overrides: HashMap<String, Overrides>,
    /// Per monitor: its device name, whether it is on, and its playlist.
    pub monitors: Vec<(String, bool, Vec<PathBuf>)>,
}

/// `%APPDATA%\Muivly\session.json`'s plainer sibling. Next to the library the
/// UI writes, because they belong to the same install and get cleaned up
/// together.
pub fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(Path::new(&appdata).join("Muivly").join("session.txt"))
}

/// Read the last session, or nothing if there is not one to read.
///
/// Every failure here is the same failure — no wallpaper is restored — and
/// none of them is worth stopping for, so they all come back as `None`.
pub fn load() -> Option<Session> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let mut session = parse(&text);

    // A wallpaper that has been deleted or moved since last time is dropped
    // rather than reported: the user did that on purpose and does not need
    // telling at logon. Kept out of `parse` so the parsing itself can be
    // tested without a disk.
    for (_, _, items) in &mut session.monitors {
        items.retain(|path| path.is_file());
    }

    Some(session)
}

/// Turn the file's text into a session. Every malformed line is skipped;
/// nothing here can fail hard, because the alternative to a partial restore
/// is no wallpaper at all.
fn parse(text: &str) -> Session {
    let mut session = Session::default();

    for line in text.lines() {
        // A line without a value is skipped, not fatal. This used to be `?`,
        // which returns from the whole function — so one blank line, or one
        // key written without its value, threw away every wallpaper and
        // setting the file held and the desktop came up empty at logon.
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "fps" => session.fps = value.parse().ok(),
            "fit" => session.fit = Fit::parse(value),
            "interval" => session.interval_secs = value.parse().ok(),
            "shuffle" => session.shuffle = Some(value == "on"),

            "visual" => {
                let numbers: Vec<f32> = value.split(' ').filter_map(|n| n.parse().ok()).collect();
                if let [brightness, saturation, blur] = numbers[..] {
                    session.visual = Some(Visual {
                        brightness,
                        saturation,
                        blur,
                    });
                }
            }

            // `sound <on|off> <volume> [duck]`. The third field arrived after
            // the first release wrote this file, so a file without it is not
            // malformed — it is last week's.
            "sound" => {
                let mut parts = value.split(' ');
                if let (Some(state), Some(volume)) = (parts.next(), parts.next()) {
                    session.sound = Some(Sound {
                        enabled: state == "on",
                        volume: volume.parse().unwrap_or(0.5),
                        duck: parts.next().map(|d| d == "on") != Some(false),
                    });
                }
            }

            // `power <battery_fps> <freeze_on_saver>`
            "power" => {
                if let Some((fps, freeze)) = value.split_once(' ') {
                    session.power = Some(PowerPolicy {
                        battery_fps: fps.parse().unwrap_or(24),
                        pause_on_saver: freeze == "on",
                    });
                }
            }

            "speed" => session.speed = value.parse().ok(),
            "fade" => session.fade = value.parse().ok().map(Duration::from_millis),
            "span" => session.span = Some(value == "on"),
            "hotkeys" => session.hotkeys = Some(value == "on"),
            "hibernate" => session.hibernate_secs = value.parse().ok(),
            "memory" => session.memory_mb = value.parse().ok(),
            "idle" => session.idle_secs = value.parse().ok(),
            "busy" => session.busy_fps = value.parse().ok(),
            "reducemotion" => session.reduce_motion = Some(value == "on"),
            "drift" => session.drift = value.parse().ok(),
            "accent" => session.accent = Some(value == "on"),
            "apps" => session.apps = crate::power::apps::parse_list(value),
            "rules" => session.rules = crate::compositor::parse_rules(value),

            // One line per arrangement rather than one line holding all of
            // them: a scene already uses both separators the protocol has.
            "scene" => {
                if let Some(scene) = crate::compositor::parse_scene(value) {
                    session.scenes.push(scene);
                }
            }

            // `sparam <path>|<name>=<value>|<name>=<value>`
            "sparam" => {
                let mut parts = value.split('|');
                if let Some(path) = parts.next().filter(|p| !p.is_empty()) {
                    let values: HashMap<String, f32> = parts
                        .filter_map(|field| field.split_once('='))
                        .filter_map(|(name, value)| Some((name.to_string(), value.parse().ok()?)))
                        .collect();
                    if !values.is_empty() {
                        session.shader_params.insert(PathBuf::from(path), values);
                    }
                }
            }

            // `motion <reactive> <parallax>`
            "motion" => {
                let numbers: Vec<f32> = value.split(' ').filter_map(|n| n.parse().ok()).collect();
                if let [reactive, parallax] = numbers[..] {
                    session.motion = Some(Motion { reactive, parallax });
                }
            }

            // `own <monitor> <fit|-> <fps> <brightness|-> <saturation> <blur>`
            "own" => {
                let parts: Vec<&str> = value.split(' ').collect();
                let [name, fit, fps, brightness, saturation, blur] = parts[..] else {
                    continue;
                };

                let visual = brightness
                    .parse::<f32>()
                    .ok()
                    .filter(|b| *b >= 0.0)
                    .map(|b| Visual {
                        brightness: b,
                        saturation: saturation.parse().unwrap_or(1.0),
                        blur: blur.parse().unwrap_or(0.0),
                    });

                session.overrides.insert(
                    name.to_string(),
                    Overrides {
                        fit: Fit::parse(fit),
                        fps: fps.parse().ok().filter(|n| *n > 0),
                        visual,
                    },
                );
            }

            // `monitor <name> <enabled> <path>|<path>`
            //
            // The playlist may be missing entirely rather than empty: a
            // monitor that is switched off but has nothing assigned writes
            // the line without it. Losing the line would lose the fact that
            // the user turned that screen off.
            "monitor" => {
                let mut parts = value.splitn(3, ' ');
                let (Some(name), Some(enabled)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let items = parts
                    .next()
                    .unwrap_or("")
                    .split('|')
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
                    .collect();
                session
                    .monitors
                    .push((name.to_string(), enabled == "true", items));
            }

            _ => {}
        }
    }

    session
}

/// Write the current session out, best effort.
///
/// Called whenever something the user chose changes, which is rarely — a few
/// times a session, not a few times a second.
pub fn save(session: &Session) {
    let Some(path) = path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    let out = written_form(session);

    // Written whole and renamed into place: a logoff halfway through a write
    // would otherwise leave a truncated file, and the next start would come
    // up with half a desktop.
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, out).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// The file's text, without the disk. Split out so a round trip can be
/// tested the way `parse` is — the pair of them is where a setting silently
/// stops being remembered.
fn written_form(session: &Session) -> String {
    let mut out = String::new();
    if let Some(fps) = session.fps {
        out.push_str(&format!("fps {fps}\n"));
    }
    if let Some(fit) = session.fit {
        out.push_str(&format!("fit {}\n", fit.name()));
    }
    if let Some(interval) = session.interval_secs {
        out.push_str(&format!("interval {interval}\n"));
    }
    if let Some(on) = session.shuffle {
        out.push_str(&format!("shuffle {}\n", if on { "on" } else { "off" }));
    }
    if let Some(visual) = session.visual {
        out.push_str(&format!(
            "visual {:.3} {:.3} {:.3}\n",
            visual.brightness, visual.saturation, visual.blur
        ));
    }
    if let Some(sound) = session.sound {
        out.push_str(&format!(
            "sound {} {:.3} {}\n",
            if sound.enabled { "on" } else { "off" },
            sound.volume,
            if sound.duck { "on" } else { "off" }
        ));
    }
    if let Some(power) = session.power {
        out.push_str(&format!(
            "power {} {}\n",
            power.battery_fps,
            if power.pause_on_saver { "on" } else { "off" }
        ));
    }
    if let Some(speed) = session.speed {
        out.push_str(&format!("speed {speed:.2}\n"));
    }
    if let Some(fade) = session.fade {
        out.push_str(&format!("fade {}\n", fade.as_millis()));
    }
    if let Some(span) = session.span {
        out.push_str(&format!("span {}\n", if span { "on" } else { "off" }));
    }
    if let Some(hotkeys) = session.hotkeys {
        out.push_str(&format!("hotkeys {}\n", if hotkeys { "on" } else { "off" }));
    }
    if let Some(secs) = session.hibernate_secs {
        out.push_str(&format!("hibernate {secs}\n"));
    }
    if let Some(motion) = session.motion {
        out.push_str(&format!(
            "motion {:.3} {:.3}\n",
            motion.reactive, motion.parallax
        ));
    }
    if let Some(mb) = session.memory_mb {
        out.push_str(&format!("memory {mb}\n"));
    }
    if let Some(secs) = session.idle_secs {
        out.push_str(&format!("idle {secs}\n"));
    }
    if let Some(fps) = session.busy_fps {
        out.push_str(&format!("busy {fps}\n"));
    }
    if let Some(on) = session.reduce_motion {
        out.push_str(&format!("reducemotion {}\n", if on { "on" } else { "off" }));
    }
    if let Some(drift) = session.drift {
        out.push_str(&format!("drift {drift:.3}\n"));
    }
    if let Some(on) = session.accent {
        out.push_str(&format!("accent {}\n", if on { "on" } else { "off" }));
    }
    if !session.apps.is_empty() {
        out.push_str(&format!("apps {}\n", session.apps.join("|")));
    }
    for scene in &session.scenes {
        out.push_str(&format!(
            "scene {}\n",
            crate::compositor::write_scene(scene)
        ));
    }
    for (path, values) in &session.shader_params {
        // Sorted, so a file that is written every time a setting changes
        // does not shuffle its own lines and look different each time.
        let mut fields: Vec<String> = values
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        fields.sort();
        out.push_str(&format!("sparam {}|{}\n", path.display(), fields.join("|")));
    }
    if !session.rules.is_empty() {
        out.push_str(&format!(
            "rules {}\n",
            crate::compositor::write_rules(&session.rules)
        ));
    }

    for (name, own) in &session.overrides {
        let visual = own.visual.unwrap_or_default();
        out.push_str(&format!(
            "own {} {} {} {:.3} {:.3} {:.3}\n",
            name,
            own.fit.map(|f| f.name()).unwrap_or("-"),
            own.fps.unwrap_or(0),
            // A negative brightness is impossible and is how "this monitor
            // has no visual settings of its own" is written down.
            if own.visual.is_some() {
                visual.brightness
            } else {
                -1.0
            },
            visual.saturation,
            visual.blur,
        ));
    }

    for (name, enabled, items) in &session.monitors {
        let paths: Vec<String> = items.iter().map(|p| p.display().to_string()).collect();
        out.push_str(&format!("monitor {name} {enabled} {}\n", paths.join("|")));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_session_round_trips() {
        let written = "fps 30\nfit contain\ninterval 900\nvisual 1.100 0.900 0.250\n\
                       sound on 0.400\nmonitor DISPLAY1 true a.mp4|b.mp4\n";
        let session = parse(written);

        assert_eq!(session.fps, Some(30));
        assert_eq!(session.fit, Some(Fit::Contain));
        assert_eq!(session.interval_secs, Some(900));
        assert_eq!(session.visual.unwrap().brightness, 1.1);
        assert!(session.sound.unwrap().enabled);
        assert_eq!(session.monitors.len(), 1);
        assert_eq!(session.monitors[0].2.len(), 2);
    }

    /// The bug this guards: one unparseable line used to abandon the whole
    /// file, so a stray blank line meant an empty desktop at logon.
    #[test]
    fn a_broken_line_does_not_take_the_rest_with_it() {
        let session = parse("fps 60\n\nnonsense\nfit stretch\n");
        assert_eq!(session.fps, Some(60));
        assert_eq!(session.fit, Some(Fit::Stretch));
    }

    #[test]
    fn a_monitor_with_no_playlist_keeps_its_switch() {
        // Written with a trailing space when the list is empty, and without
        // one by anything that trims. Both mean the same thing.
        for line in ["monitor DISPLAY2 false ", "monitor DISPLAY2 false"] {
            let session = parse(line);
            assert_eq!(session.monitors.len(), 1, "{line:?}");
            assert!(!session.monitors[0].1, "{line:?}");
            assert!(session.monitors[0].2.is_empty(), "{line:?}");
        }
    }

    #[test]
    fn the_settings_added_after_the_first_release_round_trip() {
        let session = parse(
            "power 15 on\nspeed 0.50\nfade 250\nspan on\nhotkeys off\n\
             sound on 0.400 off\n",
        );

        let power = session.power.unwrap();
        assert_eq!(power.battery_fps, 15);
        assert!(power.pause_on_saver);
        assert_eq!(session.speed, Some(0.5));
        assert_eq!(session.fade, Some(Duration::from_millis(250)));
        assert_eq!(session.span, Some(true));
        assert_eq!(session.hotkeys, Some(false));
        assert!(!session.sound.unwrap().duck);
    }

    /// The settings this session added. Written out and read back rather
    /// than parsed from a literal: the pair is where a field silently stops
    /// being remembered, and only a round trip catches that.
    #[test]
    fn the_settings_added_for_hibernation_and_motion_round_trip() {
        let before = Session {
            hibernate_secs: Some(45),
            motion: Some(Motion {
                reactive: 0.6,
                parallax: 0.25,
            }),
            memory_mb: Some(350),
            apps: vec!["photoshop.exe".to_string(), "blender".to_string()],
            rules: crate::compositor::parse_rules(r"t420|C:a bday.mp4;d1|C:dark.mp4"),
            ..Session::default()
        };

        let after = parse(&written_form(&before));

        assert_eq!(after.hibernate_secs, Some(45));
        let motion = after.motion.unwrap();
        assert!((motion.reactive - 0.6).abs() < 0.001);
        assert!((motion.parallax - 0.25).abs() < 0.001);
        assert_eq!(after.memory_mb, Some(350));
        assert_eq!(after.apps, before.apps);
        // Paths with spaces in them are the case the `|` separator exists
        // for, and a rule is a path list.
        assert_eq!(after.rules, before.rules);
    }

    /// The settings this session added, round-tripped rather than parsed
    /// from a literal — the pair of write and read is where a field silently
    /// stops being remembered.
    #[test]
    fn the_settings_added_for_idling_drifting_and_scenes_round_trip() {
        let mut shader_params = HashMap::new();
        shader_params.insert(
            PathBuf::from(r"C:\shaders\bars one.hlsl"),
            HashMap::from([("speed".to_string(), 1.5), ("glow".to_string(), 0.25)]),
        );

        let before = Session {
            idle_secs: Some(600),
            busy_fps: Some(12),
            reduce_motion: Some(false),
            drift: Some(0.4),
            accent: Some(true),
            scenes: vec![crate::compositor::parse_scene(
                r"Gece;\\.\DISPLAY1=C:\a b.mp4|C:\c.mp4;\\.\DISPLAY2=",
            )
            .unwrap()],
            shader_params,
            ..Session::default()
        };

        let after = parse(&written_form(&before));

        assert_eq!(after.idle_secs, Some(600));
        assert_eq!(after.busy_fps, Some(12));
        assert_eq!(after.reduce_motion, Some(false));
        assert!((after.drift.unwrap() - 0.4).abs() < 0.001);
        assert_eq!(after.accent, Some(true));
        assert_eq!(after.scenes, before.scenes);
        assert_eq!(after.shader_params, before.shader_params);
    }

    /// Both spellings, because a boolean written as one word and read back
    /// as another is exactly how a setting stops being remembered without
    /// anything failing.
    #[test]
    fn shuffle_round_trips_either_way() {
        for wanted in [true, false] {
            let before = Session {
                shuffle: Some(wanted),
                ..Session::default()
            };
            assert_eq!(parse(&written_form(&before)).shuffle, Some(wanted));
        }
    }

    /// A session file written before shuffle existed says nothing about it,
    /// and must leave it unset rather than off — the difference is what lets
    /// the engine's own default apply.
    #[test]
    fn a_file_without_a_shuffle_line_leaves_it_unset() {
        assert_eq!(parse("fps 30\nfit cover\n").shuffle, None);
    }

    /// A session file from before any of this existed has none of those
    /// lines. Every one of them must land on its default rather than on a
    /// zero — a missing `hibernate` line meaning "never hibernate" would
    /// silently switch the feature off for everyone upgrading.
    #[test]
    fn a_file_without_the_new_lines_leaves_them_unset() {
        let session = parse(
            "fps 30
fit cover
",
        );
        assert_eq!(session.hibernate_secs, None);
        assert!(session.motion.is_none());
        assert_eq!(session.memory_mb, None);
        assert!(session.apps.is_empty());
        assert!(session.rules.is_empty());
        // The same rule for everything added since: absent must mean "use
        // the default", never "the feature is off". A missing `idle` line
        // reading as zero would silently switch idling off for everybody
        // upgrading, which is the bug this whole test exists for.
        assert_eq!(session.idle_secs, None);
        assert_eq!(session.busy_fps, None);
        assert_eq!(session.reduce_motion, None);
        assert_eq!(session.drift, None);
        assert_eq!(session.accent, None);
        assert!(session.scenes.is_empty());
        assert!(session.shader_params.is_empty());
    }

    /// A file written before ducking existed has two fields where there are
    /// now three. It must still restore, and the missing field must land on
    /// the default rather than on `false`.
    #[test]
    fn an_older_sound_line_still_loads() {
        let session = parse("sound on 0.400\n");
        let sound = session.sound.unwrap();
        assert!(sound.enabled);
        assert_eq!(sound.volume, 0.4);
        assert!(sound.duck);
    }

    #[test]
    fn a_monitor_with_settings_of_its_own_round_trips() {
        let session = parse("own DISPLAY2 contain 10 0.500 1.000 0.250\n");
        let own = session.overrides.get("DISPLAY2").unwrap();

        assert_eq!(own.fit, Some(Fit::Contain));
        assert_eq!(own.fps, Some(10));
        assert_eq!(own.visual.unwrap().brightness, 0.5);
        assert_eq!(own.visual.unwrap().blur, 0.25);
    }

    /// `-` and `0` are how "follow the desktop" is written down, and they
    /// must not come back as a fit of cover and a cap of zero fps — the
    /// second of those would stop the monitor presenting at all.
    #[test]
    fn a_monitor_that_follows_the_desktop_has_no_settings_of_its_own() {
        let session = parse("own DISPLAY2 - 0 -1.000 1.000 0.000\n");
        let own = session.overrides.get("DISPLAY2").unwrap();

        assert_eq!(own.fit, None);
        assert_eq!(own.fps, None);
        assert_eq!(own.visual, None);
    }

    #[test]
    fn overrides_survive_a_write_and_a_read() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "DISPLAY1".to_string(),
            Overrides {
                fit: Some(Fit::Stretch),
                fps: Some(12),
                visual: None,
            },
        );

        let written = written_form(&Session {
            overrides,
            ..Default::default()
        });

        let read = parse(&written);
        let own = read.overrides.get("DISPLAY1").unwrap();
        assert_eq!(own.fit, Some(Fit::Stretch));
        assert_eq!(own.fps, Some(12));
        assert_eq!(own.visual, None);
    }

    #[test]
    fn an_empty_file_is_an_empty_session() {
        let session = parse("");
        assert!(session.fps.is_none());
        assert!(session.monitors.is_empty());
    }
}
