//! The visual builder of free layouts (see `trofeo_lcd::free`): the screen as the program draws
//! it, with a box over every element that can be dragged, resized and edited. Every change is
//! written to `layouts/<name>.layout` at once, so the program (when it shows that layout) follows
//! as you work.

use super::{render_preview, tr, PreviewState, Rendered};
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use trofeo_lcd::conf_edit::ConfDoc;
use trofeo_lcd::free::{self, Anchor, Element, Free, Kind};
use super::Lang;
use trofeo_lcd::Framebuffer;

/// What the page needs to know from the editor.
pub struct Ctx {
    pub lang: Lang,
    /// The folder of the configuration file.
    pub config_dir: PathBuf,
    pub exe: Option<PathBuf>,
    /// The configuration as text (the preview shows the layout over its background).
    pub doc_text: String,
    /// Is the layout in the `layout = ...` list (so the program shows it)?
    pub in_screens: bool,
}

pub enum Out {
    None,
    /// The button "add it to my screens" was pressed.
    AddToScreens(String),
}

#[derive(Clone, Copy)]
enum Drag {
    Move,
    Resize,
}

pub struct Builder {
    name: String,
    free: Free,
    loaded: bool,
    sel: Option<usize>,
    locked: Vec<bool>,
    new_name: String,
    msg: Option<(String, bool)>,
    undo: Vec<Free>,
    redo: Vec<Free>,
    changed_prev: bool,
    /// Bumped at every change: the preview and the file follow it.
    rev: u64,
    saved_rev: u64,
    asked_rev: Option<u64>,
    due: Option<Instant>,
    tex: Option<egui::TextureHandle>,
    tex_size: Option<[usize; 2]>,
    rx: Option<mpsc::Receiver<Result<Rendered, String>>>,
    drag: Option<Drag>,
    names: Vec<String>,
    names_at: Option<Instant>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            name: String::new(),
            free: Free::default(),
            loaded: false,
            sel: None,
            locked: Vec::new(),
            new_name: String::new(),
            msg: None,
            undo: Vec::new(),
            redo: Vec::new(),
            changed_prev: false,
            rev: 0,
            saved_rev: 0,
            asked_rev: None,
            due: None,
            tex: None,
            tex_size: None,
            rx: None,
            drag: None,
            names: Vec::new(),
            names_at: None,
        }
    }

    /// Name of the layout being edited (empty = none).
    pub fn current(&self) -> &str {
        &self.name
    }

    fn dir(cx: &Ctx) -> PathBuf {
        cx.config_dir.join(free::FOLDER)
    }

    fn open(&mut self, cx: &Ctx, name: &str) {
        match free::load(&Self::dir(cx), name) {
            Ok(f) => {
                self.name = name.to_string();
                self.locked = vec![false; f.elements.len()];
                self.free = f;
                self.sel = None;
                self.undo.clear();
                self.redo.clear();
                self.rev += 1;
                self.saved_rev = self.rev;
                self.msg = None;
            }
            Err(e) => self.msg = Some((e, true)),
        }
    }

    fn create(&mut self, cx: &Ctx, name: &str) {
        if let Err(e) = free::valid_name(name) {
            self.msg = Some((format!("'{name}': {e}"), true));
            return;
        }
        if free::path_of(&Self::dir(cx), name).exists() {
            self.msg = Some((tr(cx.lang, "A layout with that name exists already.", "Esiste già un layout con quel nome.").to_string(), true));
            return;
        }
        let mut f = Free::default();
        let mut t = Element::new(Kind::Text);
        t.x = 50.0;
        t.y = 50.0;
        t.anchor = Anchor::Center;
        t.size = 16;
        t.text = "{time}".to_string();
        f.elements.push(t);
        match free::save(&Self::dir(cx), name, &f) {
            Ok(_) => {
                self.names_at = None;
                self.open(cx, name);
                self.new_name.clear();
            }
            Err(e) => self.msg = Some((e, true)),
        }
    }

    /// A copy of a built-in design, to start from. Named `name`, or `<design>-mine` when empty.
    fn copy_builtin(&mut self, cx: &Ctx, design: &str, name: &str) {
        let f = match free::load_builtin(design) {
            Ok(f) => f,
            Err(e) => {
                self.msg = Some((e, true));
                return;
            }
        };
        let wanted = name.trim();
        let dir = Self::dir(cx);
        let name = if wanted.is_empty() {
            let mut n = format!("{design}-mine");
            let mut i = 2;
            while free::path_of(&dir, &n).exists() {
                n = format!("{design}-mine{i}");
                i += 1;
            }
            n
        } else {
            if let Err(e) = free::valid_name(wanted) {
                self.msg = Some((format!("'{wanted}': {e}"), true));
                return;
            }
            if free::path_of(&dir, wanted).exists() {
                self.msg = Some((tr(cx.lang, "A layout with that name exists already.", "Esiste già un layout con quel nome.").to_string(), true));
                return;
            }
            wanted.to_string()
        };
        match free::save(&dir, &name, &f) {
            Ok(_) => {
                self.names_at = None;
                self.open(cx, &name);
                self.new_name.clear();
            }
            Err(e) => self.msg = Some((e, true)),
        }
    }

    fn touch(&mut self) {
        self.rev += 1;
        self.redo.clear();
    }

    fn refresh_names(&mut self, cx: &Ctx) {
        if self.names_at.map_or(true, |t| t.elapsed() > Duration::from_secs(2)) {
            self.names = free::list(&Self::dir(cx));
            self.names_at = Some(Instant::now());
        }
    }

    // ---- the preview: the real program draws the layout, we put the boxes on it

    fn drive_preview(&mut self, ctx: &egui::Context, cx: &Ctx) {
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                if let Ok(img) = r {
                    self.tex_size = Some(img.image.size);
                    self.tex = Some(ctx.load_texture("builder", img.image, egui::TextureOptions::LINEAR));
                }
            }
        }
        let Some(exe) = cx.exe.clone() else { return };
        if self.name.is_empty() || self.saved_rev != self.rev {
            return; // the file is written first, then drawn
        }
        if self.asked_rev != Some(self.rev) && self.due.is_none() {
            self.due = Some(Instant::now() + Duration::from_millis(250));
        }
        if let Some(due) = self.due {
            if Instant::now() >= due && self.rx.is_none() {
                self.due = None;
                self.asked_rev = Some(self.rev);
                let mut doc = ConfDoc::parse(&cx.doc_text);
                doc.set("layout", &self.name);
                let text = doc.to_text();
                let dir = cx.config_dir.clone();
                let (tx, rx) = mpsc::channel();
                self.rx = Some(rx);
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let r = render_preview(&exe, &text, &dir, PreviewState::Idle, 0, 4.0, None);
                    let _ = tx.send(r);
                    ctx2.request_repaint();
                });
            }
        }
        if self.due.is_some() || self.rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    // ---- the page

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) -> Out {
        let l = cx.lang;
        let mut out = Out::None;
        self.refresh_names(cx);
        if !self.loaded {
            // the first time: the layout that was changed last
            self.loaded = true;
            let newest = self
                .names
                .iter()
                .max_by_key(|n| std::fs::metadata(free::path_of(&Self::dir(cx), n)).and_then(|m| m.modified()).ok())
                .cloned();
            if let Some(n) = newest {
                self.open(cx, &n);
            }
        }
        ui.heading(tr(l, "Layout builder", "Costruttore di layout"));
        ui.label(
            RichText::new(tr(
                l,
                "Design a screen of your own: texts with live values, bars, rings, rectangles and pictures, placed anywhere. Drag the boxes, drag the corner to resize, and set the details on the right. Every change is saved at once in the layouts folder; add the layout to your screens and the program shows it (and follows your changes as you make them).",
                "Disegna una schermata tua: testi con valori in tempo reale, barre, anelli, rettangoli e immagini, dove vuoi. Trascina i riquadri, trascina l'angolo per ridimensionare e imposta i dettagli a destra. Ogni modifica viene salvata subito nella cartella layouts; aggiungi il layout alle tue schermate e il programma lo mostra (e segue le modifiche mentre le fai).",
            ))
            .weak()
            .small(),
        );
        ui.add_space(4.0);
        // choose or make a layout
        ui.horizontal_wrapped(|ui| {
            ui.label(tr(l, "Layout:", "Layout:"));
            let shown = if self.name.is_empty() { tr(l, "(choose)", "(scegli)").to_string() } else { self.name.clone() };
            let mut pick: Option<String> = None;
            egui::ComboBox::from_id_salt("builder_layout").selected_text(shown).width(160.0).show_ui(ui, |ui| {
                for n in &self.names {
                    if ui.selectable_label(*n == self.name, n).clicked() {
                        pick = Some(n.clone());
                    }
                }
            });
            if let Some(n) = pick {
                self.open(cx, &n);
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(120.0).hint_text("my-screen"));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new(tr(l, "New layout", "Nuovo layout"))).clicked() {
                let n = self.new_name.trim().to_string();
                self.create(cx, &n);
            }
            ui.separator();
            let mut copy: Option<&'static str> = None;
            egui::ComboBox::from_id_salt("builder_builtin").selected_text(tr(l, "Copy a built-in design", "Copia un design integrato")).width(190.0).show_ui(ui, |ui| {
                for (n, en, it, _) in free::BUILTIN {
                    if ui.selectable_label(false, format!("{n}  -  {}", tr(l, en, it))).clicked() {
                        copy = Some(n);
                    }
                }
            });
            if let Some(d) = copy {
                let n = self.new_name.clone();
                self.copy_builtin(cx, d, &n);
            }
            if !self.name.is_empty() {
                ui.separator();
                if ui.button(tr(l, "Open the folder", "Apri la cartella")).clicked() {
                    trofeo_lcd::share::open(&Self::dir(cx).to_string_lossy());
                }
            }
        });
        if let Some((m, err)) = &self.msg {
            ui.colored_label(if *err { Color32::from_rgb(255, 190, 90) } else { Color32::from_rgb(120, 220, 140) }, m);
        }
        if self.name.is_empty() {
            ui.add_space(8.0);
            ui.label(tr(l, "Choose a layout, or give a name and press New layout.", "Scegli un layout, oppure scrivi un nome e premi Nuovo layout."));
            return out;
        }
        if !cx.in_screens {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    Color32::from_rgb(255, 190, 90),
                    tr(l, "This layout is not in your list of screens, so the program does not show it yet.", "Questo layout non è nella tua lista di schermate, quindi il programma non lo mostra ancora."),
                );
                if ui.button(tr(l, "Add it to the screens", "Aggiungilo alle schermate")).clicked() {
                    out = Out::AddToScreens(self.name.clone());
                }
            });
        }
        self.keys(ui);
        let before = self.free.clone();
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            let side = 270.0;
            let canvas_w = (ui.available_width() - side - 12.0).max(200.0);
            ui.vertical(|ui| {
                ui.set_width(canvas_w);
                self.canvas(ui, canvas_w);
                ui.add_space(4.0);
                self.add_buttons(ui, l);
            });
            ui.vertical(|ui| {
                ui.set_width(side);
                self.list(ui, l);
                ui.add_space(6.0);
                self.properties(ui, l);
            });
        });
        // changes: remember the state before (one step per drag or edit), write the file
        let changed = self.free != before;
        if changed {
            let dragging = ui.input(|i| i.pointer.any_down());
            if !(self.changed_prev && dragging) {
                self.undo.push(before);
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
            }
            self.touch();
        }
        self.changed_prev = changed;
        if self.saved_rev != self.rev && !ui.input(|i| i.pointer.any_down()) {
            match free::save(&Self::dir(cx), &self.name, &self.free) {
                Ok(_) => {
                    self.saved_rev = self.rev;
                    self.msg = Some((tr(l, "Saved.", "Salvato.").to_string(), false));
                }
                Err(e) => self.msg = Some((e, true)),
            }
        }
        self.drive_preview(ui.ctx(), cx);
        out
    }

    fn keys(&mut self, ui: &mut egui::Ui) {
        let typing = ui.ctx().wants_keyboard_input();
        if typing {
            return;
        }
        let (del, undo, redo, dup, left, right, up, down) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete),
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                i.modifiers.command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                i.modifiers.command && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if undo {
            if let Some(prev) = self.undo.pop() {
                self.redo.push(std::mem::replace(&mut self.free, prev));
                self.locked.resize(self.free.elements.len(), false);
                self.sel = None;
                self.rev += 1;
            }
        }
        if redo {
            if let Some(next) = self.redo.pop() {
                self.undo.push(std::mem::replace(&mut self.free, next));
                self.locked.resize(self.free.elements.len(), false);
                self.sel = None;
                self.rev += 1;
            }
        }
        let Some(i) = self.sel.filter(|i| *i < self.free.elements.len()) else { return };
        if del {
            self.remove(i);
        } else if dup {
            self.duplicate(i);
        } else {
            let (dx, dy) = ((right as i32 - left as i32) as f32 * 0.5, (down as i32 - up as i32) as f32 * 0.5);
            if dx != 0.0 || dy != 0.0 {
                let e = &mut self.free.elements[i];
                e.x += dx;
                e.y += dy;
            }
        }
    }

    fn remove(&mut self, i: usize) {
        self.free.elements.remove(i);
        if i < self.locked.len() {
            self.locked.remove(i);
        }
        self.sel = None;
    }

    fn duplicate(&mut self, i: usize) {
        let mut e = self.free.elements[i].clone();
        e.x += 2.0;
        e.y += 3.0;
        self.free.elements.push(e);
        self.locked.push(false);
        self.sel = Some(self.free.elements.len() - 1);
    }

    fn add_buttons(&mut self, ui: &mut egui::Ui, l: Lang) {
        ui.horizontal_wrapped(|ui| {
            ui.label(tr(l, "Add:", "Aggiungi:"));
            for k in Kind::ALL {
                let label = match k {
                    Kind::Text => tr(l, "Text", "Testo"),
                    Kind::Bar => tr(l, "Bar", "Barra"),
                    Kind::Ring => tr(l, "Ring", "Anello"),
                    Kind::Rect => tr(l, "Rectangle", "Rettangolo"),
                    Kind::Image => tr(l, "Picture", "Immagine"),
                };
                if ui.button(label).clicked() {
                    let mut e = Element::new(k);
                    // not on top of the one before
                    let n = self.free.elements.len() as f32;
                    e.x = 10.0 + (n * 3.0) % 40.0;
                    e.y = 10.0 + (n * 4.0) % 40.0;
                    self.free.elements.push(e);
                    self.locked.push(false);
                    self.sel = Some(self.free.elements.len() - 1);
                }
            }
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(tr(l, "Undo", "Annulla"))).on_hover_text("Ctrl+Z").clicked() {
                if let Some(prev) = self.undo.pop() {
                    self.redo.push(std::mem::replace(&mut self.free, prev));
                    self.locked.resize(self.free.elements.len(), false);
                    self.sel = None;
                    self.rev += 1;
                }
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new(tr(l, "Redo", "Ripeti"))).on_hover_text("Ctrl+Y").clicked() {
                if let Some(next) = self.redo.pop() {
                    self.undo.push(std::mem::replace(&mut self.free, next));
                    self.locked.resize(self.free.elements.len(), false);
                    self.sel = None;
                    self.rev += 1;
                }
            }
        });
        ui.label(
            RichText::new(tr(
                l,
                "Arrows nudge the selected element, Delete removes it, Ctrl+D copies it, Alt while dragging turns the snapping off.",
                "Le frecce spostano l'elemento scelto, Canc lo toglie, Ctrl+D lo copia, Alt mentre trascini toglie lo scatto.",
            ))
            .weak()
            .small(),
        );
    }

    /// The size of an element on the picture, in the picture's own pixels.
    fn box_of(e: &Element, img: (f32, f32)) -> (f32, f32) {
        match e.kind {
            Kind::Text => {
                let sample = free::substitute(&e.text, &|_| "00000".to_string());
                let sample = if sample.is_empty() { "?".to_string() } else { sample };
                let size = e.size.max(1);
                let mut w = Framebuffer::text_width(&sample, size) as f32;
                if e.w > 0.0 {
                    w = w.min(e.w / 100.0 * img.0);
                }
                (w, Framebuffer::text_height(size) as f32)
            }
            _ => ((e.w / 100.0 * img.0).max(6.0), (e.h / 100.0 * img.1).max(6.0)),
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, canvas_w: f32) {
        let size = self.tex_size.map(|s| (s[0] as f32, s[1] as f32)).unwrap_or((1920.0, 462.0));
        // a tall (portrait) screen must not go beyond the window
        let max_h = (ui.ctx().screen_rect().height() - 330.0).max(200.0);
        let canvas_w = canvas_w.min(max_h * size.0 / size.1);
        let canvas_h = canvas_w * size.1 / size.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(canvas_w, canvas_h), egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_rgb(10, 10, 16));
        if let Some(t) = &self.tex {
            painter.image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "...", egui::FontId::proportional(18.0), Color32::GRAY);
        }
        let scale = canvas_w / size.0;
        // a click on the empty canvas lets go of the selection
        if resp.clicked() {
            self.sel = None;
        }
        // guides at the middle
        let guide = Color32::from_rgba_unmultiplied(255, 255, 255, 28);
        painter.line_segment([rect.center_top(), rect.center_bottom()], egui::Stroke::new(1.0, guide));
        painter.line_segment([rect.left_center(), rect.right_center()], egui::Stroke::new(1.0, guide));
        let no_snap = ui.input(|i| i.modifiers.alt);
        let mut new_sel = self.sel;
        let mut drag_now: Option<Drag> = None;
        for i in 0..self.free.elements.len() {
            let e = self.free.elements[i].clone();
            let (bw, bh) = Self::box_of(&e, size);
            let (px, py) = free::place(&e, size.0, size.1, bw, bh);
            let r = egui::Rect::from_min_size(rect.min + egui::vec2(px, py) * scale, egui::vec2(bw, bh) * scale).intersect(rect.expand(40.0));
            let selected = self.sel == Some(i);
            let locked = self.locked.get(i).copied().unwrap_or(false);
            let sense = if locked { egui::Sense::click() } else { egui::Sense::click_and_drag() };
            let body = ui.interact(r, ui.id().with(("el", i)), sense);
            let stroke = if selected { egui::Stroke::new(2.0, Color32::from_rgb(255, 200, 60)) } else if body.hovered() { egui::Stroke::new(1.5, Color32::from_rgb(120, 200, 255)) } else { egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 200, 255, 90)) };
            painter.rect_stroke(r, 0.0, stroke);
            if body.clicked() || body.drag_started() {
                new_sel = Some(i);
            }
            // the handle for the size (everything but a text)
            let mut handle_drag = false;
            if selected && e.kind != Kind::Text && !locked {
                let h = egui::Rect::from_center_size(r.right_bottom(), egui::vec2(12.0, 12.0));
                let hr = ui.interact(h, ui.id().with(("handle", i)), egui::Sense::drag());
                painter.rect_filled(h, 2.0, Color32::from_rgb(255, 200, 60));
                if hr.dragged() {
                    handle_drag = true;
                    drag_now = Some(Drag::Resize);
                    let d = hr.drag_delta() / scale;
                    let el = &mut self.free.elements[i];
                    // the top left corner stays where it is, the anchor point follows the new size
                    resize_to(el, size, (px, py), ((bw + d.x).max(4.0), (bh + d.y).max(4.0)));
                }
                if hr.drag_stopped() && !no_snap {
                    // let go: the size snaps to a half percent, the corner stays where it is
                    let el = &mut self.free.elements[i];
                    let corner = free::place(el, size.0, size.1, el.w / 100.0 * size.0, el.h / 100.0 * size.1);
                    let (w, h) = (snap(el.w, true).max(0.5) / 100.0 * size.0, snap(el.h, true).max(0.5) / 100.0 * size.1);
                    resize_to(el, size, corner, (w, h));
                }
            }
            if body.dragged() && !handle_drag && !locked {
                drag_now = Some(Drag::Move);
                let d = body.drag_delta() / scale;
                let el = &mut self.free.elements[i];
                el.x += d.x / size.0 * 100.0;
                el.y += d.y / size.1 * 100.0;
            }
            if body.drag_stopped() && !locked && !no_snap {
                let el = &mut self.free.elements[i];
                el.x = snap_pos(el.x);
                el.y = snap_pos(el.y);
            }
        }
        self.sel = new_sel;
        self.drag = drag_now;
        if let Some(i) = self.sel.filter(|i| *i < self.free.elements.len()) {
            let e = &self.free.elements[i];
            painter.text(rect.left_bottom() + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("{}  x {:.1}%  y {:.1}%", e.kind.name(), e.x, e.y), egui::FontId::monospace(12.0), Color32::from_rgb(255, 200, 60));
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, l: Lang) {
        ui.label(RichText::new(tr(l, "Elements (the last one is on top)", "Elementi (l'ultimo è sopra)")).strong().color(Color32::from_rgb(140, 190, 255)));
        egui::ScrollArea::vertical().id_salt("builder_list").max_height(150.0).show(ui, |ui| {
            for i in (0..self.free.elements.len()).rev() {
                let e = &self.free.elements[i];
                let summary = match e.kind {
                    Kind::Text => format!("text  {}", e.text),
                    Kind::Bar | Kind::Ring => format!("{}  {}", e.kind.name(), e.value),
                    Kind::Image => format!("image  {}", e.file),
                    Kind::Rect => "rect".to_string(),
                };
                let lock = if self.locked.get(i).copied().unwrap_or(false) { "\u{1F512} " } else { "" };
                if ui.selectable_label(self.sel == Some(i), format!("{lock}{summary}")).clicked() {
                    self.sel = Some(i);
                }
            }
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui, l: Lang) {
        let Some(i) = self.sel.filter(|i| *i < self.free.elements.len()) else {
            ui.label(RichText::new(tr(l, "Click an element to edit it.", "Clicca un elemento per modificarlo.")).weak());
            return;
        };
        ui.label(RichText::new(tr(l, "Details", "Dettagli")).strong().color(Color32::from_rgb(140, 190, 255)));
        let mut action: Option<&str> = None;
        let mut lock = self.locked.get(i).copied().unwrap_or(false);
        {
            let e = &mut self.free.elements[i];
            egui::ScrollArea::vertical().id_salt("builder_props").max_height(330.0).show(ui, |ui| {
                egui::Grid::new("builder_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label(tr(l, "Position", "Posizione"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut e.x).speed(0.2).suffix("%").range(-100.0..=200.0).max_decimals(1));
                        ui.add(egui::DragValue::new(&mut e.y).speed(0.2).suffix("%").range(-100.0..=200.0).max_decimals(1));
                    });
                    ui.end_row();
                    ui.label(tr(l, "Anchor", "Ancoraggio"));
                    egui::ComboBox::from_id_salt("builder_anchor").selected_text(e.anchor.name()).width(70.0).show_ui(ui, |ui| {
                        for a in Anchor::ALL {
                            ui.selectable_value(&mut e.anchor, a, a.name());
                        }
                    });
                    ui.end_row();
                    if e.kind != Kind::Text {
                        ui.label(tr(l, "Size", "Dimensione"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut e.w).speed(0.2).suffix("%").range(0.5..=200.0).max_decimals(1));
                            ui.add(egui::DragValue::new(&mut e.h).speed(0.2).suffix("%").range(0.5..=200.0).max_decimals(1));
                        });
                        ui.end_row();
                    }
                    if e.kind == Kind::Text {
                        ui.label(tr(l, "Text size", "Dimensione testo"));
                        ui.add(egui::Slider::new(&mut e.size, 1..=60));
                        ui.end_row();
                        ui.label(tr(l, "Max width", "Larghezza max"));
                        ui.add(egui::DragValue::new(&mut e.w).speed(0.2).suffix("%").range(0.0..=200.0).max_decimals(1)).on_hover_text(tr(l, "0 = no limit; a longer text is made smaller", "0 = nessun limite; un testo più lungo viene rimpicciolito"));
                        ui.end_row();
                    }
                    if matches!(e.kind, Kind::Text | Kind::Ring) {
                        ui.label(if e.kind == Kind::Text { tr(l, "Text", "Testo") } else { tr(l, "Text inside", "Testo dentro") });
                        ui.vertical(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut e.text).desired_width(150.0));
                            let mut ins: Option<&str> = None;
                            egui::ComboBox::from_id_salt("builder_ins").selected_text(tr(l, "insert a value...", "inserisci un valore...")).width(150.0).show_ui(ui, |ui| {
                                for p in free::PLACEHOLDERS {
                                    if ui.selectable_label(false, format!("{{{p}}}")).clicked() {
                                        ins = Some(p);
                                    }
                                }
                            });
                            if let Some(p) = ins {
                                e.text.push_str(&format!("{{{p}}}"));
                            }
                        });
                        ui.end_row();
                    }
                    if matches!(e.kind, Kind::Bar | Kind::Ring) {
                        ui.label(tr(l, "Shows", "Mostra"));
                        egui::ComboBox::from_id_salt("builder_value")
                            .selected_text(free::METRICS.iter().find(|m| m.0 == e.value).map_or(e.value.as_str(), |m| m.1))
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for (id, label, _) in free::METRICS {
                                    if ui.selectable_label(e.value == *id, *label).clicked() {
                                        e.value = id.to_string();
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label(tr(l, "From / to", "Da / a"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut e.min).speed(1.0));
                            let mut max = e.max.unwrap_or_else(|| free::metric_max(&e.value));
                            if ui.add(egui::DragValue::new(&mut max).speed(1.0)).changed() {
                                e.max = Some(max.max(e.min + 1.0));
                            }
                            if e.max.is_some() && ui.small_button("x").on_hover_text(tr(l, "back to the usual maximum", "torna al massimo usuale")).clicked() {
                                e.max = None;
                            }
                        });
                        ui.end_row();
                        ui.label(tr(l, "Turns hot at", "Diventa caldo da"));
                        ui.horizontal(|ui| {
                            let mut on = e.hot.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                e.hot = on.then(|| free::metric_max(&e.value) * 0.85);
                            }
                            if let Some(h) = e.hot.as_mut() {
                                ui.add(egui::DragValue::new(h).speed(1.0));
                                color_button(ui, &mut e.hot_color);
                            }
                        });
                        ui.end_row();
                        ui.label(tr(l, "Track color", "Colore del fondo"));
                        color_button(ui, &mut e.back);
                        ui.end_row();
                    }
                    if e.kind == Kind::Ring {
                        ui.label(tr(l, "Thickness", "Spessore"));
                        ui.add(egui::Slider::new(&mut e.thick, 4.0..=60.0).suffix("%"));
                        ui.end_row();
                    }
                    if matches!(e.kind, Kind::Rect | Kind::Image) {
                        ui.label(tr(l, "Opacity", "Opacità"));
                        ui.add(egui::Slider::new(&mut e.opacity, 0..=100).suffix("%"));
                        ui.end_row();
                    }
                    if e.kind == Kind::Image {
                        ui.label(tr(l, "File", "File"));
                        ui.vertical(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut e.file).desired_width(150.0).hint_text("C:\\Pictures\\logo.png"));
                            if ui.small_button(tr(l, "Choose a file...", "Scegli un file...")).clicked() {
                                if let Some(p) = super::pick_file(tr(l, "Picture", "Immagine"), None) {
                                    e.file = p.to_string_lossy().to_string();
                                }
                            }
                        });
                        ui.end_row();
                    }
                    if e.kind != Kind::Image {
                        ui.label(tr(l, "Color", "Colore"));
                        ui.horizontal(|ui| {
                            let mut custom = e.color.is_some();
                            if ui.checkbox(&mut custom, tr(l, "own", "mio")).changed() {
                                e.color = custom.then(|| e.color.unwrap_or((0xF0, 0xF0, 0xF0)));
                            }
                            if let Some(c) = e.color.as_mut() {
                                color_button(ui, c);
                            } else {
                                ui.label(RichText::new(match e.kind {
                                    Kind::Text => tr(l, "(the text color)", "(il colore del testo)"),
                                    Kind::Rect => tr(l, "(dark)", "(scuro)"),
                                    _ => tr(l, "(by level)", "(secondo il livello)"),
                                }).weak());
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.checkbox(&mut lock, tr(l, "Lock (the mouse does not move it)", "Blocca (il mouse non lo sposta)"));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr(l, "Copy", "Duplica")).clicked() {
                        action = Some("dup");
                    }
                    if ui.button(tr(l, "Forward", "Avanti")).clicked() {
                        action = Some("up");
                    }
                    if ui.button(tr(l, "Backward", "Indietro")).clicked() {
                        action = Some("down");
                    }
                    if ui.button(tr(l, "Delete", "Elimina")).clicked() {
                        action = Some("del");
                    }
                });
            });
        }
        if let Some(v) = self.locked.get_mut(i) {
            *v = lock;
        }
        match action {
            Some("dup") => self.duplicate(i),
            Some("del") => self.remove(i),
            Some("up") if i + 1 < self.free.elements.len() => {
                self.free.elements.swap(i, i + 1);
                self.locked.swap(i, i + 1);
                self.sel = Some(i + 1);
            }
            Some("down") if i > 0 => {
                self.free.elements.swap(i, i - 1);
                self.locked.swap(i, i - 1);
                self.sel = Some(i - 1);
            }
            _ => {}
        }
    }
}

