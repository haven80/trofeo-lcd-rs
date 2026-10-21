//! `trofeo_config`: a small window to create and edit `trofeo.conf`.
//!
//! It is a separate program on purpose: it only reads and writes the configuration file
//! (the running `trofeo_lcd` picks the changes up by itself, it re-reads the file every
//! second), so it never needs to stay open or to load anything into the memory of the main
//! program. The preview is drawn by `trofeo_lcd.exe` itself (`--preview-png`), so it always
//! matches what the display shows.
//!
//! Layout of this file: the editing model (`Ed`: the text of the file, edited line by line
//! through `trofeo_lcd::conf_edit`), the widgets of every kind of setting, the preview runner,
//! and the window (`App`).
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use trofeo_lcd::conf_edit::ConfDoc;
use trofeo_lcd::schema::{self, Field, Kind};

/// The commented example shipped with the program: the starting point of a new file.
const EXAMPLE: &str = include_str!("../../trofeo.conf.example");

// ------------------------------------------------------------------------ language

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    It,
    En,
}

fn tr<'a>(l: Lang, en: &'a str, it: &'a str) -> &'a str {
    match l {
        Lang::It => it,
        Lang::En => en,
    }
}

fn label(f: &Field, l: Lang) -> &str {
    tr(l, &f.en, &f.it)
}

fn help(f: &Field, l: Lang) -> &str {
    tr(l, &f.help_en, &f.help_it)
}

// ------------------------------------------------------------------------ editing model

/// The file being edited and the text boxes that are being typed in.
struct Ed {
    doc: ConfDoc,
    /// Increases at every change (so the preview and the "unsaved" mark know when to look).
    rev: u64,
    /// What the user is typing in a text box whose content is not (yet) a valid value.
    bufs: HashMap<String, String>,
}

impl Ed {
    fn new(doc: ConfDoc) -> Ed {
        Ed { doc, rev: 0, bufs: HashMap::new() }
    }
    fn get(&self, key: &str) -> Option<String> {
        self.doc.get(key)
    }
    fn set(&mut self, key: &str, v: &str) {
        if self.doc.get(key).as_deref() != Some(v.trim()) {
            self.doc.set(key, v);
            self.rev += 1;
        }
    }
    fn unset(&mut self, key: &str) {
        if self.doc.is_set(key) {
            self.doc.unset(key);
            self.rev += 1;
        }
    }
    /// The value in force: the file's, or the field's default.
    fn effective(&self, f: &Field) -> String {
        self.get(&f.key).unwrap_or_else(|| self.default_of(f))
    }
    /// The default of a field, which for some effect settings depends on the animated background.
    fn default_of(&self, f: &Field) -> String {
        f.default_for(&self.get("background_effect").unwrap_or_default())
    }
    /// The value of any key, by the field table's default.
    fn value_of(&self, key: &str, fields: &[Field]) -> String {
        self.get(key).or_else(|| fields.iter().find(|f| f.key == key).map(|f| self.default_of(f))).unwrap_or_default()
    }
}

// ------------------------------------------------------------------------ the layout list

#[derive(Clone, Debug, PartialEq)]
struct LayoutEntry {
    name: String,
    secs: Option<u32>,
}

fn parse_layouts(s: &str) -> Vec<LayoutEntry> {
    schema::split_list(s)
        .into_iter()
        .map(|item| match item.split_once(':') {
            Some((n, secs)) => LayoutEntry { name: n.trim().to_string(), secs: secs.trim().parse().ok() },
            None => LayoutEntry { name: item, secs: None },
        })
        .collect()
}

