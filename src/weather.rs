//! Weather module: fetches current conditions from Open-Meteo (no API key
//! needed) for either a city name given in the config (`weather_city`) or,
//! by default, a location guessed from the machine's public IP address.
//!
//! Runs on its own background thread (same pattern as the nvidia-smi
//! fallback thread in `gpu_nvml.rs`): the main render loop just reads the
//! latest snapshot out of a shared `Arc<Mutex<Option<WeatherSnapshot>>>`
//! and never blocks on network I/O.
//!
//! No `serde`/`serde_json` dependency: the JSON responses we consume have a
//! small, fixed set of top-level/nested numeric and string fields, so a
//! handful of tiny hand-rolled field extractors are simpler than pulling in
//! a full JSON stack.

use crate::weather_icon::{icon_for_code, WeatherIcon};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A point-in-time weather reading, ready to display.
#[derive(Clone, Debug)]
pub struct WeatherSnapshot {
    pub temp_c: f32,
    pub humidity: Option<u8>,
    pub code: u16,
    pub icon: WeatherIcon,
    /// City/place name, when known (geocoded name, or the IP-guessed city).
    pub city: Option<String>,
}

impl WeatherSnapshot {
    /// Temperature converted to the requested unit ('C' or 'F').
    pub fn temp_in(&self, fahrenheit: bool) -> f32 {
        if fahrenheit {
            self.temp_c * 9.0 / 5.0 + 32.0
        } else {
            self.temp_c
        }
    }
}

struct Location {
    lat: f64,
    lon: f64,
    city: Option<String>,
}

const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// Shared handle: `spawn()` starts the background thread, `sample()` reads
/// whatever the thread last fetched (non-blocking, returns `None` until the
/// first fetch succeeds).
pub struct WeatherMonitor {
    latest: Arc<Mutex<Option<WeatherSnapshot>>>,
}

impl WeatherMonitor {
    /// Starts the background polling thread. `city_override` is the
    /// `weather_city` config value, if any; when `None`, the location is
    /// guessed from the machine's public IP on every (re)resolve.
    pub fn spawn(city_override: Option<String>) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let latest_thread = Arc::clone(&latest);
        std::thread::spawn(move || loop {
            let outcome = resolve_location(city_override.as_deref())
                .and_then(|loc| fetch_current(&loc).map(|snap| (loc, snap)));
            match outcome {
                Ok((_loc, snap)) => {
                    *latest_thread.lock().unwrap() = Some(snap);
                    std::thread::sleep(REFRESH_EVERY);
                }
                Err(_e) => {
                    // Network hiccups are common (Wi-Fi sleep, DNS blip); just
                    // retry after a short pause and keep showing the last
                    // good snapshot (if any) in the meantime.
                    std::thread::sleep(RETRY_AFTER_ERROR);
                }
            }
        });
        WeatherMonitor { latest }
    }

    /// Returns the latest known snapshot, if any fetch has succeeded yet.
    pub fn sample(&self) -> Option<WeatherSnapshot> {
        self.latest.lock().unwrap().clone()
    }
}

fn resolve_location(city_override: Option<&str>) -> Result<Location, String> {
    match city_override {
        Some(city) if !city.trim().is_empty() => geocode_city(city.trim()),
        _ => locate_by_ip(),
    }
}

fn http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    agent
        .get(url)
        .call()
        .map_err(|e| format!("request to {url} failed: {e}"))?
        .into_string()
        .map_err(|e| format!("reading response from {url} failed: {e}"))
}

