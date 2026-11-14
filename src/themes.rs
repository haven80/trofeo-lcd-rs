//! Themes: `theme = ocean` gives a whole look (background, colours, panel opacity) in one
//! line. A theme only supplies **defaults**: every setting written in the configuration file
//! (or on the command line) wins over it, so a theme is a starting point to adjust.

/// One theme: a name, a label and the settings it provides.
pub struct Theme {
    pub name: &'static str,
    pub en: &'static str,
    pub it: &'static str,
    /// `(key, value)`; only settings of the look (checked by the tests).
    pub settings: &'static [(&'static str, &'static str)],
}

pub const THEMES: &[Theme] = &[
    Theme {
        name: "midnight",
        en: "Midnight: deep blue and violet light",
        it: "Mezzanotte: luce blu profondo e viola",
        settings: &[
            ("background_effect", "aurora"),
            ("effect_palette", "midnight"),
            ("text_color", "#C9D4FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#3A5BFF, #8B5CF6"),
            ("dash_colors", "#6C8CFF, #9B6BFF, #4FD1FF"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "ocean",
        en: "Ocean: waves in blue and teal",
        it: "Oceano: onde blu e verde acqua",
        settings: &[
            ("background_effect", "waves"),
            ("effect_palette", "ocean"),
            ("text_color", "#D6F4FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#0EA5E9, #2DD4BF"),
            ("dash_colors", "#38BDF8, #2DD4BF, #818CF8"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "sunset",
        en: "Sunset: orange, pink and violet",
        it: "Tramonto: arancio, rosa e viola",
        settings: &[
            ("background_effect", "plasma"),
            ("effect_palette", "sunset"),
            ("text_color", "#FFE7D6"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#FF7A18, #FF3CAC"),
            ("dash_colors", "#FF9F43, #FF6FB1, #A76BFF"),
            ("panel_opacity", "50"),
        ],
    },
    Theme {
        name: "forest",
        en: "Forest: soft greens",
        it: "Foresta: verdi morbidi",
        settings: &[
            ("background_effect", "aurora"),
            ("effect_palette", "forest"),
            ("text_color", "#DFF7E3"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#34D399, #A3E635"),
            ("dash_colors", "#4ADE80, #A3E635, #2DD4BF"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "ember",
        en: "Ember: a lava lamp in red and amber",
        it: "Brace: una lava lamp rossa e ambra",
        settings: &[
            ("background_effect", "lava"),
            ("effect_palette", "ember"),
            ("text_color", "#FFE9CC"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#F59E0B, #EF4444"),
            ("dash_colors", "#FBBF24, #F97316, #EF4444"),
            ("panel_opacity", "50"),
        ],
    },
    Theme {
        name: "neon",
        en: "Neon: a network of magenta and cyan",
        it: "Neon: una rete di magenta e azzurro",
        settings: &[
            ("background_effect", "network"),
            ("effect_palette", "neon"),
            ("text_color", "#F2E9FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#FF2BD6, #22D3EE"),
            ("dash_colors", "#FF2BD6, #22D3EE, #A78BFA"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "matrix",
        en: "Matrix: green digital rain",
        it: "Matrix: pioggia digitale verde",
        settings: &[
            ("background_effect", "rain"),
            ("effect_palette", "matrix"),
            ("text_color", "#9BFFB0"),
            ("clock_color", "#D8FFE0"),
            ("spectrum_gradient", "#16A34A, #86EFAC"),
            ("dash_colors", "#22C55E, #86EFAC, #4ADE80"),
            ("panel_opacity", "60"),
        ],
    },
    Theme {
        name: "mono",
        en: "Mono: black and white, no animation",
        it: "Mono: bianco e nero, senza animazioni",
        settings: &[
            ("background_color", "#0B0B0D"),
            ("text_color", "#D0D0D4"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#8A8A92, #F2F2F5"),
            ("dash_colors", "#E5E5EA, #A1A1AA, #71717A"),
            ("panel_opacity", "40"),
        ],
    },
];

/// The names a `theme` can have, for the messages and the editor.
pub fn names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}

pub fn find(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{check_value, fields};

    #[test]
    fn every_theme_sets_only_known_valid_settings_of_the_look() {
        let all = fields();
        for t in THEMES {
            assert!(!t.settings.is_empty() && !t.en.is_empty() && !t.it.is_empty(), "{}", t.name);
            for (k, v) in t.settings {
                let f = all.iter().find(|f| f.key == *k).unwrap_or_else(|| panic!("{}: unknown setting {k}", t.name));
                assert!(crate::share::shareable(f), "{}: {k} is not part of the look", t.name);
                assert!(check_value(f, v), "{}: {k} = {v} is not valid", t.name);
            }
            let mut keys: Vec<&str> = t.settings.iter().map(|(k, _)| *k).collect();
            keys.sort();
            let n = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), n, "{}: a setting twice", t.name);
        }
    }

    #[test]
    fn the_editor_offers_exactly_the_themes_there_are() {
        let mut offered: Vec<&str> = crate::schema::THEME_NAMES.iter().copied().filter(|n| *n != "none").collect();
        let mut real = names();
        offered.sort();
        real.sort();
        assert_eq!(offered, real);
    }

    #[test]
    fn a_theme_gives_defaults_and_the_file_wins() {
        use crate::config::ConfigFile;
        let f = ConfigFile::parse("theme = ocean\nclock_color = #FF0000\n").unwrap();
        assert_eq!(f.get("clock_color"), Some("#FF0000"), "the file wins");
        assert_eq!(f.get("background_effect"), Some("waves"), "the theme fills in the rest");
        let none = ConfigFile::parse("theme = none\n").unwrap();
        assert_eq!(none.get("background_effect"), None);
        assert!(ConfigFile::parse("theme = nope\n").is_err());
    }

    #[test]
    fn names_are_unique_and_found_in_any_case() {
        let mut n = names();
        n.sort();
        let len = n.len();
        n.dedup();
        assert_eq!(n.len(), len);
        assert_eq!(find(" Ocean ").map(|t| t.name), Some("ocean"));
        assert!(find("nope").is_none());
    }
}