fn layouts_to_string(v: &[LayoutEntry]) -> String {
    v.iter()
        .map(|e| match e.secs {
            Some(s) => format!("{}:{}", e.name, s),
            None => e.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ------------------------------------------------------------------------ the tiles

#[derive(Clone, Debug, PartialEq)]
struct Tile {
    kind: String,
    ring: bool,
    tall: bool,
}

const DEFAULT_TILES: &str = "clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, gpu:ring";

/// `None` when the text uses something the widget cannot show (then it is edited as text).
fn parse_tiles(s: &str) -> Option<Vec<Vec<Tile>>> {
    let mut rows = Vec::new();
    for row in s.split('|') {
        let mut tiles = Vec::new();
        for item in row.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let mut parts = item.split(':');
            let kind = parts.next()?.trim().to_ascii_lowercase();
            if !schema::TILE_KINDS.iter().any(|(k, _, _)| *k == kind) {
                return None;
            }
            let (mut ring, mut tall) = (false, false);
            for m in parts {
                match m.trim().to_ascii_lowercase().as_str() {
                    "ring" => ring = true,
                    "bar" => ring = false,
                    "tall" => tall = true,
                    _ => return None,
                }
            }
            tiles.push(Tile { kind, ring, tall });
        }
        if !tiles.is_empty() {
            rows.push(tiles);
        }
    }
    (!rows.is_empty()).then_some(rows)
}

fn tiles_to_string(rows: &[Vec<Tile>]) -> String {
    rows.iter()
        .filter(|r| !r.is_empty())
        .map(|r| {
            r.iter()
                .map(|t| {
                    let mut s = t.kind.clone();
                    if t.ring {
                        s.push_str(":ring");
                    }
                    if t.tall {
                        s.push_str(":tall");
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

// ------------------------------------------------------------------------ multi-choice lists

/// The new text of a multi-choice list after `word` was switched on or off. `all` / `none`
/// stand alone: choosing one clears the others, choosing another clears them.
fn toggle_multi(current: &str, word: &str, options: &[&str]) -> String {
    let stand_alone = |w: &str| w == "all" || w == "none";
    let mut on: Vec<String> = schema::split_list(current);
    if let Some(i) = on.iter().position(|w| w == word) {
        on.remove(i);
    } else if stand_alone(word) {
        on = vec![word.to_string()];
    } else {
        on.retain(|w| !stand_alone(w));
        on.push(word.to_string());
    }
    // keep the order of the options, so the file stays tidy
    on.sort_by_key(|w| options.iter().position(|o| o == w).unwrap_or(usize::MAX));
    on.join(", ")
}

// ------------------------------------------------------------------------ the preview

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviewState {
    Idle,
    Playing,
    Music,
}

impl PreviewState {
    fn arg(self) -> &'static str {
        match self {
            PreviewState::Idle => "idle",
            PreviewState::Playing => "playing",
            PreviewState::Music => "music",
        }
    }
}

struct Rendered {
    image: egui::ColorImage,
}

/// Looks for `trofeo_lcd(.exe)` next to this program (or on the PATH).
fn find_main_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "trofeo_lcd.exe" } else { "trofeo_lcd" };
    if let Ok(p) = std::env::var("TROFEO_LCD_EXE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file()))
}

/// Runs the main program once to draw a frame of `text` (the configuration being edited).
fn render_preview(exe: &Path, text: &str, workdir: &Path, state: PreviewState, layout: usize, secs: f64) -> Result<Rendered, String> {
    let tmp = std::env::temp_dir();
    let id = std::process::id();
    let conf = tmp.join(format!("trofeo_config_{id}.conf"));
    let png = tmp.join(format!("trofeo_config_{id}.png"));
    std::fs::write(&conf, text).map_err(|e| format!("cannot write the temporary file: {e}"))?;
    let _ = std::fs::remove_file(&png);
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--config")
        .arg(&conf)
        .arg("--preview-png")
        .arg(&png)
        .arg("--preview-state")
        .arg(state.arg())
        .arg("--preview-layout")
        .arg(layout.to_string())
        .arg("--preview-time")
        .arg(format!("{secs:.2}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if workdir.is_dir() {
        cmd.current_dir(workdir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    let _ = std::fs::remove_file(&conf);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().find(|l| !l.trim().is_empty() && !l.starts_with("Stack backtrace")).unwrap_or("the program failed");
        return Err(line.trim_start_matches("Error: ").to_string());
    }
    let bytes = std::fs::read(&png).map_err(|e| format!("no picture was produced: {e}"))?;
    let _ = std::fs::remove_file(&png);
    let img = image::load_from_memory(&bytes).map_err(|e| format!("cannot read the picture: {e}"))?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Ok(Rendered { image: egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()) })
}

struct Preview {
    exe: Option<PathBuf>,
    tex: Option<egui::TextureHandle>,
    error: Option<String>,
    rx: Option<mpsc::Receiver<Result<Rendered, String>>>,
    /// What was asked for last: (revision, state, layout, time step).
    asked: Option<(u64, PreviewState, usize, i64)>,
    /// When the next render may start (waits a moment after the last change).
    due: Option<Instant>,
}

impl Preview {
    fn new() -> Preview {
        Preview { exe: find_main_exe(), tex: None, error: None, rx: None, asked: None, due: None }
    }
}

// ------------------------------------------------------------------------ widgets

/// A text box that keeps what is being typed, and tells whether it is a valid value.
/// Returns `Some(Some(text))` when a valid new value was typed, `Some(None)` when it was
/// emptied, `None` when nothing changed.
fn text_box(ui: &mut egui::Ui, ed: &mut Ed, key: &str, width: f32, valid: impl Fn(&str) -> bool, hint: &str) -> Option<Option<String>> {
    let stored = ed.get(key).unwrap_or_default();
    let mut text = ed.bufs.get(key).cloned().unwrap_or_else(|| stored.clone());
    let bad = !text.trim().is_empty() && !valid(&text);
    let mut edit = egui::TextEdit::singleline(&mut text).desired_width(width).hint_text(hint);
    if bad {
        edit = edit.text_color(Color32::from_rgb(255, 120, 120));
    }
    let resp = ui.add(edit);
    let mut out = None;
    if resp.changed() {
        ed.bufs.insert(key.to_string(), text.clone());
        if text.trim().is_empty() {
            out = Some(None);
        } else if valid(&text) {
            out = Some(Some(text.trim().to_string()));
        }
    }
    if !resp.has_focus() && !resp.changed() {
        ed.bufs.remove(key);
    }
    out
}

fn apply(ed: &mut Ed, key: &str, v: Option<Option<String>>) {
    match v {
        Some(Some(t)) => ed.set(key, &t),
        Some(None) => ed.unset(key),
        None => {}
    }
}

fn c32(c: (u8, u8, u8)) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// One color: a picker button, a box for a name or a hex code, and the names as a menu.
fn color_widget(ui: &mut egui::Ui, ed: &mut Ed, key: &str, l: Lang, default_word: Option<&str>) {
    let cur = ed.get(key);
    let shown = cur.clone().or_else(|| default_word.map(str::to_string)).unwrap_or_default();
    let mut rgb = schema::parse_color(&shown).map(c32).unwrap_or([0x80, 0x80, 0x80]);
    let known = schema::parse_color(&shown).is_some();
    let before = rgb;
    ui.add_enabled_ui(true, |ui| {
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() && rgb != before {
            ed.bufs.remove(key);
            ed.set(key, &schema::to_hex((rgb[0], rgb[1], rgb[2])));
        }
    });
    let dw = default_word.map(str::to_string);
    let out = text_box(ui, ed, key, 110.0, |t| schema::parse_color(t).is_some() || dw.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(t.trim())), if known || cur.is_some() { "" } else { tr(l, "none", "nessuno") });
    apply(ed, key, out);
    ui.menu_button(tr(l, "Names", "Nomi"), |ui| {
        ui.set_max_width(210.0);
        ui.horizontal_wrapped(|ui| {
            for (name, c) in schema::COLOR_NAMES {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(c.0, c.1, c.2));
                ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, Color32::from_gray(90)));
                if resp.on_hover_text(*name).clicked() {
                    ed.bufs.remove(key);
                    ed.set(key, name);
                    ui.close_menu();
                }
            }
        });
    });
}

/// A list of colors: a picker per color, and add / remove.
fn color_list_widget(ui: &mut egui::Ui, ed: &mut Ed, key: &str, l: Lang) {
    let mut items = schema::split_list(&ed.get(key).unwrap_or_default());
    let mut changed = false;
    let mut remove = None;
    for (i, it) in items.iter_mut().enumerate() {
        let c = schema::parse_color(it).map(c32).unwrap_or([0x80, 0x80, 0x80]);
        let mut rgb = c;
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() && rgb != c {
            *it = schema::to_hex((rgb[0], rgb[1], rgb[2]));
            changed = true;
        }
        if ui.small_button("x").on_hover_text(tr(l, "Remove this color", "Togli questo colore")).clicked() {
            remove = Some(i);
        }
        ui.add_space(6.0);
    }
    if let Some(i) = remove {
        items.remove(i);
        changed = true;
    }
    if ui.button(tr(l, "+ color", "+ colore")).clicked() {
        items.push(items.last().cloned().unwrap_or_else(|| "#00C8FF".to_string()));
        changed = true;
    }
    if changed {
        if items.is_empty() {
            ed.unset(key);
        } else {
            ed.set(key, &items.join(", "));
        }
    }
}

fn multi_widget(ui: &mut egui::Ui, ed: &mut Ed, f: &Field, options: &[&str], l: Lang) {
    let cur = ed.effective(f);
    let on = schema::split_list(&cur);
    let none_set = ed.get(&f.key).is_none();
    ui.horizontal_wrapped(|ui| {
        for o in options {
            let selected = on.iter().any(|w| w == o) && !(none_set && f.default.is_empty());
            if ui.selectable_label(selected, *o).clicked() {
                let base = if none_set { ed.default_of(f) } else { cur.clone() };
                let new = toggle_multi(&base, o, options);
                if new.is_empty() {
                    ed.unset(&f.key);
                } else {
                    ed.set(&f.key, &new);
                }
            }
        }
    });
    let _ = l;
}

fn layout_widget(ui: &mut egui::Ui, ed: &mut Ed, key: &str, l: Lang) {
    let mut entries = parse_layouts(&ed.get(key).unwrap_or_default());
    let mut changed = false;
    let (mut remove, mut up) = (None, None);
    let n = entries.len();
    for (i, e) in entries.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{}.", i + 1));
            let shown = schema::LAYOUT_NAMES.iter().find(|(n, _, _)| *n == e.name).map(|(n, _, _)| *n).unwrap_or(e.name.as_str()).to_string();
            egui::ComboBox::from_id_salt(format!("layout_{i}")).selected_text(shown).width(150.0).show_ui(ui, |ui| {
                for (name, en, it) in schema::LAYOUT_NAMES {
                    if ui.selectable_label(e.name == *name, format!("{name}  -  {}", tr(l, en, it))).clicked() {
                        e.name = name.to_string();
                        changed = true;
                    }
                }
            });
            let mut own = e.secs.is_some();
            if ui.checkbox(&mut own, tr(l, "own duration", "durata propria")).changed() {
                e.secs = own.then_some(15);
                changed = true;
            }
            if let Some(s) = e.secs.as_mut() {
                let mut v = *s;
                if ui.add(egui::DragValue::new(&mut v).range(2..=3600).suffix(" s")).changed() {
                    *s = v;
                    changed = true;
                }
            }
            if i > 0 && ui.small_button("^").on_hover_text(tr(l, "Move up", "Sposta su")).clicked() {
                up = Some(i);
            }
            if ui.small_button("x").on_hover_text(tr(l, "Remove", "Togli")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = up {
        entries.swap(i, i - 1);
        changed = true;
    }
    if let Some(i) = remove {
        entries.remove(i);
        changed = true;
    }
    ui.menu_button(tr(l, "+ add a screen", "+ aggiungi una schermata"), |ui| {
        for (name, en, it) in schema::LAYOUT_NAMES {
            if ui.button(format!("{name}  -  {}", tr(l, en, it))).clicked() {
                entries.push(LayoutEntry { name: name.to_string(), secs: None });
                changed = true;
                ui.close_menu();
            }
        }
    });
    let _ = n;
    if changed {
        if entries.is_empty() {
            ed.unset(key);
        } else {
            ed.set(key, &layouts_to_string(&entries));
        }
    }
}

fn tiles_widget(ui: &mut egui::Ui, ed: &mut Ed, key: &str, l: Lang) {
    ui.vertical(|ui| tiles_inner(ui, ed, key, l));
}

fn tiles_inner(ui: &mut egui::Ui, ed: &mut Ed, key: &str, l: Lang) {
    let raw = ed.get(key);
    let text = raw.clone().unwrap_or_else(|| DEFAULT_TILES.to_string());
    let Some(mut rows) = parse_tiles(&text) else {
        ui.colored_label(Color32::from_rgb(255, 170, 90), tr(l, "This arrangement uses names the editor does not know: edit it as text.", "Questa disposizione usa nomi che l'editor non conosce: modificala come testo."));
        let out = text_box(ui, ed, key, 520.0, |_| true, "");
        apply(ed, key, out);
        return;
    };
    let mut changed = false;
    let total: usize = rows.iter().map(Vec::len).sum();
    let mut remove_row = None;
    let n_rows = rows.len();
    for (ri, row) in rows.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} {}", tr(l, "Row", "Riga"), ri + 1)).weak());
            let mut remove = None;
            for (ti, t) in row.iter_mut().enumerate() {
                if ti > 0 {
                    ui.label(RichText::new("|").weak());
                }
                let name = schema::TILE_KINDS.iter().find(|(k, _, _)| *k == t.kind).map(|(_, en, it)| tr(l, en, it)).unwrap_or(&t.kind).to_string();
                egui::ComboBox::from_id_salt(format!("tile_{ri}_{ti}")).selected_text(name).width(120.0).show_ui(ui, |ui| {
                    for (k, en, it) in schema::TILE_KINDS {
                        if ui.selectable_label(t.kind == *k, tr(l, en, it)).clicked() {
                            t.kind = k.to_string();
                            changed = true;
                        }
                    }
                });
                if ui.selectable_label(t.ring, tr(l, "ring", "anello")).on_hover_text(tr(l, "A ring instead of a bar", "Un anello invece di una barra")).clicked() {
                    t.ring = !t.ring;
                    changed = true;
                }
                if ri == 0 && ti == 0 && ui.selectable_label(t.tall, tr(l, "tall", "alto")).on_hover_text(tr(l, "Spans two rows (first tile only)", "Occupa due righe (solo il primo tile)")).clicked() {
                    t.tall = !t.tall;
                    changed = true;
                }
                if ui.small_button("x").on_hover_text(tr(l, "Remove this tile", "Togli questo tile")).clicked() {
                    remove = Some(ti);
                }
            }
            if let Some(ti) = remove {
                row.remove(ti);
                changed = true;
            }
            if total < 16 && ui.button(tr(l, "+ tile", "+ tile")).clicked() {
                row.push(Tile { kind: "cpu".into(), ring: false, tall: false });
                changed = true;
            }
            if n_rows > 1 && ui.small_button(tr(l, "remove row", "togli riga")).clicked() {
                remove_row = Some(ri);
            }
        });
    }
    if let Some(ri) = remove_row {
        rows.remove(ri);
        changed = true;
    }
    if ui.button(tr(l, "+ row", "+ riga")).clicked() && total < 16 {
        rows.push(vec![Tile { kind: "cpu".into(), ring: true, tall: false }]);
        changed = true;
    }
    if changed {
        // a tall tile only works as the first tile of the first row
        for (ri, row) in rows.iter_mut().enumerate() {
            for (ti, t) in row.iter_mut().enumerate() {
                if ri != 0 || ti != 0 {
                    t.tall = false;
                }
            }
        }
        rows.retain(|r| !r.is_empty());
        if rows.is_empty() {
            ed.unset(key);
        } else {
            ed.set(key, &tiles_to_string(&rows));
        }
    }
    ui.label(RichText::new(format!("{total} / 16 tiles")).weak().small());
}

