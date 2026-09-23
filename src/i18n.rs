//! Interface strings in multiple languages (en / it). The bitmap font is
//! uppercase ASCII only and has no accents: Italian weekday names that end
//! with an accent are written with an apostrophe instead (LUNEDI'), as is
//! common on uppercase-only displays.

use chrono::{DateTime, Datelike, Local, Weekday};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    It,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "eng" | "english" | "inglese" => Some(Lang::En),
            "it" | "ita" | "italian" | "italiano" => Some(Lang::It),
            _ => None,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_language(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::It } else { Lang::En }
}

pub struct Strings {
    pub uptime: &'static str,
    pub mem: &'static str,
    pub net: &'static str,
    pub net_down: &'static str,
    pub net_up: &'static str,
    pub disk: &'static str,
    pub disk_read: &'static str,
    pub disk_write: &'static str,
    pub mute: &'static str,
    pub now_playing: &'static str,
    pub need_admin: &'static str,
    pub weather: &'static str,
    pub news: &'static str,
    /// Condition names, in the same order as `weather_icon::WeatherIcon`:
    /// Clear, PartlyCloudy, Cloudy, Fog, Drizzle, Rain, Snow, Thunder.
    pub weather_conditions: [&'static str; 8],
}

const EN: Strings = Strings {
    uptime: "UP",
    mem: "MEM",
    net: "NET",
    net_down: "DN",
    net_up: "UP",
    disk: "DISK",
    disk_read: "R",
    disk_write: "W",
    mute: "MUTE",
    now_playing: "NOW PLAYING",
    need_admin: "RUN AS ADMIN",
    weather: "WEATHER",
    news: "NEWS",
    weather_conditions: [
        "CLEAR", "PARTLY CLOUDY", "CLOUDY", "FOG", "DRIZZLE", "RAIN", "SNOW", "THUNDERSTORM",
    ],
};

const IT: Strings = Strings {
    uptime: "ATTIVO",
    mem: "RAM",
    net: "RETE",
    net_down: "IN",
    net_up: "OUT",
    disk: "DISCO",
    disk_read: "L",
    disk_write: "S",
    mute: "MUTO",
    now_playing: "IN RIPRODUZIONE",
    need_admin: "AVVIA COME ADMIN",
    weather: "METEO",
    news: "NOTIZIE",
    weather_conditions: [
        "SERENO", "POCO NUVOLOSO", "NUVOLOSO", "NEBBIA", "PIOVIGGINE", "PIOGGIA", "NEVE", "TEMPORALE",
    ],
};

pub fn t() -> &'static Strings {
    match language() {
        Lang::En => &EN,
        Lang::It => &IT,
    }
}

const IT_WEEKDAYS: [&str; 7] = [
    "Lunedi'", "Martedi'", "Mercoledi'", "Giovedi'", "Venerdi'", "Sabato", "Domenica",
];
const IT_MONTHS: [&str; 12] = [
    "Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno", "Luglio", "Agosto",
    "Settembre", "Ottobre", "Novembre", "Dicembre",
];

fn wd_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

/// Weekday name ("Monday" / "Lunedi'").
pub fn weekday(now: &DateTime<Local>) -> String {
    match language() {
        Lang::En => now.format("%A").to_string(),
        Lang::It => IT_WEEKDAYS[wd_index(now.weekday())].to_string(),
    }
}

/// "21 September 2026" / "21 Settembre 2026".
pub fn day_month_year(now: &DateTime<Local>) -> String {
    match language() {
        Lang::En => now.format("%d %B %Y").to_string(),
        Lang::It => format!("{:02} {} {}", now.day(), IT_MONTHS[now.month0() as usize], now.year()),
    }
}

/// "Monday, 21 September 2026" / "Lunedi', 21 Settembre 2026".
pub fn date_long(now: &DateTime<Local>) -> String {
    format!("{}, {}", weekday(now), day_month_year(now))
}
