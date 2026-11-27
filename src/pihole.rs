//! Pi-hole 6 statistics: the numbers of the `pihole*` screens, of the `{pihole_...}` texts and of
//! the `pihole_percent` bars of the free layouts, and of the `pihole` panel of the classic ones.
//!
//! Runs on its own background thread, like the weather (the screen never waits for the
//! network): it asks `GET /api/stats/summary` every `pihole_interval` seconds, with the session
//! id (`sid`) that `POST /api/auth` gave. The same `sid` is used again and again; only a
//! `401` (the session has expired) makes it log in once more. Pi-hole keeps a limited number of
//! sessions, so logging in at every refresh would soon use them all up.
//!
//! No `serde`: the reply is read by the small JSON reader at the bottom of this file.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What `pihole_...` in the configuration says. `None` of it (no `pihole_url`) = the feature is off.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// `http://host:port`, no trailing slash, no `/admin`.
    pub url: String,
    /// The (app) password; empty = Pi-hole has none and the API is open.
    pub password: String,
    /// Seconds between two refreshes.
    pub interval: u32,
    /// Seconds to wait for Pi-hole to answer.
    pub timeout: u32,
}

pub const DEFAULT_INTERVAL: u32 = 15;
pub const DEFAULT_TIMEOUT: u32 = 5;
/// After a refused login (or too many requests) wait at least this long before trying again, so a
/// wrong password is not tried every few seconds.
const AUTH_BACKOFF_SECS: u64 = 60;

/// The address as the user may have written it: `192.168.1.2`, `pi.hole:8080/`, `http://pi/admin`.
pub fn normalize_url(raw: &str) -> Option<String> {
    let mut u = raw.trim().to_string();
    if u.is_empty() {
        return None;
    }
    let lower = u.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        u = format!("http://{u}");
    }
    let mut u = u.trim_end_matches('/').to_string();
    for tail in ["/admin", "/api"] {
        if u.to_ascii_lowercase().ends_with(tail) {
            u.truncate(u.len() - tail.len());
        }
    }
    let u = u.trim_end_matches('/').to_string();
    // something must follow the scheme
    let host = u.split_once("://").map(|(_, h)| h).unwrap_or("");
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    Some(u)
}

impl Settings {
    /// From the configuration: `Ok(None)` when `pihole_url` is not set (the feature is off).
    pub fn parse(file: &crate::config::ConfigFile) -> Result<Option<Settings>, String> {
        let raw = file.get("pihole_url").unwrap_or("").trim().to_string();
        let interval = match file.get_u32("pihole_interval")? {
            None => DEFAULT_INTERVAL,
            Some(v) if (5..=3600).contains(&v) => v,
            Some(v) => return Err(format!("pihole_interval: must be between 5 and 3600 seconds (got {v})")),
        };
        let timeout = match file.get_u32("pihole_timeout")? {
            None => DEFAULT_TIMEOUT,
            Some(v) if (1..=60).contains(&v) => v,
            Some(v) => return Err(format!("pihole_timeout: must be between 1 and 60 seconds (got {v})")),
        };
        if raw.is_empty() {
            return Ok(None);
        }
        let url = normalize_url(&raw).ok_or_else(|| format!("pihole_url: '{raw}' is not an address (try http://192.168.1.2:80)"))?;
        let password = file.get("pihole_password").unwrap_or("").to_string();
        Ok(Some(Settings { url, password, interval, timeout }))
    }
}

/// The figures of `GET /api/stats/summary`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stats {
    /// Queries in the last 24 hours.
    pub total: u64,
    pub blocked: u64,
    /// 0-100.
    pub percent: f32,
    /// Domains in the block lists (gravity).
    pub domains: u64,
    pub cached: u64,
    pub forwarded: u64,
    /// Clients that asked something.
    pub clients: u64,
}

/// How the last refresh went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// `pihole_url` is not set, or the first answer has not come yet.
    Waiting,
    Online,
    /// The password was refused.
    AuthFailed,
    /// Pi-hole did not answer (the text says why).
    Offline(String),
}

impl Status {
    /// The word shown by `{pihole_status}`.
    pub fn word(&self) -> &'static str {
        match self {
            Status::Waiting => "WAIT",
            Status::Online => "ONLINE",
            Status::AuthFailed => "LOGIN?",
            Status::Offline(_) => "OFFLINE",
        }
    }
}