/// One column of tiles (`frame_left` / `frame_right`): a row of choices, possibly none.
fn tile_column_widget(ui: &mut egui::Ui, ed: &mut Ed, f: &Field, l: Lang) {
    let text = ed.effective(f);
    let tiles: Vec<Tile> = if text.trim().eq_ignore_ascii_case("none") { Vec::new() } else { parse_tiles(&text).map(|r| r.into_iter().flatten().collect()).unwrap_or_default() };
    let mut tiles: Vec<Tile> = tiles.into_iter().map(|t| Tile { tall: false, ..t }).collect();
    let mut changed = false;
    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            let mut remove = None;
            for (ti, t) in tiles.iter_mut().enumerate() {
                let name = schema::TILE_KINDS.iter().find(|(k, _, _)| *k == t.kind).map(|(_, en, it)| tr(l, en, it)).unwrap_or(&t.kind).to_string();
                egui::ComboBox::from_id_salt(format!("{}_{ti}", f.key)).selected_text(name).width(120.0).show_ui(ui, |ui| {
                    for (k, en, it) in schema::TILE_KINDS {
                        if ui.selectable_label(t.kind == *k, tr(l, en, it)).clicked() {
                            t.kind = k.to_string();
                            changed = true;
                        }
                    }
                });
                if ui.selectable_label(t.ring, tr(l, "ring", "anello")).on_hover_text(tr(l, "A ring instead of a bar", "Un anello invece di una barra")).clicked() {
                    t.ring = !t.ring;
                    changed = true;
                }
                if ui.small_button("x").on_hover_text(tr(l, "Remove this tile", "Togli questo tile")).clicked() {
                    remove = Some(ti);
                }
                ui.label(RichText::new("|").weak());
            }
            if let Some(ti) = remove {
                tiles.remove(ti);
                changed = true;
            }
            if tiles.len() < 4 && ui.button("+ tile").clicked() {
                tiles.push(Tile { kind: "cpu".into(), ring: true, tall: false });
                changed = true;
            }
            if tiles.is_empty() {
                ui.label(RichText::new(tr(l, "no tiles: this side is left to the pictures", "nessun tile: questo lato è lasciato alle immagini")).weak());
            }
        });
    });
    if changed {
        if tiles.is_empty() {
            ed.set(&f.key, "none");
        } else {
            ed.set(&f.key, &tiles_to_string(&[tiles]));
        }
    }
}

