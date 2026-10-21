//! Fan speeds and a few more sensors from **LibreHardwareMonitor** (LHM).
//!
//! LHM is a free, separate program that reads the sensors of the motherboard's Super-I/O
//! chip (fans, temperatures), CPUs, GPUs and drives. Turn on its web server (*Options* ->
//! *Remote Web Server* -> *Run*, default port 8085) and `trofeo_lcd` reads
//! `http://127.0.0.1:8085/data.json` from a background thread every `lhm_interval` seconds.
//! Nothing here is linked to LHM: if it is not running, the program just does not have
//! those values (and tries again every few seconds, which costs nothing).
//!
//! What is made of the sensors (see [`Readings`]):
//! * the **CPU fan**, the **case fans** (the other fans of the motherboard, averaged), the
//!   **pump** of a liquid cooler and the **GPU fan** (also for NVIDIA, which the AMD-only
//!   driver of the program cannot read);
//! * the **motherboard** and the hottest **drive** temperature;
//! * the CPU package temperature and power, used when the program's own sensor
//!   (AMD Zen only) has none: this makes them work on Intel CPUs.
//!
//! Boards name their fan headers as they like, so every role can be pointed at a sensor by
//! name (`lhm_cpu_fan = Fan #2`); `trofeo_lcd --lhm-list` prints what LHM reports and what each
//! role picks.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- a tiny JSON reader

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn children(&self) -> &[Json] {
        match self.get("Children") {
            Some(Json::Arr(v)) => v,
            _ => &[],
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Json::Obj(out));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    out.push((k, self.value(depth + 1)?));
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Json::Obj(out));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Json::Arr(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Json::Arr(out));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Json::Bool(true))
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Json::Bool(false))
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(Json::Null)
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok().map(Json::Num)
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' | b'f' => {}
                        b'u' => {
                            let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
                            let mut cp = u32::from_str_radix(hex, 16).ok()?;
                            self.i += 4;
                            if (0xD800..0xDC00).contains(&cp) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                                if let Some(lo) = std::str::from_utf8(self.b.get(self.i + 2..self.i + 6)?).ok().and_then(|h| u32::from_str_radix(h, 16).ok()) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                                    self.i += 6;
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

pub fn parse_json(s: &str) -> Option<Json> {
    let mut r = Reader { b: s.as_bytes(), i: 0 };
    let v = r.value(0)?;
    r.ws();
    (r.i == r.b.len()).then_some(v)
}

// ------------------------------------------------------------------------ the sensors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fan,
    Temp,
    Power,
    Load,
    Clock,
    Control,
    Other,
}

/// What kind of device a sensor belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Board,
    Cpu,
    Gpu,
    Storage,
    Memory,
    Cooler,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    pub class: Class,
    pub kind: Kind,
    /// The device: "Nuvoton NCT6798D", "AMD Ryzen 7 7800X3D", ...
    pub hardware: String,
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub id: String,
    /// `name` and `hardware / name`, lowercase, for matching.
    full: String,
}

/// The number at the start of an LHM value text ("1081 RPM", "45,5 °C", "1 234 RPM") and what follows it.
pub fn parse_value(text: &str) -> Option<(f64, String)> {
    let t = text.trim();
    let end = t
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | ',')))
        .map_or(t.len(), |(i, _)| i);
    let num = t[..end].trim_end_matches([',', '.']);
    if num.is_empty() {
        return None;
    }
    // "45,5" (decimal comma) or "1,234" / "1.234" (thousands): for RPM, a single separator followed by
    // exactly three digits is a thousands separator (fans do not turn at 1.5 rpm); anywhere else it is decimal
    let is_rpm = t[end..].trim_start().to_ascii_lowercase().starts_with("rpm");
    let thousands = |sep: char| -> bool {
        is_rpm && num.matches(sep).count() == 1 && {
            let mut it = num.split(sep);
            let before = it.next().unwrap_or("");
            let after = it.next().unwrap_or("");
            after.len() == 3 && !before.is_empty() && before.len() <= 3 && !before.starts_with(['-', '+'])
        }
    };
    let norm = if num.contains(',') && !num.contains('.') {
        if thousands(',') { num.replace(',', "") } else { num.replace(',', ".") }
    } else if num.contains('.') && !num.contains(',') && thousands('.') {
        num.replace('.', "")
    } else {
        num.to_string()
    };
    let v: f64 = norm.parse().ok()?;
    v.is_finite().then(|| (v, t[end..].trim().to_string()))
}

fn kind_from(type_name: Option<&str>, group: &str, unit: &str) -> Kind {
    let by_name = |s: &str| -> Option<Kind> {
        let s = s.to_ascii_lowercase();
        Some(match s.as_str() {
            "fan" | "fans" => Kind::Fan,
            "temperature" | "temperatures" => Kind::Temp,
            "power" | "powers" => Kind::Power,
            "load" | "loads" => Kind::Load,
            "clock" | "clocks" => Kind::Clock,
            "control" | "controls" => Kind::Control,
            _ => return None,
        })
    };
    if let Some(k) = type_name.and_then(by_name) {
        return k;
    }
    if let Some(k) = by_name(group.trim()) {
        return k;
    }
    let u = unit.to_ascii_lowercase();
    if u.contains("rpm") {
        Kind::Fan
    } else if u.ends_with('c') || u.ends_with('f') && u.contains('\u{b0}') {
        Kind::Temp
    } else if u == "w" {
        Kind::Power
    } else if u == "mhz" {
        Kind::Clock
    } else {
        Kind::Other
    }
}