/// What the screens read.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub stats: Option<Stats>,
    pub status: Status,
}

static STATE: Mutex<Snapshot> = Mutex::new(Snapshot { stats: None, status: Status::Waiting });
/// A sample for the pictures of `--preview-png` and the layout builder (no network).
static SAMPLE: Mutex<bool> = Mutex::new(false);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The latest figures and how the refresh went.
pub fn snapshot() -> Snapshot {
    if *lock(&SAMPLE) {
        return Snapshot { stats: Some(sample_stats()), status: Status::Online };
    }
    lock(&STATE).clone()
}

pub fn stats() -> Option<Stats> {
    snapshot().stats
}

/// What `{...}` in a text of a free layout can say about Pi-hole.
pub const PLACEHOLDERS: &[&str] = &[
    "pihole_queries",
    "pihole_blocked",
    "pihole_allowed",
    "pihole_percent",
    "pihole_cached_percent",
    "pihole_forwarded_percent",
    "pihole_domains",
    "pihole_cached",
    "pihole_forwarded",
    "pihole_clients",
    "pihole_status",
];

/// The numbers (0-100) a bar or a ring of a free layout can show (`value=...`).
pub const METRICS: &[&str] = &["pihole_percent", "pihole_cached_percent", "pihole_forwarded_percent"];

/// Share of the queries, in percent.
fn share(n: u64, total: u64) -> f32 {
    if total == 0 { 0.0 } else { (n as f32 * 100.0 / total as f32).clamp(0.0, 100.0) }
}

/// The number of one of [`METRICS`] (`None` before the first figures, or for another name).
pub fn metric(name: &str, snap: &Snapshot) -> Option<f32> {
    let s = snap.stats.as_ref()?;
    match name {
        "pihole_percent" => Some(s.percent),
        "pihole_cached_percent" => Some(share(s.cached, s.total)),
        "pihole_forwarded_percent" => Some(share(s.forwarded, s.total)),
        _ => None,
    }
}

/// The text of one of [`PLACEHOLDERS`] (`--` when there are no figures yet); `None` for any other name.
pub fn placeholder(name: &str, snap: &Snapshot) -> Option<String> {
    if !PLACEHOLDERS.contains(&name) {
        return None;
    }
    if name == "pihole_status" {
        return Some(snap.status.word().to_string());
    }
    let Some(s) = &snap.stats else { return Some("--".to_string()) };
    Some(match name {
        "pihole_queries" => compact(s.total),
        "pihole_blocked" => compact(s.blocked),
        "pihole_allowed" => compact(s.total.saturating_sub(s.blocked)),
        "pihole_percent" => format!("{}%", percent_text(s.percent)),
        "pihole_cached_percent" => format!("{}%", percent_text(share(s.cached, s.total))),
        "pihole_forwarded_percent" => format!("{}%", percent_text(share(s.forwarded, s.total))),
        "pihole_domains" => compact(s.domains),
        "pihole_cached" => compact(s.cached),
        "pihole_forwarded" => compact(s.forwarded),
        _ => compact(s.clients),
    })
}

/// Make the pictures that do not talk to the network (previews, the builder) show figures.
pub fn use_sample(on: bool) {
    *lock(&SAMPLE) = on;
}

pub fn sample_stats() -> Stats {
    Stats { total: 48_213, blocked: 11_876, percent: 24.6, domains: 1_062_447, cached: 17_390, forwarded: 18_947, clients: 14 }
}

fn publish(stats: Option<Stats>, status: Status) {
    let mut s = lock(&STATE);
    // keep showing the last good figures while Pi-hole is out of reach
    let stats = stats.or_else(|| s.stats.clone());
    *s = Snapshot { stats, status };
}

// ------------------------------------------------------------------ formatting

/// 987, 12 345, 123K, 4.2M: short enough for a tile.
pub fn compact(n: u64) -> String {
    match n {
        0..=99_999 => n.to_string(),
        100_000..=999_999 => format!("{}K", n / 1000),
        1_000_000..=99_999_999 => {
            let m = n as f64 / 1_000_000.0;
            if m < 10.0 { format!("{m:.2}M") } else { format!("{m:.1}M") }
        }
        _ => format!("{}M", n / 1_000_000),
    }
}