/// `deck`, `orbit` (one list) and `cards` (columns separated by `|`, cards of a column stacked).
fn panels_widget(ui: &mut egui::Ui, ed: &mut Ed, f: &Field, columns: bool, l: Lang) {
    let text = ed.effective(f);
    let mut cols: Vec<Vec<String>> = text
        .split('|')
        .map(|c| c.split(',').map(|k| k.trim().to_ascii_lowercase()).filter(|k| !k.is_empty()).collect::<Vec<_>>())
        .filter(|c| !c.is_empty())
        .collect();
    if !columns && cols.len() > 1 {
        cols = vec![cols.into_iter().flatten().collect()];
    }
    let max_items = if columns { 3 } else if f.key == "orbit" { 4 } else { 5 };
    let mut changed = false;
    let mut remove_col = None;
    ui.vertical(|ui| {
        for (ci, col) in cols.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                if columns {
                    ui.label(RichText::new(format!("{}.", ci + 1)).weak());
                }
                let mut remove = None;
                for (ki, kind) in col.iter_mut().enumerate() {
                    let name = schema::PANEL_KINDS.iter().find(|(k, _, _)| *k == kind.as_str()).map(|(_, en, it)| tr(l, en, it)).unwrap_or(kind.as_str()).to_string();
                    egui::ComboBox::from_id_salt(format!("{}_{ci}_{ki}", f.key)).selected_text(name).width(130.0).show_ui(ui, |ui| {
                        for (k, en, it) in schema::PANEL_KINDS {
                            if ui.selectable_label(kind == k, tr(l, en, it)).clicked() {
                                *kind = k.to_string();
                                changed = true;
                            }
                        }
                    });
                    if ui.small_button("x").on_hover_text(tr(l, "Remove", "Togli")).clicked() {
                        remove = Some(ki);
                    }
                }
                if let Some(ki) = remove {
                    col.remove(ki);
                    changed = true;
                }
                if col.len() < max_items && ui.button(if columns { tr(l, "+ stacked", "+ sotto") } else { "+" }).clicked() {
                    col.push("cpu".into());
                    changed = true;
                }
                if columns && ui.small_button(tr(l, "remove column", "togli colonna")).clicked() {
                    remove_col = Some(ci);
                }
            });
        }
        if columns && cols.len() < 6 && ui.button(tr(l, "+ column", "+ colonna")).clicked() {
            cols.push(vec!["cpu".into()]);
            changed = true;
        }
    });
    if let Some(ci) = remove_col {
        cols.remove(ci);
        changed = true;
    }
    cols.retain(|c| !c.is_empty());
    if changed && !cols.is_empty() {
        let joined = cols.iter().map(|c| c.join(", ")).collect::<Vec<_>>().join(" | ");
        ed.set(&f.key, &joined);
    }
}

fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

fn pick_file(title: &str, start: Option<&Path>) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let mut d = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = start.and_then(Path::parent) {
            d = d.set_directory(dir);
        }
        d.pick_file()
    }
    #[cfg(not(windows))]
    {
        let _ = (title, start);
        None
    }
}

fn save_file_dialog(start: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let mut d = rfd::FileDialog::new().set_title("Save").add_filter("trofeo.conf", &["conf", "config"]).set_file_name("trofeo.conf");
        if let Some(dir) = start.parent() {
            d = d.set_directory(dir);
        }
        d.save_file()
    }
    #[cfg(not(windows))]
    {
        let _ = start;
        None
    }
}

/// Draws one setting. Returns nothing: it changes `ed`.
fn field_row(ui: &mut egui::Ui, ed: &mut Ed, f: &Field, l: Lang) {
    let is_set = ed.get(&f.key).is_some();
    let wide = matches!(f.kind, Kind::Multi(_) | Kind::Layout | Kind::Tiles | Kind::TileColumn | Kind::Panels { .. } | Kind::ColorList | Kind::BarColor);
    let mut text = RichText::new(label(f, l));
    if is_set {
        text = text.strong();
    }
    ui.horizontal(|ui| {
        let width = if wide { 230.0 } else { 250.0 };
        ui.allocate_ui_with_layout(egui::vec2(width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let r = ui.label(text.clone());
            let h = help(f, l);
            let tip = if f.restart { format!("{h}\n({})", tr(l, "needs a restart of the program", "serve riavviare il programma")) } else { h.to_string() };
            if !tip.trim().is_empty() {
                r.on_hover_text(tip);
            }
            if f.restart {
                ui.label(RichText::new("*").color(Color32::from_rgb(255, 190, 90)));
            }
        });
        if !wide {
            field_widget(ui, ed, f, l);
        }
        if is_set && ui.small_button("x").on_hover_text(tr(l, "Back to the default (the line is commented out)", "Torna al valore predefinito (la riga viene commentata)")).clicked() {
            ed.bufs.remove(&f.key);
            ed.unset(&f.key);
        }
    });
    if wide {
        ui.indent(format!("w_{}", f.key), |ui| {
            ui.horizontal_wrapped(|ui| field_widget(ui, ed, f, l));
        });
    }
    let h = help(f, l);
    if !h.is_empty() {
        ui.indent(format!("h_{}", f.key), |ui| {
            ui.label(RichText::new(h).weak().small());
        });
    }
    ui.add_space(3.0);
}