fn class_from_id(id: &str) -> Option<Class> {
    let first = id.trim_start_matches('/').split('/').next()?.to_ascii_lowercase();
    if first.is_empty() {
        return None;
    }
    let f = first.as_str();
    Some(if f.starts_with("lpc") || f.starts_with("motherboard") || f.starts_with("mainboard") || f.starts_with("superio") || f.starts_with("embeddedcontroller") || f == "ec" {
        Class::Board
    } else if f.contains("cpu") {
        Class::Cpu
    } else if f.starts_with("gpu") || f.contains("gpu") {
        Class::Gpu
    } else if f.starts_with("nvme") || f.starts_with("hdd") || f.starts_with("ssd") || f.starts_with("storage") {
        Class::Storage
    } else if f.starts_with("ram") || f.starts_with("memory") {
        Class::Memory
    } else if ["aio", "heatmaster", "aquacomputer", "corsair", "nzxt", "bigng", "cooler", "kraken", "commander"].iter().any(|p| f.contains(p)) {
        Class::Cooler
    } else {
        return None;
    })
}

fn class_from_ancestors(path: &[(String, String)]) -> Class {
    for (text, image) in path.iter().rev() {
        let (t, i) = (text.to_ascii_lowercase(), image.to_ascii_lowercase());
        if i.contains("mainboard") || i.contains("motherboard") || i.contains("chip") {
            return Class::Board;
        }
        if i.contains("cpu") {
            return Class::Cpu;
        }
        if i.contains("nvidia") || i.contains("ati") || i.contains("gpu") || i.contains("intel_gpu") {
            return Class::Gpu;
        }
        if i.contains("hdd") || i.contains("nvme") || i.contains("ssd") {
            return Class::Storage;
        }
        if i.contains("ram") || i.contains("memory") {
            return Class::Memory;
        }
        if t.contains("nvidia") || t.contains("radeon") || t.contains("geforce") {
            return Class::Gpu;
        }
    }
    Class::Other
}

fn walk(node: &Json, path: &mut Vec<(String, String)>, out: &mut Vec<Sensor>) {
    let text = node.str("Text").unwrap_or("").to_string();
    let image = node.str("ImageURL").unwrap_or("").to_string();
    let children = node.children();
    if children.is_empty() {
        let value = node.str("Value").unwrap_or("");
        if let (false, Some((v, unit))) = (value.trim().is_empty(), parse_value(value)) {
            let id = node.str("SensorId").unwrap_or("").to_string();
            let group = path.last().map(|(t, _)| t.as_str()).unwrap_or("");
            let kind = kind_from(node.str("Type"), group, &unit);
            let class = class_from_id(&id).unwrap_or_else(|| class_from_ancestors(path));
            // the device: the nearest ancestor above the group
            let hardware = path.iter().rev().skip(1).map(|(t, _)| t.as_str()).find(|t| !t.is_empty()).unwrap_or("").to_string();
            let full = format!("{} / {}", hardware, text).to_ascii_lowercase();
            out.push(Sensor { class, kind, hardware, name: text, value: v, unit, id, full });
        }
        return;
    }
    path.push((text, image));
    for c in children {
        walk(c, path, out);
    }
    path.pop();
}

/// Every sensor of an LHM `data.json`.
pub fn parse_sensors(json: &str) -> Option<Vec<Sensor>> {
    let root = parse_json(json)?;
    root.get("Children")?;
    let mut out = Vec::new();
    // the root ("Sensor") and the computer are not devices
    walk(&root, &mut Vec::new(), &mut out);
    Some(out)
}

// ---------------------------------------------------------------- what is made of them

/// Names (substrings, any case) that point a role at a sensor instead of the automatic choice.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub cpu_fan: Option<String>,
    pub case_fan: Option<String>,
    pub pump: Option<String>,
    pub mb_temp: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Readings {
    pub cpu_fan: Option<u32>,
    pub case_fan: Option<u32>,
    pub pump: Option<u32>,
    pub gpu_fan: Option<u32>,
    pub mb_temp: Option<f32>,
    pub disk_temp: Option<f32>,
    /// CPU package temperature / power, for CPUs the program's own sensor does not know.
    pub cpu_temp: Option<f32>,
    pub cpu_power: Option<f32>,
}

impl Readings {
    pub fn any_fan(&self) -> bool {
        self.cpu_fan.is_some() || self.case_fan.is_some() || self.pump.is_some() || self.gpu_fan.is_some()
    }

    pub fn is_empty(&self) -> bool {
        *self == Readings::default()
    }

