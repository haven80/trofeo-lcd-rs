//! Interface strings in multiple languages (en / it / de / es / fr / pt). The
//! bitmap font is uppercase ASCII only and has no accents: accented letters
//! are either dropped (Italian, Spanish, French, Portuguese — e.g. "Lunedi'"
//! keeps the Italian apostrophe-for-accent convention; the others simply
//! drop the accent, which is a normal, widely-accepted way to write those
//! languages in all-caps/no-diacritics contexts) or spelled out (German
//! "AE"/"OE"/"UE" for ä/ö/ü, "SS" for ß — the standard substitution used
//! whenever umlauts aren't available, e.g. on older systems/keyboards).

use chrono::{DateTime, Datelike, Local, Weekday};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    It,
    De,
    Es,
    Fr,
    Pt,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "eng" | "english" | "inglese" => Some(Lang::En),
            "it" | "ita" | "italian" | "italiano" => Some(Lang::It),
            "de" | "ger" | "deu" | "german" | "deutsch" | "tedesco" => Some(Lang::De),
            "es" | "spa" | "spanish" | "espanol" | "español" | "spagnolo" => Some(Lang::Es),
            "fr" | "fra" | "french" | "francais" | "français" | "francese" => Some(Lang::Fr),
            "pt" | "por" | "portuguese" | "portugues" | "português" | "portoghese" => Some(Lang::Pt),
            _ => None,
        }
    }

    /// All valid values, for error messages and `--help`.
    pub const NAMES: &'static str = "en | it | de | es | fr | pt";
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_language(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::It,
        2 => Lang::De,
        3 => Lang::Es,
        4 => Lang::Fr,
        5 => Lang::Pt,
        _ => Lang::En,
    }
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

// German: "EMPFANGEN"/"SENDEN" (receive/send) are the standard terms German
// router/OS network UIs use, abbreviated the same way EN/IT abbreviate
// theirs. Umlauts are spelled out (AE/OE/UE), "ß" would become "SS" (none of
// these words happen to need one).
const DE: Strings = Strings {
    uptime: "AKTIV",
    mem: "RAM",
    net: "NETZ",
    net_down: "EMP",
    net_up: "SEN",
    disk: "DISK",
    disk_read: "L",
    disk_write: "S",
    mute: "STUMM",
    now_playing: "JETZT LAEUFT",
    need_admin: "ALS ADMIN STARTEN",
    weather: "WETTER",
    news: "NACHRICHTEN",
    weather_conditions: [
        "KLAR", "TEILS BEWOELKT", "BEWOELKT", "NEBEL", "NIESELREGEN", "REGEN", "SCHNEE", "GEWITTER",
    ],
};

// Spanish: accents simply dropped (a long-standing, widely accepted
// convention for Spanish written in all caps). "BAJ"/"SUB" (bajada/subida =
// download/upload) mirror the standard terms Spanish ISPs and routers use.
const ES: Strings = Strings {
    uptime: "ACTIVO",
    mem: "RAM",
    net: "RED",
    net_down: "BAJ",
    net_up: "SUB",
    disk: "DISCO",
    disk_read: "L",
    disk_write: "E",
    mute: "SILENCIO",
    now_playing: "REPRODUCIENDO",
    need_admin: "EJECUTAR COMO ADMIN",
    weather: "CLIMA",
    news: "NOTICIAS",
    weather_conditions: [
        "DESPEJADO", "ALGO NUBLADO", "NUBLADO", "NIEBLA", "LLOVIZNA", "LLUVIA", "NIEVE", "TORMENTA",
    ],
};

