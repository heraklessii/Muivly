//! Browsing and downloading from motionbgs.com.
//!
//! Muivly works entirely offline and always will — this is a place to get
//! wallpapers from, not a service the app depends on. Nothing here runs
//! unless the user opens the browse view or presses download. There is no
//! account, no key, no identifier of any kind sent: the requests are the
//! same ones a browser would make for a public page.
//!
//! Only fetching and saving happen in Rust. The HTML is parsed in the
//! frontend with `DOMParser`, which the WebView already has — an HTML parser
//! crate would be a large dependency to do worse what is already there.

use std::path::PathBuf;

/// The one host these commands will talk to.
///
/// Not a formality. Without it these commands are a general-purpose proxy
/// with the file system attached, reachable by anything that can reach the
/// WebView — which is a much larger promise than "shows a wallpaper site".
const HOST: &str = "motionbgs.com";

/// What the site is told about us. A blank user agent gets refused by most
/// front ends, and pretending to be a browser would be a lie told to a
/// server that has done nothing wrong.
const USER_AGENT: &str = concat!(
    "Muivly/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/heraklessii/Muivly)"
);

/// How long to wait before giving up. Long enough for a slow connection,
/// short enough that a dead link does not look like a frozen app.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Refuse anything that is not an https URL on the one allowed host.
///
/// Checked on the authority component rather than with `contains`, so
/// `https://motionbgs.com.example.net/` is rejected the way it should be.
fn allowed(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("only https addresses are allowed".to_string());
    };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");

    if host == HOST || host.ends_with(&format!(".{HOST}")) {
        Ok(())
    } else {
        Err(format!("Muivly only downloads from {HOST}"))
    }
}

/// How many redirects one request may follow before it is treated as a loop.
const MAX_HOPS: usize = 5;

/// The largest file this will write to disk.
///
/// A live wallpaper is tens of megabytes; a gigabyte is far past anything the
/// site serves. The cap is not about whether the site behaves — it is what
/// bounds the damage when a response does not, and it is the reason the body
/// is written as it arrives rather than collected whole first.
const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        // The allowlist is checked on the URL we were handed, and a redirect
        // is by definition a URL we were not. Followed blindly, one
        // `Location` header turns these commands back into the general
        // proxy with a file system attached that the allowlist exists to
        // prevent — so every hop is checked the way the first one is.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_HOPS {
                return attempt.error("too many redirects");
            }
            match allowed(attempt.url().as_str()) {
                Ok(()) => attempt.follow(),
                // Stopped rather than errored, so the caller sees the
                // redirect response itself and can say what happened.
                Err(_) => attempt.stop(),
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// Fetch one page as text, for the frontend to pick apart.
#[tauri::command]
pub async fn web_fetch(url: String) -> Result<String, String> {
    allowed(&url)?;

    let response = client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("the site answered {}", response.status().as_u16()));
    }

    response.text().await.map_err(|e| e.to_string())
}

/// Download a wallpaper and return where it landed.
///
/// Saved beside the library rather than in Downloads: these are files Muivly
/// created and will keep referring to, and a user tidying their Downloads
/// folder should not be quietly deleting their wallpapers.
#[tauri::command]
pub async fn web_download(url: String, name: String) -> Result<String, String> {
    allowed(&url)?;

    let folder = wallpapers_dir()?;
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let target = folder.join(safe_name(&name));
    // Already downloaded: hand back the copy rather than fetching it twice.
    if target.is_file() {
        return Ok(target.display().to_string());
    }

    let mut response = client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("the site answered {}", response.status().as_u16()));
    }

    if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err("that file is far larger than a wallpaper; refusing it".to_string());
    }

    // Written under a temporary name and renamed: an interrupted download
    // must not leave something that looks like a finished wallpaper.
    let partial = target.with_extension("part");
    let written = stream_to_file(&mut response, &partial).await;

    let written = match written {
        Ok(written) => written,
        Err(e) => {
            // Half a file is worse than none: it would be renamed into the
            // library on a later attempt only if it happened to survive, and
            // it is dead weight on disk either way.
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
    };

    if written == 0 {
        let _ = std::fs::remove_file(&partial);
        return Err("the download was empty".to_string());
    }

    std::fs::rename(&partial, &target).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        e.to_string()
    })?;

    Ok(target.display().to_string())
}