    /// The fans that exist, as (label, rpm), in the order of the "fans" tile.
    pub fn fan_rows(&self) -> Vec<(&'static str, u32)> {
        [("CPU", self.cpu_fan), ("Case", self.case_fan), ("Pump", self.pump), ("GPU", self.gpu_fan)]
            .into_iter()
            .filter_map(|(l, v)| v.map(|v| (l, v)))
            .collect()
    }

    /// Made-up values for the settings editor's preview.
    pub fn sample() -> Readings {
        Readings {
            cpu_fan: Some(1240),
            case_fan: Some(780),
            pump: Some(2650),
            gpu_fan: Some(1480),
            mb_temp: Some(38.0),
            disk_temp: Some(44.0),
            cpu_temp: None,
            cpu_power: None,
        }
    }
}

fn is_pump(s: &Sensor) -> bool {
    s.full.contains("pump")
}

fn is_cpu_fan(s: &Sensor) -> bool {
    !is_pump(s) && (s.class == Class::Cpu || s.name.to_ascii_lowercase().contains("cpu"))
}

fn plausible_temp(v: f64) -> bool {
    v > 0.0 && v < 125.0
}

fn matching<'a>(sensors: &'a [Sensor], kind: Kind, filter: &Option<String>) -> Option<Vec<&'a Sensor>> {
    let f = filter.as_ref()?.trim().to_ascii_lowercase();
    if f.is_empty() {
        return None;
    }
    let v: Vec<&Sensor> = sensors.iter().filter(|s| s.kind == kind && s.full.contains(&f)).collect();
    Some(v)
}

fn max_rpm(v: &[&Sensor]) -> Option<u32> {
    v.iter().map(|s| s.value.max(0.0).round() as u32).max()
}

/// The average of the fans that spin; 0 when there are some but none turns.
fn avg_rpm(v: &[&Sensor]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    let on: Vec<f64> = v.iter().map(|s| s.value).filter(|r| *r > 0.0).collect();
    Some(if on.is_empty() { 0 } else { (on.iter().sum::<f64>() / on.len() as f64).round() as u32 })
}

pub fn derive(sensors: &[Sensor], f: &Filters) -> Readings {
    let fans: Vec<&Sensor> = sensors.iter().filter(|s| s.kind == Kind::Fan && s.value.is_finite()).collect();
    let mut r = Readings::default();

    r.cpu_fan = match matching(sensors, Kind::Fan, &f.cpu_fan) {
        Some(m) => max_rpm(&m),
        None => max_rpm(&fans.iter().copied().filter(|s| s.class != Class::Gpu && is_cpu_fan(s)).collect::<Vec<_>>()),
    };
    r.pump = match matching(sensors, Kind::Fan, &f.pump) {
        Some(m) => max_rpm(&m),
        None => max_rpm(&fans.iter().copied().filter(|s| is_pump(s)).collect::<Vec<_>>()),
    };
    r.case_fan = match matching(sensors, Kind::Fan, &f.case_fan) {
        Some(m) => avg_rpm(&m),
        None => avg_rpm(&fans.iter().copied().filter(|s| !matches!(s.class, Class::Gpu | Class::Cpu) && !is_pump(s) && !is_cpu_fan(s)).collect::<Vec<_>>()),
    };
    r.gpu_fan = max_rpm(&fans.iter().copied().filter(|s| s.class == Class::Gpu).collect::<Vec<_>>());

    let temps = |class: Class| -> Vec<&Sensor> {
        sensors
            .iter()
            .filter(|s| s.kind == Kind::Temp && s.class == class && plausible_temp(s.value))
            .filter(|s| {
                let n = s.name.to_ascii_lowercase();
                !(n.contains("warning") || n.contains("critical") || n.contains("limit") || n.contains("threshold"))
            })
            .collect()
    };
    r.mb_temp = match matching(sensors, Kind::Temp, &f.mb_temp) {
        Some(m) => m.iter().map(|s| s.value as f32).filter(|v| plausible_temp(*v as f64)).reduce(f32::max),
        None => {
            let t = temps(Class::Board);
            ["motherboard", "mainboard", "system", "board"]
                .iter()
                .find_map(|k| t.iter().find(|s| s.name.to_ascii_lowercase().contains(k)))
                .or_else(|| t.first())
                .map(|s| s.value as f32)
        }
    };
    r.disk_temp = temps(Class::Storage).iter().map(|s| s.value as f32).reduce(f32::max);

    let cpu_t = temps(Class::Cpu);
    r.cpu_temp = ["package", "tctl", "tdie", "core max", "cpu"]
        .iter()
        .find_map(|k| cpu_t.iter().find(|s| s.name.to_ascii_lowercase().contains(k)))
        .map(|s| s.value as f32)
        .or_else(|| cpu_t.iter().map(|s| s.value as f32).reduce(f32::max));
    r.cpu_power = sensors
        .iter()
        .find(|s| s.kind == Kind::Power && s.class == Class::Cpu && s.name.to_ascii_lowercase().contains("package") && s.value > 0.0)
        .map(|s| s.value as f32);
    r
}

