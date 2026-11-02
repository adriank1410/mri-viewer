//! Okno programu (egui).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Color32, ColorImage, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};

use crate::cache::FrameCache;
use crate::imaging::{self, Frame};
use crate::library::{self, Library, Plane, Series, fmt_date};
use crate::mpr;

const STORAGE_LAST_FOLDER: &str = "last_folder";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Window,
    Pan,
    Measure,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewPlane {
    Native,
    Reformat(Plane),
}

/// Jedno okno podglądu (w trybie porównania są dwa).
struct Viewport {
    series_id: Option<usize>,
    slice: usize,
    plane: ViewPlane,
    reformat_pos: usize,
    reformat_positions: usize,
    window: Option<(f32, f32)>,
    invert: bool,
    zoom: f32,
    pan: Vec2,
    texture: Option<TextureHandle>,
    tex_key: Option<(usize, usize, u8, usize, i64, i64, bool)>,
    /// Wymiary aktualnie wyświetlanego obrazu i odstęp pikseli (pion, poziom).
    shown: Option<(u32, u32, (f64, f64))>,
    shown_range: (f32, f32),
    shown_default_window: (f32, f32),
    measures: HashMap<(usize, u8, usize), Vec<(Pos2, Pos2)>>,
    pending_point: Option<Pos2>,
    playing: bool,
    play_accum: f64,
    scroll_accum: f32,
    reformat_cache: Option<((usize, u8, usize), Arc<Frame>, (f64, f64))>,
}

impl Viewport {
    fn new() -> Self {
        Self {
            series_id: None,
            slice: 0,
            plane: ViewPlane::Native,
            reformat_pos: 0,
            reformat_positions: 0,
            window: None,
            invert: false,
            zoom: 1.0,
            pan: Vec2::ZERO,
            texture: None,
            tex_key: None,
            shown: None,
            shown_range: (0.0, 1.0),
            shown_default_window: (0.5, 1.0),
            measures: HashMap::new(),
            pending_point: None,
            playing: false,
            play_accum: 0.0,
            scroll_accum: 0.0,
            reformat_cache: None,
        }
    }

    fn reset_view(&mut self) {
        self.window = None;
        self.invert = false;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    fn plane_code(&self) -> u8 {
        match self.plane {
            ViewPlane::Native => 0,
            ViewPlane::Reformat(Plane::Axial) => 1,
            ViewPlane::Reformat(Plane::Coronal) => 2,
            ViewPlane::Reformat(Plane::Sagittal) => 3,
            ViewPlane::Reformat(_) => 4,
        }
    }

    fn position(&self) -> usize {
        match self.plane {
            ViewPlane::Native => self.slice,
            ViewPlane::Reformat(_) => self.reformat_pos,
        }
    }

    fn count(&self, series: &Series) -> usize {
        match self.plane {
            ViewPlane::Native => series.slices.len(),
            ViewPlane::Reformat(_) => self.reformat_positions.max(1),
        }
    }

    fn step(&mut self, series: &Series, delta: i64) {
        let n = self.count(series) as i64;
        let p = (self.position() as i64 + delta).clamp(0, n - 1) as usize;
        match self.plane {
            ViewPlane::Native => self.slice = p,
            ViewPlane::Reformat(_) => self.reformat_pos = p,
        }
    }
}

enum ScanMsg {
    Progress(usize, usize),
    Done(Library),
}

/// Wczytywanie wszystkich przekrojów serii w tle, zaczynając od bieżącego.
struct Loader {
    generation: Arc<AtomicUsize>,
    target: Arc<AtomicUsize>,
}

pub struct App {
    lib: Option<Arc<Library>>,
    folder: Option<PathBuf>,
    scan_rx: Option<Receiver<ScanMsg>>,
    scan_progress: (usize, usize),
    cache: Arc<FrameCache>,
    viewports: Vec<Viewport>,
    active: usize,
    compare: bool,
    sync_scroll: bool,
    tool: Tool,
    loaders: HashMap<usize, Loader>,
    thumbs: HashMap<usize, TextureHandle>,
    thumb_tx: Sender<(usize, ColorImage)>,
    thumb_rx: Receiver<(usize, ColorImage)>,
    thumbs_requested: HashSet<usize>,
    errors: Arc<Mutex<HashMap<usize, String>>>,
    show_help: bool,
    show_info: bool,
    status: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Self {
        let (thumb_tx, thumb_rx) = channel();
        let mut app = Self {
            lib: None,
            folder: None,
            scan_rx: None,
            scan_progress: (0, 0),
            cache: Arc::new(FrameCache::new(2_500_000_000)),
            viewports: vec![Viewport::new()],
            active: 0,
            compare: false,
            sync_scroll: false,
            tool: Tool::Window,
            loaders: HashMap::new(),
            thumbs: HashMap::new(),
            thumb_tx,
            thumb_rx,
            thumbs_requested: HashSet::new(),
            errors: Arc::new(Mutex::new(HashMap::new())),
            show_help: false,
            show_info: true,
            status: String::new(),
        };
        let start = folder.or_else(|| {
            cc.storage
                .and_then(|s| s.get_string(STORAGE_LAST_FOLDER))
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
        });
        if let Some(f) = start {
            app.open_folder(f, &cc.egui_ctx);
        }
        app
    }

