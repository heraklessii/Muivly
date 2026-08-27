//! Applications the wallpaper stands down for.
//!
//! Coverage handles the common case already: a fullscreen game hides the
//! desktop and nothing is drawn. What it does not handle is the application
//! that leaves the desktop visible and still wants the machine — a render, a
//! compile, a game in borderless windowed mode on the second screen, a call
//! where the laptop fan is the problem.
//!
//! So the user can name them. While one of them is the window in front, the
//! wallpaper freezes: the last frame stays on screen and nothing is decoded
//! or drawn, exactly as though the user had frozen it by hand.
//!
//! The foreground window, rather than every running process. "Photoshop is
//! open" is true for eight hours a day and would mean the wallpaper never
//! ran; "Photoshop is what I am looking at" is the moment that actually
//! costs something. It is also the cheaper question by far — three calls
//! against an enumeration of every process on the machine.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// How often the foreground application is looked up.
///
/// Twice a second. Alt-tabbing into a game and having the wallpaper stop
/// half a second later is imperceptible; doing this every frame would be
/// three syscalls at 60 Hz for an answer that changes a few times an hour.
const RECHECK_AFTER: Duration = Duration::from_millis(500);

/// Watches what is in front against a list of names.
#[derive(Default)]
pub struct AppWatch {
    matched: bool,
    checked: Option<Instant>,
}

impl AppWatch {
    /// Whether the window in front belongs to one of `names`.
    ///
    /// An empty list is the common case and costs nothing at all — not even
    /// the lookup.
    pub fn matches(&mut self, names: &[String]) -> bool {
        if names.is_empty() {
            self.matched = false;
            return false;
        }

        let now = Instant::now();
        if self
            .checked
            .is_some_and(|last| now.duration_since(last) < RECHECK_AFTER)
        {
            return self.matched;
        }
        self.checked = Some(now);

        self.matched = foreground_process()
            .map(|process| names.iter().any(|name| same_app(name, &process)))
            .unwrap_or(false);
        self.matched
    }
}

/// Whether a name the user typed refers to this executable.
///
/// Forgiving on purpose: people write "Photoshop", "photoshop.exe" and
/// "Adobe Photoshop.exe" for the same thing, and a rule that silently never
/// fires is worse than one that occasionally fires wide. The comparison is
/// case-insensitive and ignores a `.exe` on either side.
fn same_app(name: &str, process: &str) -> bool {
    let trim = |text: &str| {
        text.trim()
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_ascii_lowercase()
    };

    let name = trim(name);
    !name.is_empty() && trim(process) == name
}

/// The file name of the process owning the foreground window.
fn foreground_process() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        // LIMITED_INFORMATION rather than QUERY_INFORMATION: it is the right
        // access for reading an image name and it is granted for processes
        // this one could not otherwise open, which is most of them on a
        // machine with UAC on.
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

/// Parse the list as the UI and the session file write it: names separated
/// by `|`, blanks dropped.
pub fn parse_list(text: &str) -> Vec<String> {
    text.split('|')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exe_suffix_is_optional_on_either_side() {
        assert!(same_app("photoshop", "Photoshop.exe"));
        assert!(same_app("Photoshop.exe", "photoshop.exe"));
        assert!(same_app("PHOTOSHOP.EXE", "Photoshop.exe"));
    }

    #[test]
    fn a_different_application_does_not_match() {
        assert!(!same_app("photoshop", "explorer.exe"));
        // Substrings must not match, or "code" would freeze the wallpaper
        // for every process with those four letters in its name.
        assert!(!same_app("code", "vscode.exe"));
    }

    #[test]
    fn an_empty_name_matches_nothing() {
        // A trailing separator in the settings box used to leave one of
        // these in the list, and an empty rule that matched everything
        // would freeze the wallpaper for good.
        assert!(!same_app("", "explorer.exe"));
        assert!(!same_app("   ", "explorer.exe"));
    }

    #[test]
    fn the_list_drops_blanks_and_whitespace() {
        assert_eq!(
            parse_list(" photoshop.exe | | blender "),
            vec!["photoshop.exe".to_string(), "blender".to_string()]
        );
        assert!(parse_list("").is_empty());
        assert!(parse_list("|||").is_empty());
    }

    #[test]
    fn an_empty_list_is_never_a_match() {
        let mut watch = AppWatch::default();
        assert!(!watch.matches(&[]));
    }
}