fn field_widget(ui: &mut egui::Ui, ed: &mut Ed, f: &Field, l: Lang) {
    let eff = ed.effective(f);
    match &f.kind {
        Kind::Bool => {
            let mut b = eff.eq_ignore_ascii_case("true");
            if ui.checkbox(&mut b, "").changed() {
                ed.set(&f.key, if b { "true" } else { "false" });
            }
        }
        Kind::Choice(opts) => {
            let shown = if eff.is_empty() { tr(l, "(not set)", "(non impostato)").to_string() } else { eff.clone() };
            egui::ComboBox::from_id_salt(&f.key).selected_text(shown).width(170.0).show_ui(ui, |ui| {
                for o in opts.iter() {
                    if ui.selectable_label(eff == *o, *o).clicked() {
                        ed.set(&f.key, o);
                    }
                }
            });
        }
        Kind::Int { min, max } => {
            let mut v: i64 = eff.trim().parse().unwrap_or(*min);
            if ui.add(egui::Slider::new(&mut v, *min..=*max).clamping(egui::SliderClamping::Always)).changed() {
                ed.set(&f.key, &v.to_string());
            }
            if f.default.is_empty() && ed.get(&f.key).is_none() {
                ui.label(RichText::new(tr(l, "automatic (the program decides)", "automatico (decide il programma)")).weak());
            }
        }
        Kind::Float { min, max } => {
            let mut v: f32 = eff.trim().parse().unwrap_or(*min);
            if ui.add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.001).max_decimals(4)).changed() {
                ed.set(&f.key, &format!("{v}"));
            }
        }
        Kind::Color => color_widget(ui, ed, &f.key, l, (f.default == "default").then_some("default")),
        Kind::ColorList => color_list_widget(ui, ed, &f.key, l),
        Kind::BarColor => {
            let auto = {
                let v = ed.get(&f.key).unwrap_or_default();
                v.trim().is_empty() || v.trim().eq_ignore_ascii_case("auto") || v.trim().eq_ignore_ascii_case("default")
            };
            let mut a = auto;
            if ui.checkbox(&mut a, tr(l, "automatic (by level)", "automatico (in base al livello)")).changed() {
                if a {
                    ed.unset(&f.key);
                } else {
                    ed.set(&f.key, "#00C8FF");
                }
            }
            if !auto {
                color_list_widget(ui, ed, &f.key, l);
            }
        }
        Kind::Multi(opts) => multi_widget(ui, ed, f, opts, l),
        Kind::Text => {
            let out = text_box(ui, ed, &f.key, 320.0, |_| true, &f.default);
            apply(ed, &f.key, out);
        }
        Kind::Path => {
            let out = text_box(ui, ed, &f.key, 320.0, |t| !t.contains('#'), "");
            apply(ed, &f.key, out);
            if ui.button(tr(l, "Browse...", "Sfoglia...")).clicked() {
                if let Some(p) = pick_file(label(f, l), None) {
                    ed.bufs.remove(&f.key);
                    ed.set(&f.key, &p.display().to_string());
                }
            }
            if ed.bufs.get(&f.key).is_some_and(|t| t.contains('#')) {
                ui.colored_label(Color32::from_rgb(255, 120, 120), tr(l, "the character # is not allowed in a path", "il carattere # non è ammesso in un percorso"));
            }
        }
        Kind::Folder => {
            let out = text_box(ui, ed, &f.key, 320.0, |t| !t.contains('#'), "");
            apply(ed, &f.key, out);
            if ui.button(tr(l, "Browse...", "Sfoglia...")).clicked() {
                if let Some(p) = pick_folder(label(f, l)) {
                    ed.bufs.remove(&f.key);
                    ed.set(&f.key, &p.display().to_string());
                }
            }
            #[cfg(not(windows))]
            ui.label(RichText::new(tr(l, "type the path of the folder", "scrivi il percorso della cartella")).weak());
            if ed.bufs.get(&f.key).is_some_and(|t| t.contains('#')) {
                ui.colored_label(Color32::from_rgb(255, 120, 120), tr(l, "the character # is not allowed in a path", "il carattere # non è ammesso in un percorso"));
            }
        }
        Kind::Layout => layout_widget(ui, ed, &f.key, l),
        Kind::Tiles => tiles_widget(ui, ed, &f.key, l),
        Kind::TileColumn => tile_column_widget(ui, ed, f, l),
        Kind::Panels { columns } => panels_widget(ui, ed, f, *columns, l),
    }
}

// ------------------------------------------------------------------------ the window

enum Confirm {
    Close,
    Open(PathBuf),
    New,
}

struct App {
    lang: Lang,
    path: PathBuf,
    ed: Ed,
    saved_text: String,
    dirty: bool,
    dirty_rev: u64,
    fields: Vec<Field>,
    section: String,
    status: Option<(String, Instant, bool)>,
    preview: Preview,
    state: PreviewState,
    layout_idx: usize,
    /// The section the preview's screen was last matched to.
    pv_section: String,
    animate: bool,
    anim_start: Instant,
    raw_text: Option<String>,
    confirm: Option<Confirm>,
    allow_close: bool,
    backed_up: bool,
    path_text: String,
    /// Developer option: save a screenshot of the window and quit.
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_requested: bool,
}

fn default_path() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("trofeo.conf"))).unwrap_or_else(|| PathBuf::from("trofeo.conf"))
}

impl App {
    fn new(path: PathBuf, lang: Lang, section: Option<String>, screenshot: Option<PathBuf>) -> App {
        let mut app = App {
            lang,
            path: path.clone(),
            ed: Ed::new(ConfDoc::default()),
            saved_text: String::new(),
            dirty: false,
            dirty_rev: 0,
            fields: schema::fields(),
            section: section.unwrap_or_else(|| "background".to_string()),
            status: None,
            preview: Preview::new(),
            state: PreviewState::Idle,
            layout_idx: 0,
            pv_section: String::new(),
            animate: false,
            anim_start: Instant::now(),
            raw_text: None,
            confirm: None,
            allow_close: false,
            backed_up: false,
            path_text: String::new(),
            screenshot,
            frames: 0,
            shot_requested: false,
        };
        app.load(&path);
        app
    }