    fn open_folder(&mut self, folder: PathBuf, ctx: &egui::Context) {
        // nowe pliki → czyścimy stan
        for l in self.loaders.values() {
            l.generation.fetch_add(1, Ordering::SeqCst);
        }
        self.loaders.clear();
        self.cache.clear();
        self.thumbs.clear();
        self.thumbs_requested.clear();
        self.errors.lock().unwrap().clear();
        self.viewports = vec![Viewport::new()];
        self.active = 0;
        self.compare = false;
        self.lib = None;
        self.folder = Some(folder.clone());
        let (tx, rx) = channel();
        self.scan_rx = Some(rx);
        self.scan_progress = (0, 0);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let tx2 = Mutex::new(tx.clone());
            let ctx2 = ctx.clone();
            let progress = move |a: usize, b: usize| {
                let _ = tx2.lock().unwrap().send(ScanMsg::Progress(a, b));
                ctx2.request_repaint();
            };
            let lib = library::scan(&folder, Some(&progress));
            let _ = tx.send(ScanMsg::Done(lib));
            ctx.request_repaint();
        });
    }

    fn choose_folder(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new().set_title("Wybierz folder z badaniem (np. kopię płyty)");
        if let Some(f) = &self.folder {
            dlg = dlg.set_directory(f);
        }
        if let Some(f) = dlg.pick_folder() {
            self.open_folder(f, ctx);
        }
    }

    fn series(&self, id: usize) -> Option<&Series> {
        self.lib.as_ref()?.series_by_id(id).map(|(_, s)| s)
    }

    fn assign_series(&mut self, vp_index: usize, id: usize, ctx: &egui::Context) {
        let Some(series) = self.series(id).cloned() else { return };
        let vp = &mut self.viewports[vp_index];
        vp.series_id = Some(id);
        vp.slice = series.slices.len() / 2;
        vp.plane = ViewPlane::Native;
        vp.reset_view();
        vp.playing = false;
        vp.pending_point = None;
        vp.tex_key = None;
        vp.reformat_cache = None;
        self.ensure_loader(&series, ctx);
    }

    fn ensure_loader(&mut self, series: &Series, ctx: &egui::Context) {
        if self.loaders.contains_key(&series.id) {
            return;
        }
        // ograniczamy liczbę serii wczytywanych naraz
        let in_use: HashSet<usize> = self.viewports.iter().filter_map(|v| v.series_id).collect();
        self.loaders.retain(|id, l| {
            let keep = in_use.contains(id);
            if !keep {
                l.generation.fetch_add(1, Ordering::SeqCst);
            }
            keep
        });
        let generation = Arc::new(AtomicUsize::new(0));
        let target = Arc::new(AtomicUsize::new(series.slices.len() / 2));
        let loader = Loader {
            generation: generation.clone(),
            target: target.clone(),
        };
        self.loaders.insert(series.id, loader);
        let slices = series.slices.clone();
        let cache = self.cache.clone();
        let errors = self.errors.clone();
        let sid = series.id;
        let ctx = ctx.clone();
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6);
        let done: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(vec![false; slices.len()]));
        for _ in 0..workers {
            let slices = slices.clone();
            let cache = cache.clone();
            let errors = errors.clone();
            let generation = generation.clone();
            let target = target.clone();
            let ctx = ctx.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                loop {
                    if generation.load(Ordering::SeqCst) != 0 {
                        return;
                    }
                    // najbliższy jeszcze nie wczytany przekrój
                    let t = target.load(Ordering::Relaxed);
                    let next = {
                        let mut d = done.lock().unwrap();
                        let mut best: Option<usize> = None;
                        for (i, flag) in d.iter().enumerate() {
                            if !flag && best.is_none_or(|b| i.abs_diff(t) < b.abs_diff(t)) {
                                best = Some(i);
                            }
                        }
                        if let Some(b) = best {
                            d[b] = true;
                        }
                        best
                    };
                    let Some(i) = next else { return };
                    if let Err(e) = cache.get(&slices[i]) {
                        errors.lock().unwrap().insert(sid, format!("{e:#}"));
                    }
                    ctx.request_repaint();
                }
            });
        }
    }

    fn request_thumbnail(&mut self, series: &Series, ctx: &egui::Context) {
        if !self.thumbs_requested.insert(series.id) {
            return;
        }
        let slice = series.slices[series.slices.len() / 2].clone();
        let tx = self.thumb_tx.clone();
        let id = series.id;
        let spacing = series.pixel_spacing;
        let ctx = ctx.clone();
        rayon::spawn(move || {
            let Ok(frame) = imaging::load_frame(&slice.path, slice.frame) else {
                return;
            };
            let (c, w) = frame.default_window();
            let gray = imaging::render_gray8(&frame, c, w, false);
            let (tw, th) = imaging::target_size(frame.width, frame.height, spacing, 72);
            let small = imaging::resize_gray(&gray, frame.width, frame.height, tw, th);
            let img = ColorImage::from_gray([tw as usize, th as usize], &small);
            let _ = tx.send((id, img));
            ctx.request_repaint();
        });
    }

    fn loaded_count(&self, series: &Series) -> usize {
        series.slices.iter().filter(|s| self.cache.peek(s).is_some()).count()
    }

    // ------------------------------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        if ui
            .add_sized([ui.available_width(), 32.0], egui::Button::new("📂  Otwórz folder z badaniem…"))
            .clicked()
        {
            self.choose_folder(&ctx);
        }
        if let Some(f) = &self.folder {
            ui.small(f.display().to_string());
        }
        ui.separator();
        if self.scan_rx.is_some() {
            let (a, b) = self.scan_progress;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if b > 0 { format!("Czytam pliki… {a}/{b}") } else { "Szukam plików…".into() });
            });
            return;
        }
        let Some(lib) = self.lib.clone() else {
            ui.label("Kliknij przycisk powyżej i wskaż folder skopiowany z płyty (np. Pobrane/MRI). Możesz też przeciągnąć folder na to okno.");
            return;
        };
        if lib.studies.is_empty() {
            ui.colored_label(Color32::LIGHT_RED, "W tym folderze nie znaleziono obrazów DICOM.");
            return;
        }
        let mut clicked: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for st in &lib.studies {
                ui.add_space(4.0);
                let head = format!(
                    "{}  {}",
                    fmt_date(&st.date),
                    if st.description.is_empty() { "Badanie" } else { &st.description }
                );
                egui::CollapsingHeader::new(egui::RichText::new(head).strong())
                    .id_salt(&st.uid)
                    .default_open(true)
                    .show(ui, |ui| {
                        if !st.patient_name.is_empty() {
                            ui.small(format!(
                                "{}{}{}",
                                st.patient_name,
                                if st.age.is_empty() { String::new() } else { format!(", wiek {}", st.age.trim_start_matches('0')) },
                                if st.institution.is_empty() { String::new() } else { format!(" · {}", st.institution) }
                            ));
                        }
                        for se in &st.series {
                            self.request_thumbnail(se, &ctx);
                            let selected = self.viewports.iter().any(|v| v.series_id == Some(se.id));
                            let resp = ui
                                .horizontal(|ui| {
                                    let size = Vec2::splat(72.0);
                                    if let Some(t) = self.thumbs.get(&se.id) {
                                        let ts = t.size_vec2();
                                        let scale = (size.x / ts.x).min(size.y / ts.y);
                                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                                        ui.painter().rect_filled(rect, 2.0, Color32::BLACK);
                                        let r = Rect::from_center_size(rect.center(), ts * scale);
                                        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                                    } else {
                                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                                        ui.painter().rect_filled(rect, 2.0, Color32::from_gray(20));
                                    }
                                    ui.vertical(|ui| {
                                        ui.set_width(ui.available_width());
                                        let desc = if se.description.is_empty() { "(bez opisu)" } else { &se.description };
                                        let t = egui::RichText::new(format!(
                                            "{}{}",
                                            se.number.map(|n| format!("{n}. ")).unwrap_or_default(),
                                            desc
                                        ));
                                        ui.label(if selected { t.strong().color(Color32::from_rgb(120, 190, 255)) } else { t });
                                        ui.small(format!(
                                            "{} · {} {}",
                                            se.plane.pl(),
                                            se.slices.len(),
                                            library::plural_obraz(se.slices.len())
                                        ));
                                        if let Some(err) = self.errors.lock().unwrap().get(&se.id) {
                                            ui.small(egui::RichText::new(format!("⚠ {err}")).color(Color32::LIGHT_RED));
                                        }
                                    });
                                })
                                .response;
                            let resp = ui.interact(resp.rect, ui.scope_id().with(("series", se.id)), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_stroke(resp.rect, 3.0, Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Outside);
                            }
                            let resp = resp.on_hover_text(se.title());
                            if resp.clicked() {
                                clicked = Some((se.id, ui.input(|i| i.modifiers.shift)));
                            }
                            ui.add_space(2.0);
                        }
                    });
            }
            if lib.skipped_files > 0 {
                ui.add_space(8.0);
                ui.small(format!("Pominięto plików innych niż obrazy: {}", lib.skipped_files));
            }
        });
        if let Some((id, second)) = clicked {
            let target = if second && self.compare { 1 } else { self.active };
            self.assign_series(target, id, &ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Narzędzie:");
            ui.selectable_value(&mut self.tool, Tool::Window, "☀ Jasność/kontrast")
                .on_hover_text("Przeciągaj lewym przyciskiem: w poziomie kontrast, w pionie jasność");
            ui.selectable_value(&mut self.tool, Tool::Pan, "✋ Przesuwanie")
                .on_hover_text("Przeciągaj, aby przesunąć obraz (prawy przycisk działa zawsze)");
            ui.selectable_value(&mut self.tool, Tool::Measure, "📏 Pomiar")
                .on_hover_text("Kliknij dwa punkty, aby zmierzyć odległość w mm");
            ui.separator();
            let has_series = self.viewports[self.active].series_id.is_some();
            let vp = &mut self.viewports[self.active];
            if ui.add_enabled(has_series, egui::Button::new("↺ Reset")).on_hover_text("Przywróć jasność, kontrast i powiększenie (albo dwuklik na obrazie)").clicked() {
                vp.reset_view();
            }
            if ui.add_enabled(has_series, egui::Button::selectable(vp.invert, "◐ Negatyw")).clicked() {
                vp.invert = !vp.invert;
            }
            let play_label = if vp.playing { "⏸ Stop" } else { "▶ Odtwarzaj" };
            if ui.add_enabled(has_series, egui::Button::new(play_label)).on_hover_text("Automatyczne przewijanie przekrojów (spacja)").clicked() {
                vp.playing = !vp.playing;
            }
            if ui.add_enabled(has_series, egui::Button::new("🗑 Usuń pomiary")).clicked() {
                vp.measures.clear();
                vp.pending_point = None;
            }
            // płaszczyzna (rekonstrukcja)
            if let Some(series) = vp.series_id.and_then(|id| self.lib.as_ref()?.series_by_id(id).map(|(_, s)| s.clone())) {
                let planes = mpr::available_planes(&series);
                if !planes.is_empty() {
                    ui.separator();
                    let current = match vp.plane {
                        ViewPlane::Native => format!("oryginalna ({})", series.plane.pl()),
                        ViewPlane::Reformat(p) => format!("{} (rekonstrukcja)", p.pl()),
                    };
                    egui::ComboBox::from_id_salt("plane")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(vp.plane == ViewPlane::Native, format!("oryginalna ({})", series.plane.pl())).clicked() {
                                vp.plane = ViewPlane::Native;
                                vp.tex_key = None;
                            }
                            for p in planes {
                                if ui.selectable_label(vp.plane == ViewPlane::Reformat(p), format!("{} (rekonstrukcja)", p.pl())).clicked() {
                                    vp.plane = ViewPlane::Reformat(p);
                                    vp.reformat_pos = usize::MAX; // środek po wczytaniu
                                    vp.tex_key = None;
                                    vp.reformat_cache = None;
                                }
                            }
                        })
                        .response
                        .on_hover_text("Rekonstrukcja w innej płaszczyźnie z wczytanych przekrojów. Przy grubych warstwach obraz będzie rozmyty – to normalne.");
                }
            }
            ui.separator();
            if ui.checkbox(&mut self.compare, "Porównanie (2 okna)").on_hover_text("Dwa obrazy obok siebie, np. różne serie lub stare i nowe badanie. Kliknij okno, aby je wybrać, potem serię po lewej.").changed() {
                if self.compare && self.viewports.len() < 2 {
                    self.viewports.push(Viewport::new());
                    self.active = 1;
                } else if !self.compare {
                    self.viewports.truncate(1);
                    self.active = 0;
                }
            }
            if self.compare {
                ui.checkbox(&mut self.sync_scroll, "Przewijaj razem");
            }
            ui.separator();
            ui.checkbox(&mut self.show_info, "Opisy na obrazie");
            if ui.add_enabled(has_series, egui::Button::new("💾 Zapisz PNG")).on_hover_text("Zapisz bieżący obraz (np. żeby wysłać go Claude lub lekarzowi)").clicked() {
                self.export_png();
            }
            if ui.button("❓ Pomoc").clicked() {
                self.show_help = !self.show_help;
            }
        });
    }

    fn export_png(&mut self) {
        let Some(series) = self.viewports[self.active].series_id.and_then(|id| self.series(id)).cloned() else { return };
        let Some((frame, spacing)) = self.current_frame(self.active, &series) else { return };
        let vp = &self.viewports[self.active];
        let (c, w) = vp.window.unwrap_or(frame.default_window());
        let gray = imaging::render_gray8(&frame, c, w, vp.invert);
        let (tw, th) = imaging::target_size(frame.width, frame.height, Some(spacing), frame.width.max(frame.height).max(512));
        let scaled = imaging::resize_gray(&gray, frame.width, frame.height, tw, th);
        let name = format!(
            "{}_{}_{}.png",
            series.number.unwrap_or(series.id as i32),
            series.description.replace(|c: char| !c.is_alphanumeric(), "_"),
            vp.position() + 1
        );
        if let Some(path) = rfd::FileDialog::new().set_file_name(&name).add_filter("PNG", &["png"]).save_file() {
            match imaging::encode_png_gray(&scaled, tw, th).and_then(|png| Ok(std::fs::write(&path, png)?)) {
                Ok(()) => self.status = format!("Zapisano {}", path.display()),
                Err(e) => self.status = format!("Błąd zapisu: {e}"),
            }
        }
    }

    /// Bieżący obraz okna (przekrój albo rekonstrukcja) i odstęp pikseli (pion, poziom).
    fn current_frame(&mut self, vi: usize, series: &Series) -> Option<(Arc<Frame>, (f64, f64))> {
        let spacing = series.pixel_spacing.unwrap_or((1.0, 1.0));
        let vp = &mut self.viewports[vi];
        match vp.plane {
            ViewPlane::Native => {
                let s = series.slices.get(vp.slice)?;
                self.cache.peek(s).map(|f| (f, spacing))
            }
            ViewPlane::Reformat(plane) => {
                let mut frames = Vec::with_capacity(series.slices.len());
                for s in &series.slices {
                    frames.push(self.cache.peek(s)?);
                }
                let key_pos = vp.reformat_pos;
                if let Some((k, f, sp)) = &vp.reformat_cache {
                    if *k == (series.id, vp.plane_code(), key_pos) {
                        return Some((f.clone(), *sp));
                    }
                }
                let idx = if key_pos == usize::MAX { None } else { Some(key_pos) };
                match mpr::reformat(series, &frames, plane, idx) {
                    Ok(rf) => {
                        vp.reformat_positions = rf.positions;
                        if vp.reformat_pos == usize::MAX {
                            vp.reformat_pos = rf.positions / 2;
                        }
                        let f = Arc::new(rf.frame);
                        vp.reformat_cache = Some(((series.id, vp.plane_code(), vp.reformat_pos), f.clone(), rf.spacing));
                        Some((f, rf.spacing))
                    }
                    Err(e) => {
                        self.status = format!("Rekonstrukcja niemożliwa: {e}");
                        vp.plane = ViewPlane::Native;
                        None
                    }
                }
            }
        }
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui, vi: usize) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        if self.compare && vi == self.active {
            painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, Color32::from_rgb(80, 150, 255)), egui::StrokeKind::Inside);
        }
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            self.active = vi;
        }
        let Some(series) = self.viewports[vi].series_id.and_then(|id| self.series(id)).cloned() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.lib.is_some() { "Wybierz serię z listy po lewej" } else { "Otwórz folder z badaniem" },
                egui::FontId::proportional(18.0),
                Color32::GRAY,
            );
            return;
        };

        // --- wejście: kółko, klawisze, odtwarzanie -----------------------------------------
        let hovered = resp.hovered();
        let mut slice_delta: i64 = 0;
        let (zoom_delta, cmd) = ui.input(|i| (i.zoom_delta(), i.modifiers.command || i.modifiers.ctrl));
        if hovered {
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                if let egui::Event::MouseWheel { unit, delta, .. } = ev {
                    if cmd {
                        continue;
                    }
                    let d = if delta.y.abs() >= delta.x.abs() { delta.y } else { delta.x };
                    let steps = match unit {
                        egui::MouseWheelUnit::Point => {
                            let vp = &mut self.viewports[vi];
                            vp.scroll_accum += d;
                            let s = (vp.scroll_accum / 24.0).trunc();
                            vp.scroll_accum -= s * 24.0;
                            s as i64
                        }
                        egui::MouseWheelUnit::Line => d.signum() as i64 * (d.abs().round().max(1.0) as i64),
                        egui::MouseWheelUnit::Page => d.signum() as i64 * 10,
                    };
                    slice_delta -= steps;
                }
            }
            if zoom_delta != 1.0 {
                let vp = &mut self.viewports[vi];
                vp.zoom = (vp.zoom * zoom_delta).clamp(0.2, 20.0);
            }
        }
        if vi == self.active {
            ui.input(|i| {
                if i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::PageUp) {
                    slice_delta -= 1;
                }
                if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::PageDown) {
                    slice_delta += 1;
                }
                if i.key_pressed(Key::Home) {
                    slice_delta -= 100_000;
                }
                if i.key_pressed(Key::End) {
                    slice_delta += 100_000;
                }
            });
            if ui.input(|i| i.key_pressed(Key::Space)) {
                let vp = &mut self.viewports[vi];
                vp.playing = !vp.playing;
            }
            if ui.input(|i| i.key_pressed(Key::I)) {
                let vp = &mut self.viewports[vi];
                vp.invert = !vp.invert;
            }
            if ui.input(|i| i.key_pressed(Key::R)) {
                self.viewports[vi].reset_view();
            }
        }
        {
            let vp = &mut self.viewports[vi];
            if vp.playing {
                let dt = ui.input(|i| i.stable_dt) as f64;
                vp.play_accum += dt * 8.0; // ok. 8 obrazów na sekundę
                let n = vp.count(&series);
                while vp.play_accum >= 1.0 {
                    vp.play_accum -= 1.0;
                    let p = (vp.position() + 1) % n.max(1);
                    match vp.plane {
                        ViewPlane::Native => vp.slice = p,
                        ViewPlane::Reformat(_) => vp.reformat_pos = p,
                    }
                }
                ctx.request_repaint();
            }
        }
        if slice_delta != 0 {
            self.viewports[vi].step(&series, slice_delta);
            if self.compare && self.sync_scroll {
                for (oi, ov) in self.viewports.iter_mut().enumerate() {
                    if oi != vi {
                        if let Some(os) = ov.series_id.and_then(|id| self.lib.as_ref()?.series_by_id(id).map(|(_, s)| s.clone())) {
                            ov.step(&os, slice_delta);
                        }
                    }
                }
            }
        }
        if let Some(l) = self.loaders.get(&series.id) {
            if self.viewports[vi].plane == ViewPlane::Native {
                l.target.store(self.viewports[vi].slice, Ordering::Relaxed);
            }
        } else {
            self.ensure_loader(&series, &ctx);
        }

        // --- obraz ----------------------------------------------------------------------
        let Some((frame, spacing)) = self.current_frame(vi, &series) else {
            let loaded = self.loaded_count(&series);
            let msg = if let Some(e) = self.errors.lock().unwrap().get(&series.id) {
                format!("Nie udało się odczytać obrazu:\n{e}")
            } else {
                format!("Wczytywanie… {loaded}/{}", series.slices.len())
            };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(16.0), Color32::GRAY);
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            return;
        };
        let vp = &mut self.viewports[vi];
        let (dc, dw) = frame.default_window();
        let (mut wc, mut ww) = vp.window.unwrap_or((dc, dw));

        // przeciąganie
        let pan_drag = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (self.tool == Tool::Pan && resp.dragged_by(egui::PointerButton::Primary));
        if pan_drag {
            vp.pan += resp.drag_delta();
        } else if self.tool == Tool::Window && resp.dragged_by(egui::PointerButton::Primary) {
            let d = resp.drag_delta();
            let range = (frame.max - frame.min).max(1.0);
            let k = range / 600.0;
            ww = (ww + d.x * k).max(1.0);
            wc -= d.y * k;
            vp.window = Some((wc, ww));
        }
        if resp.double_clicked() {
            vp.reset_view();
            (wc, ww) = (dc, dw);
        }

        // tekstura
        let key = (series.id, vp.position(), vp.plane_code(), vp.slice, wc.to_bits() as i64, ww.to_bits() as i64, vp.invert);
        if vp.tex_key != Some(key) || vp.texture.is_none() {
            let rgba = imaging::render_rgba(&frame, wc, ww, vp.invert);
            let img = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &rgba);
            match &mut vp.texture {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => vp.texture = Some(ctx.load_texture(format!("vp{vi}"), img, TextureOptions::LINEAR)),
            }
            vp.tex_key = Some(key);
        }
        vp.shown = Some((frame.width, frame.height, spacing));
        vp.shown_range = (frame.min, frame.max);
        vp.shown_default_window = (dc, dw);

        // położenie obrazu na ekranie
        let phys = Vec2::new(frame.width as f32 * spacing.1 as f32, frame.height as f32 * spacing.0 as f32);
        let fit = (rect.width() / phys.x).min(rect.height() / phys.y);
        let size = phys * fit * vp.zoom;
        // powiększanie względem kursora
        if hovered && zoom_delta != 1.0 {
            if let Some(p) = resp.hover_pos() {
                let center = rect.center() + vp.pan;
                let rel = p - center;
                vp.pan += rel - rel * zoom_delta;
            }
        }
        let img_rect = Rect::from_center_size(rect.center() + vp.pan, size);
        if let Some(t) = &vp.texture {
            painter.image(t.id(), img_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let to_img = |p: Pos2| -> Pos2 {
            Pos2::new(
                (p.x - img_rect.min.x) / img_rect.width() * frame.width as f32,
                (p.y - img_rect.min.y) / img_rect.height() * frame.height as f32,
            )
        };
        let to_screen = |p: Pos2| -> Pos2 {
            Pos2::new(
                img_rect.min.x + p.x / frame.width as f32 * img_rect.width(),
                img_rect.min.y + p.y / frame.height as f32 * img_rect.height(),
            )
        };

        // pomiary
        let mkey = (series.id, vp.plane_code(), vp.position());
        if self.tool == Tool::Measure && resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let ip = to_img(p);
                match vp.pending_point.take() {
                    None => vp.pending_point = Some(ip),
                    Some(a) => vp.measures.entry(mkey).or_default().push((a, ip)),
                }
            }
        }
        let mm = |a: Pos2, b: Pos2| -> f64 {
            let dx = (b.x - a.x) as f64 * spacing.1;
            let dy = (b.y - a.y) as f64 * spacing.0;
            (dx * dx + dy * dy).sqrt()
        };
        let yellow = Color32::from_rgb(255, 210, 60);
        if let Some(list) = vp.measures.get(&mkey) {
            for &(a, b) in list {
                let (sa, sb) = (to_screen(a), to_screen(b));
                painter.line_segment([sa, sb], Stroke::new(2.0, yellow));
                painter.circle_filled(sa, 3.0, yellow);
                painter.circle_filled(sb, 3.0, yellow);
                painter.text(
                    sb + Vec2::new(6.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.1} mm", mm(a, b)),
                    egui::FontId::proportional(15.0),
                    yellow,
                );
            }
        }
        if let Some(a) = vp.pending_point {
            let sa = to_screen(a);
            painter.circle_filled(sa, 3.0, yellow);
            if let Some(h) = resp.hover_pos() {
                painter.line_segment([sa, h], Stroke::new(1.0, yellow));
                painter.text(h + Vec2::new(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("{:.1} mm", mm(a, to_img(h))), egui::FontId::proportional(14.0), yellow);
            }
        }

        // opisy w rogach
        if self.show_info {
            let font = egui::FontId::proportional(13.0);
            let col = Color32::from_rgb(230, 230, 230);
            let n = vp.count(&series);
            let mut tl = format!(
                "{}{}\n{}",
                series.number.map(|n| format!("Seria {n}: ")).unwrap_or_default(),
                if series.description.is_empty() { "(bez opisu)" } else { &series.description },
                match vp.plane {
                    ViewPlane::Native => format!("Płaszczyzna {}", series.plane.pl()),
                    ViewPlane::Reformat(p) => format!("Rekonstrukcja: {}", p.pl()),
                }
            );
            tl.push_str(&format!("\nObraz {} / {}", vp.position() + 1, n));
            if vp.plane == ViewPlane::Native {
                if let Some(l) = series.slices[vp.slice].slice_location {
                    tl.push_str(&format!("  ({l:.1} mm)"));
                }
            }
            painter.text(rect.min + Vec2::new(8.0, 8.0), egui::Align2::LEFT_TOP, tl, font.clone(), col);
            let tr = format!(
                "W: {:.0}  L: {:.0}{}\nPowiększenie {:.0}%",
                ww,
                wc,
                if vp.invert { "  (negatyw)" } else { "" },
                vp.zoom * 100.0
            );
            painter.text(Pos2::new(rect.max.x - 8.0, rect.min.y + 8.0), egui::Align2::RIGHT_TOP, tr, font.clone(), col);
            let q = &series.sequence;
            let mut bl = Vec::new();
            if let Some(t) = series.slice_thickness {
                bl.push(format!("Grubość warstwy {t:.1} mm"));
            }
            if let (Some(tr), Some(te)) = (q.repetition_time, q.echo_time) {
                bl.push(format!("TR {tr:.0} ms  TE {te:.0} ms"));
            }
            if let Some(b) = q.magnetic_field_strength {
                bl.push(format!("{b:.1} T"));
            }
            painter.text(Pos2::new(rect.min.x + 8.0, rect.max.y - 8.0), egui::Align2::LEFT_BOTTOM, bl.join("\n"), font.clone(), col);
            let loaded = self.cache.as_ref();
            let n_loaded = series.slices.iter().filter(|s| loaded.peek(s).is_some()).count();
            if n_loaded < series.slices.len() {
                painter.text(
                    Pos2::new(rect.max.x - 8.0, rect.max.y - 8.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("wczytano {n_loaded}/{}", series.slices.len()),
                    font,
                    Color32::GRAY,
                );
            }
        }
        // hover: wartość piksela
        if let Some(h) = resp.hover_pos() {
            let ip = to_img(h);
            if ip.x >= 0.0 && ip.y >= 0.0 && (ip.x as u32) < frame.width && (ip.y as u32) < frame.height {
                if let imaging::Pixels::Gray(v) = &frame.pixels {
                    let val = v[(ip.y as u32 * frame.width + ip.x as u32) as usize];
                    self.status = format!("Piksel ({}, {}) = {:.0}", ip.x as u32, ip.y as u32, val);
                }
            }
        }

        // suwak przekrojów na dole okna
        let n = vp.count(&series);
        if n > 1 {
            let bar = Rect::from_min_max(Pos2::new(rect.min.x + 200.0, rect.max.y - 26.0), Pos2::new(rect.max.x - 160.0, rect.max.y - 6.0));
            if bar.width() > 80.0 {
                let mut pos = vp.position() + 1;
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar));
                child.spacing_mut().slider_width = bar.width() - 60.0;
                if child.add(egui::Slider::new(&mut pos, 1..=n).show_value(true)).changed() {
                    let d = pos as i64 - 1 - vp.position() as i64;
                    vp.step(&series, d);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // komunikaty z wątków w tle
        if let Some(rx) = &self.scan_rx {
            let mut done = None;
            while let Ok(m) = rx.try_recv() {
                match m {
                    ScanMsg::Progress(a, b) => self.scan_progress = (a, b),
                    ScanMsg::Done(lib) => done = Some(lib),
                }
            }
            if let Some(lib) = done {
                self.scan_rx = None;
                let first = lib.all_series().max_by_key(|(_, s)| s.slices.len()).map(|(_, s)| s.id);
                self.status = format!(
                    "Znaleziono badań: {}, serii: {}",
                    lib.studies.len(),
                    lib.series_count()
                );
                self.lib = Some(Arc::new(lib));
                if let Some(id) = first {
                    self.assign_series(0, id, &ctx);
                }
            }
        }
        while let Ok((id, img)) = self.thumb_rx.try_recv() {
            let t = ctx.load_texture(format!("thumb{id}"), img, TextureOptions::LINEAR);
            self.thumbs.insert(id, t);
        }
        // przeciągnięty folder
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.into_iter().next() {
            let dir = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
            self.open_folder(dir, &ctx);
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::O)) {
            self.choose_folder(&ctx);
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small(egui::RichText::new("Narzędzie poglądowe – nie służy do diagnozy. Wiążący jest opis radiologa.").color(Color32::from_rgb(230, 170, 80)));
                ui.separator();
                ui.small(&self.status);
            });
        });
        egui::Panel::left("series").default_size(300.0).show(ui, |ui| {
            self.sidebar(ui);
        });
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let n = self.viewports.len();
            let w = full.width() / n as f32;
            for vi in 0..n {
                let r = Rect::from_min_size(Pos2::new(full.min.x + w * vi as f32, full.min.y), Vec2::new(w - if vi + 1 < n { 2.0 } else { 0.0 }, full.height()));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(("viewport", vi)));
                self.viewport_ui(&mut child, vi);
            }
        });

        if self.show_help {
            let mut open = true;
            egui::Window::new("Pomoc").open(&mut open).collapsible(false).show(&ctx, |ui| {
                ui.label(HELP);
            });
            self.show_help = open;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(f) = &self.folder {
            storage.set_string(STORAGE_LAST_FOLDER, f.display().to_string());
        }
    }
}

