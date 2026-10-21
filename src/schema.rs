//! The table of settings shown by the settings editor (`trofeo_config`): for every key of
//! `trofeo.conf` its section, a label and a hint in English and Italian, the kind of value
//! (a switch, a choice, a number in a range, a color, ...) and the default.
//!
//! It is plain data. The program's own tests check every value offered here against the
//! real parsers (see the `schema_*` tests in `main.rs`), so the editor cannot offer a value
//! the program would refuse.

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `true` / `false`.
    Bool,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// Several of these words, comma-separated (`all` / `none`, when listed, stand alone).
    Multi(&'static [&'static str]),
    Int { min: i64, max: i64 },
    Float { min: f32, max: f32 },
    /// A color: a name, `#RRGGBB` or `R,G,B`.
    Color,
    /// One or more colors, comma-separated (two or more make a gradient).
    ColorList,
    /// `auto`, or one or more colors (the bars of the tiles).
    BarColor,
    Text,
    /// A file.
    Path,
    /// A folder.
    Folder,
    /// A single column of tiles (`frame_left` / `frame_right`).
    TileColumn,
    /// The `layout` list (name and optionally `:seconds`).
    Layout,
    /// The `tiles` arrangement.
    Tiles,
    /// A list of panels for `deck`, `orbit` (one line) or `cards` (`|` between columns, `,` stacks).
    Panels { columns: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub key: String,
    /// Id of the section (see [`SECTIONS`]).
    pub section: &'static str,
    /// A sub-heading inside the section ("" = none).
    pub group: &'static str,
    pub en: String,
    pub it: String,
    pub help_en: String,
    pub help_it: String,
    pub kind: Kind,
    /// What applies when the key is not in the file ("" = nothing).
    pub default: String,
    /// Defaults that differ by animated background (`background_effect` value -> default).
    pub style_defaults: &'static [(&'static str, &'static str)],
    /// Shown only while `key` has one of these values (the effective one: the file's, or the default).
    pub when: Option<(&'static str, &'static [&'static str])>,
    /// Changing it needs a restart of `trofeo_lcd`.
    pub restart: bool,
}

impl Field {
    fn new(key: &str, section: &'static str, en: &str, it: &str, kind: Kind) -> Field {
        Field {
            key: key.to_string(),
            section,
            group: "",
            en: en.to_string(),
            it: it.to_string(),
            help_en: String::new(),
            help_it: String::new(),
            kind,
            default: String::new(),
            style_defaults: &[],
            when: None,
            restart: false,
        }
    }
    fn style_def(mut self, d: &'static [(&'static str, &'static str)]) -> Field {
        self.style_defaults = d;
        self
    }
    /// What applies when the key is not in the file, given the animated background in use.
    pub fn default_for(&self, effect: &str) -> String {
        self.style_defaults.iter().find(|(e, _)| *e == effect).map(|(_, d)| d.to_string()).unwrap_or_else(|| self.default.clone())
    }
    fn def(mut self, d: &str) -> Field {
        self.default = d.to_string();
        self
    }
    fn help(mut self, en: &str, it: &str) -> Field {
        self.help_en = en.to_string();
        self.help_it = it.to_string();
        self
    }
    fn when(mut self, key: &'static str, values: &'static [&'static str]) -> Field {
        self.when = Some((key, values));
        self
    }
    fn restart(mut self) -> Field {
        self.restart = true;
        self
    }
    fn group(mut self, g: &'static str) -> Field {
        self.group = g;
        self
    }
}

/// (id, English, Italian), in the order of the editor's list.
pub const SECTIONS: &[(&str, &str, &str)] = &[
    ("screen", "Screen", "Schermo"),
    ("background", "Background", "Sfondo"),
    ("effect", "Animated background", "Sfondo animato"),
    ("layouts", "Screens (layouts)", "Schermate (layout)"),
    ("tiles", "Tiles", "Tiles"),
    ("dash", "Deck, Cards and Orbit", "Deck, Cards e Orbit"),
    ("frame", "Digital frame", "Cornice digitale"),
    ("lhm", "Fans and sensors (LibreHardwareMonitor)", "Ventole e sensori (LibreHardwareMonitor)"),
    ("show", "What to show", "Cosa mostrare"),
    ("look", "Colors and text", "Colori e testo"),
    ("spectrum", "Spectrum", "Spettro audio"),
    ("items", "Single items", "Singole voci"),
    ("music", "Music card", "Scheda musica"),
    ("game", "Pixel game", "Mini gioco"),
    ("online", "Weather and news", "Meteo e notizie"),
    ("system", "System", "Sistema"),
];

pub const POSITIONS: &[&str] =
    &["top-left", "top", "top-right", "center-left", "center", "center-right", "bottom-left", "bottom", "bottom-right"];
pub const LANGUAGES: &[&str] = &["en", "it", "de", "es", "fr", "pt"];
pub const EFFECTS: &[&str] = &["none", "plasma", "aurora", "lava", "waves", "matrix", "rain", "aquarium", "network", "globe"];
pub const EFFECT_PALETTES: &[&str] = &[
    "pastel", "fluo", "neon", "candy", "sunset", "ocean", "aurora", "fire", "forest", "ice", "mono", "rainbow", "matrix", "amber",
    "violet", "crimson", "snow", "lagoon", "deep", "tropical", "custom", "all",
];
pub const SHOW_ITEMS: &[&str] = &[
    "cpu", "gpu", "uptime", "time", "date", "mem", "net", "disk", "volume", "nowplaying", "weather", "spectrum", "ticker", "clock",
    "clock_date", "dashboard",
];
pub const HIDE_ITEMS: &[&str] = &[
    "cpu", "gpu", "uptime", "time", "date", "mem", "net", "disk", "volume", "nowplaying", "weather", "spectrum", "ticker", "clock",
    "clock_date", "dashboard", "cpu_freq", "cpu_temp", "cpu_power", "gpu_temp", "gpu_power", "gpu_fan", "gpu_clock", "gpu_fps",
    "cpu_fan", "case_fan", "pump",
];
pub const SPECTRUM_STYLES: &[&str] = &["bars", "led", "peaks", "area", "mirror", "all"];
pub const SPECTRUM_PALETTES: &[&str] =
    &["default", "rainbow", "fire", "ocean", "sunset", "neon", "ice", "matrix", "purple", "custom", "all"];
pub const MUSIC_BG: &[&str] = &["gradient", "blur", "color", "solid", "none", "effect"];
pub const MUSIC_STATS: &[&str] = &["cpu", "gpu", "ram", "net", "disk", "time", "uptime", "all", "none"];
pub const FRAME_STYLES: &[&str] = &["mosaic", "polaroid"];
pub const FRAME_BG: &[&str] = &["blur", "dark", "global"];
pub const DASH_PALETTES: &[&str] = &[
    "default", "pastel", "fluo", "neon", "candy", "sunset", "ocean", "aurora", "fire", "forest", "ice", "mono", "rainbow", "matrix", "amber",
    "violet", "crimson", "snow", "lagoon", "deep", "tropical",
];
pub const TILES_BG: &[&str] = &["global", "color", "gradient", "effect", "image"];

/// The items of the info block with per-item options (`<item>_size`, ...): (key, English, Italian).
pub const ITEMS: &[(&str, &str, &str)] = &[
    ("cpu", "CPU", "CPU"),
    ("gpu", "GPU", "GPU"),
    ("uptime", "Uptime", "Tempo acceso"),
    ("time", "Time", "Ora"),
    ("date", "Date", "Data"),
    ("mem", "RAM", "RAM"),
    ("net", "Network", "Rete"),
    ("disk", "Disk", "Disco"),
    ("volume", "Volume", "Volume"),
    ("nowplaying", "Now playing", "Brano in ascolto"),
    ("weather", "Weather", "Meteo"),
];

/// The screens of `layout = ...`: (name, English, Italian).
pub const LAYOUT_NAMES: &[(&str, &str, &str)] = &[
    ("default", "Standard screen (status, spectrum, big clock)", "Schermata standard (stato, spettro, orologio grande)"),
    ("default2", "Standard screen with per-item size/position/color", "Standard con dimensione/posizione/colore per voce"),
    ("cpu-gpu", "CPU and GPU side by side", "CPU e GPU affiancate"),
    ("temps", "CPU and GPU temperatures, big", "Temperature CPU e GPU, grandi"),
    ("overview", "CPU, GPU, RAM and clock", "CPU, GPU, RAM e orologio"),
    ("grid6", "3x2 grid", "Griglia 3x2"),
    ("clock-center", "Clock in the middle, CPU left, GPU right", "Orologio al centro, CPU a sinistra, GPU a destra"),
    ("io", "Network, disk and RAM", "Rete, disco e RAM"),
    ("music", "Current track, big, and clock", "Brano in ascolto, grande, e orologio"),
    ("gaming", "FPS, GPU and temperatures", "FPS, GPU e temperature"),
    ("cpu", "CPU only, big", "Solo CPU, grande"),
    ("gpu", "GPU only, big", "Solo GPU, grande"),
    ("tiles", "Glass tiles (choose them in the Tiles section)", "Tiles di vetro (si scelgono nella sezione Tiles)"),
    ("deck", "Instrument columns with graphs (choose them in the Deck, Cards and Orbit section)", "Colonne strumento con grafici (si scelgono nella sezione Deck, Cards e Orbit)"),
    ("cards", "Colourful cards in a mosaic (see the Deck, Cards and Orbit section)", "Card colorate a mosaico (vedi la sezione Deck, Cards e Orbit)"),
    ("orbit", "Concentric rings per device (see the Deck, Cards and Orbit section)", "Anelli concentrici per dispositivo (vedi la sezione Deck, Cards e Orbit)"),
    ("frame", "Digital picture frame (the folder is set in the Digital frame section)", "Cornice digitale (la cartella si sceglie nella sezione Cornice digitale)"),
    ("weather", "Weather, big", "Meteo, grande"),
    ("news", "News headlines, scrolling", "Notizie scorrevoli"),
];

/// The tiles of `tiles = ...`: (name, English, Italian).
pub const TILE_KINDS: &[(&str, &str, &str)] = &[
    ("clock", "Clock", "Orologio"),
    ("cpu", "CPU load", "Carico CPU"),
    ("cpu_temp", "CPU temperature", "Temperatura CPU"),
    ("cpu_watt", "CPU power", "Consumo CPU"),
    ("gpu", "GPU load", "Carico GPU"),
    ("gpu_temp", "GPU temperature", "Temperatura GPU"),
    ("gpu_watt", "GPU power", "Consumo GPU"),
    ("ram", "RAM", "RAM"),
    ("net", "Network", "Rete"),
    ("disk", "Disk", "Disco"),
    ("fps", "FPS", "FPS"),
    ("weather", "Weather", "Meteo"),
    ("music", "Now playing", "Brano in ascolto"),
    ("cpu_fan", "CPU fan (LibreHardwareMonitor)", "Ventola CPU (LibreHardwareMonitor)"),
    ("case_fan", "Case fans (LibreHardwareMonitor)", "Ventole del case (LibreHardwareMonitor)"),
    ("pump", "Pump / AIO (LibreHardwareMonitor)", "Pompa / AIO (LibreHardwareMonitor)"),
    ("gpu_fan", "GPU fan", "Ventola GPU"),
    ("fans", "All the fans in one tile", "Tutte le ventole in un tile"),
    ("mb_temp", "Motherboard temperature", "Temperatura scheda madre"),
    ("disk_temp", "Disk temperature", "Temperatura disco"),
];

/// What a deck column / card / orbit cluster can show: (name, English, Italian).
pub const PANEL_KINDS: &[(&str, &str, &str)] = &[
    ("cpu", "CPU", "CPU"),
    ("gpu", "GPU", "GPU"),
    ("ram", "RAM", "RAM"),
    ("fans", "Fans (LibreHardwareMonitor)", "Ventole (LibreHardwareMonitor)"),
    ("net", "Network", "Rete"),
    ("disk", "Disk", "Disco"),
    ("clock", "Clock, weather and music", "Orologio, meteo e musica"),
    ("weather", "Weather", "Meteo"),
    ("music", "Now playing", "Brano in ascolto"),
];

/// Named colors offered as swatches (all of them are understood by the program).
pub const COLOR_NAMES: &[(&str, (u8, u8, u8))] = &[
    ("black", (0x00, 0x00, 0x00)),
    ("white", (0xFF, 0xFF, 0xFF)),
    ("gray", (0x80, 0x80, 0x80)),
    ("lightgray", (0xD3, 0xD3, 0xD3)),
    ("darkgray", (0x40, 0x40, 0x40)),
    ("silver", (0xC0, 0xC0, 0xC0)),
    ("red", (0xFF, 0x00, 0x00)),
    ("darkred", (0x80, 0x00, 0x00)),
    ("orange", (0xFF, 0xA5, 0x00)),
    ("gold", (0xFF, 0xD7, 0x00)),
    ("yellow", (0xFF, 0xFF, 0x00)),
    ("green", (0x00, 0xFF, 0x00)),
    ("lightgreen", (0x90, 0xEE, 0x90)),
    ("darkgreen", (0x00, 0x64, 0x00)),
    ("olive", (0x80, 0x80, 0x00)),
    ("teal", (0x00, 0x80, 0x80)),
    ("turquoise", (0x40, 0xE0, 0xD0)),
    ("cyan", (0x00, 0xFF, 0xFF)),
    ("skyblue", (0x87, 0xCE, 0xEB)),
    ("lightblue", (0xAD, 0xD8, 0xE6)),
    ("blue", (0x00, 0x00, 0xFF)),
    ("navy", (0x00, 0x00, 0x80)),
    ("indigo", (0x4B, 0x00, 0x82)),
    ("purple", (0x80, 0x00, 0x80)),
    ("violet", (0xEE, 0x82, 0xEE)),
    ("lavender", (0xE6, 0xE6, 0xFA)),
    ("magenta", (0xFF, 0x00, 0xFF)),
    ("hotpink", (0xFF, 0x69, 0xB4)),
    ("pink", (0xFF, 0xC0, 0xCB)),
    ("salmon", (0xFA, 0x80, 0x72)),
    ("coral", (0xFF, 0x7F, 0x50)),
    ("brown", (0x8B, 0x45, 0x13)),
    ("beige", (0xF5, 0xF5, 0xDC)),
];

/// `#RRGGBB` (or `RRGGBB`) to a color.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((b(0)?, b(2)?, b(4)?))
}

/// A color as the config file writes it: a name from [`COLOR_NAMES`], `#RRGGBB` or `R,G,B`.
pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim();
    if let Some((_, c)) = COLOR_NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(t)) {
        return Some(*c);
    }
    if let Some(c) = parse_hex(t) {
        return Some(c);
    }
    let parts: Vec<&str> = t.split(',').collect();
    if parts.len() == 3 {
        let v: Vec<u8> = parts.iter().filter_map(|p| p.trim().parse().ok()).collect();
        if v.len() == 3 {
            return Some((v[0], v[1], v[2]));
        }
    }
    None
}