/// `24.6`, or `25` once it is 100 or more... a percentage for the screens: one decimal below 10,
/// none from 10 up (`4.2`, `24`).
pub fn percent_text(p: f32) -> String {
    if p < 10.0 { format!("{p:.1}") } else { format!("{p:.0}") }
}

// ------------------------------------------------------------------ the client

/// Why a refresh did not work.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The password was refused (or one is needed and there is none).
    Auth(String),
    /// Too many requests: wait.
    Busy,
    /// No answer, or one that cannot be read.
    Net(String),
}

/// Talks to one Pi-hole and remembers the session id between refreshes.
pub struct Client {
    pub settings: Settings,
    agent: ureq::Agent,
    sid: Option<String>,
    /// How many times it has logged in (the tests check that this stays small).
    pub logins: u32,
}

impl Client {
    pub fn new(settings: Settings) -> Client {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(settings.timeout as u64)).build();
        Client { settings, agent, sid: None, logins: 0 }
    }

    fn api(&self, path: &str) -> String {
        format!("{}/api/{path}", self.settings.url)
    }

    /// `POST /api/auth` -> the session id.
    fn login(&mut self) -> Result<(), Failure> {
        self.logins += 1;
        let body = format!("{{\"password\":{}}}", json_quote(&self.settings.password));
        let reply = self.agent.post(&self.api("auth")).set("Content-Type", "application/json").send_string(&body);
        let text = match reply {
            Ok(r) => r.into_string().map_err(|e| Failure::Net(format!("reading the login answer: {e}")))?,
            Err(ureq::Error::Status(401, r)) | Err(ureq::Error::Status(403, r)) => {
                let msg = r.into_string().ok().and_then(|t| session_message(&t)).unwrap_or_else(|| "password refused".into());
                return Err(Failure::Auth(msg));
            }
            Err(ureq::Error::Status(429, _)) => return Err(Failure::Busy),
            Err(ureq::Error::Status(code, _)) => return Err(Failure::Net(format!("login: HTTP {code}"))),
            Err(e) => return Err(Failure::Net(format!("login: {e}"))),
        };
        match parse_login(&text) {
            Ok(sid) => {
                self.sid = sid;
                Ok(())
            }
            Err(msg) => Err(Failure::Auth(msg)),
        }
    }

    /// One `GET /api/stats/summary`; `Err(None)` = the session is not (or no longer) valid.
    fn summary(&self) -> Result<Stats, Option<Failure>> {
        let mut req = self.agent.get(&self.api("stats/summary"));
        if let Some(sid) = &self.sid {
            req = req.set("X-FTL-SID", sid);
        }
        match req.call() {
            Ok(r) => {
                let text = r.into_string().map_err(|e| Some(Failure::Net(format!("reading the answer: {e}"))))?;
                parse_summary(&text).map_err(|e| Some(Failure::Net(e)))
            }
            Err(ureq::Error::Status(401, _)) => Err(None),
            Err(ureq::Error::Status(429, _)) => Err(Some(Failure::Busy)),
            Err(ureq::Error::Status(code, _)) => Err(Some(Failure::Net(format!("HTTP {code}")))),
            Err(e) => Err(Some(Failure::Net(e.to_string()))),
        }
    }

    /// One refresh. The `sid` of the last one is used again; only when Pi-hole answers `401` does
    /// it log in once more (and ask again), never otherwise.
    pub fn fetch(&mut self) -> Result<Stats, Failure> {
        if self.sid.is_none() && !self.settings.password.is_empty() {
            self.login()?;
        }
        match self.summary() {
            Ok(s) => Ok(s),
            Err(Some(f)) => Err(f),
            Err(None) => {
                // 401: the session has expired (or one is needed)
                self.sid = None;
                if self.settings.password.is_empty() {
                    return Err(Failure::Auth("Pi-hole wants a password (pihole_password)".into()));
                }
                self.login()?;
                match self.summary() {
                    Ok(s) => Ok(s),
                    Err(Some(f)) => Err(f),
                    Err(None) => {
                        self.sid = None;
                        Err(Failure::Auth("the session was refused again".into()))
                    }
                }
            }
        }
    }
}