// ---------------------------------------------------------------------- the settings

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub url: String,
    pub interval: u32,
    pub filters: Filters,
}

pub const DEFAULT_URL: &str = "http://127.0.0.1:8085";

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, url: DEFAULT_URL.to_string(), interval: 2, filters: Filters::default() }
    }
}

pub fn parse(file: &trofeo_lcd::config::ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get_bool("lhm").map_err(|e| anyhow::anyhow!(e))? {
        s.enabled = v;
    }
    if let Some(v) = file.get("lhm_url") {
        let v = v.trim().trim_matches('"');
        if !v.is_empty() {
            s.url = v.to_string();
        }
    }
    if let Some(v) = file.get_u32("lhm_interval").map_err(|e| anyhow::anyhow!(e))? {
        if !(1..=60).contains(&v) {
            anyhow::bail!("lhm_interval: must be between 1 and 60 seconds (got {v})");
        }
        s.interval = v;
    }
    let name = |key: &str| file.get(key).map(|v| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty());
    s.filters = Filters { cpu_fan: name("lhm_cpu_fan"), case_fan: name("lhm_case_fan"), pump: name("lhm_pump"), mb_temp: name("lhm_mb_temp") };
    Ok(s)
}

/// `host:port` or a full URL -> the URL of `data.json`.
pub fn data_url(url: &str) -> String {
    let u = url.trim();
    let with_scheme = if u.contains("://") { u.to_string() } else { format!("http://{u}") };
    let after = with_scheme.split_once("://").map_or("", |(_, r)| r);
    if after.contains('/') && after.split_once('/').is_some_and(|(_, p)| !p.is_empty()) {
        with_scheme
    } else {
        format!("{}/data.json", with_scheme.trim_end_matches('/'))
    }
}

// ----------------------------------------------------------------------- the client

struct Shared {
    sensors: Vec<Sensor>,
    at: Option<Instant>,
    seq: u64,
}

/// Polls LHM from a background thread.
pub struct Client {
    shared: Arc<Mutex<Shared>>,
    stale_after: Duration,
}

pub fn fetch(url: &str, timeout: Duration) -> Result<Vec<Sensor>, String> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let body = agent.get(&data_url(url)).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    parse_sensors(&body).ok_or_else(|| "not an LibreHardwareMonitor data.json".to_string())
}

impl Client {
    pub fn start(url: &str, interval_secs: u32) -> Client {
        let shared = Arc::new(Mutex::new(Shared { sensors: Vec::new(), at: None, seq: 0 }));
        let (sh, url) = (shared.clone(), url.to_string());
        let interval = Duration::from_secs(interval_secs.max(1) as u64);
        std::thread::Builder::new()
            .name("lhm".into())
            .spawn(move || loop {
                let wait = match fetch(&url, Duration::from_secs(3)) {
                    Ok(sensors) => {
                        if let Ok(mut g) = sh.lock() {
                            g.sensors = sensors;
                            g.at = Some(Instant::now());
                            g.seq += 1;
                        }
                        interval
                    }
                    // not running (yet): look again in a few seconds
                    Err(_) => Duration::from_secs(5).max(interval),
                };
                std::thread::sleep(wait);
            })
            .ok();
        Client { shared, stale_after: Duration::from_secs((interval_secs as u64 * 4).max(10)) }
    }

    /// The latest sensors with a counter that changes at every update; `None` when LHM
    /// has not answered lately (not running, or the server is off).
    pub fn snapshot(&self) -> Option<(u64, Vec<Sensor>)> {
        let g = self.shared.lock().ok()?;
        let fresh = g.at.is_some_and(|t| t.elapsed() < self.stale_after);
        fresh.then(|| (g.seq, g.sensors.clone()))
    }
}

/// Keeps a [`Client`] in step with the settings (started, stopped or restarted when they change)
/// and turns what it reads into [`Readings`], only when something new arrived.
pub struct Link {
    client: Option<(Client, String, u32)>,
    seen: Option<(u64, Filters)>,
    last: Readings,
}

impl Link {
    pub fn new() -> Link {
        Link { client: None, seen: None, last: Readings::default() }
    }

    pub fn update(&mut self, s: &Settings) -> Readings {
        if !s.enabled {
            self.client = None;
            self.seen = None;
            self.last = Readings::default();
            return self.last.clone();
        }
        let same = self.client.as_ref().is_some_and(|(_, url, iv)| *url == s.url && *iv == s.interval);
        if !same {
            self.client = Some((Client::start(&s.url, s.interval), s.url.clone(), s.interval));
            self.seen = None;
        }
        let Some((client, _, _)) = &self.client else { return Readings::default() };
        match client.snapshot() {
            Some((seq, sensors)) => {
                if self.seen.as_ref().is_none_or(|(q, f)| *q != seq || *f != s.filters) {
                    self.last = derive(&sensors, &s.filters);
                    self.seen = Some((seq, s.filters.clone()));
                }
            }
            None => {
                self.seen = None;
                self.last = Readings::default();
            }
        }
        self.last.clone()
    }
}