    fn say(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), Instant::now(), error));
    }

    /// Reads `path` (or starts from the commented example when the file does not exist yet).
    fn load(&mut self, path: &Path) {
        let l = self.lang;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.saved_text = text.clone();
                self.ed = Ed::new(ConfDoc::parse(&text));
                self.say(format!("{} {}", tr(l, "Opened", "Aperto"), path.display()), false);
            }
            Err(_) if !path.exists() => {
                self.saved_text = String::new();
                self.ed = Ed::new(ConfDoc::parse(EXAMPLE));
                self.say(tr(l, "New file: it starts from the example, nothing is saved until you press Save.", "Nuovo file: parte dall'esempio, non viene salvato nulla finché non premi Salva."), false);
            }
            Err(e) => {
                self.saved_text = String::new();
                self.ed = Ed::new(ConfDoc::default());
                self.say(format!("{}: {e}", tr(l, "Cannot read the file", "Impossibile leggere il file")), true);
            }
        }
        self.path = path.to_path_buf();
        self.path_text = path.display().to_string();
        self.raw_text = None;
        self.backed_up = false;
        self.dirty_rev = u64::MAX;
        self.preview.asked = None;
        self.layout_idx = 0;
    }

    fn refresh_dirty(&mut self) {
        if self.dirty_rev != self.ed.rev {
            self.dirty = self.ed.doc.to_text() != self.saved_text;
            self.dirty_rev = self.ed.rev;
        }
    }

    fn save_to(&mut self, path: &Path) -> bool {
        let l = self.lang;
        let text = self.ed.doc.to_text();
        // a copy of the previous file, once per session
        if !self.backed_up && path.is_file() {
            let bak = path.with_extension("conf.bak");
            if std::fs::copy(path, &bak).is_ok() {
                self.backed_up = true;
            }
        }
        let tmp = path.with_extension("conf.tmp");
        let res = std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, path));
        match res {
            Ok(()) => {
                self.saved_text = text;
                self.path = path.to_path_buf();
                self.path_text = path.display().to_string();
                self.dirty_rev = u64::MAX;
                self.say(format!("{} {}", tr(l, "Saved", "Salvato"), path.display()), false);
                true
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                self.say(format!("{}: {e}", tr(l, "Cannot save", "Impossibile salvare")), true);
                false
            }
        }
    }

    fn save(&mut self) -> bool {
        let p = self.path.clone();
        self.save_to(&p)
    }

    /// The names of the screens in the `layout` list (what the preview can show).
    fn layout_names(&self) -> Vec<String> {
        let list = parse_layouts(&self.ed.get("layout").unwrap_or_default());
        list.into_iter().map(|e| e.name).collect()
    }

    // ---- preview plumbing

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.preview.rx else { return };
        match rx.try_recv() {
            Ok(Ok(r)) => {
                self.preview.error = None;
                match &mut self.preview.tex {
                    Some(t) => t.set(r.image, egui::TextureOptions::LINEAR),
                    None => self.preview.tex = Some(ctx.load_texture("preview", r.image, egui::TextureOptions::LINEAR)),
                }
                self.preview.rx = None;
            }
            Ok(Err(e)) => {
                self.preview.error = Some(e);
                self.preview.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.preview.rx = None,
        }
    }

    fn drive_preview(&mut self, ctx: &egui::Context) {
        self.poll_preview(ctx);
        let Some(exe) = self.preview.exe.clone() else { return };
        let t_step = if self.animate { (self.anim_start.elapsed().as_secs_f64() * 2.0) as i64 } else { 0 };
        let want = (self.ed.rev, self.state, self.layout_idx, t_step);
        if self.preview.asked != Some(want) && self.preview.due.is_none() {
            // wait a moment: the user may still be dragging a slider
            self.preview.due = Some(Instant::now() + Duration::from_millis(if self.preview.asked.is_none() { 0 } else { 300 }));
        }
        if let Some(due) = self.preview.due {
            if Instant::now() >= due && self.preview.rx.is_none() {
                self.preview.due = None;
                self.preview.asked = Some(want);
                let (tx, rx) = mpsc::channel();
                self.preview.rx = Some(rx);
                let text = self.ed.doc.to_text();
                let workdir = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
                let (state, layout) = (self.state, self.layout_idx);
                let secs = if self.animate { self.anim_start.elapsed().as_secs_f64() } else { 4.0 };
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let r = render_preview(&exe, &text, &workdir, state, layout, secs);
                    let _ = tx.send(r);
                    ctx2.request_repaint();
                });
            }
        }
        if self.preview.due.is_some() || self.preview.rx.is_some() || self.animate {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    // ---- panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        ui.horizontal(|ui| {
            if ui.button(tr(l, "New", "Nuovo")).clicked() {
                if self.dirty {
                    self.confirm = Some(Confirm::New);
                } else {
                    let p = self.path.clone();
                    self.ed = Ed::new(ConfDoc::parse(EXAMPLE));
                    self.saved_text = String::new();
                    self.raw_text = None;
                    self.dirty_rev = u64::MAX;
                    self.path = p;
                    self.say(tr(l, "New file from the example (not saved yet).", "Nuovo file dall'esempio (non ancora salvato)."), false);
                }
            }
            if ui.button(tr(l, "Open...", "Apri...")).clicked() {
                if let Some(p) = pick_file(tr(l, "Open a configuration", "Apri una configurazione"), Some(&self.path)) {
                    if self.dirty {
                        self.confirm = Some(Confirm::Open(p));
                    } else {
                        self.load(&p);
                    }
                }
            }
            if ui.add_enabled(true, egui::Button::new(tr(l, "Save", "Salva"))).on_hover_text("Ctrl+S").clicked() {
                self.save();
            }
            if ui.button(tr(l, "Save as...", "Salva con nome...")).clicked() {
                if let Some(p) = save_file_dialog(&self.path) {
                    self.save_to(&p);
                }
            }
            if ui.button(tr(l, "Reload", "Ricarica")).on_hover_text(tr(l, "Read the file from the disk again (drops unsaved changes)", "Rilegge il file dal disco (scarta le modifiche non salvate)")).clicked() {
                let p = self.path.clone();
                self.load(&p);
            }
            ui.separator();
            ui.label(RichText::new(self.path.display().to_string()).weak());
            if self.dirty {
                ui.label(RichText::new(tr(l, "● unsaved changes", "● modifiche non salvate")).color(Color32::from_rgb(255, 190, 90)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.lang == Lang::En, "EN").clicked() {
                    self.lang = Lang::En;
                }
                if ui.selectable_label(self.lang == Lang::It, "IT").clicked() {
                    self.lang = Lang::It;
                }
            });
        });
        // a path box for systems without file dialogs
        if !cfg!(windows) {
            ui.horizontal(|ui| {
                ui.label(tr(l, "File:", "File:"));
                ui.add(egui::TextEdit::singleline(&mut self.path_text).desired_width(420.0));
                if ui.button(tr(l, "Open this path", "Apri questo percorso")).clicked() {
                    let p = PathBuf::from(self.path_text.trim());
                    self.load(&p);
                }
            });
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, side: bool) {
        let l = self.lang;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr(l, "Preview", "Anteprima")).strong());
            egui::ComboBox::from_id_salt("pv_state")
                .selected_text(match self.state {
                    PreviewState::Idle => tr(l, "Nothing playing", "Niente in riproduzione"),
                    PreviewState::Playing => tr(l, "Music playing (spectrum)", "Musica (spettro)"),
                    PreviewState::Music => tr(l, "Music card", "Scheda musica"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.state, PreviewState::Idle, tr(l, "Nothing playing", "Niente in riproduzione"));
                    ui.selectable_value(&mut self.state, PreviewState::Playing, tr(l, "Music playing (spectrum)", "Musica (spettro)"));
                    ui.selectable_value(&mut self.state, PreviewState::Music, tr(l, "Music card", "Scheda musica"));
                });
            if self.state == PreviewState::Music && self.ed.value_of("music_screen", &self.fields) != "true" {
                ui.colored_label(Color32::from_rgb(255, 190, 90), tr(l, "turn on \"Music card\" in the Music section", "attiva \"Scheda musica\" nella sezione Musica"));
            }
            let names = self.layout_names();
            if names.len() > 1 {
                self.layout_idx = self.layout_idx.min(names.len() - 1);
                let cur = names[self.layout_idx].clone();
                egui::ComboBox::from_id_salt("pv_layout").selected_text(format!("{}: {cur}", tr(l, "screen", "schermata"))).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.layout_idx, i, format!("{}. {n}", i + 1));
                    }
                });
            }
            if ui.checkbox(&mut self.animate, tr(l, "Animate", "Anima")).on_hover_text(tr(l, "Redraws the preview over and over, so animated backgrounds move", "Ridisegna di continuo l'anteprima, così gli sfondi animati si muovono")).changed() {
                self.anim_start = Instant::now();
            }
            if ui.button(tr(l, "Refresh", "Aggiorna")).clicked() {
                self.preview.asked = None;
            }
            if self.preview.rx.is_some() {
                ui.label(RichText::new("...").weak());
            }
        });
        if self.preview.exe.is_none() {
            ui.colored_label(
                Color32::from_rgb(255, 170, 90),
                tr(l, "The preview needs trofeo_lcd.exe in the same folder as this program.", "L'anteprima richiede trofeo_lcd.exe nella stessa cartella di questo programma."),
            );
        }
        if let Some(e) = &self.preview.error {
            ui.colored_label(Color32::from_rgb(255, 120, 120), format!("{}: {e}", tr(l, "The program says", "Il programma dice")));
        }
        if let Some(t) = &self.preview.tex {
            let sz = t.size_vec2();
            let scale = if side { (ui.available_width() / sz.x).min(ui.available_height() / sz.y).min(1.0) } else { (ui.available_width() / sz.x).min(0.62) };
            let scale = scale.max(0.05);
            ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), sz * scale)).rounding(4.0));
        }
    }

    fn sections_panel(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        ui.add_space(4.0);
        for (id, en, it) in schema::SECTIONS {
            let n_set = self.fields.iter().filter(|f| f.section == *id && self.ed.doc.is_set(&f.key)).count();
            let text = if n_set > 0 { format!("{}  ({n_set})", tr(l, en, it)) } else { tr(l, en, it).to_string() };
            if ui.selectable_label(self.section == *id, text).clicked() {
                self.section = id.to_string();
            }
        }
        ui.separator();
        if ui.selectable_label(self.section == "raw", tr(l, "Raw file", "File grezzo")).clicked() {
            self.section = "raw".to_string();
            self.raw_text = None;
        }
        ui.add_space(8.0);
        ui.label(RichText::new(tr(l, "* needs a restart of trofeo_lcd", "* serve riavviare trofeo_lcd")).weak().small());
        ui.label(RichText::new(tr(l, "(n) = settings you changed", "(n) = impostazioni che hai cambiato")).weak().small());
    }

    fn form(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        if self.section == "raw" {
            let text = self.raw_text.get_or_insert_with(|| self.ed.doc.to_text());
            ui.label(RichText::new(tr(l, "The file as text. Edit freely; the changes apply as you type.", "Il file come testo. Modificalo liberamente; le modifiche valgono man mano che scrivi.")).weak());
            let before = text.clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(30).code_editor());
            });
            if *text != before {
                let new = text.clone();
                self.ed.doc = ConfDoc::parse(&new);
                self.ed.rev += 1;
            }
            return;
        }
        let fields: Vec<Field> = self.fields.iter().filter(|f| f.section == self.section).cloned().collect();
        let sec = schema::SECTIONS.iter().find(|(id, _, _)| *id == self.section).map(|(_, en, it)| tr(l, en, it)).unwrap_or("");
        ui.heading(sec);
        ui.add_space(4.0);
        // The preview shows the screens of the `layout` list: jump to the one this section is about.
        if self.section == "frame" || self.section == "tiles" {
            let name = self.section.clone();
            let names = self.layout_names();
            match names.iter().position(|n| *n == name) {
                Some(i) => {
                    if self.pv_section != self.section {
                        self.layout_idx = i;
                    }
                }
                None => {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            Color32::from_rgb(255, 190, 90),
                            tr(l, "This screen is not in your list of screens, so the program does not show it yet.", "Questa schermata non è nella tua lista di schermate, quindi il programma non la mostra ancora."),
                        );
                        if ui.button(tr(l, "Add it to the screens", "Aggiungila alle schermate")).clicked() {
                            let mut entries = parse_layouts(&self.ed.get("layout").unwrap_or_default());
                            entries.push(LayoutEntry { name: name.clone(), secs: None });
                            let n = entries.len();
                            self.ed.set("layout", &layouts_to_string(&entries));
                            self.layout_idx = n - 1;
                        }
                    });
                    ui.add_space(4.0);
                }
            }
        }
        self.pv_section = self.section.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_group = "";
            for f in &fields {
                if let Some((k, vals)) = f.when {
                    let cur = self.ed.value_of(k, &self.fields);
                    if !vals.iter().any(|v| cur.eq_ignore_ascii_case(v)) {
                        continue;
                    }
                }
                if f.group != last_group {
                    last_group = f.group;
                    if !f.group.is_empty() {
                        ui.add_space(8.0);
                        let (en, it) = schema::group_title(f.group);
                        ui.label(RichText::new(tr(l, en, it)).strong().color(Color32::from_rgb(140, 190, 255)));
                        ui.separator();
                    }
                }
                field_row(ui, &mut self.ed, f, l);
            }
        });
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let l = self.lang;
        if self.confirm.is_none() {
            return;
        }
        let mut action: Option<u8> = None; // 0 = save, 1 = discard, 2 = cancel
        egui::Window::new(tr(l, "Unsaved changes", "Modifiche non salvate")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(tr(l, "You have changes that are not saved.", "Hai modifiche non salvate."));
            ui.horizontal(|ui| {
                if ui.button(tr(l, "Save", "Salva")).clicked() {
                    action = Some(0);
                }
                if ui.button(tr(l, "Discard", "Scarta")).clicked() {
                    action = Some(1);
                }
                if ui.button(tr(l, "Cancel", "Annulla")).clicked() {
                    action = Some(2);
                }
            });
        });
        let Some(a) = action else { return };
        let what = std::mem::replace(&mut self.confirm, None);
        if a == 2 {
            return;
        }
        if a == 0 && !self.save() {
            return;
        }
        match what {
            Some(Confirm::Close) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Confirm::Open(p)) => self.load(&p),
            Some(Confirm::New) => {
                self.ed = Ed::new(ConfDoc::parse(EXAMPLE));
                self.saved_text = String::new();
                self.dirty_rev = u64::MAX;
                self.raw_text = None;
            }
            None => {}
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames += 1;
        self.refresh_dirty();
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Close);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save();
        }
        self.drive_preview(ctx);

        egui::TopBottomPanel::top("bar").show(ctx, |ui| self.top_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let (msg, err) = match &self.status {
                Some((m, _, e)) => (m.clone(), *e),
                None => (String::new(), false),
            };
            ui.horizontal(|ui| {
                if err {
                    ui.colored_label(Color32::from_rgb(255, 120, 120), msg);
                } else {
                    ui.label(RichText::new(msg).weak());
                }
            });
        });
        let portrait = self.preview.tex.as_ref().is_some_and(|t| t.size_vec2().y > t.size_vec2().x);
        if portrait {
            egui::SidePanel::right("preview").resizable(true).default_width(300.0).show(ctx, |ui| self.preview_panel(ui, true));
        } else {
            egui::TopBottomPanel::top("preview").show(ctx, |ui| self.preview_panel(ui, false));
        }
        egui::SidePanel::left("sections").resizable(false).exact_width(190.0).show(ctx, |ui| self.sections_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.form(ui));
        self.confirm_window(ctx);

        // developer option: screenshot of the window
        if let Some(path) = self.screenshot.clone() {
            let ready = self.preview.rx.is_none() && (self.preview.tex.is_some() || self.preview.error.is_some() || self.preview.exe.is_none());
            if ready && self.frames > 12 && !self.shot_requested {
                self.shot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                let (w, h) = (img.size[0] as u32, img.size[1] as u32);
                let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w, h, raw) {
                    let _ = buf.save(&path);
                }
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path: Option<PathBuf> = None;
    let (mut lang, mut section, mut shot) = (Lang::It, None, None);
    let mut size = [1280.0f32, 860.0];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lang" => lang = if it.next().is_some_and(|v| v.starts_with("en")) { Lang::En } else { Lang::It },
            "--section" => section = it.next().cloned(),
            "--screenshot" => shot = it.next().map(PathBuf::from),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|v| v.split_once('x')) {
                    size = [w.parse().unwrap_or(1280.0), h.parse().unwrap_or(860.0)];
                }
            }
            other if !other.starts_with("--") => path = Some(PathBuf::from(other)),
            _ => {}
        }
    }
    let path = path.unwrap_or_else(default_path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(size).with_min_inner_size([900.0, 560.0]).with_title("Trofeo LCD - config"),
        ..Default::default()
    };
    eframe::run_native(
        "Trofeo LCD - config",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(8.0, 5.0);
            style.spacing.slider_width = 200.0;
            cc.egui_ctx.set_style(style);
            Ok(Box::new(App::new(path.clone(), lang, section.clone(), shot.clone())))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_list_round_trip() {
        let v = parse_layouts("default:30, weather:5, cpu-gpu");
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], LayoutEntry { name: "default".into(), secs: Some(30) });
        assert_eq!(v[2].secs, None);
        assert_eq!(layouts_to_string(&v), "default:30, weather:5, cpu-gpu");
        assert!(parse_layouts("").is_empty());
    }

    #[test]
    fn tiles_round_trip() {
        let rows = parse_tiles(DEFAULT_TILES).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0][0].tall && rows[1][0].ring);
        assert_eq!(tiles_to_string(&rows), "clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, gpu:ring");
        assert_eq!(parse_tiles("clock, nonsense"), None);
        assert_eq!(parse_tiles("cpu:giant"), None);
        assert_eq!(parse_tiles(""), None);
        assert_eq!(tiles_to_string(&parse_tiles("cpu:ring:tall | gpu").unwrap()), "cpu:ring:tall | gpu");
    }

    #[test]
    fn multi_lists_toggle_and_keep_the_order_of_the_options() {
        let opts = schema::SPECTRUM_STYLES;
        assert_eq!(toggle_multi("", "led", opts), "led");
        assert_eq!(toggle_multi("led", "bars", opts), "bars, led");
        assert_eq!(toggle_multi("bars, led", "bars", opts), "led");
        // `all` stands alone
        assert_eq!(toggle_multi("bars, led", "all", opts), "all");
        assert_eq!(toggle_multi("all", "area", opts), "area");
        assert_eq!(toggle_multi("led", "led", opts), "");
        let stats = schema::MUSIC_STATS;
        assert_eq!(toggle_multi("cpu, gpu", "none", stats), "none");
    }

    #[test]
    fn the_editor_writes_what_the_program_reads() {
        // an edit session on the shipped example, through the same calls the widgets make
        let mut ed = Ed::new(ConfDoc::parse(EXAMPLE));
        ed.set("background_effect", "aquarium");
        ed.set("effect_theme", "light");
        ed.set("effect_level", "70");
        ed.set("layout", &layouts_to_string(&parse_layouts("tiles:20, weather:5")));
        ed.set("tiles", &tiles_to_string(&parse_tiles(DEFAULT_TILES).unwrap()));
        ed.unset("language");
        let text = ed.doc.to_text();
        let cfg = trofeo_lcd::config::ConfigFile::parse(&text).unwrap();
        assert_eq!(cfg.get("background_effect"), Some("aquarium"));
        assert_eq!(cfg.get("effect_level"), Some("70"));
        assert_eq!(cfg.get("layout"), Some("tiles:20, weather:5"));
        assert_eq!(cfg.get("language"), None);
        // the example's own lines are still there
        assert_eq!(cfg.get("orientation"), Some("landscape"));
        assert!(text.contains("# --- Orientation"));
    }

    #[test]
    fn every_field_is_reachable_and_the_example_documents_it() {
        // each setting the editor shows is one the shipped example also mentions
        let example = EXAMPLE.to_ascii_lowercase();
        let skip = ["tiles", "tile_", "tiles_", "panel", "panel_size"];
        for f in schema::fields() {
            if skip.iter().any(|s| f.key.starts_with(s)) || f.key.ends_with("_backdrop") || f.key.ends_with("_position") && f.section == "items" {
                continue;
            }
            if ["items"].contains(&f.section) {
                continue;
            }
            assert!(example.contains(&f.key), "{} is not in trofeo.conf.example", f.key);
        }
    }
}