/// Starts the thread that keeps the numbers up to date (does nothing without a `pihole_url`).
pub fn spawn(settings: Option<Settings>) {
    let Some(settings) = settings else { return };
    println!("Pi-hole: {} every {} s", settings.url, settings.interval);
    std::thread::spawn(move || {
        let interval = Duration::from_secs(settings.interval as u64);
        let mut client = Client::new(settings);
        // what the console said last, so that it speaks only when something changes
        let mut said = String::new();
        let mut say = |text: String| {
            if text != said {
                println!("Pi-hole: {text}");
                said = text;
            }
        };
        loop {
            let started = Instant::now();
            let wait = match client.fetch() {
                Ok(stats) => {
                    publish(Some(stats), Status::Online);
                    say("online".into());
                    interval
                }
                Err(f @ Failure::Auth(_)) => {
                    say(describe(&f));
                    publish(None, Status::AuthFailed);
                    interval.max(Duration::from_secs(AUTH_BACKOFF_SECS))
                }
                Err(f @ Failure::Busy) => {
                    say(describe(&f));
                    publish(None, Status::Offline("too many requests".into()));
                    interval.max(Duration::from_secs(AUTH_BACKOFF_SECS))
                }
                Err(Failure::Net(msg)) => {
                    say(format!("not reachable ({msg})"));
                    publish(None, Status::Offline(msg));
                    interval
                }
            };
            std::thread::sleep(wait.saturating_sub(started.elapsed()).max(Duration::from_secs(1)));
        }
    });
}

/// A failure in words, for the console.
pub fn describe(failure: &Failure) -> String {
    match failure {
        Failure::Auth(m) => format!("login refused: {m}"),
        Failure::Busy => "too many requests".into(),
        Failure::Net(m) => m.clone(),
    }
}

// ------------------------------------------------------------------ the replies

fn parse_login(text: &str) -> Result<Option<String>, String> {
    let v = Json::parse(text).map_err(|e| format!("login answer: {e}"))?;
    let session = v.get("session");
    let valid = session.and_then(|s| s.get("valid")).and_then(Json::as_bool).unwrap_or(false);
    if !valid {
        let msg = session.and_then(|s| s.get("message")).and_then(Json::as_str).unwrap_or("password refused");
        return Err(msg.to_string());
    }
    // `sid` is null when Pi-hole has no password: then there is nothing to send
    Ok(session.and_then(|s| s.get("sid")).and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_string))
}

fn session_message(text: &str) -> Option<String> {
    let v = Json::parse(text).ok()?;
    v.get("session")?.get("message")?.as_str().map(str::to_string).or_else(|| v.get("error")?.get("message")?.as_str().map(str::to_string))
}

/// The numbers of `/api/stats/summary`.
pub fn parse_summary(text: &str) -> Result<Stats, String> {
    let v = Json::parse(text).map_err(|e| format!("summary: {e}"))?;
    let q = v.get("queries").ok_or("summary: no \"queries\" in the answer (is this Pi-hole 6?)")?;
    let n = |o: Option<&Json>, key: &str| o.and_then(|j| j.get(key)).and_then(Json::as_f64);
    let total = n(Some(q), "total").ok_or("summary: no queries.total")?.max(0.0) as u64;
    let blocked = n(Some(q), "blocked").unwrap_or(0.0).max(0.0) as u64;
    let percent = n(Some(q), "percent_blocked").map(|p| p as f32).unwrap_or(if total > 0 { blocked as f32 * 100.0 / total as f32 } else { 0.0 });
    Ok(Stats {
        total,
        blocked,
        percent: percent.clamp(0.0, 100.0),
        domains: n(v.get("gravity"), "domains_being_blocked").unwrap_or(0.0).max(0.0) as u64,
        cached: n(Some(q), "cached").unwrap_or(0.0).max(0.0) as u64,
        forwarded: n(Some(q), "forwarded").unwrap_or(0.0).max(0.0) as u64,
        clients: n(v.get("clients"), "active").unwrap_or(0.0).max(0.0) as u64,
    })
}

// ------------------------------------------------------------------ a small JSON reader

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("unexpected text at {}", p.i));
        }
        Ok(v)
    }
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        if let Json::Num(n) = self { Some(*n) } else { None }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }
}