pub fn to_hex(c: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Splits `a, b, c` into trimmed non-empty words.
pub fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

// ------------------------------------------------------------------------ the fields

fn int(min: i64, max: i64) -> Kind {
    Kind::Int { min, max }
}

/// Every setting of the editor, in the order they are shown.
pub fn fields() -> Vec<Field> {
    use Kind::*;
    let on_effect: &[&str] = &["plasma", "aurora", "lava", "waves", "matrix", "rain", "aquarium", "network", "globe"];
    let themed: &[&str] = &["matrix", "rain", "aquarium", "network", "globe"];
    let music_on: &[&str] = &["true"];
    let mut v: Vec<Field> = Vec::new();
    let mut add = |f: Field| v.push(f);

    // ---- screen
    add(Field::new("orientation", "screen", "Orientation", "Orientamento", Choice(&["landscape", "portrait"]))
        .def("landscape")
        .help("landscape = horizontal; portrait = the display is mounted vertically", "landscape = orizzontale; portrait = display montato in verticale"));
    add(Field::new("flip", "screen", "Rotate 180°", "Ruota di 180°", Bool)
        .def("false")
        .help("If the picture comes out upside down", "Se l'immagine esce capovolta"));
    add(Field::new("panel", "screen", "Display model", "Modello di display", Choice(&["trofeo-9.16", "trofeo-11.3"]))
        .help("Unset = recognised automatically when the display is connected", "Non impostato = riconosciuto in automatico quando il display è collegato"));
    add(Field::new("panel_size", "screen", "Own picture size", "Dimensione immagine personalizzata", Text)
        .help("Only for unusual panels, e.g. 1920x462", "Solo per pannelli particolari, es. 1920x462"));
    add(Field::new("brightness", "screen", "Brightness", "Luminosità", int(0, 100)).def("100"));
    add(Field::new("language", "screen", "Language of the display", "Lingua del display", Choice(LANGUAGES)).def("en"));
    for (k, en, it) in [
        ("margin", "All sides", "Tutti i lati"),
        ("margin_top", "Top", "Alto"),
        ("margin_bottom", "Bottom", "Basso"),
        ("margin_left", "Left", "Sinistra"),
        ("margin_right", "Right", "Destra"),
    ] {
        let max = if k == "margin" { 150 } else { 220 };
        add(Field::new(k, "screen", en, it, int(0, max)).def("0").group("margins"));
    }

    // ---- background
    add(Field::new("background", "background", "Picture, GIF or video", "Immagine, GIF o video", Path)
        .help("jpg, png, bmp, gif or a video (needs ffmpeg)", "jpg, png, bmp, gif o un video (serve ffmpeg)"));
    add(Field::new("background_color", "background", "Solid color", "Colore solido", Color)
        .help("Used when there is no picture and no animated background", "Usato se non ci sono né immagine né sfondo animato"));
    add(Field::new("background_dim", "background", "Darken the picture", "Scurisci l'immagine", int(0, 100)).def("40"));
    add(Field::new("background_fit", "background", "Fit", "Adattamento", Choice(&["cover", "stretch", "contain", "original"])).def("cover"));
    add(Field::new("background_position", "background", "Position", "Posizione", Choice(POSITIONS)).def("center"));
    add(Field::new("background_offset_x", "background", "Shift right (px)", "Sposta a destra (px)", int(-2000, 2000)).def("0"));
    add(Field::new("background_offset_y", "background", "Shift down (px)", "Sposta in basso (px)", int(-2000, 2000)).def("0"));
    add(Field::new("ffmpeg", "background", "Path of ffmpeg", "Percorso di ffmpeg", Path)
        .help("Only if ffmpeg.exe is not next to the program", "Solo se ffmpeg.exe non è accanto al programma"));

    // ---- animated background
    add(Field::new("background_effect", "effect", "Effect", "Effetto", Choice(EFFECTS))
        .def("none")
        .help("It replaces the picture while on", "Sostituisce l'immagine finché è attivo"));
    add(Field::new("effect_theme", "effect", "Dark or light", "Scuro o chiaro", Choice(&["dark", "light"]))
        .def("dark")
        .when("background_effect", themed)
        .help("light = a white background (matrix, rain, aquarium, network, globe)", "light = sfondo bianco (matrix, rain, acquario, network, globe)"));
    add(Field::new("effect_level", "effect", "Water level / number of nodes", "Livello dell'acqua / numero di nodi", int(0, 100))
        .def("50")
        .when("background_effect", &["aquarium", "network", "globe"])
        .help("Aquarium: how much of the panel is water. Network and globe: how many nodes", "Acquario: quanta parte del pannello è acqua. Network e globe: quanti nodi"));
    add(Field::new("effect_palette", "effect", "Colors", "Colori", Multi(EFFECT_PALETTES))
        .def("neon")
        .style_def(&[("matrix", "matrix"), ("rain", "matrix"), ("aquarium", "lagoon"), ("network", "ice"), ("globe", "ice")])
        .when("background_effect", on_effect)
        .help("Several = they rotate. matrix/amber/violet/crimson/snow are for matrix and rain; lagoon/deep/tropical for the aquarium; network and globe use ice unless you choose", "Più di uno = ruotano. matrix/amber/violet/crimson/snow per matrix e rain; lagoon/deep/tropical per l'acquario; network e globe usano ice se non scegli"));
    add(Field::new("effect_colors", "effect", "Own colors", "Colori personalizzati", ColorList)
        .when("background_effect", on_effect)
        .help("2 or more colors; used when no palette is chosen, or add \"custom\" to the palettes", "2 o più colori; usati se non scegli una palette, oppure aggiungi \"custom\" alle palette"));
    add(Field::new("effect_speed", "effect", "Speed", "Velocità", int(0, 100)).def("30").when("background_effect", on_effect));
    add(Field::new("effect_scale", "effect", "Size of the shapes (%)", "Dimensione delle forme (%)", int(25, 400)).def("100").when("background_effect", on_effect));
    add(Field::new("effect_brightness", "effect", "Brightness", "Luminosità", int(0, 100))
        .def("40")
        .style_def(&[("aquarium", "65"), ("network", "70"), ("globe", "70")])
        .when("background_effect", on_effect)
        .help("Lower = text easier to read. Unset: 65 for the aquarium and the light variants, 70 for network and globe", "Più basso = testo più leggibile. Non impostato: 65 per l'acquario e le varianti chiare, 70 per network e globe"));
    add(Field::new("effect_fps", "effect", "Frames per second", "Fotogrammi al secondo", int(1, 30))
        .def("8")
        .when("background_effect", on_effect)
        .help("Lower = lighter on the CPU", "Più basso = più leggero per la CPU"));
    add(Field::new("effect_palette_interval", "effect", "Seconds per palette", "Secondi per palette", int(5, 3600))
        .def("60")
        .when("background_effect", on_effect));

    // ---- layouts
    add(Field::new("layout", "layouts", "Screens", "Schermate", Layout)
        .help("One screen, or several that take turns (each can have its own duration)", "Una schermata, oppure più schermate a rotazione (ognuna con la sua durata)"));
    add(Field::new("layout_interval", "layouts", "Seconds per screen", "Secondi per schermata", int(2, 3600)).def("15"));
    add(Field::new("layout_spectrum", "layouts", "Spectrum over the panels", "Spettro sopra i pannelli", Bool)
        .def("false")
        .help("Show the audio spectrum on top of a panel while music plays", "Mostra lo spettro sopra un pannello mentre suona la musica"));
    add(Field::new("layout_hold_on_music", "layouts", "Stay on the first screen while music plays", "Resta sulla prima schermata mentre suona la musica", Bool).def("false"));
    add(Field::new("panel_opacity", "layouts", "Panel opacity", "Opacità dei pannelli", int(0, 100))
        .def("100")
        .help("100 = solid, 0 = see-through (only the border)", "100 = pieno, 0 = trasparente (resta il bordo)"));

    // ---- tiles
    add(Field::new("tiles", "tiles", "Tiles to show", "Tiles da mostrare", Tiles)
        .help("Used by the \"tiles\" screen", "Usato dalla schermata \"tiles\""));
    add(Field::new("tiles_font", "tiles", "Font", "Font", Choice(&["smooth", "pixel"])).def("smooth"));
    add(Field::new("tiles_theme", "tiles", "Theme", "Tema", Choice(&["dark", "light"])).def("dark"));
    add(Field::new("tiles_color", "tiles", "Tile color", "Colore dei tile", Color));
    add(Field::new("tiles_text_color", "tiles", "Text color", "Colore del testo", Color));
    add(Field::new("tiles_opacity", "tiles", "Opacity", "Opacità", int(0, 100)));
    add(Field::new("tiles_radius", "tiles", "Corner radius", "Raggio degli angoli", int(0, 80)));
    add(Field::new("tiles_gap", "tiles", "Gap between tiles", "Spazio tra i tile", int(0, 60)).def("14"));
    add(Field::new("tiles_clock_seconds", "tiles", "Seconds on the clock", "Secondi sull'orologio", Bool).def("false"));
    add(Field::new("tiles_cpu_watt_max", "tiles", "CPU power at 100% (W)", "Consumo CPU al 100% (W)", int(10, 2000)).def("200"));
    add(Field::new("tiles_gpu_watt_max", "tiles", "GPU power at 100% (W)", "Consumo GPU al 100% (W)", int(10, 2000)).def("400"));
    add(Field::new("tiles_fps_max", "tiles", "FPS at 100%", "FPS al 100%", int(30, 1000)).def("240"));
    add(Field::new("tiles_rpm", "tiles", "Fan speeds on the CPU tiles", "Giri delle ventole sui tile CPU", Bool)
        .def("true")
        .help("Adds a line like \"CPU 1240 / CASE 780 RPM\" to the CPU tiles (needs LibreHardwareMonitor)", "Aggiunge ai tile CPU una riga tipo \"CPU 1240 / CASE 780 RPM\" (serve LibreHardwareMonitor)"));
    add(Field::new("tiles_fan_max", "tiles", "Fan speed at 100% (RPM)", "Giri della ventola al 100% (RPM)", int(500, 10000)).def("2500"));
    add(Field::new("tiles_bg", "tiles", "Background behind the tiles", "Sfondo dietro i tile", Choice(TILES_BG))
        .def("global")
        .group("background")
        .help("global = the normal background of the program", "global = lo sfondo normale del programma"));
    add(Field::new("tiles_bg_color", "tiles", "Color(s)", "Colore/i", ColorList).group("background").when("tiles_bg", &["color", "gradient"]));
    add(Field::new("tiles_bg_image", "tiles", "Picture", "Immagine", Path).group("background").when("tiles_bg", &["image"]));
    add(Field::new("tiles_bg_dim", "tiles", "Darken", "Scurisci", int(0, 100)).def("0").group("background").when("tiles_bg", &["image"]));
    add(Field::new("tiles_palette", "tiles", "Tile colors from the color presets", "Colori dei tile dai preset", Multi(EFFECT_PALETTES))
        .group("palette")
        .help(
            "Every tile gets its own color from these presets, and the colors drift and fade from preset to preset (pick several, or \"all\"). Empty = every tile has the same color",
            "Ogni tile prende un suo colore da questi preset, e i colori scorrono e sfumano da un preset all'altro (scegline più di uno, o \"all\"). Vuoto = tutti i tile dello stesso colore",
        ));
    add(Field::new("tiles_colors", "tiles", "Own colors (for \"custom\")", "Colori propri (per \"custom\")", ColorList)
        .group("palette")
        .help("Two or more colors, used when \"custom\" is chosen above", "Due o più colori, usati quando sopra si sceglie \"custom\""));
    add(Field::new("tiles_palette_interval", "tiles", "Seconds per preset", "Secondi per preset", int(5, 3600))
        .def("20")
        .group("palette")
        .help("How long a preset stays before the next one fades in", "Quanto resta un preset prima che sfumi il successivo"));
    add(Field::new("tiles_palette_stagger", "tiles", "Tiles change at different times (%)", "I tile cambiano in momenti diversi (%)", int(0, 100))
        .def("50")
        .group("palette")
        .help("0 = all tiles change preset together; 100 = spread over the whole interval", "0 = tutti i tile cambiano preset insieme; 100 = distribuiti su tutto l'intervallo"));
    add(Field::new("tiles_bar_color", "tiles", "All bars", "Tutte le barre", BarColor)
        .def("auto")
        .group("bars")
        .help("auto = by level; one color, or two or more for a gradient", "auto = in base al livello; un colore, o due o più per un gradiente"));
    for (k, en, it) in TILE_KINDS {
        add(Field::new(&format!("tile_{k}_color"), "tiles", en, it, BarColor).def("auto").group("bars"));
    }

    // ---- deck, cards, orbit
    add(Field::new("deck", "dash", "Columns", "Colonne", Panels { columns: false })
        .def("cpu, gpu, ram, clock")
        .group("deck")
        .help("Layout \"deck\": instrument columns with a live gauge, a big value, a graph and four readings", "Layout \"deck\": colonne strumento con un indicatore, un valore grande, un grafico e quattro letture"));
    add(Field::new("deck_gauge", "dash", "Gauge line on top of each column", "Linea indicatore sopra ogni colonna", Bool).def("true").group("deck"));
    add(Field::new("cards", "dash", "Cards", "Card", Panels { columns: true })
        .def("clock | cpu | gpu | ram, disk | music, net")
        .group("cards")
        .help("Layout \"cards\": every group is a column, cards in the same group are stacked", "Layout \"cards\": ogni gruppo è una colonna, le card dello stesso gruppo sono impilate"));
    add(Field::new("cards_radius", "dash", "Corner radius", "Raggio degli angoli", int(0, 60)).group("cards"));
    add(Field::new("cards_opacity", "dash", "Card opacity (%)", "Opacità delle card (%)", int(0, 100))
        .def("100")
        .group("cards")
        .help("Lower it to see the background through the cards", "Abbassala per vedere lo sfondo attraverso le card"));
    add(Field::new("cards_fan", "dash", "Fan speed in the card", "Giri ventola nella card", Bool)
        .def("true")
        .group("cards")
        .help(
            "The fan RPM at the top right of the CPU and GPU cards (needs LibreHardwareMonitor)",
            "I giri della ventola in alto a destra nelle card CPU e GPU (serve LibreHardwareMonitor)",
        ));
    add(Field::new("cards_gap", "dash", "Gap between cards", "Spazio tra le card", int(0, 40)).def("12").group("cards"));
    add(Field::new("orbit", "dash", "Devices", "Dispositivi", Panels { columns: false })
        .def("cpu, gpu, ram")
        .group("orbit")
        .help("Layout \"orbit\": a cluster of concentric rings for each (up to 4)", "Layout \"orbit\": un gruppo di anelli concentrici per ognuno (fino a 4)"));
    add(Field::new("orbit_clock", "dash", "Clock block on the right", "Blocco orologio a destra", Bool).def("true").group("orbit"));
    add(Field::new("orbit_legend", "dash", "Legend under the rings", "Legenda sotto gli anelli", Bool).def("true").group("orbit"));
    add(Field::new("dash_palette", "dash", "Colors from a preset", "Colori da un preset", Choice(DASH_PALETTES))
        .def("default")
        .group("look")
        .help("The accent colors of the rings, cards and graphs (the same presets as the animated backgrounds)", "I colori di anelli, card e grafici (gli stessi preset degli sfondi animati)"));
    add(Field::new("dash_colors", "dash", "Own colors", "Colori propri", ColorList)
        .group("look")
        .help("One or more colors, used instead of the preset", "Uno o più colori, usati al posto del preset"));
    add(Field::new("dash_hero", "dash", "Big number of CPU and GPU", "Numero grande di CPU e GPU", Choice(&["temp", "load"]))
        .def("temp")
        .group("look")
        .help(
            "temp = the temperature in big, the load becomes a small reading; load = the opposite (deck, cards and orbit)",
            "temp = in grande la temperatura, il carico diventa una lettura piccola; load = il contrario (deck, cards e orbit)",
        ));
    add(Field::new("dash_font", "dash", "Font", "Font", Choice(&["smooth", "pixel"])).def("smooth").group("look"));
    add(Field::new("dash_theme", "dash", "Theme", "Tema", Choice(&["dark", "light"]))
        .def("dark")
        .group("look")
        .help(
            "light = a white plate, dark text and pastel cards",
            "light = fondo bianco, testo scuro e card pastello",
        ));
    add(Field::new("dash_text_color", "dash", "Text color", "Colore del testo", Color).group("look").help("Empty = by theme", "Vuoto = in base al tema"));
    add(Field::new("dash_plate_color", "dash", "Plate color (deck, orbit)", "Colore del fondo (deck, orbit)", Color)
        .group("look")
        .help("The panel behind deck and orbit. Empty = by theme (dark, or white)", "Il pannello dietro deck e orbit. Vuoto = in base al tema (scuro, o bianco)"));
    add(Field::new("dash_plate", "dash", "Plate opacity, deck and orbit (%)", "Opacità del fondo, deck e orbit (%)", int(0, 100))
        .def("50")
        .group("look")
        .help("50 on the dark theme, 85 on the light one. Lower it to see the animated background through the plate; 0 = no plate", "50 col tema scuro, 85 con quello chiaro. Abbassala per vedere lo sfondo animato attraverso il pannello; 0 = nessun pannello"));
    add(Field::new("dash_graph", "dash", "Graphs of the last minutes", "Grafici degli ultimi minuti", Bool).def("true").group("graphs"));
    add(Field::new("dash_history", "dash", "Seconds of history", "Secondi di storico", int(20, 600)).def("90").group("graphs").when("dash_graph", &["true"]));
    add(Field::new("dash_net_max", "dash", "Network speed at 100% (MB/s)", "Velocità di rete al 100% (MB/s)", int(1, 1000)).def("25").group("graphs"));
    add(Field::new("dash_bg", "dash", "Background", "Sfondo", Choice(TILES_BG))
        .def("global")
        .group("background")
        .help("global = the normal background of the program", "global = lo sfondo normale del programma"));
    add(Field::new("dash_bg_color", "dash", "Color(s)", "Colore/i", ColorList).group("background").when("dash_bg", &["color", "gradient"]));
    add(Field::new("dash_bg_image", "dash", "Picture", "Immagine", Path).group("background").when("dash_bg", &["image"]));
    add(Field::new("dash_bg_dim", "dash", "Darken", "Scurisci", int(0, 100)).def("0").group("background").when("dash_bg", &["image"]));

    // ---- LibreHardwareMonitor
    add(Field::new("lhm", "lhm", "Read fans and sensors from LibreHardwareMonitor", "Leggi ventole e sensori da LibreHardwareMonitor", Bool)
        .def("true")
        .help(
            "Start LibreHardwareMonitor (as administrator), then Options > Remote Web Server > Run. Nothing happens if it is not running",
            "Avvia LibreHardwareMonitor (come amministratore), poi Options > Remote Web Server > Run. Se non è in esecuzione non succede nulla",
        ));
    add(Field::new("lhm_url", "lhm", "Address", "Indirizzo", Text).def("http://127.0.0.1:8085").when("lhm", &["true"]));
    add(Field::new("lhm_interval", "lhm", "Seconds between readings", "Secondi tra una lettura e l'altra", int(1, 60)).def("2").when("lhm", &["true"]));
    add(Field::new("lhm_cpu_fan", "lhm", "CPU fan: part of its name", "Ventola CPU: parte del nome", Text)
        .group("names")
        .when("lhm", &["true"])
        .help("Empty = automatic. Run trofeo_lcd --lhm-list to see the names", "Vuoto = automatico. Esegui trofeo_lcd --lhm-list per vedere i nomi"));
    add(Field::new("lhm_case_fan", "lhm", "Case fans: part of their name", "Ventole del case: parte del nome", Text).group("names").when("lhm", &["true"]));
    add(Field::new("lhm_pump", "lhm", "Pump: part of its name", "Pompa: parte del nome", Text).group("names").when("lhm", &["true"]));
    add(Field::new("lhm_mb_temp", "lhm", "Motherboard temperature: part of its name", "Temperatura scheda madre: parte del nome", Text).group("names").when("lhm", &["true"]));

    // ---- digital frame
    add(Field::new("frame_folder", "frame", "Folder with the pictures", "Cartella con le immagini", Folder)
        .help("JPG, PNG, WebP, BMP, GIF. Used by the \"frame\" screen", "JPG, PNG, WebP, BMP, GIF. Usata dalla schermata \"frame\""));
    add(Field::new("frame_style", "frame", "Style", "Stile", Choice(FRAME_STYLES))
        .def("mosaic")
        .help("mosaic = a tidy collage; polaroid = scattered prints with a white border", "mosaic = un collage ordinato; polaroid = stampe sparse con il bordo bianco"));
    add(Field::new("frame_photos", "frame", "Pictures on screen", "Immagini sullo schermo", int(1, 9)).def("5"));
    add(Field::new("frame_interval", "frame", "Seconds between changes", "Secondi tra un cambio e l'altro", int(2, 3600))
        .def("8")
        .help("Every time ONE picture is replaced", "Ogni volta viene sostituita UNA sola immagine"));
    add(Field::new("frame_order", "frame", "Order", "Ordine", Choice(&["shuffle", "name"])).def("shuffle"));
    add(Field::new("frame_transition", "frame", "Transition", "Transizione", Choice(&["fade", "slide"]))
        .def("fade")
        .help("slide = a wipe (mosaic) or a print dropping in (polaroid)", "slide = una tendina (mosaico) o una stampa che cade (polaroid)"));
    add(Field::new("frame_recursive", "frame", "Include subfolders", "Includi le sottocartelle", Bool).def("false"));
    add(Field::new("frame_refresh", "frame", "Look for new pictures every (s)", "Cerca nuove immagini ogni (s)", int(5, 3600)).def("60"));
    add(Field::new("frame_bg", "frame", "Background", "Sfondo", Choice(FRAME_BG))
        .def("blur")
        .group("look")
        .help("blur = the current picture, blurred and dark; global = the normal background of the program", "blur = l'immagine corrente, sfocata e scura; global = lo sfondo normale del programma"));
    add(Field::new("frame_dim", "frame", "Darken the blurred background (%)", "Scurisci lo sfondo sfocato (%)", int(0, 100)).def("60").group("look").when("frame_bg", &["blur"]));
    add(Field::new("frame_gap", "frame", "Gap between pictures", "Spazio tra le immagini", int(0, 40)).def("10").group("look").when("frame_style", &["mosaic"]));
    add(Field::new("frame_radius", "frame", "Corner radius", "Raggio degli angoli", int(0, 60)).def("14").group("look").when("frame_style", &["mosaic"]));
    add(Field::new("frame_left", "frame", "Left side", "Lato sinistro", TileColumn)
        .group("sides")
        .def("clock, ram:ring")
        .help("The tiles in the left column (top strip on a portrait screen)", "I tile nella colonna di sinistra (striscia in alto con lo schermo verticale)"));
    add(Field::new("frame_right", "frame", "Right side", "Lato destro", TileColumn)
        .group("sides")
        .def("cpu:ring, gpu:ring")
        .help("The tiles in the right column (bottom strip on a portrait screen)", "I tile nella colonna di destra (striscia in basso con lo schermo verticale)"));
    add(Field::new("frame_side_width", "frame", "Width of the sides (%)", "Larghezza dei lati (%)", int(8, 35))
        .def("17")
        .group("sides")
        .help("Their look comes from the Tiles section (theme, colors, font)", "L'aspetto viene dalla sezione Tiles (tema, colori, font)"));

    // ---- what to show
    add(Field::new("show", "show", "Show only these", "Mostra solo queste", Multi(SHOW_ITEMS))
        .help("Empty = everything is shown", "Vuoto = si mostra tutto"));
    add(Field::new("hide", "show", "Hide these", "Nascondi queste", Multi(HIDE_ITEMS)));
    add(Field::new("status_style", "show", "Style of the info block", "Stile del blocco info", Choice(&["auto", "lines", "list", "items"]))
        .def("auto")
        .help("lines = three long lines; list = a vertical list", "lines = tre righe lunghe; list = elenco verticale"));
    add(Field::new("status_position", "show", "Info block position", "Posizione del blocco info", Choice(POSITIONS)).def("top-left"));
    add(Field::new("clock_position", "show", "Clock position", "Posizione dell'orologio", Choice(POSITIONS)).def("center"));
    add(Field::new("spectrum_position", "show", "Spectrum position", "Posizione dello spettro", Choice(POSITIONS)).def("center"));
    add(Field::new("spectrum_width", "show", "Spectrum width (%)", "Larghezza dello spettro (%)", int(1, 100)).def("100"));
    add(Field::new("spectrum_height", "show", "Spectrum height (%)", "Altezza dello spettro (%)", int(1, 100)).def("100"));

    // ---- colors and text
    add(Field::new("color", "look", "Spectrum bars", "Barre dello spettro", Color)
        .def("default")
        .help("default = green to red by level", "default = dal verde al rosso in base al livello"));
    add(Field::new("font", "look", "Font of all the text", "Font di tutti i testi", Choice(&["pixel", "smooth"]))
        .def("pixel")
        .help("pixel = the blocky 5x7 font; smooth = anti-aliased Poppins, in every screen", "pixel = il font a blocchi 5x7; smooth = Poppins morbido, in ogni schermata"));
    add(Field::new("font_weight", "look", "Weight of the smooth font", "Spessore del font morbido", Choice(&["auto", "light", "regular", "medium"]))
        .def("auto")
        .when("font", &["smooth"])
        .help("auto = bolder for small text, lighter for big text", "auto = più marcato per il testo piccolo, più leggero per quello grande"));
    add(Field::new("text_color", "look", "Info text", "Testo informativo", Color));
    add(Field::new("clock_color", "look", "Big clock", "Orologio grande", Color));
    add(Field::new("text_backdrop", "look", "Panel behind the text", "Pannello dietro il testo", Bool).def("false"));
    add(Field::new("clock_backdrop", "look", "Panel behind the clock", "Pannello dietro l'orologio", Bool));
    add(Field::new("clock_time_size", "look", "Clock size", "Dimensione dell'orologio", int(1, 60)).def("20"));
    add(Field::new("clock_date_size", "look", "Date size", "Dimensione della data", int(1, 30)).def("6"));
    add(Field::new("net_unit", "look", "Network unit", "Unità della rete", Choice(&["kb", "mb", "auto"])).def("kb"));
    add(Field::new("mem_unit", "look", "RAM unit", "Unità della RAM", Choice(&["mb", "gb"])).def("mb"));
    add(Field::new("percent_decimals", "look", "Decimals in percentages", "Decimali nelle percentuali", Choice(&["0", "1"])).def("0"));

    // ---- spectrum
    add(Field::new("spectrum_style", "spectrum", "Style", "Stile", Multi(SPECTRUM_STYLES)).def("bars").help("Several = they rotate", "Più di uno = ruotano"));
    add(Field::new("spectrum_style_interval", "spectrum", "Seconds per style", "Secondi per stile", int(2, 3600)).def("30"));
    add(Field::new("spectrum_palette", "spectrum", "Colors", "Colori", Multi(SPECTRUM_PALETTES))
        .help("Unset = the color of \"Spectrum bars\"", "Non impostato = il colore di \"Barre dello spettro\""));
    add(Field::new("spectrum_palette_interval", "spectrum", "Seconds per palette", "Secondi per palette", int(2, 3600)).def("20"));
    add(Field::new("spectrum_gradient", "spectrum", "Own gradient", "Gradiente personalizzato", ColorList)
        .help("2 or more colors, quiet to loud", "2 o più colori, dal piano al forte"));
    add(Field::new("spectrum_rainbow_speed", "spectrum", "Rainbow speed (°/s)", "Velocità arcobaleno (°/s)", int(0, 360)).def("30"));

    // ---- single items
    for (k, en, it) in ITEMS {
        let g: &'static str = en;
        v.push(Field::new(&format!("{k}_size"), "items", &format!("{en}: size"), &format!("{it}: dimensione"), int(1, 30)).def("3").group(g));
        v.push(Field::new(&format!("{k}_position"), "items", &format!("{en}: position"), &format!("{it}: posizione"), Choice(POSITIONS)).group(g));
        v.push(Field::new(&format!("{k}_color"), "items", &format!("{en}: color"), &format!("{it}: colore"), Color).group(g));
        v.push(Field::new(&format!("{k}_backdrop"), "items", &format!("{en}: panel behind"), &format!("{it}: pannello dietro"), Bool).group(g));
    }
    let mut add = |f: Field| v.push(f);
    add(Field::new("nowplaying_label", "items", "Show the \"NOW PLAYING\" label", "Mostra la scritta \"NOW PLAYING\"", Bool).def("true").group("Now playing"));
    add(Field::new("nowplaying_width", "items", "Width of the track (%)", "Larghezza del brano (%)", int(1, 100)).def("100").group("Now playing"));

    // ---- music card
    add(Field::new("music_screen", "music", "Music card", "Scheda musica", Bool)
        .def("false")
        .help("While a track plays: cover, title, progress bar and spectrum", "Mentre suona un brano: copertina, titolo, barra di avanzamento e spettro"));
    add(Field::new("music_bg", "music", "Background", "Sfondo", Choice(MUSIC_BG)).def("gradient").when("music_screen", music_on));
    add(Field::new("music_bg_brightness", "music", "Background brightness", "Luminosità dello sfondo", int(0, 100)).def("40").when("music_screen", music_on));
    add(Field::new("music_bg_blur", "music", "Blur", "Sfocatura", int(0, 100)).def("70").when("music_screen", music_on));
    add(Field::new("music_bg_fit", "music", "Blurred cover fit", "Adattamento copertina sfocata", Choice(&["center", "stretch"])).def("center").when("music_screen", music_on));
    add(Field::new("music_bg_color", "music", "Solid color", "Colore solido", Color).def("#121218").when("music_screen", music_on));
    add(Field::new("music_progress", "music", "Progress bar and times", "Barra di avanzamento e tempi", Bool).def("true").when("music_screen", music_on));
    add(Field::new("music_spectrum_color", "music", "Spectrum color", "Colore dello spettro", Text)
        .def("cover")
        .when("music_screen", music_on)
        .help("cover, main, a color, several colors, or palette names", "cover, main, un colore, più colori o nomi di palette"));
    add(Field::new("music_stats", "music", "Small info at the bottom", "Info in basso", Multi(MUSIC_STATS)).def("cpu, gpu, ram").when("music_screen", music_on));

    // ---- pixel game
    add(Field::new("pixel_game", "game", "Pixel game under the clock", "Mini gioco sotto l'orologio", Bool).def("true"));
    add(Field::new("pixel_game_hero", "game", "Hero", "Eroe", Choice(&["knight", "cat", "both"])).def("both"));
    add(Field::new("pixel_game_height", "game", "Height (%)", "Altezza (%)", int(20, 70)).def("46"));
    add(Field::new("pixel_game_hud", "game", "Coin counter", "Contatore di monete", Bool).def("true"));

    // ---- weather and news
    add(Field::new("weather_city", "online", "City", "Città", Text)
        .restart()
        .group("weather")
        .help("Empty = found from the internet address", "Vuoto = trovata dall'indirizzo internet"));
    add(Field::new("weather_unit", "online", "Unit", "Unità", Choice(&["c", "f"])).def("c").group("weather"));
    add(Field::new("ticker_source", "online", "Sources", "Fonti", Text)
        .restart()
        .group("news")
        .help("RSS/Atom addresses, text addresses or files, comma-separated", "Indirizzi RSS/Atom, indirizzi di testo o file, separati da virgola"));
    add(Field::new("ticker_refresh_min", "online", "Refresh every (minutes)", "Aggiorna ogni (minuti)", int(1, 1440)).def("10").restart().group("news"));
    add(Field::new("ticker_position", "online", "Position", "Posizione", Choice(POSITIONS)).def("bottom").group("news"));
    add(Field::new("ticker_size", "online", "Size", "Dimensione", int(1, 30)).def("3").group("news"));
    add(Field::new("ticker_color", "online", "Color", "Colore", Color).group("news"));
    add(Field::new("ticker_backdrop", "online", "Panel behind", "Pannello dietro", Bool).group("news"));
    add(Field::new("ticker_width", "online", "Width (%)", "Larghezza (%)", int(1, 100)).def("100").group("news"));

    // ---- system
    add(Field::new("silence_threshold", "system", "Silence threshold", "Soglia di silenzio", Float { min: 0.0, max: 1.0 })
        .def("0.005")
        .help("Lower = the spectrum reacts to quieter sound", "Più basso = lo spettro reagisce a suoni più bassi"));
    add(Field::new("silence_timeout_ms", "system", "Quiet time before idle (ms)", "Silenzio prima dello stato di riposo (ms)", int(0, 60000)).def("800"));
    add(Field::new("deepcool", "system", "Drive the DeepCool display", "Pilota il display DeepCool", Bool).def("true").restart());
    add(Field::new("deepcool_source", "system", "DeepCool section", "Sezione del display DeepCool", Choice(&["cpu", "gpu", "alternate"]))
        .def("cpu")
        .when("deepcool", &["true"])
        .help(
            "The display has a CPU and a GPU section: show one of them, or both in turn (CH170 / CH270 / CH690 DIGITAL)",
            "Il display ha una sezione CPU e una GPU: mostra una delle due, oppure entrambe a turno (CH170 / CH270 / CH690 DIGITAL)",
        ));
    add(Field::new("deepcool_mode", "system", "DeepCool case display shows", "Il display DeepCool del case mostra", Choice(&["freq", "fan", "alternate"]))
        .def("freq")
        .when("deepcool", &["true"])
        .help(
            "CH170 / CH270 / CH690 DIGITAL: the CPU frequency, the fan speed in RPM (needs LibreHardwareMonitor), or both in turn",
            "CH170 / CH270 / CH690 DIGITAL: la frequenza della CPU, i giri della ventola in RPM (serve LibreHardwareMonitor), o le due cose a turno",
        ));
    add(Field::new("deepcool_fan", "system", "Fan for the RPM", "Ventola per gli RPM", Choice(&["cpu", "case", "pump", "gpu", "both"]))
        .def("cpu")
        .when("deepcool", &["true"])
        .help("both = the CPU fan and the case fans in turn", "both = ventola CPU e ventole del case a turno"));
    add(Field::new("deepcool_alternate", "system", "Seconds before switching", "Secondi prima di cambiare", int(2, 120))
        .def("6")
        .when("deepcool", &["true"])
        .help("Also used to switch between the CPU and GPU sections", "Usato anche per passare tra sezione CPU e GPU"));
    add(Field::new("fps_monitor", "system", "Measure the FPS of games", "Misura gli FPS dei giochi", Bool)
        .def("true")
        .restart()
        .help("Needs the program started as administrator", "Richiede il programma avviato come amministratore"));
    v
}

/// The sub-heading of a group in both languages.
pub fn group_title(group: &str) -> (&str, &str) {
    match group {
        "margins" => ("Margins (pixels)", "Margini (pixel)"),
        "background" => ("Background", "Sfondo"),
        "bars" => ("Bar colors", "Colori delle barre"),
        "weather" => ("Weather", "Meteo"),
        "news" => ("News ticker", "Notizie scorrevoli"),
        "palette" => ("Colors from the presets", "Colori dai preset"),
        "look" => ("Look", "Aspetto"),
        "sides" => ("Side columns", "Colonne laterali"),
        other => (other, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_sections_exist() {
        let f = fields();
        let mut keys: Vec<&str> = f.iter().map(|x| x.key.as_str()).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "a key is listed twice");
        for x in &f {
            assert!(SECTIONS.iter().any(|(id, _, _)| *id == x.section), "{}: unknown section {}", x.key, x.section);
            assert!(!x.en.is_empty() && !x.it.is_empty(), "{}: a label is missing", x.key);
        }
        for (id, _, _) in SECTIONS {
            assert!(f.iter().any(|x| x.section == *id), "section {id} is empty");
        }
    }

    #[test]
    fn defaults_fit_their_kind() {
        for x in fields() {
            match &x.kind {
                Kind::Choice(o) if !x.default.is_empty() => assert!(o.contains(&x.default.as_str()), "{}: default {}", x.key, x.default),
                Kind::Int { min, max } if !x.default.is_empty() => {
                    let d: i64 = x.default.parse().unwrap_or_else(|_| panic!("{}: default {}", x.key, x.default));
                    assert!((*min..=*max).contains(&d), "{}: default {} out of range", x.key, d);
                }
                Kind::Multi(o) if !x.style_defaults.is_empty() => {
                    for (_, d) in x.style_defaults {
                        assert!(o.contains(d), "{}: style default {d}", x.key);
                    }
                }
                Kind::Bool if !x.default.is_empty() => assert!(x.default == "true" || x.default == "false", "{}", x.key),
                _ => {}
            }
            if let Some((k, vals)) = x.when {
                let dep = fields().into_iter().find(|f| f.key == k).unwrap_or_else(|| panic!("{}: depends on unknown {k}", x.key));
                if let Kind::Choice(o) = dep.kind {
                    assert!(vals.iter().all(|v| o.contains(v)), "{}: condition values", x.key);
                } else {
                    assert!(matches!(dep.kind, Kind::Bool), "{}: condition on {k}", x.key);
                }
            }
        }
    }

    #[test]
    fn colors_round_trip() {
        assert_eq!(parse_color("pink"), Some((0xFF, 0xC0, 0xCB)));
        assert_eq!(parse_color("#1b2A4a"), Some((0x1B, 0x2A, 0x4A)));
        assert_eq!(parse_color("10, 20,30"), Some((10, 20, 30)));
        assert_eq!(parse_color("nonsense"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(to_hex((0, 255, 16)), "#00FF10");
        assert_eq!(split_list("a, b ,,c"), ["a", "b", "c"]);
        for (n, c) in COLOR_NAMES {
            assert_eq!(parse_color(n), Some(*c));
        }
    }
}