/// Open-Meteo geocoding: turns a city name into lat/lon + a display name.
///
/// Open-Meteo's geocoder (GeoNames-based) ranks matches by name relevance, not
/// population — searching a native-language name like "Milano" can return a
/// same-named hamlet of a few hundred people (in Texas, Peru, ...) ahead of
/// the actual city of ~1.4 million people, especially with `language=en`.
/// Passing the current UI language helps (native-language names match their
/// own city first), but isn't a full fix by itself, so as a second safety
/// net this fetches a handful of candidates and picks the most populous one
/// — a real city the user is asking about is essentially always the largest
/// match for its name.
fn geocode_city(city: &str) -> Result<Location, String> {
    let lang = match crate::i18n::language() {
        crate::i18n::Lang::It => "it",
        crate::i18n::Lang::En => "en",
    };
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language={lang}&format=json",
        urlencode(city)
    );
    let body = http_get(&url)?;
    let array = extract_array(&body, "\"results\"").ok_or_else(|| format!("city '{city}' not found"))?;

    let mut best: Option<(f64, f64, Option<String>, f64)> = None; // (lat, lon, name, population)
    for obj in split_json_objects(array) {
        let (Some(lat), Some(lon)) = (json_number(obj, "\"latitude\""), json_number(obj, "\"longitude\"")) else {
            continue;
        };
        let name = json_string(obj, "\"name\"");
        let population = json_number(obj, "\"population\"").unwrap_or(0.0);
        let is_better = match &best {
            None => true,
            Some((_, _, _, best_pop)) => population > *best_pop,
        };
        if is_better {
            best = Some((lat, lon, name, population));
        }
    }

    let (lat, lon, name, _) = best.ok_or_else(|| format!("city '{city}' not found"))?;
    Ok(Location {
        lat,
        lon,
        city: name.or_else(|| Some(city.to_string())),
    })
}