const HELP: &str = "\
Jak oglądać:
• Kółko myszy lub strzałki ↑ ↓ – kolejne przekroje (warstwy).
• Przeciąganie lewym przyciskiem (narzędzie „Jasność/kontrast”) – w poziomie kontrast, w pionie jasność.
• Prawy przycisk lub narzędzie „Przesuwanie” – przesuwanie obrazu.
• Ctrl/Cmd + kółko albo gest szczypania – powiększenie.
• Dwuklik albo klawisz R – reset widoku.  I – negatyw.  Spacja – odtwarzanie.
• „Pomiar” – kliknij dwa punkty, aby zmierzyć odległość w milimetrach.
• „Porównanie” – dwa okna obok siebie. Kliknij okno, a potem serię po lewej (Shift+klik wczytuje do drugiego okna).

Co to są serie? Rezonans robi kilka „serii”, czyli zestawów zdjęć w różnych ustawieniach (np. T1, T2, FLAIR) \
i płaszczyznach: poprzecznej (jak plastry od góry), strzałkowej (z boku) i czołowej (od przodu).

To narzędzie poglądowe. Nie służy do stawiania diagnozy – wiążący jest opis radiologa.";

pub fn run(folder: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Przeglądarka MRI")
            .with_inner_size([1280.0, 820.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Przeglądarka MRI",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new(cc, folder)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