/// A JSON string with the quotes.
pub fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        let Some(&c) = self.s.get(self.i) else { return Err("the text ends too soon".into()) };
        match c {
            b'{' => {
                self.depth += 1;
                if self.depth > 32 {
                    return Err("nested too deep".into());
                }
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.eat("}") {
                        break;
                    }
                    if !items.is_empty() && !self.eat(",") {
                        return Err(format!("expected , at {}", self.i));
                    }
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err(format!("expected : at {}", self.i));
                    }
                    let v = self.value()?;
                    items.push((k, v));
                }
                self.depth -= 1;
                Ok(Json::Obj(items))
            }
            b'[' => {
                self.depth += 1;
                if self.depth > 32 {
                    return Err("nested too deep".into());
                }
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.eat("]") {
                        break;
                    }
                    if !items.is_empty() && !self.eat(",") {
                        return Err(format!("expected , at {}", self.i));
                    }
                    items.push(self.value()?);
                }
                self.depth -= 1;
                Ok(Json::Arr(items))
            }
            b'"' => Ok(Json::Str(self.string()?)),
            b't' if self.eat("true") => Ok(Json::Bool(true)),
            b'f' if self.eat("false") => Ok(Json::Bool(false)),
            b'n' if self.eat("null") => Ok(Json::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .and_then(|t| t.parse::<f64>().ok())
                    .map(Json::Num)
                    .ok_or_else(|| format!("bad number at {start}"))
            }
            _ => Err(format!("unexpected '{}' at {}", c as char, self.i)),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if !self.eat("\"") {
            return Err(format!("expected a string at {}", self.i));
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err("a string is not closed".into()) };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return Err("a string is not closed".into()) };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u32::from_str_radix(h, 16).ok());
                            let Some(cp) = hex else { return Err("bad \\u escape".into()) };
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| "a string is not text".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const SUMMARY: &str = r#"{"queries":{"total":48213,"blocked":11876,"percent_blocked":24.632,"unique_domains":3011,"forwarded":18947,"cached":17390,"frequency":0.5,"types":{"A":1},"status":{"UNKNOWN":0},"replies":{"NODATA":3}},"clients":{"active":14,"total":31},"gravity":{"domains_being_blocked":1062447,"last_update":1700000000},"took":0.002}"#;

    #[test]
    fn the_summary_is_read() {
        let s = parse_summary(SUMMARY).unwrap();
        assert_eq!((s.total, s.blocked, s.domains, s.cached, s.forwarded, s.clients), (48213, 11876, 1_062_447, 17390, 18947, 14));
        assert!((s.percent - 24.632).abs() < 0.001);
        // the clients' own "total" must not be taken for the queries'
        let swapped = r#"{"clients":{"total":5,"active":1},"queries":{"total":100,"blocked":25}}"#;
        let s = parse_summary(swapped).unwrap();
        assert_eq!((s.total, s.blocked), (100, 25));
        assert!((s.percent - 25.0).abs() < 0.001, "the percentage is worked out when Pi-hole does not give it");
        assert!(parse_summary("{}").unwrap_err().contains("Pi-hole 6"));
        assert!(parse_summary("not json").is_err());
    }

    #[test]
    fn the_login_answer_is_read() {
        assert_eq!(parse_login(r#"{"session":{"valid":true,"totp":false,"sid":"abc/DEF+1=","csrf":"x","validity":300,"message":"success"},"took":0.1}"#), Ok(Some("abc/DEF+1=".into())));
        assert_eq!(parse_login(r#"{"session":{"valid":true,"sid":null,"message":"no password set"}}"#), Ok(None));
        assert_eq!(parse_login(r#"{"session":{"valid":false,"sid":null,"message":"password incorrect"}}"#), Err("password incorrect".into()));
    }

    #[test]
    fn the_json_reader_handles_escapes_and_nesting() {
        let j = Json::parse(r#" {"a":[1,2.5,-3e2,true,false,null,"x\"yè\n"],"b":{"c":{}}} "#).unwrap();
        let Json::Arr(a) = j.get("a").unwrap() else { panic!() };
        assert_eq!(a[2], Json::Num(-300.0));
        assert_eq!(a[6].as_str(), Some("x\"y\u{e8}\n"));
        assert!(Json::parse("{\"a\":1,}").is_err());
        assert!(Json::parse("[1 2]").is_err());
        assert!(Json::parse(&"[".repeat(100)).is_err());
        assert_eq!(json_quote("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn the_address_is_tidied() {
        assert_eq!(normalize_url("http://192.168.178.10:8080").as_deref(), Some("http://192.168.178.10:8080"));
        assert_eq!(normalize_url(" pi.hole:8080/ ").as_deref(), Some("http://pi.hole:8080"));
        assert_eq!(normalize_url("https://pi/admin/").as_deref(), Some("https://pi"));
        assert_eq!(normalize_url("http://pi/api").as_deref(), Some("http://pi"));
        assert_eq!(normalize_url(""), None);
        assert_eq!(normalize_url("http://"), None);
    }

    #[test]
    fn the_options_are_read_and_checked() {
        use crate::config::ConfigFile;
        let f = ConfigFile::parse("pihole_url = 192.168.178.10:8080\npihole_password = \"a#b\"\npihole_interval = 30\n").unwrap();
        let s = Settings::parse(&f).unwrap().unwrap();
        assert_eq!((s.url.as_str(), s.password.as_str(), s.interval, s.timeout), ("http://192.168.178.10:8080", "a#b", 30, DEFAULT_TIMEOUT));
        assert_eq!(Settings::parse(&ConfigFile::parse("layout = pihole\n").unwrap()).unwrap(), None);
        assert!(Settings::parse(&ConfigFile::parse("pihole_url = http://x\npihole_interval = 2\n").unwrap()).is_err());
        assert!(Settings::parse(&ConfigFile::parse("pihole_url = http://x\npihole_timeout = 0\n").unwrap()).is_err());
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(compact(987), "987");
        assert_eq!(compact(48_213), "48213");
        assert_eq!(compact(123_456), "123K");
        assert_eq!(compact(1_062_447), "1.06M");
        assert_eq!(compact(12_345_678), "12.3M");
        assert_eq!(percent_text(4.23), "4.2");
        assert_eq!(percent_text(24.6), "25");
    }

    /// A very small Pi-hole: answers what `script` says, one request per connection, and writes down
    /// the requests it got (`METHOD path sid-header`).
    struct Fake {
        url: String,
        log: Arc<Mutex<Vec<String>>>,
    }

    fn fake(script: Vec<(u16, String)>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = Arc::clone(&log);
        std::thread::spawn(move || {
            for (code, body) in script {
                let Ok((mut sock, _)) = listener.accept() else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 2048];
                // read the head, then the body (Content-Length)
                loop {
                    let n = sock.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                let method_path: Vec<&str> = first.split(' ').collect();
                let sid = text.lines().find_map(|l| l.to_ascii_lowercase().starts_with("x-ftl-sid:").then(|| l.split_once(':').unwrap().1.trim().to_string())).unwrap_or_default();
                let body_sent = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                log2.lock().unwrap().push(format!("{} {} [{}] {}", method_path[0], method_path.get(1).unwrap_or(&""), sid, body_sent));
                let reason = if code == 200 { "OK" } else { "Error" };
                let reply = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(reply.as_bytes());
            }
        });
        Fake { url, log }
    }

    fn login_ok(sid: &str) -> (u16, String) {
        (200, format!(r#"{{"session":{{"valid":true,"sid":"{sid}","message":"success"}}}}"#))
    }

    fn settings(url: &str, password: &str) -> Settings {
        Settings { url: url.to_string(), password: password.to_string(), interval: 15, timeout: 3 }
    }

    #[test]
    fn it_logs_in_once_and_reuses_the_session() {
        let f = fake(vec![login_ok("SID1"), (200, SUMMARY.into()), (200, SUMMARY.into()), (200, SUMMARY.into())]);
        let mut c = Client::new(settings(&f.url, "p\"w"));
        for _ in 0..3 {
            assert_eq!(c.fetch().unwrap().total, 48213);
        }
        assert_eq!(c.logins, 1, "one login for three refreshes");
        let log = f.log.lock().unwrap().clone();
        assert!(log[0].starts_with("POST /api/auth []") && log[0].contains(r#"{"password":"p\"w"}"#), "{log:?}");
        for l in &log[1..] {
            assert!(l.starts_with("GET /api/stats/summary [SID1]"), "{l}");
        }
    }

    #[test]
    fn a_401_makes_it_log_in_again_once() {
        let f = fake(vec![login_ok("OLD"), (200, SUMMARY.into()), (401, r#"{"error":{"key":"unauthorized"}}"#.into()), login_ok("NEW"), (200, SUMMARY.into()), (200, SUMMARY.into())]);
        let mut c = Client::new(settings(&f.url, "pw"));
        c.fetch().unwrap();
        c.fetch().unwrap(); // the 401 happens here, and is mended inside this refresh
        c.fetch().unwrap();
        assert_eq!(c.logins, 2);
        let log = f.log.lock().unwrap().clone();
        assert!(log[2].contains("[OLD]") && log[3].starts_with("POST /api/auth") && log[4].contains("[NEW]") && log[5].contains("[NEW]"), "{log:?}");
    }

    #[test]
    fn a_refused_password_is_reported_and_not_repeated_inside_a_refresh() {
        let f = fake(vec![(401, r#"{"session":{"valid":false,"sid":null,"message":"password incorrect"}}"#.into())]);
        let mut c = Client::new(settings(&f.url, "wrong"));
        assert_eq!(c.fetch(), Err(Failure::Auth("password incorrect".into())));
        assert_eq!(c.logins, 1);
        assert_eq!(f.log.lock().unwrap().len(), 1, "no request after the refusal");
    }

    #[test]
    fn without_a_password_nothing_is_sent_to_log_in() {
        let f = fake(vec![(200, SUMMARY.into()), (401, "{}".into())]);
        let mut c = Client::new(settings(&f.url, ""));
        assert_eq!(c.fetch().unwrap().blocked, 11876);
        assert!(matches!(c.fetch(), Err(Failure::Auth(_))), "a 401 with no password to try");
        assert_eq!(c.logins, 0);
        assert!(f.log.lock().unwrap().iter().all(|l| l.starts_with("GET ")));
    }

    #[test]
    fn a_server_that_is_not_there_is_a_network_failure() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        let mut c = Client::new(settings(&url, ""));
        assert!(matches!(c.fetch(), Err(Failure::Net(_))));
        let f = fake(vec![(429, "{}".into())]);
        assert_eq!(Client::new(settings(&f.url, "")).fetch(), Err(Failure::Busy));
    }

    #[test]
    fn the_last_good_figures_stay_while_pihole_is_away() {
        publish(Some(sample_stats()), Status::Online);
        publish(None, Status::Offline("x".into()));
        // (not snapshot(): another test may be making a preview with the sample on)
        let s = lock(&STATE).clone();
        assert_eq!(s.status, Status::Offline("x".into()));
        assert_eq!(s.stats, Some(sample_stats()));
    }

    #[test]
    fn the_texts_for_the_layouts() {
        let snap = Snapshot { stats: Some(sample_stats()), status: Status::Online };
        assert_eq!(placeholder("pihole_queries", &snap).as_deref(), Some("48213"));
        assert_eq!(placeholder("pihole_blocked", &snap).as_deref(), Some("11876"));
        assert_eq!(placeholder("pihole_allowed", &snap).as_deref(), Some("36337"));
        assert_eq!(placeholder("pihole_percent", &snap).as_deref(), Some("25%"));
        assert_eq!(placeholder("pihole_domains", &snap).as_deref(), Some("1.06M"));
        assert_eq!(placeholder("pihole_status", &snap).as_deref(), Some("ONLINE"));
        assert_eq!(placeholder("cpu", &snap), None);
        assert_eq!(placeholder("pihole_cached_percent", &snap).as_deref(), Some("36%"));
        assert_eq!(metric("pihole_forwarded_percent", &snap).map(|v| v.round()), Some(39.0));
        assert_eq!(metric("pihole_percent", &snap), Some(24.6));
        assert_eq!(metric("cpu", &snap), None);
        let none = Snapshot { stats: None, status: Status::AuthFailed };
        assert_eq!(metric("pihole_percent", &none), None);
        assert_eq!(placeholder("pihole_queries", &none).as_deref(), Some("--"));
        assert_eq!(placeholder("pihole_status", &none).as_deref(), Some("LOGIN?"));
        for n in PLACEHOLDERS {
            assert!(placeholder(n, &snap).is_some(), "{n}");
        }
    }
}