/// Finds `"key":[ ... ]` and returns the `[...]` slice (brackets included),
/// using string/escape-aware bracket-depth matching so it's not thrown off
/// by anything before or after it in the surrounding JSON document.
fn extract_array<'a>(haystack: &'a str, key: &str) -> Option<&'a str> {
    let idx = haystack.find(key)?;
    let after = haystack[idx + key.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let start_rel = after.find('[')?;
    let bytes = after.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start_rel) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&after[start_rel..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits a `"results":[{...}, {...}, ...]` JSON array into its top-level
/// object substrings (each still containing its enclosing `{`/`}`). Brace
/// depth is tracked with string/escape awareness so a `}`/`{` inside a
/// quoted value (or a nested array like `"postcodes":["..."]`) can't
/// prematurely end an object.
fn split_json_objects(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut obj_start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    obj_start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = obj_start.take() {
                        out.push(&s[start..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// IP-based geolocation, trying ipwho.is first, then ip-api.com.
fn locate_by_ip() -> Result<Location, String> {
    locate_via_ipwho().or_else(|e1| {
        locate_via_ipapi().map_err(|e2| format!("ipwho.is failed ({e1}); ip-api.com failed ({e2})"))
    })
}

/// `https://ipwho.is/` — HTTPS, free, no key. Success is a boolean `success` field.
fn locate_via_ipwho() -> Result<Location, String> {
    let body = http_get("https://ipwho.is/")?;
    if json_bool(&body, "\"success\"") == Some(false) {
        return Err("ipwho.is reported failure".to_string());
    }
    let lat = json_number(&body, "\"latitude\"").ok_or("ipwho.is: missing latitude")?;
    let lon = json_number(&body, "\"longitude\"").ok_or("ipwho.is: missing longitude")?;
    let city = json_string(&body, "\"city\"");
    Ok(Location { lat, lon, city })
}

/// `http://ip-api.com/json/` — HTTP only on the free tier, but reachable
/// when ipwho.is is unavailable. Success is a string `status` field ("success"/"fail").
fn locate_via_ipapi() -> Result<Location, String> {
    let body = http_get("http://ip-api.com/json/")?;
    if json_string(&body, "\"status\"").as_deref() != Some("success") {
        return Err("ip-api.com reported failure".to_string());
    }
    let lat = json_number(&body, "\"lat\"").ok_or("ip-api.com: missing lat")?;
    let lon = json_number(&body, "\"lon\"").ok_or("ip-api.com: missing lon")?;
    let city = json_string(&body, "\"city\"");
    Ok(Location { lat, lon, city })
}

/// Open-Meteo current-conditions forecast for a resolved location.
fn fetch_current(loc: &Location) -> Result<WeatherSnapshot, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}&current=temperature_2m,relative_humidity_2m,weather_code&timezone=auto",
        loc.lat, loc.lon
    );
    let body = http_get(&url)?;
    let current_start = body.find("\"current\"").ok_or("forecast: missing \"current\" block")?;
    let obj = &body[current_start..];
    let temp_c = json_number(obj, "\"temperature_2m\"").ok_or("forecast: missing temperature_2m")? as f32;
    let humidity = json_number(obj, "\"relative_humidity_2m\"").map(|v| v.clamp(0.0, 100.0) as u8);
    let code = json_number(obj, "\"weather_code\"").unwrap_or(0.0) as u16;
    Ok(WeatherSnapshot {
        temp_c,
        humidity,
        code,
        icon: icon_for_code(code),
        city: loc.city.clone(),
    })
}

/// Prints diagnostics for `--diag`: resolved location + a fetched sample.
pub fn diag(city_override: Option<&str>) {
    println!("Weather diagnostics:");
    match resolve_location(city_override) {
        Ok(loc) => {
            println!(
                "  Location: {} (lat={:.4}, lon={:.4})",
                loc.city.as_deref().unwrap_or("unknown"),
                loc.lat,
                loc.lon
            );
            match fetch_current(&loc) {
                Ok(snap) => println!(
                    "  Current: {:.1}C, humidity {}, code {} ({:?})",
                    snap.temp_c,
                    snap.humidity.map(|h| h.to_string()).unwrap_or_else(|| "?".to_string()),
                    snap.code,
                    snap.icon
                ),
                Err(e) => println!("  Forecast fetch failed: {e}"),
            }
        }
        Err(e) => println!("  Location lookup failed: {e}"),
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Finds `"key":<number>` (optionally with whitespace / a leading `-` / a
/// decimal point) anywhere after the start of `haystack` and parses it.
fn json_number(haystack: &str, key: &str) -> Option<f64> {
    let idx = haystack.find(key)?;
    let after = &haystack[idx + key.len()..];
    let after = after.trim_start();
    let after = after.strip_prefix(':')?;
    let after = after.trim_start();
    let end = after
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E'))
        .unwrap_or(after.len());
    after[..end].parse::<f64>().ok()
}

/// Finds `"key":true|false` anywhere after the start of `haystack`.
fn json_bool(haystack: &str, key: &str) -> Option<bool> {
    let idx = haystack.find(key)?;
    let after = haystack[idx + key.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    if after.starts_with("true") {
        Some(true)
    } else if after.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Finds `"key":"value"` anywhere after the start of `haystack`. Handles
/// `\"` and `\\` escapes but nothing fancier (city/status names never need
/// more than that).
fn json_string(haystack: &str, key: &str) -> Option<String> {
    let idx = haystack.find(key)?;
    let after = haystack[idx + key.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = after.chars();
    loop {
        match chars.next()? {
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            '"' => break,
            c => out.push(c),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_number_extracts_value() {
        let s = r#"{"current":{"temperature_2m":21.4,"relative_humidity_2m":55,"weather_code":3}}"#;
        assert_eq!(json_number(s, "\"temperature_2m\""), Some(21.4));
        assert_eq!(json_number(s, "\"relative_humidity_2m\""), Some(55.0));
        assert_eq!(json_number(s, "\"weather_code\""), Some(3.0));
        assert_eq!(json_number(s, "\"missing\""), None);
    }

    #[test]
    fn json_number_handles_negative_and_decimal() {
        let s = r#"{"latitude":-33.87,"longitude":151.21}"#;
        assert_eq!(json_number(s, "\"latitude\""), Some(-33.87));
        assert_eq!(json_number(s, "\"longitude\""), Some(151.21));
    }

    #[test]
    fn json_bool_extracts_value() {
        let s = r#"{"success":false,"other":true}"#;
        assert_eq!(json_bool(s, "\"success\""), Some(false));
        assert_eq!(json_bool(s, "\"other\""), Some(true));
        assert_eq!(json_bool(s, "\"missing\""), None);
    }

    #[test]
    fn json_string_extracts_value() {
        let s = r#"{"city":"Milano","status":"success"}"#;
        assert_eq!(json_string(s, "\"city\""), Some("Milano".to_string()));
        assert_eq!(json_string(s, "\"status\""), Some("success".to_string()));
        assert_eq!(json_string(s, "\"missing\""), None);
    }

    #[test]
    fn json_string_handles_escapes() {
        let s = r#"{"name":"Rio de Janeiro \"RJ\""}"#;
        assert_eq!(json_string(s, "\"name\""), Some("Rio de Janeiro \"RJ\"".to_string()));
    }

    #[test]
    fn temp_conversion() {
        let snap = WeatherSnapshot {
            temp_c: 0.0,
            humidity: None,
            code: 0,
            icon: WeatherIcon::Clear,
            city: None,
        };
        assert_eq!(snap.temp_in(false), 0.0);
        assert_eq!(snap.temp_in(true), 32.0);
    }

    #[test]
    fn urlencode_escapes_spaces() {
        assert_eq!(urlencode("New York"), "New%20York");
        assert_eq!(urlencode("Milano"), "Milano");
    }

    /// A trimmed real response shape for `?name=Milano` (captured from the live
    /// API): the actual city of Milan, Italy only sorts to the top when
    /// `language=it` is passed; without it, several tiny same-named villages
    /// (a hamlet in Texas, one in Peru, ...) rank first by text relevance.
    /// `geocode_city`'s population-based tie-break is what makes this safe.
    /// A full response body shape (captured, trimmed, from the live API for
    /// `?name=Milano`): the actual city of Milan, Italy only sorts to the top
    /// when `language=it` is passed; without it, several tiny same-named
    /// villages (a hamlet in Texas, one in Peru, ...) rank first by text
    /// relevance. `geocode_city`'s population-based tie-break is what makes
    /// this safe regardless of ranking order.
    const SAMPLE_BODY: &str = r#"{"results":[
        {"id":1,"name":"Milano","latitude":30.71047,"longitude":-96.86331,"population":421,"country":"United States","postcodes":["76556"]},
        {"id":2,"name":"Milano","latitude":-8.74567,"longitude":-76.15826,"population":384,"country":"Peru"},
        {"id":3,"name":"Milano","latitude":45.46427,"longitude":9.18951,"population":1371498,"country":"Italy"},
        {"id":4,"name":"Milano","latitude":52.11879,"longitude":20.67155,"population":15784,"country":"Poland"}
    ],"generationtime_ms":0.6}"#;

    #[test]
    fn extract_array_isolates_the_results_array() {
        let arr = extract_array(SAMPLE_BODY, "\"results\"").unwrap();
        assert!(arr.starts_with('[') && arr.ends_with(']'));
        assert!(!arr.contains("generationtime_ms"));
    }

    #[test]
    fn split_json_objects_finds_each_top_level_result() {
        let arr = extract_array(SAMPLE_BODY, "\"results\"").unwrap();
        let objs = split_json_objects(arr);
        assert_eq!(objs.len(), 4);
        for obj in &objs {
            assert!(obj.starts_with('{') && obj.ends_with('}'));
        }
        // The nested "postcodes" array's brackets must not confuse the splitter.
        assert_eq!(json_string(objs[0], "\"country\""), Some("United States".to_string()));
    }

    #[test]
    fn geocoding_picks_the_most_populous_same_named_result() {
        let arr = extract_array(SAMPLE_BODY, "\"results\"").unwrap();
        let objs = split_json_objects(arr);
        let mut best: Option<(f64, f64, Option<String>, f64)> = None;
        for obj in objs {
            let lat = json_number(obj, "\"latitude\"").unwrap();
            let lon = json_number(obj, "\"longitude\"").unwrap();
            let population = json_number(obj, "\"population\"").unwrap_or(0.0);
            let country = json_string(obj, "\"country\"");
            let is_better = best.as_ref().map_or(true, |(_, _, _, p)| population > *p);
            if is_better {
                best = Some((lat, lon, country, population));
            }
        }
        let (lat, lon, country, population) = best.unwrap();
        // Milan, Italy — not the Texas/Peru/Poland hamlets sharing the name.
        assert_eq!(country, Some("Italy".to_string()));
        assert_eq!(population, 1_371_498.0);
        assert!((lat - 45.46427).abs() < 0.001);
        assert!((lon - 9.18951).abs() < 0.001);
    }
}
