//! Wallpapers that change themselves.
//!
//! Two triggers, because between them they cover what people actually ask
//! for and neither needs a scheduler:
//!
//! - **A time of day.** Rules are start times, not ranges: whichever one
//!   started most recently is the one in force. That is what makes a pair of
//!   them — 07:00 and 20:00 — mean "day and night" without anybody having to
//!   write down that night ends when day begins, and it is why there is no
//!   way to leave a gap by mistake.
//! - **The Windows theme.** A desktop that goes dark at sunset and takes the
//!   wallpaper with it, without either of them being told what time sunset
//!   is.
//!
//! A theme rule beats a time rule. Someone who has set both has said what
//! they want twice, and the theme is the more specific of the two: it
//! follows a switch the user (or Windows) actually flipped.

use std::path::PathBuf;

use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// What makes a rule apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Minutes since midnight, local time.
    Time(u32),
    /// True for the dark Windows theme.
    Theme(bool),
}

/// One rule: when, and what to put on every screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub trigger: Trigger,
    pub items: Vec<PathBuf>,
}

/// Which rule is in force, given the clock and the theme.
///
/// `None` means the user's own choice stands — which is what an empty rule
/// list means, and also what a set of time rules means before the first of
/// them has come round on a machine that started at 03:00 with the earliest
/// rule at 07:00. In that case the latest rule of the day wins, because the
/// day before it ran and nobody has changed anything since.
pub fn choose(rules: &[Rule], now_minutes: u32, dark: Option<bool>) -> Option<&Rule> {
    if let Some(dark) = dark {
        if let Some(rule) = rules
            .iter()
            .find(|rule| rule.trigger == Trigger::Theme(dark))
        {
            return Some(rule);
        }
    }

    let times: Vec<(&Rule, u32)> = rules
        .iter()
        .filter_map(|rule| match rule.trigger {
            Trigger::Time(at) => Some((rule, at)),
            Trigger::Theme(_) => None,
        })
        .collect();

    // The most recent start at or before now.
    times
        .iter()
        .filter(|(_, at)| *at <= now_minutes)
        .max_by_key(|(_, at)| *at)
        // Before the first rule of the day: the one that started last night
        // is still the one on screen.
        .or_else(|| times.iter().max_by_key(|(_, at)| *at))
        .map(|(rule, _)| *rule)
}

/// Minutes since local midnight.
pub fn now_minutes() -> u32 {
    let time = unsafe { GetLocalTime() };
    time.wHour as u32 * 60 + time.wMinute as u32
}

/// Whether Windows is using its dark theme for applications.
///
/// `None` when the value cannot be read, which is a machine where the
/// setting does not exist rather than one that is definitely light — and
/// treating it as light would silently apply the wrong rule.
pub fn dark_theme() -> Option<bool> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;

    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut _),
            Some(&mut size),
        )
    };

    // Read only. The one registry value this project reads that it does not
    // also write; see CLAUDE.md on what Muivly is allowed to put there.
    status.is_ok().then_some(value == 0)
}

/// `t<minutes>|<path>|<path>` or `d<0|1>|<path>...`, rules separated by `;`.
///
/// Its own function, and the same one the session file and the pipe both
/// use: a rule that survives a restart but not a reconnect (or the other way
/// round) is the kind of bug that takes a day to see.
pub fn parse(text: &str) -> Vec<Rule> {
    text.split(';')
        .filter(|entry| !entry.trim().is_empty())
        .filter_map(parse_one)
        .collect()
}

fn parse_one(entry: &str) -> Option<Rule> {
    let mut parts = entry.trim().split('|');
    let head = parts.next()?;
    let (kind, value) = head.split_at_checked(1)?;

    let trigger = match kind {
        "t" => Trigger::Time(value.parse::<u32>().ok().filter(|m| *m < 24 * 60)?),
        "d" => Trigger::Theme(value == "1"),
        _ => return None,
    };

    let items: Vec<PathBuf> = parts
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect();

    // A rule with nothing to show is not a rule. Dropping it here is what
    // stops a stray separator from blanking every screen at 07:00.
    (!items.is_empty()).then_some(Rule { trigger, items })
}

