//! Ticker module: fetches short text lines (news headlines, or any custom
//! feed) from one or more sources and makes them available to the render
//! loop, either as a scrolling ticker line (a chyron, like a news channel)
//! and/or the dedicated "news" preset layout.
//!
//! Sources (`ticker_source = ...` in trofeo.conf, comma-separated, same
//! convention as `layout = ...`):
//! - An `http(s)://` URL pointing to an RSS or Atom feed: headlines are
//!   extracted from each `<item>`/`<entry>` block's `<title>`.
//! - Any other `http(s)://` URL: the response body is not XML, so each
//!   non-empty line of the response is used directly as one ticker item —
//!   a simple way to plug in a script, webhook or plain-text endpoint
//!   without writing a dedicated integration for it.
//! - A local file path: each non-empty line (not starting with `#`) is one
//!   ticker item, re-read on every refresh — handy for a script that writes
//!   its own text file.
//!
//! Same background-thread pattern as `weather.rs`: a thread refreshes on a
//! timer and the render loop only ever reads the latest cached snapshot
//! (never blocks on network/disk I/O). Like `weather_city`, the source list
//! is read once at startup — changing `ticker_source`/`ticker_refresh_min`
//! requires a restart (see the check in `main.rs`).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(60);
/// Hard cap on how many items are kept/scrolled, regardless of how many a
/// feed returns — a big feed shouldn't grow memory or take minutes to loop.
const MAX_ITEMS: usize = 30;
/// Hard cap on a single item's length (characters) — a malformed/huge entry
/// shouldn't make one ticker item scroll forever.
const MAX_ITEM_LEN: usize = 200;

/// Shared handle: `spawn()` starts the background thread (a no-op thread if
/// there are no sources), `sample()` reads whatever it last fetched
/// (non-blocking, empty until the first fetch succeeds).
pub struct TickerMonitor {
    latest: Arc<Mutex<Vec<String>>>,
}

impl TickerMonitor {
    pub fn spawn(sources: Vec<String>, refresh_every: Duration) -> Self {
        let latest = Arc::new(Mutex::new(Vec::new()));
        if sources.is_empty() {
            return TickerMonitor { latest };
        }
        let latest_thread = Arc::clone(&latest);
        std::thread::spawn(move || loop {
            let mut items = Vec::new();
            for src in &sources {
                match fetch_source(src) {
                    Ok(mut v) => items.append(&mut v),
                    Err(e) => eprintln!("ticker: {src}: {e}"),
                }
            }
            items.truncate(MAX_ITEMS);
            let ok = !items.is_empty();
            *latest_thread.lock().unwrap() = items;
            // Network hiccups / a temporarily-empty feed: retry sooner than
            // the normal refresh interval, same as weather.rs.
            std::thread::sleep(if ok { refresh_every } else { RETRY_AFTER_ERROR });
        });
        TickerMonitor { latest }
    }

    /// Latest known items, if any fetch has produced at least one yet.
    pub fn sample(&self) -> Vec<String> {
        self.latest.lock().unwrap().clone()
    }
}

fn fetch_source(src: &str) -> Result<Vec<String>, String> {
    let trimmed = src.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let body = http_get(trimmed)?;
        let items = if looks_like_feed(&body) { parse_feed(&body) } else { lines_to_items(&body) };
        if items.is_empty() {
            return Err("no items found".to_string());
        }
        Ok(items)
    } else {
        let text = std::fs::read_to_string(Path::new(trimmed))
            .map_err(|e| format!("unable to read file: {e}"))?;
        Ok(lines_to_items(&text))
    }
}

fn http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    agent
        .get(url)
        .call()
        .map_err(|e| format!("request failed: {e}"))?
        .into_string()
        .map_err(|e| format!("reading response failed: {e}"))
}

/// RSS/Atom sniffing: good enough without a real content-type check (some
/// servers mislabel feeds as text/html, and we don't have HTTP headers from
/// `ureq`'s `into_string()` anymore anyway).
fn looks_like_feed(body: &str) -> bool {
    let trimmed = body.trim_start();
    let head_len = trimmed.len().min(200);
    let head = trimmed[..head_len].to_ascii_lowercase();
    head.starts_with("<?xml") || head.contains("<rss") || head.contains("<feed")
}

fn lines_to_items(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(truncate)
        .collect()
}

/// Pulls the first `<title>` out of each `<item>...</item>` (RSS) or
/// `<entry>...</entry>` (Atom) block — the feed's OWN title (outside any
/// item/entry) is intentionally skipped, only per-headline titles are kept.
/// A small hand-rolled scan rather than a full XML parser, same philosophy
/// as the hand-rolled JSON field extraction already used in `weather.rs`.
fn parse_feed(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for block in ["item", "entry"] {
        let open = format!("<{block}");
        let close = format!("</{block}>");
        let mut rest = xml;
        while let Some(start) = rest.find(&open) {
            let after_open = &rest[start..];
            let Some(end) = after_open.find(&close) else { break };
            let chunk = &after_open[..end];
            if let Some(title) = extract_title(chunk) {
                out.push(title);
            }
            rest = &after_open[end + close.len()..];
        }
    }
    out
}

fn extract_title(chunk: &str) -> Option<String> {
    let start = chunk.find("<title")?;
    let after = &chunk[start..];
    let tag_end = after.find('>')? + 1;
    let after_tag = &after[tag_end..];
    let end = after_tag.find("</title>")?;
    let raw = strip_cdata(after_tag[..end].trim());
    let text = decode_entities(raw.trim());
    (!text.is_empty()).then(|| truncate(&text))
}

fn strip_cdata(s: &str) -> &str {
    s.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")).unwrap_or(s)
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

fn truncate(s: &str) -> String {
    if s.chars().count() > MAX_ITEM_LEN {
        let mut t: String = s.chars().take(MAX_ITEM_LEN).collect();
        t.push('\u{2026}'); // "…"
        t
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_items() {
        let xml = r#"<rss><channel><title>Feed name (skipped)</title>
            <item><title>First headline &amp; more</title><description>x</description></item>
            <item><title><![CDATA[Second <headline>]]></title></item>
            </channel></rss>"#;
        assert_eq!(parse_feed(xml), vec!["First headline & more", "Second <headline>"]);
    }

    #[test]
    fn parses_atom_entries() {
        let xml = r#"<feed><title>Feed</title><entry><title type="text">Atom headline</title></entry></feed>"#;
        assert_eq!(parse_feed(xml), vec!["Atom headline"]);
    }

    #[test]
    fn skips_items_with_empty_title() {
        let xml = r#"<rss><channel><item><title>   </title></item><item><title>Real one</title></item></channel></rss>"#;
        assert_eq!(parse_feed(xml), vec!["Real one"]);
    }

    #[test]
    fn plain_text_falls_back_to_lines() {
        assert_eq!(lines_to_items("first\n\n# comment\nsecond  \n"), vec!["first", "second"]);
    }

    #[test]
    fn detects_feed_vs_plain_text() {
        assert!(looks_like_feed("<?xml version=\"1.0\"?><rss></rss>"));
        assert!(looks_like_feed("  <feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(!looks_like_feed("just some text\nline two"));
    }

    #[test]
    fn long_item_is_truncated() {
        let s = "x".repeat(500);
        let t = truncate(&s);
        assert_eq!(t.chars().count(), MAX_ITEM_LEN + 1);
        assert!(t.ends_with('\u{2026}'));
    }

    #[test]
    fn spawn_with_no_sources_samples_empty() {
        let m = TickerMonitor::spawn(Vec::new(), Duration::from_secs(1));
        assert!(m.sample().is_empty());
    }
}
