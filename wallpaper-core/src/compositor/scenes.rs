//! Named sets of wallpapers, one entry per monitor.
//!
//! Everything else in the engine describes one arrangement: this monitor
//! shows that, the other shows this. A scene is that arrangement with a name
//! on it, so somebody with two screens and three moods does not have to
//! rebuild the arrangement each time. Recalling one is exactly the same work
//! as assigning each monitor by hand, done in one message.
//!
//! Scenes carry wallpapers and nothing else. Brightness, frame rate and the
//! rest are settings for the desktop rather than for an arrangement, and a
//! scene that quietly changed the frame rate would be a scene nobody could
//! predict.
//!
//! The written form matches the rest of the protocol: `;` between monitors,
//! `|` between paths, which is the same trade `rules.rs` makes — a Windows
//! path can technically contain a semicolon and in practice never does.

use std::path::PathBuf;

/// One saved arrangement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub name: String,
    /// Device name, and what that screen was showing. A monitor with nothing
    /// assigned is kept with an empty list, because "this screen is bare" is
    /// part of the arrangement.
    pub monitors: Vec<(String, Vec<PathBuf>)>,
}

/// How many scenes one desktop may keep.
///
/// Enough for the moods anybody actually has, and low enough that the session
/// file stays something a person can read.
pub const MAX: usize = 12;

/// Characters a name may not contain, because they separate the fields it
/// would be written between.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && !name.contains([';', '|', '=', '\n', '\r'])
        && name.trim() == name
}

/// `<name>;<monitor>=<path>|<path>;<monitor>=` back into a scene.
pub fn parse(text: &str) -> Option<Scene> {
    let mut parts = text.split(';');
    let name = parts.next()?.trim();
    if !valid_name(name) {
        return None;
    }

    let monitors = parts
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let (monitor, list) = entry.split_once('=')?;
            let items = list
                .split('|')
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .collect();
            Some((monitor.to_string(), items))
        })
        .collect();

    Some(Scene {
        name: name.to_string(),
        monitors,
    })
}

/// One scene as one line's worth of text.
pub fn written_form(scene: &Scene) -> String {
    let mut out = scene.name.clone();
    for (monitor, items) in &scene.monitors {
        let paths: Vec<String> = items.iter().map(|p| p.display().to_string()).collect();
        out.push_str(&format!(";{monitor}={}", paths.join("|")));
    }
    out
}

/// Add a scene, or replace the one already using that name.
///
/// Saving over a name is what somebody means by saving twice; a second scene
/// called "Work" would be two identical buttons.
pub fn store(scenes: &mut Vec<Scene>, scene: Scene) -> bool {
    if !valid_name(&scene.name) {
        return false;
    }

    if let Some(existing) = scenes.iter_mut().find(|s| s.name == scene.name) {
        *existing = scene;
        return true;
    }
    if scenes.len() >= MAX {
        return false;
    }
    scenes.push(scene);
    true
}

pub fn remove(scenes: &mut Vec<Scene>, name: &str) -> bool {
    let before = scenes.len();
    scenes.retain(|scene| scene.name != name);
    scenes.len() != before
}

pub fn find<'a>(scenes: &'a [Scene], name: &str) -> Option<&'a Scene> {
    scenes.iter().find(|scene| scene.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        Scene {
            name: "Work".to_string(),
            monitors: vec![
                (
                    r"\\.\DISPLAY1".to_string(),
                    vec![
                        PathBuf::from(r"C:\clips\a b.mp4"),
                        PathBuf::from(r"C:\c.mp4"),
                    ],
                ),
                (r"\\.\DISPLAY2".to_string(), Vec::new()),
            ],
        }
    }

    #[test]
    fn a_scene_round_trips() {
        assert_eq!(parse(&written_form(&scene())), Some(scene()));
    }

    /// A screen with nothing on it is part of the arrangement: recalling a
    /// scene has to be able to clear a monitor, not just fill one.
    #[test]
    fn an_empty_screen_survives_the_round_trip() {
        let read = parse(&written_form(&scene())).unwrap();
        assert_eq!(read.monitors[1].1.len(), 0);
        assert_eq!(read.monitors.len(), 2);
    }

    #[test]
    fn a_name_may_not_carry_a_separator() {
        assert!(valid_name("Gece"));
        assert!(!valid_name("a;b"));
        assert!(!valid_name("a|b"));
        assert!(!valid_name(""));
        assert!(!valid_name(" padded"));
    }

    #[test]
    fn saving_twice_replaces_rather_than_duplicates() {
        let mut scenes = vec![scene()];
        let mut second = scene();
        second.monitors.clear();

        assert!(store(&mut scenes, second));
        assert_eq!(scenes.len(), 1);
        assert!(scenes[0].monitors.is_empty());
    }

    #[test]
    fn the_list_has_an_end() {
        let mut scenes = Vec::new();
        for i in 0..MAX {
            assert!(store(
                &mut scenes,
                Scene {
                    name: format!("s{i}"),
                    monitors: Vec::new(),
                }
            ));
        }
        assert!(!store(
            &mut scenes,
            Scene {
                name: "one too many".to_string(),
                monitors: Vec::new(),
            }
        ));
        assert_eq!(scenes.len(), MAX);
    }

    #[test]
    fn a_malformed_line_is_not_a_scene() {
        assert!(parse("").is_none());
        assert!(parse(";DISPLAY1=a.mp4").is_none());
    }

    #[test]
    fn removing_says_whether_it_removed_anything() {
        let mut scenes = vec![scene()];
        assert!(!remove(&mut scenes, "Nope"));
        assert!(remove(&mut scenes, "Work"));
        assert!(scenes.is_empty());
    }
}