// ------------------------------------------------- the values the screens read

thread_local! {
    static CURRENT: RefCell<Readings> = RefCell::new(Readings::default());
}

/// What the screens show: set by the main loop (and by the preview), read while drawing.
pub fn current() -> Readings {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn set_current(r: Readings) {
    if let Ok(mut g) = SHARED.lock() {
        *g = r.clone();
    }
    CURRENT.with(|c| *c.borrow_mut() = r);
}

/// The same readings for other threads (the DeepCool display driver): the latest `set_current`.
static SHARED: Mutex<Readings> = Mutex::new(Readings {
    cpu_fan: None,
    case_fan: None,
    pump: None,
    gpu_fan: None,
    mb_temp: None,
    disk_temp: None,
    cpu_temp: None,
    cpu_power: None,
});

pub fn shared() -> Readings {
    SHARED.lock().map(|g| g.clone()).unwrap_or_default()
}

/// `trofeo_lcd --lhm-list`: what LHM reports and what each role picks.
pub fn diag(url: &str, filters: &Filters) -> bool {
    println!("LibreHardwareMonitor: {}", data_url(url));
    let sensors = match fetch(url, Duration::from_secs(5)) {
        Ok(s) => s,
        Err(e) => {
            println!("  not reachable: {e}");
            println!("  Start LibreHardwareMonitor, then Options > Remote Web Server > Run (port 8085).");
            return false;
        }
    };
    println!("  {} sensors", sensors.len());
    for (title, kind) in [("Fans", Kind::Fan), ("Temperatures", Kind::Temp)] {
        println!("\n{title}:");
        for s in sensors.iter().filter(|s| s.kind == kind) {
            println!("  [{:?}] {} / {}  =  {} {}", s.class, s.hardware, s.name, s.value, s.unit);
        }
    }
    let r = derive(&sensors, filters);
    println!("\nWhat the program uses:");
    let fan = |v: Option<u32>| v.map_or("-".to_string(), |v| format!("{v} rpm"));
    let temp = |v: Option<f32>| v.map_or("-".to_string(), |v| format!("{v:.0} C"));
    println!("  CPU fan    : {}   (lhm_cpu_fan)", fan(r.cpu_fan));
    println!("  Case fans  : {}   (lhm_case_fan, the average of the others)", fan(r.case_fan));
    println!("  Pump       : {}   (lhm_pump)", fan(r.pump));
    println!("  GPU fan    : {}", fan(r.gpu_fan));
    println!("  Motherboard: {}   (lhm_mb_temp)", temp(r.mb_temp));
    println!("  Hottest drive: {}", temp(r.disk_temp));
    println!("  CPU (fallback for CPUs the program cannot read): {} / {}", temp(r.cpu_temp), r.cpu_power.map_or("-".to_string(), |p| format!("{p:.0} W")));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed-down data.json of a recent LHM (with SensorId and Type).
    pub const NEW_JSON: &str = r#"{"id":0,"Text":"Sensor","Min":"Min","Value":"Value","Max":"Max","ImageURL":"","Children":[
      {"id":1,"Text":"GAMING-PC","Min":"","Value":"","Max":"","ImageURL":"images_icon/computer.png","Children":[
        {"id":2,"Text":"ASUS ROG STRIX B650-A","Min":"","Value":"","Max":"","ImageURL":"images_icon/mainboard.png","Children":[
          {"id":3,"Text":"Nuvoton NCT6799D","Min":"","Value":"","Max":"","ImageURL":"images_icon/chip.png","Children":[
            {"id":4,"Text":"Temperatures","Min":"","Value":"","Max":"","ImageURL":"images_icon/temperature.png","Children":[
              {"id":5,"Text":"CPU Core","Min":"30,0 °C","Value":"51,0 °C","Max":"70,0 °C","ImageURL":"images_icon/temperature.png","SensorId":"/lpc/nct6799d/0/temperature/0","Type":"Temperature","Children":[]},
              {"id":6,"Text":"Motherboard","Min":"","Value":"36,5 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/lpc/nct6799d/0/temperature/2","Type":"Temperature","Children":[]},
              {"id":7,"Text":"VRM","Min":"","Value":"41,0 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/lpc/nct6799d/0/temperature/3","Type":"Temperature","Children":[]}
            ]},
            {"id":8,"Text":"Fans","Min":"","Value":"","Max":"","ImageURL":"images_icon/fan.png","Children":[
              {"id":9,"Text":"Fan #1","Min":"0 RPM","Value":"0 RPM","Max":"2000 RPM","ImageURL":"images_icon/fan.png","SensorId":"/lpc/nct6799d/0/fan/0","Type":"Fan","Children":[]},
              {"id":10,"Text":"CPU Fan","Min":"0 RPM","Value":"1240 RPM","Max":"2100 RPM","ImageURL":"images_icon/fan.png","SensorId":"/lpc/nct6799d/0/fan/1","Type":"Fan","Children":[]},
              {"id":11,"Text":"Chassis Fan #1","Min":"0 RPM","Value":"800 RPM","Max":"1000 RPM","ImageURL":"images_icon/fan.png","SensorId":"/lpc/nct6799d/0/fan/2","Type":"Fan","Children":[]},
              {"id":12,"Text":"Chassis Fan #2","Min":"0 RPM","Value":"760 RPM","Max":"1000 RPM","ImageURL":"images_icon/fan.png","SensorId":"/lpc/nct6799d/0/fan/3","Type":"Fan","Children":[]},
              {"id":13,"Text":"AIO Pump","Min":"0 RPM","Value":"2650 RPM","Max":"3000 RPM","ImageURL":"images_icon/fan.png","SensorId":"/lpc/nct6799d/0/fan/4","Type":"Fan","Children":[]}
            ]},
            {"id":14,"Text":"Controls","Min":"","Value":"","Max":"","ImageURL":"images_icon/control.png","Children":[
              {"id":15,"Text":"Fan Control #2","Min":"0,0 %","Value":"45,0 %","Max":"100,0 %","ImageURL":"images_icon/control.png","SensorId":"/lpc/nct6799d/0/control/1","Type":"Control","Children":[]}
            ]}
          ]}
        ]},
        {"id":20,"Text":"AMD Ryzen 7 7800X3D","Min":"","Value":"","Max":"","ImageURL":"images_icon/cpu.png","Children":[
          {"id":21,"Text":"Temperatures","Min":"","Value":"","Max":"","ImageURL":"images_icon/temperature.png","Children":[
            {"id":22,"Text":"Core (Tctl/Tdie)","Min":"","Value":"58,2 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/amdcpu/0/temperature/2","Type":"Temperature","Children":[]}
          ]},
          {"id":23,"Text":"Powers","Min":"","Value":"","Max":"","ImageURL":"images_icon/power.png","Children":[
            {"id":24,"Text":"Package","Min":"","Value":"61,3 W","Max":"","ImageURL":"images_icon/power.png","SensorId":"/amdcpu/0/power/0","Type":"Power","Children":[]}
          ]}
        ]},
        {"id":30,"Text":"NVIDIA GeForce RTX 4070","Min":"","Value":"","Max":"","ImageURL":"images_icon/nvidia.png","Children":[
          {"id":31,"Text":"Fans","Min":"","Value":"","Max":"","ImageURL":"images_icon/fan.png","Children":[
            {"id":32,"Text":"GPU Fan","Min":"","Value":"1480 RPM","Max":"","ImageURL":"images_icon/fan.png","SensorId":"/gpu-nvidia/0/fan/1","Type":"Fan","Children":[]}
          ]}
        ]},
        {"id":40,"Text":"Samsung SSD 990 PRO","Min":"","Value":"","Max":"","ImageURL":"images_icon/nvme.png","Children":[
          {"id":41,"Text":"Temperatures","Min":"","Value":"","Max":"","ImageURL":"images_icon/temperature.png","Children":[
            {"id":42,"Text":"Temperature","Min":"","Value":"44,0 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/nvme/0/temperature/0","Type":"Temperature","Children":[]},
            {"id":43,"Text":"Warning Temperature","Min":"","Value":"82,0 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/nvme/0/temperature/5","Type":"Temperature","Children":[]},
            {"id":44,"Text":"Critical Temperature","Min":"","Value":"85,0 °C","Max":"","ImageURL":"images_icon/temperature.png","SensorId":"/nvme/0/temperature/6","Type":"Temperature","Children":[]}
          ]}
        ]}
      ]}
    ]}"#;

    /// An old LHM / OpenHardwareMonitor: no SensorId, no Type.
    const OLD_JSON: &str = r#"{"id":0,"Text":"Sensor","Min":"Min","Value":"Value","Max":"Max","ImageURL":"","Children":[
      {"id":1,"Text":"PC","Min":"","Value":"","Max":"","ImageURL":"images_icon/computer.png","Children":[
        {"id":2,"Text":"Board","Min":"","Value":"","Max":"","ImageURL":"images_icon/mainboard.png","Children":[
          {"id":3,"Text":"Nuvoton NCT6798D","Min":"","Value":"","Max":"","ImageURL":"images_icon/chip.png","Children":[
            {"id":4,"Text":"Fans","Min":"","Value":"","Max":"","ImageURL":"images_icon/fan.png","Children":[
              {"id":5,"Text":"CPU Fan","Min":"700 RPM","Value":"1.100 RPM","Max":"2000 RPM","ImageURL":"images_icon/fan.png","Children":[]},
              {"id":6,"Text":"Fan #2","Min":"0 RPM","Value":"900 RPM","Max":"1000 RPM","ImageURL":"images_icon/fan.png","Children":[]}
            ]},
            {"id":7,"Text":"Temperatures","Min":"","Value":"","Max":"","ImageURL":"images_icon/temperature.png","Children":[
              {"id":8,"Text":"System","Min":"","Value":"33 °C","Max":"","ImageURL":"images_icon/temperature.png","Children":[]}
            ]}
          ]}
        ]},
        {"id":9,"Text":"Intel Core i7-12700K","Min":"","Value":"","Max":"","ImageURL":"images_icon/cpu.png","Children":[
          {"id":10,"Text":"Temperatures","Min":"","Value":"","Max":"","ImageURL":"images_icon/temperature.png","Children":[
            {"id":11,"Text":"Core #1","Min":"","Value":"48 °C","Max":"","ImageURL":"images_icon/temperature.png","Children":[]},
            {"id":12,"Text":"CPU Package","Min":"","Value":"55 °C","Max":"","ImageURL":"images_icon/temperature.png","Children":[]}
          ]},
          {"id":13,"Text":"Powers","Min":"","Value":"","Max":"","ImageURL":"images_icon/power.png","Children":[
            {"id":14,"Text":"CPU Package","Min":"","Value":"72,5 W","Max":"","ImageURL":"images_icon/power.png","Children":[]}
          ]}
        ]}
      ]}
    ]}"#;

    #[test]
    fn the_json_reader_handles_what_lhm_sends() {
        assert_eq!(parse_json(r#"{"a":[1,2.5,-3e2,true,false,null,"x\"y°\n"],"b":{}}"#).unwrap().get("a"),
            Some(&Json::Arr(vec![Json::Num(1.0), Json::Num(2.5), Json::Num(-300.0), Json::Bool(true), Json::Bool(false), Json::Null, Json::Str("x\"y\u{b0}\n".into())])));
        assert_eq!(parse_json("  [ ]  "), Some(Json::Arr(vec![])));
        assert_eq!(parse_json(r#""😀""#), Some(Json::Str("\u{1F600}".into())));
        for bad in ["", "{", "{\"a\":}", "[1,]", "{\"a\" 1}", "nul", "[1] x", "\"abc"] {
            assert_eq!(parse_json(bad), None, "{bad}");
        }
        // deeply nested garbage does not blow the stack
        assert_eq!(parse_json(&"[".repeat(10_000)), None);
    }

    #[test]
    fn values_come_out_whatever_the_language_of_the_pc() {
        assert_eq!(parse_value("1081 RPM"), Some((1081.0, "RPM".into())));
        assert_eq!(parse_value("45,5 \u{b0}C"), Some((45.5, "\u{b0}C".into())));
        assert_eq!(parse_value("45.5 \u{b0}C"), Some((45.5, "\u{b0}C".into())));
        assert_eq!(parse_value("1,234 RPM"), Some((1234.0, "RPM".into())));
        assert_eq!(parse_value("1.100 RPM"), Some((1100.0, "RPM".into())));
        assert_eq!(parse_value("61,3 W"), Some((61.3, "W".into())));
        assert_eq!(parse_value("-3 %"), Some((-3.0, "%".into())));
        assert_eq!(parse_value("0 RPM"), Some((0.0, "RPM".into())));
        assert_eq!(parse_value(""), None);
        assert_eq!(parse_value("Value"), None);
        assert_eq!(parse_value("N/A"), None);
    }

    #[test]
    fn a_recent_tree_gives_every_role_its_sensor() {
        let s = parse_sensors(NEW_JSON).unwrap();
        // hardware nodes and groups are not sensors
        assert!(s.iter().all(|x| !x.name.is_empty() && x.hardware != ""));
        let fan = |name: &str| s.iter().find(|x| x.name == name).unwrap();
        assert_eq!((fan("CPU Fan").kind, fan("CPU Fan").class, fan("CPU Fan").value), (Kind::Fan, Class::Board, 1240.0));
        assert_eq!(fan("Fan Control #2").kind, Kind::Control);
        assert_eq!(fan("GPU Fan").class, Class::Gpu);
        assert_eq!(fan("Temperature").class, Class::Storage);
        assert_eq!(fan("Core (Tctl/Tdie)").class, Class::Cpu);
        assert_eq!(fan("CPU Fan").hardware, "Nuvoton NCT6799D");
        let r = derive(&s, &Filters::default());
        assert_eq!(r.cpu_fan, Some(1240));
        // the fan that is not plugged in (0 rpm, "Fan #1") does not drag the case average down
        assert_eq!(r.case_fan, Some(780));
        assert_eq!(r.pump, Some(2650));
        assert_eq!(r.gpu_fan, Some(1480));
        assert_eq!(r.mb_temp, Some(36.5));
        assert_eq!(r.disk_temp, Some(44.0), "the warning / critical limits are not temperatures");
        assert_eq!(r.cpu_temp.map(|t| t.round()), Some(58.0));
        assert_eq!(r.cpu_power.map(|t| t.round()), Some(61.0));
        assert!(r.any_fan());
        assert_eq!(r.fan_rows(), vec![("CPU", 1240), ("Case", 780), ("Pump", 2650), ("GPU", 1480)]);
    }

    #[test]
    fn an_old_tree_without_ids_still_works() {
        let s = parse_sensors(OLD_JSON).unwrap();
        let r = derive(&s, &Filters::default());
        assert_eq!(r.cpu_fan, Some(1100), "'1.100 RPM'");
        assert_eq!(r.case_fan, Some(900));
        assert_eq!(r.pump, None);
        assert_eq!(r.gpu_fan, None);
        assert_eq!(r.mb_temp, Some(33.0));
        // Intel: the package temperature wins over the cores
        assert_eq!(r.cpu_temp, Some(55.0));
        assert_eq!(r.cpu_power, Some(72.5));
    }

    #[test]
    fn names_can_point_a_role_at_another_sensor() {
        let s = parse_sensors(NEW_JSON).unwrap();
        let f = Filters { cpu_fan: Some("chassis fan #1".into()), case_fan: Some("FAN #".into()), pump: Some("cpu fan".into()), mb_temp: Some("vrm".into()) };
        let r = derive(&s, &f);
        assert_eq!(r.cpu_fan, Some(800));
        assert_eq!(r.case_fan, Some(780), "Chassis Fan #1 (800) and #2 (760) average; Fan #1 does not spin");
        assert_eq!(r.pump, Some(1240));
        assert_eq!(r.mb_temp, Some(41.0));
        // a name nothing matches: the role is empty rather than guessed
        let none = derive(&s, &Filters { cpu_fan: Some("no such fan".into()), ..Filters::default() });
        assert_eq!(none.cpu_fan, None);
        // no sensors at all
        assert!(derive(&[], &Filters::default()).is_empty());
        // a stopped fan reads 0, a header with nothing on it is not "no fans"
        let stopped = parse_sensors(&NEW_JSON.replace("800 RPM", "0 RPM").replace("760 RPM", "0 RPM")).unwrap();
        assert_eq!(derive(&stopped, &Filters::default()).case_fan, Some(0));
    }

    #[test]
    fn settings_parse() {
        let p = |t: &str| parse(&trofeo_lcd::config::ConfigFile::parse(t).unwrap());
        let d = p("").unwrap();
        assert_eq!(d, Settings::default());
        assert!(d.enabled && d.interval == 2 && d.url == DEFAULT_URL);
        let s = p("lhm = false\nlhm_url = localhost:9000\nlhm_interval = 5\nlhm_cpu_fan = \"Fan #2\"\nlhm_pump = AIO\nlhm_mb_temp = vrm\nlhm_case_fan = chassis\n").unwrap();
        assert!(!s.enabled);
        assert_eq!((s.url.as_str(), s.interval), ("localhost:9000", 5));
        assert_eq!(s.filters, Filters { cpu_fan: Some("Fan #2".into()), case_fan: Some("chassis".into()), pump: Some("AIO".into()), mb_temp: Some("vrm".into()) });
        assert!(p("lhm_interval = 0\n").is_err());
        assert!(p("lhm_interval = 61\n").is_err());
        assert!(p("lhm = maybe\n").is_err());
    }

    #[test]
    fn the_url_gets_its_scheme_and_path() {
        assert_eq!(data_url("http://127.0.0.1:8085"), "http://127.0.0.1:8085/data.json");
        assert_eq!(data_url("localhost:8085"), "http://localhost:8085/data.json");
        assert_eq!(data_url("http://pc:8085/"), "http://pc:8085/data.json");
        assert_eq!(data_url("http://pc:8085/custom.json"), "http://pc:8085/custom.json");
    }

    #[test]
    fn the_client_reads_a_server_and_notices_when_it_goes_away() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok((mut c, _)) = listener.accept() {
                    c.set_nonblocking(false).ok();
                    let mut buf = [0u8; 1024];
                    let _ = c.read(&mut buf);
                    let head = String::from_utf8_lossy(&buf).to_string();
                    assert!(head.starts_with("GET /data.json"), "{head}");
                    let body = NEW_JSON;
                    let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        let client = Client::start(&format!("127.0.0.1:{port}"), 1);
        let t = Instant::now();
        let got = loop {
            if let Some(s) = client.snapshot() {
                break Some(s);
            }
            if t.elapsed() > Duration::from_secs(10) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let (seq, sensors) = got.expect("the client reads the server");
        assert!(seq >= 1);
        assert_eq!(derive(&sensors, &Filters::default()).cpu_fan, Some(1240));
        // a second answer arrives and the counter moves
        let t = Instant::now();
        while client.snapshot().map_or(true, |(s, _)| s <= seq) && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(client.snapshot().is_some_and(|(s, _)| s > seq));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        server.join().unwrap();
        // fetch() reports a server that is not there
        assert!(fetch("127.0.0.1:1", Duration::from_millis(500)).is_err());
        // and a client of a dead address never has a snapshot
        let dead = Client::start("127.0.0.1:1", 1);
        std::thread::sleep(Duration::from_millis(300));
        assert!(dead.snapshot().is_none());
    }

    #[test]
    fn the_current_readings_are_per_thread() {
        set_current(Readings::sample());
        assert_eq!(current().cpu_fan, Some(1240));
        std::thread::spawn(|| assert!(current().is_empty())).join().unwrap();
        set_current(Readings::default());
        assert!(current().is_empty());
    }
}