// French: accents dropped the same way (also common in all-caps French
// signage). "REC"/"ENV" (réception/envoi = receive/send) mirror the German
// choice's logic with native French words.
const FR: Strings = Strings {
    uptime: "ACTIF",
    mem: "RAM",
    net: "RESEAU",
    net_down: "REC",
    net_up: "ENV",
    disk: "DISQUE",
    disk_read: "L",
    disk_write: "E",
    mute: "MUET",
    now_playing: "LECTURE EN COURS",
    need_admin: "EXECUTER EN ADMIN",
    weather: "METEO",
    news: "INFOS",
    weather_conditions: [
        "DEGAGE", "PEU NUAGEUX", "NUAGEUX", "BROUILLARD", "BRUINE", "PLUIE", "NEIGE", "ORAGE",
    ],
};

// Portuguese: accents dropped, same convention. "REC"/"ENV"
// (recebido/enviado = received/sent) is standard Brazilian/Portuguese
// networking terminology.
const PT: Strings = Strings {
    uptime: "ATIVO",
    mem: "RAM",
    net: "REDE",
    net_down: "REC",
    net_up: "ENV",
    disk: "DISCO",
    disk_read: "L",
    disk_write: "E",
    mute: "MUDO",
    now_playing: "TOCANDO AGORA",
    need_admin: "EXECUTAR COMO ADMIN",
    weather: "TEMPO",
    news: "NOTICIAS",
    weather_conditions: [
        "LIMPO", "PARCIALMENTE NUBLADO", "NUBLADO", "NEVOEIRO", "CHUVISCO", "CHUVA", "NEVE", "TROVOADA",
    ],
};

pub fn t() -> &'static Strings {
    match language() {
        Lang::En => &EN,
        Lang::It => &IT,
        Lang::De => &DE,
        Lang::Es => &ES,
        Lang::Fr => &FR,
        Lang::Pt => &PT,
    }
}

const IT_WEEKDAYS: [&str; 7] = [
    "Lunedi'", "Martedi'", "Mercoledi'", "Giovedi'", "Venerdi'", "Sabato", "Domenica",
];
// None of the German weekday names have accents/umlauts at all.
const DE_WEEKDAYS: [&str; 7] = [
    "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag",
];
// "Miercoles"/"Sabado": accent dropped (Miércoles/Sábado).
const ES_WEEKDAYS: [&str; 7] = [
    "Lunes", "Martes", "Miercoles", "Jueves", "Viernes", "Sabado", "Domingo",
];
// None of the French weekday names have accents.
const FR_WEEKDAYS: [&str; 7] = [
    "Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche",
];
// "Terca-feira"/"Sabado": accent/cedilla dropped (Terça-feira/Sábado).
const PT_WEEKDAYS: [&str; 7] = [
    "Segunda-feira", "Terca-feira", "Quarta-feira", "Quinta-feira", "Sexta-feira", "Sabado", "Domingo",
];

const EN_WEEKDAYS_SHORT: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
const IT_WEEKDAYS_SHORT: [&str; 7] = ["LUN", "MAR", "MER", "GIO", "VEN", "SAB", "DOM"];
// The standard German 2-letter weekday abbreviations (Mo/Di/Mi/Do/Fr/Sa/So) —
// shorter than the others' 3 letters, which is simply how German abbreviates
// them; the forecast strip sizes each column to fit whatever text it gets.
const DE_WEEKDAYS_SHORT: [&str; 7] = ["MO", "DI", "MI", "DO", "FR", "SA", "SO"];
const ES_WEEKDAYS_SHORT: [&str; 7] = ["LUN", "MAR", "MIE", "JUE", "VIE", "SAB", "DOM"];
const FR_WEEKDAYS_SHORT: [&str; 7] = ["LUN", "MAR", "MER", "JEU", "VEN", "SAM", "DIM"];
// Portuguese weekday abbreviations follow the (unrelated) names above:
// segunda/terça/quarta/quinta/sexta -> SEG/TER/QUA/QUI/SEX.
const PT_WEEKDAYS_SHORT: [&str; 7] = ["SEG", "TER", "QUA", "QUI", "SEX", "SAB", "DOM"];