/// Write a response body to a file as it arrives, refusing to run past the
/// cap. Returns how many bytes landed.
///
/// The body used to be collected with `bytes()` first, which for a 4K clip is
/// a few hundred megabytes held in memory at once — in the process whose
/// whole pitch is that it is small — and no bound at all on what a server
/// could make it hold.
async fn stream_to_file(
    response: &mut reqwest::Response,
    path: &std::path::Path,
) -> Result<u64, String> {
    use std::io::Write;

    let mut file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
    let mut written = 0u64;

    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        written += chunk.len() as u64;
        if written > MAX_DOWNLOAD {
            return Err("that file is far larger than a wallpaper; refusing it".to_string());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
    }

    file.flush().map_err(|e| e.to_string())?;
    Ok(written)
}

/// Where downloaded wallpapers live.
#[tauri::command]
pub fn wallpapers_path() -> Result<String, String> {
    Ok(wallpapers_dir()?.display().to_string())
}

pub(crate) fn wallpapers_dir() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    Ok(PathBuf::from(appdata).join("Muivly").join("wallpapers"))
}

/// Reduce a name from the web to something safe to write to disk.
///
/// A file name is the one thing here that comes from a page and ends up in a
/// path, so it is the one place a hostile page could try to write outside
/// the folder. Only these characters survive.
fn safe_name(name: &str) -> String {
    let mut cleaned = String::with_capacity(name.len());

    for c in name.chars() {
        let c = match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '-',
        };

        // A run of dots is collapsed to one. Separators are already gone by
        // the time we get here, so `..` cannot climb anywhere on its own —
        // but leaving it in means writing files named after a traversal
        // attempt, and there is no name worth keeping that needs two dots
        // in a row.
        if c == '.' && cleaned.ends_with('.') {
            continue;
        }
        cleaned.push(c);
    }

    let cleaned = cleaned.trim_matches('.');

    // Windows refuses a path over 260 characters unless long paths are
    // switched on, and the folder already eats some of that. A name from a
    // page has no reason to be near the limit, and one that is would fail
    // the write rather than the check — with a message about the path
    // instead of about the name.
    const MAX_NAME: usize = 120;
    let cleaned = match cleaned.char_indices().nth(MAX_NAME) {
        Some((cut, _)) => &cleaned[..cut],
        None => cleaned,
    };

    if cleaned.is_empty() {
        "wallpaper.mp4".to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_site_itself_is_allowed() {
        assert!(allowed("https://motionbgs.com/dl/4k/9982").is_ok());
        assert!(allowed("https://www.motionbgs.com/x").is_ok());
    }

    #[test]
    fn a_lookalike_host_is_not() {
        // The trap this check exists for: a host that merely starts with the
        // allowed name.
        assert!(allowed("https://motionbgs.com.evil.example/x").is_err());
        assert!(allowed("https://notmotionbgs.com/x").is_err());
    }

    #[test]
    fn credentials_in_the_url_do_not_smuggle_a_host_past() {
        assert!(allowed("https://motionbgs.com@evil.example/x").is_err());
    }

    #[test]
    fn plain_http_is_refused() {
        assert!(allowed("http://motionbgs.com/x").is_err());
    }

    #[test]
    fn a_name_cannot_climb_out_of_the_folder() {
        assert!(!safe_name("../../windows/system32/evil.exe").contains(".."));
        assert!(!safe_name("..").is_empty());
        assert_eq!(safe_name("a b/c.mp4"), "a-b-c.mp4");
    }

    #[test]
    fn an_absurdly_long_name_is_cut_down() {
        let name = safe_name(&"a".repeat(4000));
        assert!(name.len() <= 120, "{}", name.len());
    }

    #[test]
    fn an_ordinary_name_survives() {
        assert_eq!(
            safe_name("remielle-dan-crimson-wings.3840x2160.mp4"),
            "remielle-dan-crimson-wings.3840x2160.mp4"
        );
    }
}
