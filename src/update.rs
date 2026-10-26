//! "A new version is available" notice.
//!
//! Optional (`update_check = true`, off by default). A background thread asks GitHub for the
//! latest release of the project (one small HTTPS request, nothing is sent about the computer),
//! compares its tag with the version of this program and, when the release is newer, hands the
//! version over to the render loop, which shows a banner at the bottom of the screen for
//! `update_notice_secs` seconds. The program never downloads or installs anything.
//!
//! Same pattern as `weather.rs` / `ticker.rs`: the render loop only reads a shared slot and
//! never waits for the network. A failed request (no network, GitHub down, rate limit) is
//! silent and retried a few times; there is never an error on the screen.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The version of this program.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Where the releases are published.
const LATEST_URL: &str = "https://api.github.com/repos/haven80/trofeo-lcd-rs/releases/latest";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// A failed check is retried after this long, up to `MAX_TRIES` times in a row.
const RETRY_AFTER: Duration = Duration::from_secs(120);
const MAX_TRIES: u32 = 5;

/// `"v2.5.1"`, `"2.5.1"` or `"2.5"` → `(2, 5, 1)`. A suffix such as `-beta` is ignored.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let core = s.split(|c: char| c == '-' || c == '+' || c.is_whitespace()).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    let patch = parts.next().map(|p| p.parse().ok()).unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

/// Is `latest` a higher version than `current`? Unreadable versions are never "newer".
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// The `tag_name` of a GitHub "latest release" answer.
pub fn tag_from_json(body: &str) -> Option<String> {
    let idx = body.find("\"tag_name\"")?;
    let after = body[idx + "\"tag_name\"".len()..].trim_start().strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    let tag = after[..end].trim();
    (!tag.is_empty()).then(|| tag.to_string())
}

fn fetch_latest_tag() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    let body = agent
        .get(LATEST_URL)
        .set("User-Agent", &format!("trofeo_lcd/{CURRENT}"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("update check failed: {e}"))?
        .into_string()
        .map_err(|e| format!("update check: reading the answer failed: {e}"))?;
    tag_from_json(&body).ok_or_else(|| "update check: no tag_name in the answer".to_string())
}

/// Handle on the background check. `take_new()` returns a newer version once.
pub struct UpdateMonitor {
    pending: Arc<Mutex<Option<String>>>,
}

impl UpdateMonitor {
    /// Starts the check. `every_hours == 0`: only once, at start-up; otherwise it asks again
    /// every that many hours (a version already announced is not announced twice).
    pub fn spawn(every_hours: u32) -> Self {
        let pending = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&pending);
        std::thread::spawn(move || {
            let mut announced: Option<String> = None;
            let mut tries = 0u32;
            loop {
                match fetch_latest_tag() {
                    Ok(tag) => {
                        tries = 0;
                        let version = tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string();
                        if is_newer(&version, CURRENT) && announced.as_deref() != Some(version.as_str()) {
                            println!("A newer version is available: {version} (this is {CURRENT})");
                            announced = Some(version.clone());
                            *slot.lock().unwrap() = Some(version);
                        }
                        if every_hours == 0 {
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(every_hours as u64 * 3600));
                    }
                    Err(_) => {
                        tries += 1;
                        if tries >= MAX_TRIES {
                            if every_hours == 0 {
                                return;
                            }
                            tries = 0;
                            std::thread::sleep(Duration::from_secs(every_hours as u64 * 3600));
                        } else {
                            std::thread::sleep(RETRY_AFTER);
                        }
                    }
                }
            }
        });
        UpdateMonitor { pending }
    }

    /// A monitor that never finds anything (update check off).
    pub fn off() -> Self {
        UpdateMonitor { pending: Arc::new(Mutex::new(None)) }
    }

    /// Pretends that `version` was just found (`--update-demo`).
    pub fn demo(version: &str) -> Self {
        UpdateMonitor { pending: Arc::new(Mutex::new(Some(version.to_string()))) }
    }

    /// The newly found version, once; `None` the rest of the time.
    pub fn take_new(&self) -> Option<String> {
        self.pending.lock().unwrap().take()
    }
}

/// The words of the banner in the interface language (the bitmap font is uppercase, no accents).
pub fn banner_text(lang: crate::i18n::Lang, latest: &str, current: &str) -> String {
    use crate::i18n::Lang::*;
    match lang {
        En => format!("NEW VERSION {latest} AVAILABLE (YOU HAVE {current})"),
        It => format!("NUOVA VERSIONE {latest} DISPONIBILE (HAI LA {current})"),
        De => format!("NEUE VERSION {latest} VERFUEGBAR (INSTALLIERT: {current})"),
        Es => format!("NUEVA VERSION {latest} DISPONIBLE (TIENES LA {current})"),
        Fr => format!("NOUVELLE VERSION {latest} DISPONIBLE (VOUS AVEZ LA {current})"),
        Pt => format!("NOVA VERSAO {latest} DISPONIVEL (VOCE TEM A {current})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_by_number_not_by_text() {
        assert!(is_newer("2.5.2", "2.5.1"));
        assert!(is_newer("v2.10.0", "2.9.9"));
        assert!(is_newer("3.0", "2.99.99"));
        assert!(!is_newer("2.5.1", "2.5.1"));
        assert!(!is_newer("2.5.0", "2.5.1"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }

    #[test]
    fn odd_tags_are_never_newer() {
        assert!(!is_newer("nightly", "2.5.1"));
        assert!(!is_newer("", "2.5.1"));
        assert!(!is_newer("2.5.1", "garbage"));
        assert_eq!(parse_version("v2.6.0-beta.1"), Some((2, 6, 0)));
        assert_eq!(parse_version(" V2.6 "), Some((2, 6, 0)));
    }

    #[test]
    fn tag_is_read_from_the_github_answer() {
        let body = r#"{"url":"https://api.github.com/x","id":1,"tag_name": "v2.6.0","name":"v2.6.0 notes","draft":false}"#;
        assert_eq!(tag_from_json(body).as_deref(), Some("v2.6.0"));
        assert_eq!(tag_from_json(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_from_json(r#"{"tag_name":""}"#), None);
    }

    #[test]
    fn the_program_version_is_readable() {
        assert!(parse_version(CURRENT).is_some(), "{CURRENT}");
    }

    #[test]
    fn demo_announces_once() {
        let m = UpdateMonitor::demo("9.9.9");
        assert_eq!(m.take_new().as_deref(), Some("9.9.9"));
        assert_eq!(m.take_new(), None);
        assert_eq!(UpdateMonitor::off().take_new(), None);
    }

    #[test]
    fn banner_is_in_every_language_and_in_capitals() {
        use crate::i18n::Lang::*;
        for l in [En, It, De, Es, Fr, Pt] {
            let t = banner_text(l, "2.6.0", "2.5.1");
            assert!(t.contains("2.6.0") && t.contains("2.5.1"), "{t}");
            assert_eq!(t, t.to_uppercase(), "{t}");
            assert!(t.is_ascii(), "{t}");
        }
    }
}