const IT_MONTHS: [&str; 12] = [
    "Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno", "Luglio", "Agosto",
    "Settembre", "Ottobre", "Novembre", "Dicembre",
];
// "Maerz": umlaut spelled out (März).
const DE_MONTHS: [&str; 12] = [
    "Januar", "Februar", "Maerz", "April", "Mai", "Juni", "Juli", "August",
    "September", "Oktober", "November", "Dezember",
];
const ES_MONTHS: [&str; 12] = [
    "Enero", "Febrero", "Marzo", "Abril", "Mayo", "Junio", "Julio", "Agosto",
    "Septiembre", "Octubre", "Noviembre", "Diciembre",
];
// "Fevrier"/"Aout"/"Decembre": accents dropped (Février/Août/Décembre).
const FR_MONTHS: [&str; 12] = [
    "Janvier", "Fevrier", "Mars", "Avril", "Mai", "Juin", "Juillet", "Aout",
    "Septembre", "Octobre", "Novembre", "Decembre",
];
// "Marco": cedilla dropped (Março).
const PT_MONTHS: [&str; 12] = [
    "Janeiro", "Fevereiro", "Marco", "Abril", "Maio", "Junho", "Julho", "Agosto",
    "Setembro", "Outubro", "Novembro", "Dezembro",
];

fn wd_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

/// Weekday name ("Monday" / "Lunedi'" / "Montag" / ...).
pub fn weekday(now: &DateTime<Local>) -> String {
    match language() {
        Lang::En => now.format("%A").to_string(),
        Lang::It => IT_WEEKDAYS[wd_index(now.weekday())].to_string(),
        Lang::De => DE_WEEKDAYS[wd_index(now.weekday())].to_string(),
        Lang::Es => ES_WEEKDAYS[wd_index(now.weekday())].to_string(),
        Lang::Fr => FR_WEEKDAYS[wd_index(now.weekday())].to_string(),
        Lang::Pt => PT_WEEKDAYS[wd_index(now.weekday())].to_string(),
    }
}

/// Abbreviated, uppercase weekday name ("MON" / "LUN" / "MO" / ...) — used in
/// the weather panel's 7-day forecast strip, where a full name wouldn't fit
/// per column.
pub fn weekday_short(w: Weekday) -> &'static str {
    let idx = wd_index(w);
    match language() {
        Lang::En => EN_WEEKDAYS_SHORT[idx],
        Lang::It => IT_WEEKDAYS_SHORT[idx],
        Lang::De => DE_WEEKDAYS_SHORT[idx],
        Lang::Es => ES_WEEKDAYS_SHORT[idx],
        Lang::Fr => FR_WEEKDAYS_SHORT[idx],
        Lang::Pt => PT_WEEKDAYS_SHORT[idx],
    }
}

/// "21 September 2026" / "21 Settembre 2026" / "21 September 2026" (DE) / ...
pub fn day_month_year(now: &DateTime<Local>) -> String {
    match language() {
        Lang::En => now.format("%d %B %Y").to_string(),
        Lang::It => format!("{:02} {} {}", now.day(), IT_MONTHS[now.month0() as usize], now.year()),
        Lang::De => format!("{:02} {} {}", now.day(), DE_MONTHS[now.month0() as usize], now.year()),
        Lang::Es => format!("{:02} {} {}", now.day(), ES_MONTHS[now.month0() as usize], now.year()),
        Lang::Fr => format!("{:02} {} {}", now.day(), FR_MONTHS[now.month0() as usize], now.year()),
        Lang::Pt => format!("{:02} {} {}", now.day(), PT_MONTHS[now.month0() as usize], now.year()),
    }
}