/// The written form `parse` reads back.
pub fn written_form(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|rule| {
            let head = match rule.trigger {
                Trigger::Time(at) => format!("t{at}"),
                Trigger::Theme(dark) => format!("d{}", if dark { 1 } else { 0 }),
            };
            let paths: Vec<String> = rule.items.iter().map(|p| p.display().to_string()).collect();
            format!("{head}|{}", paths.join("|"))
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(trigger: Trigger, path: &str) -> Rule {
        Rule {
            trigger,
            items: vec![PathBuf::from(path)],
        }
    }

    fn day_and_night() -> Vec<Rule> {
        vec![
            rule(Trigger::Time(7 * 60), "day.mp4"),
            rule(Trigger::Time(20 * 60), "night.mp4"),
        ]
    }

    #[test]
    fn the_most_recent_start_is_the_one_in_force() {
        let rules = day_and_night();
        assert_eq!(
            choose(&rules, 12 * 60, None).unwrap().items[0].as_os_str(),
            "day.mp4"
        );
        assert_eq!(
            choose(&rules, 22 * 60, None).unwrap().items[0].as_os_str(),
            "night.mp4"
        );
    }

    #[test]
    fn a_rule_takes_effect_on_the_minute_it_names() {
        let rules = day_and_night();
        assert_eq!(
            choose(&rules, 7 * 60, None).unwrap().items[0].as_os_str(),
            "day.mp4"
        );
        assert_eq!(
            choose(&rules, 7 * 60 - 1, None).unwrap().items[0].as_os_str(),
            "night.mp4"
        );
    }

    #[test]
    fn before_the_first_rule_the_night_before_still_stands() {
        // 03:00 with rules at 07:00 and 20:00. Nothing has started today,
        // and the answer is not "no wallpaper" — it is last night's.
        let rules = day_and_night();
        assert_eq!(
            choose(&rules, 3 * 60, None).unwrap().items[0].as_os_str(),
            "night.mp4"
        );
    }

    #[test]
    fn the_theme_wins_over_the_clock() {
        let mut rules = day_and_night();
        rules.push(rule(Trigger::Theme(true), "dark.mp4"));
        assert_eq!(
            choose(&rules, 12 * 60, Some(true)).unwrap().items[0].as_os_str(),
            "dark.mp4"
        );
        // ...but only the one that matches. A dark rule on a light desktop
        // must not shadow the time rules.
        assert_eq!(
            choose(&rules, 12 * 60, Some(false)).unwrap().items[0].as_os_str(),
            "day.mp4"
        );
    }

    #[test]
    fn no_rules_means_the_users_own_choice_stands() {
        assert!(choose(&[], 12 * 60, Some(true)).is_none());
    }

    #[test]
    fn rules_round_trip_through_their_written_form() {
        let rules = vec![
            Rule {
                trigger: Trigger::Time(420),
                items: vec![PathBuf::from(r"C:\a b\day.mp4"), PathBuf::from("x.mp4")],
            },
            rule(Trigger::Theme(true), "dark.mp4"),
        ];
        assert_eq!(parse(&written_form(&rules)), rules);
    }

    #[test]
    fn a_rule_with_no_wallpaper_is_dropped() {
        // Otherwise a trailing separator in the settings window is a rule
        // that clears every screen when it comes round.
        assert!(parse("t420").is_empty());
        assert!(parse("t420|").is_empty());
    }

    #[test]
    fn a_time_outside_the_day_is_not_a_rule() {
        assert!(parse("t1500|x.mp4").is_empty());
    }

    #[test]
    fn nonsense_is_skipped_rather_than_fatal() {
        let rules = parse("wat|x.mp4;t60|good.mp4;;");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].trigger, Trigger::Time(60));
    }
}