fn color_button(ui: &mut egui::Ui, c: &mut (u8, u8, u8)) {
    let mut rgb = [c.0, c.1, c.2];
    if ui.color_edit_button_srgb(&mut rgb).changed() {
        *c = (rgb[0], rgb[1], rgb[2]);
    }
}

/// Give an element a new size (in picture pixels) with its top left corner (`corner`, picture
/// pixels) where it is: the anchor point (`x`, `y`) moves to fit.
fn resize_to(e: &mut Element, img: (f32, f32), corner: (f32, f32), new: (f32, f32)) {
    let (fx, fy) = e.anchor.fractions();
    e.w = (new.0 / img.0 * 100.0).max(0.5);
    e.h = (new.1 / img.1 * 100.0).max(0.5);
    e.x = (corner.0 + fx * e.w / 100.0 * img.0) / img.0 * 100.0;
    e.y = (corner.1 + fy * e.h / 100.0 * img.1) / img.1 * 100.0;
}

/// Sizes snap to a half percent.
fn snap(v: f32, on: bool) -> f32 {
    if on { (v * 2.0).round() / 2.0 } else { v }
}

/// A position snaps to a half percent, and to the edges and the middle when it is close (1%).
pub fn snap_pos(v: f32) -> f32 {
    for guide in [0.0, 50.0, 100.0] {
        if (v - guide).abs() <= 1.0 {
            return guide;
        }
    }
    (v * 2.0).round() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_snap_to_the_guides_and_the_half_percent() {
        assert_eq!(snap_pos(0.7), 0.0);
        assert_eq!(snap_pos(49.2), 50.0);
        assert_eq!(snap_pos(99.5), 100.0);
        assert_eq!(snap_pos(20.3), 20.5);
        assert_eq!(snap_pos(20.2), 20.0);
        assert_eq!(snap(12.3, true), 12.5);
        assert_eq!(snap(12.3, false), 12.3);
    }

    #[test]
    fn resizing_keeps_the_corner_for_every_anchor() {
        let img = (1000.0, 400.0);
        for a in Anchor::ALL {
            let mut e = Element::new(Kind::Rect);
            e.anchor = a;
            e.x = 40.0;
            e.y = 30.0;
            e.w = 20.0;
            e.h = 10.0;
            let corner = free::place(&e, img.0, img.1, e.w / 100.0 * img.0, e.h / 100.0 * img.1);
            resize_to(&mut e, img, corner, (400.0, 120.0));
            assert!((e.w - 40.0).abs() < 0.001 && (e.h - 30.0).abs() < 0.001, "{a:?}: {} {}", e.w, e.h);
            let after = free::place(&e, img.0, img.1, e.w / 100.0 * img.0, e.h / 100.0 * img.1);
            assert!((after.0 - corner.0).abs() < 0.01 && (after.1 - corner.1).abs() < 0.01, "{a:?}: {after:?} vs {corner:?}");
        }
    }

    #[test]
    fn a_text_box_follows_the_text_and_its_size() {
        let mut e = Element::new(Kind::Text);
        e.size = 4;
        e.text = "{cpu}".to_string();
        let (w1, h1) = Builder::box_of(&e, (1920.0, 462.0));
        e.size = 8;
        let (w2, h2) = Builder::box_of(&e, (1920.0, 462.0));
        assert!(w2 > w1 && h2 > h1);
        e.w = 1.0;
        assert!(Builder::box_of(&e, (1920.0, 462.0)).0 <= 19.3, "a max width limits it");
        let bar = Element::new(Kind::Bar);
        assert_eq!(Builder::box_of(&bar, (1000.0, 400.0)), (300.0, 24.0));
    }

    #[test]
    fn a_layout_can_be_made_opened_and_changed_in_the_builder() {
        let dir = std::env::temp_dir().join(format!("trofeo_builder_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cx = Ctx { lang: Lang::En, config_dir: dir.clone(), exe: None, doc_text: String::new(), in_screens: false };
        let mut b = Builder::new();
        b.create(&cx, "demo");
        assert_eq!(b.current(), "demo");
        assert_eq!(b.free.elements.len(), 1);
        b.duplicate(0);
        assert_eq!((b.free.elements.len(), b.sel), (2, Some(1)));
        b.free.elements.push(Element::new(Kind::Bar));
        b.locked.push(false);
        free::save(&Builder::dir(&cx), "demo", &b.free).unwrap();
        let mut b2 = Builder::new();
        b2.open(&cx, "demo");
        assert_eq!(b2.free, b.free);
        b2.remove(0);
        assert_eq!((b2.free.elements.len(), b2.locked.len()), (2, 2));
        b2.create(&cx, "demo"); // exists
        assert!(b2.msg.as_ref().is_some_and(|m| m.1));
        b2.create(&cx, "Bad Name");
        assert!(b2.msg.as_ref().is_some_and(|m| m.1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_built_in_design_can_be_copied_to_start_from() {
        let dir = std::env::temp_dir().join(format!("trofeo_builder_copy_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cx = Ctx { lang: Lang::En, config_dir: dir.clone(), exe: None, doc_text: String::new(), in_screens: false };
        let mut b = Builder::new();
        b.copy_builtin(&cx, "halo", "");
        assert_eq!(b.current(), "halo-mine");
        assert_eq!(b.free, free::load_builtin("halo").unwrap());
        b.copy_builtin(&cx, "halo", "");
        assert_eq!(b.current(), "halo-mine2");
        b.copy_builtin(&cx, "gauges", "my-gauges");
        assert_eq!(b.current(), "my-gauges");
        b.copy_builtin(&cx, "gauges", "my-gauges"); // exists
        assert!(b.msg.as_ref().is_some_and(|m| m.1));
        b.copy_builtin(&cx, "gauges", "Bad Name");
        assert!(b.msg.as_ref().is_some_and(|m| m.1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