/// "Monday, 21 September 2026" / "Lunedi', 21 Settembre 2026" / ...
pub fn date_long(now: &DateTime<Local>) -> String {
    format!("{}, {}", weekday(now), day_month_year(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const ALL_LANGS: [Lang; 6] = [Lang::En, Lang::It, Lang::De, Lang::Es, Lang::Fr, Lang::Pt];
    const ALL_WEEKDAYS: [Weekday; 7] = [
        Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun,
    ];

    #[test]
    fn parse_recognizes_every_language_and_some_aliases() {
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("IT"), Some(Lang::It));
        assert_eq!(Lang::parse("deutsch"), Some(Lang::De));
        assert_eq!(Lang::parse("ES"), Some(Lang::Es));
        assert_eq!(Lang::parse("francese"), Some(Lang::Fr));
        assert_eq!(Lang::parse("portoghese"), Some(Lang::Pt));
        assert_eq!(Lang::parse("  de  "), Some(Lang::De));
        assert_eq!(Lang::parse("klingon"), None);
    }

    /// The bitmap font (`font.rs`) only has glyphs for plain ASCII — anything
    /// else (an accented letter left in by mistake) silently renders as a
    /// blank box (`glyph()`'s fallback arm). Every translated string, in
    /// every language, must therefore be pure ASCII: this is the automated
    /// equivalent of proofreading each translation for a stray á/ü/ç/etc.
    #[test]
    fn every_translation_is_pure_ascii_renderable() {
        for lang in ALL_LANGS {
            set_language(lang);
            let s = t();
            for (name, val) in [
                ("uptime", s.uptime), ("mem", s.mem), ("net", s.net), ("net_down", s.net_down),
                ("net_up", s.net_up), ("disk", s.disk), ("disk_read", s.disk_read),
                ("disk_write", s.disk_write), ("mute", s.mute), ("now_playing", s.now_playing),
                ("need_admin", s.need_admin), ("weather", s.weather), ("news", s.news),
            ] {
                assert!(val.is_ascii(), "{lang:?}.{name} = {val:?} is not pure ASCII (font can't render it)");
                assert!(!val.is_empty(), "{lang:?}.{name} is empty");
            }
            for (i, cond) in s.weather_conditions.iter().enumerate() {
                assert!(cond.is_ascii(), "{lang:?}.weather_conditions[{i}] = {cond:?} is not pure ASCII");
            }
            for w in ALL_WEEKDAYS {
                let short = weekday_short(w);
                assert!(short.is_ascii(), "{lang:?} weekday_short({w:?}) = {short:?} is not pure ASCII");
                assert!(!short.is_empty(), "{lang:?} weekday_short({w:?}) is empty");
            }
            for month in 1..=12u32 {
                let dt = Local.with_ymd_and_hms(2026, month, 15, 12, 0, 0).single().unwrap();
                let wd = weekday(&dt);
                let dmy = day_month_year(&dt);
                assert!(wd.is_ascii(), "{lang:?} weekday() for month {month} = {wd:?} is not pure ASCII");
                assert!(dmy.is_ascii(), "{lang:?} day_month_year() for month {month} = {dmy:?} is not pure ASCII");
            }
        }
        set_language(Lang::default());
    }

    /// Every one of the 7 weekdays and 12 months must be reachable (no
    /// out-of-bounds panic) and distinct from each other, for every language
    /// — guards against a copy-paste duplicate entry in one of the arrays.
    #[test]
    fn every_language_has_7_distinct_weekdays_and_12_distinct_months() {
        for lang in ALL_LANGS {
            set_language(lang);
            let shorts: Vec<&str> = ALL_WEEKDAYS.iter().map(|&w| weekday_short(w)).collect();
            let unique: std::collections::HashSet<&str> = shorts.iter().copied().collect();
            assert_eq!(unique.len(), 7, "{lang:?} has duplicate weekday_short entries: {shorts:?}");

            let months: Vec<String> = (1..=12u32)
                .map(|m| day_month_year(&Local.with_ymd_and_hms(2026, m, 1, 0, 0, 0).single().unwrap()))
                .collect();
            let unique_months: std::collections::HashSet<&String> = months.iter().collect();
            assert_eq!(unique_months.len(), 12, "{lang:?} has duplicate month names: {months:?}");
        }
        set_language(Lang::default());
    }
}
