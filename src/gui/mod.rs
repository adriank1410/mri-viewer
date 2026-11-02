//! Okno programu (egui): lista serii, okna podglądu, menu, ustawienia, eksport.

mod actions;
mod annotations;
mod export;
mod settings;
mod viewport;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, ColorImage, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};

use crate::cache::FrameCache;
use crate::imaging::{self, Colormap, Frame, Pixels};
use crate::library::{self, Library, Series, Study, fmt_date};
use crate::mpr;
use actions::Action;
use annotations::Store;
use export::{JobHandle, RenderOpts};
use settings::{Settings, StartAt};
use viewport::{ImagePlane, Tool, ViewInput, ViewPlane, Viewport};

enum ScanMsg {
    Progress(usize, usize),
    Done(Library),
}

struct Loader {
    generation: Arc<AtomicUsize>,
    target: Arc<AtomicUsize>,
    alive: Arc<AtomicUsize>,
}

#[derive(Clone)]
enum ShotPurpose {
    Save(PathBuf),
    Clipboard,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InfoTab {
    Series,
    Tags,
    Histogram,
}

/// Gotowe ustawienia okna (szerokość, poziom) dla tomografii.
const CT_PRESETS: &[(&str, f32, f32)] = &[
    ("Mózg", 40.0, 80.0),
    ("Krwiak / tkanki mózgu", 75.0, 215.0),
    ("Tkanki miękkie", 40.0, 400.0),
    ("Śródpiersie", 50.0, 350.0),
    ("Wątroba", 60.0, 150.0),
    ("Płuca", -600.0, 1500.0),
    ("Kości", 400.0, 1800.0),
];

pub struct App {
    lib: Option<Arc<Library>>,
    folder: Option<PathBuf>,
    scan_rx: Option<Receiver<ScanMsg>>,
    scan_progress: (usize, usize),
    cache: Arc<FrameCache>,
    viewports: Vec<Viewport>,
    active: usize,
    tool: Tool,
    loaders: HashMap<usize, Loader>,
    thumbs: HashMap<usize, TextureHandle>,
    thumb_tx: Sender<(usize, ColorImage)>,
    thumb_rx: Receiver<(usize, ColorImage)>,
    thumbs_requested: HashSet<usize>,
    errors: Arc<Mutex<HashMap<usize, String>>>,
    settings: Settings,
    store: Store,
    cursor3d: Option<[f64; 3]>,
    show_settings: bool,
    show_help: bool,
    show_about: bool,
    show_connect: bool,
    connect_show_patient: bool,
    connect_msg: String,
    status: String,
    hover_status: String,
    job: JobHandle,
    shot_tx: Sender<(ShotPurpose, Rect, f32, Arc<ColorImage>)>,
    shot_rx: Receiver<(ShotPurpose, Rect, f32, Arc<ColorImage>)>,
    tags_cache: HashMap<PathBuf, Vec<(String, String, String)>>,
    tags_filter: String,
    info_tab: InfoTab,
    pending: Vec<Action>,
    settings_dirty: bool,
    #[cfg(target_os = "macos")]
    _native_menu: Option<actions::native::NativeMenu>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, settings::STORAGE_KEY))
            .unwrap_or_default();
        settings.apply(&cc.egui_ctx);
        // strzałki i trójkąty są tylko w czcionce Hack – używamy jej jako zapasowej
        let mut fonts = egui::FontDefinitions::default();
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            if !fam.iter().any(|f| f == "Hack") {
                fam.push("Hack".into());
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let (thumb_tx, thumb_rx) = channel();
        let (shot_tx, shot_rx) = channel();
        let mut app = Self {
            lib: None,
            folder: None,
            scan_rx: None,
            scan_progress: (0, 0),
            cache: Arc::new(FrameCache::new(settings.cache_mb.max(300) * 1_000_000)),
            viewports: vec![Viewport::new()],
            active: 0,
            tool: Tool::Window,
            loaders: HashMap::new(),
            thumbs: HashMap::new(),
            thumb_tx,
            thumb_rx,
            thumbs_requested: HashSet::new(),
            errors: Arc::new(Mutex::new(HashMap::new())),
            settings,
            store: Store::default(),
            cursor3d: None,
            show_settings: false,
            show_help: false,
            show_about: false,
            show_connect: false,
            connect_show_patient: false,
            connect_msg: String::new(),
            status: String::new(),
            hover_status: String::new(),
            job: Arc::new(Mutex::new(None)),
            shot_tx,
            shot_rx,
            tags_cache: HashMap::new(),
            tags_filter: String::new(),
            info_tab: InfoTab::Series,
            pending: Vec::new(),
            settings_dirty: false,
            #[cfg(target_os = "macos")]
            _native_menu: Some(actions::native::install()),
        };
        let start = folder.or_else(|| app.settings.last_folder.clone().map(PathBuf::from).filter(|p| p.is_dir()));
        if let Some(f) = start {
            app.open_folder(f, &cc.egui_ctx);
        }
        app
    }

    // --- wczytywanie --------------------------------------------------------------------------------

    fn open_folder(&mut self, folder: PathBuf, ctx: &egui::Context) {
        self.close_study();
        self.folder = Some(folder.clone());
        self.settings.add_recent(&folder.display().to_string());
        self.settings_dirty = true;
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

    fn close_study(&mut self) {
        for l in self.loaders.values() {
            l.generation.fetch_add(1, Ordering::SeqCst);
        }
        self.loaders.clear();
        self.cache.clear();
        self.thumbs.clear();
        self.thumbs_requested.clear();
        self.errors.lock().unwrap().clear();
        self.tags_cache.clear();
        let n = self.viewports.len();
        self.viewports = (0..n).map(|_| Viewport::new()).collect();
        self.active = 0;
        self.lib = None;
        self.folder = None;
        self.store.reset();
        self.cursor3d = None;
    }

    fn choose_folder(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new().set_title("Wybierz folder z badaniem (np. kopię płyty)");
        if let Some(f) = self.folder.clone().or_else(|| self.settings.last_folder.clone().map(PathBuf::from)) {
            dlg = dlg.set_directory(f);
        }
        if let Some(f) = dlg.pick_folder() {
            self.open_folder(f, ctx);
        }
    }

    fn choose_file(&mut self, ctx: &egui::Context) {
        if let Some(f) = rfd::FileDialog::new().set_title("Wybierz plik DICOM").pick_file() {
            // otwieramy cały folder, w którym leży plik – seria zwykle składa się z wielu plików
            if let Some(dir) = f.parent() {
                self.open_folder(dir.to_path_buf(), ctx);
            }
        }
    }

    fn series(&self, id: usize) -> Option<Series> {
        self.lib.as_ref()?.series_by_id(id).map(|(_, s)| s.clone())
    }

    fn study_of(&self, id: usize) -> Option<Study> {
        self.lib.as_ref()?.series_by_id(id).map(|(st, _)| st.clone())
    }

    fn assign_series(&mut self, vi: usize, id: usize, ctx: &egui::Context) {
        let Some(series) = self.series(id) else { return };
        let start = match self.settings.start_at {
            StartAt::Middle => series.slices.len() / 2,
            StartAt::First => 0,
        };
        let cmap = if self.settings.pro_mode { self.settings.default_colormap } else { Colormap::Gray };
        let vp = &mut self.viewports[vi];
        *vp = Viewport::new();
        vp.series_id = Some(id);
        vp.slice = start;
        vp.colormap = cmap;
        self.ensure_loader(&series, ctx);
    }

    fn ensure_loader(&mut self, series: &Series, ctx: &egui::Context) {
        if let Some(l) = self.loaders.get(&series.id) {
            if l.alive.load(Ordering::SeqCst) > 0 || series.slices.iter().all(|s| self.cache.peek(s).is_some()) {
                return;
            }
            // wątki skończyły, ale część obrazów wypadła z pamięci – wczytujemy ponownie
            self.loaders.remove(&series.id);
        }
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
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6);
        let alive = Arc::new(AtomicUsize::new(workers));
        self.loaders.insert(
            series.id,
            Loader { generation: generation.clone(), target: target.clone(), alive: alive.clone() },
        );
        let slices = series.slices.clone();
        let sid = series.id;
        let done: Arc<Mutex<Vec<bool>>> =
            Arc::new(Mutex::new(slices.iter().map(|s| self.cache.peek(s).is_some()).collect()));
        for _ in 0..workers {
            let slices = slices.clone();
            let cache = self.cache.clone();
            let errors = self.errors.clone();
            let generation = generation.clone();
            let target = target.clone();
            let ctx = ctx.clone();
            let done = done.clone();
            let alive = alive.clone();
            std::thread::spawn(move || {
                loop {
                    if generation.load(Ordering::SeqCst) != 0 {
                        break;
                    }
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
                    let Some(i) = next else { break };
                    if let Err(e) = cache.get(&slices[i]) {
                        errors.lock().unwrap().insert(sid, format!("{e:#}"));
                    }
                    ctx.request_repaint();
                }
                alive.fetch_sub(1, Ordering::SeqCst);
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
        let cache = self.cache.clone();
        rayon::spawn(move || {
            let Ok(frame) = cache.get(&slice) else { return };
            let (c, w) = frame.default_window();
            let gray = imaging::render_gray8(&frame, c, w, false);
            let (tw, th) = imaging::target_size(frame.width, frame.height, spacing, 80);
            let small = imaging::resize_gray(&gray, frame.width, frame.height, tw, th);
            let _ = tx.send((id, ColorImage::from_gray([tw as usize, th as usize], &small)));
            ctx.request_repaint();
        });
    }

    fn loaded_count(&self, series: &Series) -> usize {
        series.slices.iter().filter(|s| self.cache.peek(s).is_some()).count()
    }

    /// Bieżący obraz okna, odstęp pikseli (pion, poziom) i położenie płaszczyzny w ciele.
    fn current_frame(&mut self, vi: usize, series: &Series) -> Option<(Arc<Frame>, (f64, f64), Option<ImagePlane>)> {
        let spacing = series.pixel_spacing.unwrap_or((1.0, 1.0));
        let vp = &mut self.viewports[vi];
        match vp.plane {
            ViewPlane::Native => {
                let s = series.slices.get(vp.slice)?;
                let f = self.cache.peek(s)?;
                let plane = match (series.orientation, s.position) {
                    (Some(o), Some(p)) => Some(ImagePlane {
                        origin: p,
                        xdir: [o[0], o[1], o[2]],
                        ydir: [o[3], o[4], o[5]],
                        sx: spacing.1,
                        sy: spacing.0,
                        w: f.width,
                        h: f.height,
                    }),
                    _ => None,
                };
                Some((f, spacing, plane))
            }
            ViewPlane::Reformat(plane) => {
                if let Some((k, f, sp, pl)) = &vp.reformat_cache {
                    if *k == (series.id, vp.plane_code(), vp.reformat_pos) {
                        return Some((f.clone(), *sp, *pl));
                    }
                }
                let mut frames = Vec::with_capacity(series.slices.len());
                for s in &series.slices {
                    frames.push(self.cache.peek(s)?);
                }
                let idx = if vp.reformat_pos == usize::MAX { None } else { Some(vp.reformat_pos) };
                match mpr::reformat(series, &frames, plane, idx) {
                    Ok(rf) => {
                        vp.reformat_positions = rf.positions;
                        if vp.reformat_pos == usize::MAX {
                            vp.reformat_pos = rf.positions / 2;
                        }
                        let f = Arc::new(rf.frame);
                        let pl = rf.origin.map(|o| ImagePlane {
                            origin: o,
                            xdir: rf.xdir,
                            ydir: rf.ydir,
                            sx: rf.spacing.1,
                            sy: rf.spacing.0,
                            w: f.width,
                            h: f.height,
                        });
                        vp.reformat_cache = Some(((series.id, vp.plane_code(), vp.reformat_pos), f.clone(), rf.spacing, pl));
                        Some((f, rf.spacing, pl))
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

    fn active_series(&self) -> Option<Series> {
        self.viewports.get(self.active)?.series_id.and_then(|id| self.series(id))
    }

    fn set_layout(&mut self, n: usize, ctx: &egui::Context) {
        let old = self.viewports.len();
        if n > old {
            // nowe okna dostają kolejne serie (np. różne płaszczyzny), żeby od razu było co porównać
            // pierwszeństwo: serie diagnostyczne (nie lokalizatory) w płaszczyznach, których jeszcze nie widać
            let mut used: HashSet<usize> = self.viewports.iter().filter_map(|v| v.series_id).collect();
            let mut planes: Vec<library::Plane> =
                self.viewports.iter().filter_map(|v| v.series_id.and_then(|id| self.series(id))).map(|s| s.plane).collect();
            let lib = self.lib.clone();
            for k in old..n {
                self.viewports.push(Viewport::new());
                let pick = lib.as_ref().and_then(|l| {
                    l.all_series()
                        .filter(|(_, s)| !used.contains(&s.id) && s.slices.len() > 1)
                        .min_by_key(|(_, s)| (is_localizer(s), planes.contains(&s.plane), s.slices.len() <= 3, s.id))
                        .map(|(_, s)| (s.id, s.plane))
                });
                if let Some((id, plane)) = pick {
                    used.insert(id);
                    planes.push(plane);
                    self.assign_series(k, id, ctx);
                }
            }
        } else {
            self.viewports.truncate(n.max(1));
        }
        self.active = self.active.min(self.viewports.len() - 1);
    }

    // --- synchronizacja okien ------------------------------------------------------------------------

    /// Po zmianie przekroju w oknie `vi` ustawia pozostałe okna z równoległą płaszczyzną na to samo miejsce.
    fn sync_from(&mut self, vi: usize) {
        if self.viewports.len() < 2 || !self.settings.sync_scroll {
            return;
        }
        let Some(src_series) = self.viewports[vi].series_id.and_then(|id| self.series(id)) else { return };
        let Some((_, _, Some(src_plane))) = self.current_frame(vi, &src_series) else { return };
        let center = src_plane.to_patient(Pos2::new(src_plane.w as f32 / 2.0, src_plane.h as f32 / 2.0));
        let n_src = src_plane.normal();
        for oi in 0..self.viewports.len() {
            if oi == vi || self.viewports[oi].plane != ViewPlane::Native {
                continue;
            }
            let Some(s) = self.viewports[oi].series_id.and_then(|id| self.series(id)) else { continue };
            let Some(o) = s.orientation else { continue };
            let n = viewport::cross([o[0], o[1], o[2]], [o[3], o[4], o[5]]);
            if viewport::dot(n, n_src).abs() < 0.95 {
                continue;
            }
            if let Some(best) = nearest_slice(&s, center, n) {
                self.viewports[oi].set_position(&s, best as i64);
            }
        }
    }

    fn apply_cursor3d(&mut self, src: usize, p: [f64; 3]) {
        self.cursor3d = Some(p);
        for oi in 0..self.viewports.len() {
            if oi == src || self.viewports[oi].plane != ViewPlane::Native {
                continue;
            }
            let Some(s) = self.viewports[oi].series_id.and_then(|id| self.series(id)) else { continue };
            let Some(o) = s.orientation else { continue };
            let n = viewport::cross([o[0], o[1], o[2]], [o[3], o[4], o[5]]);
            if let Some(best) = nearest_slice(&s, p, n) {
                self.viewports[oi].set_position(&s, best as i64);
            }
        }
    }

    // --- polecenia ------------------------------------------------------------------------------------

    fn render_opts(&self, vi: usize) -> RenderOpts {
        let vp = &self.viewports[vi];
        RenderOpts { window: vp.window, invert: vp.invert, colormap: vp.colormap, xf: vp.xf }
    }

    fn request_view_shot(&mut self, ctx: &egui::Context, purpose: ShotPurpose) {
        let rect = self.viewports[self.active].rect;
        if !rect.is_positive() {
            return;
        }
        let tx = self.shot_tx.clone();
        let ppp = ctx.pixels_per_point();
        let ctx2 = ctx.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let _ = tx.send((purpose, rect, ppp, img));
            ctx2.request_repaint();
        })));
    }

    fn handle_shot(&mut self, purpose: ShotPurpose, rect: Rect, ppp: f32, img: Arc<ColorImage>) {
        let [w, h] = img.size;
        let x0 = ((rect.min.x * ppp).round() as usize).min(w);
        let y0 = ((rect.min.y * ppp).round() as usize).min(h);
        let x1 = ((rect.max.x * ppp).round() as usize).min(w);
        let y1 = ((rect.max.y * ppp).round() as usize).min(h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let mut out = image::RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let c = img.pixels[y * w + x];
                out.put_pixel((x - x0) as u32, (y - y0) as u32, image::Rgba([c.r(), c.g(), c.b(), 255]));
            }
        }
        match purpose {
            ShotPurpose::Save(path) => {
                let fmt = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) {
                    settings::ExportFormat::Jpeg
                } else {
                    settings::ExportFormat::Png
                };
                self.status = match export::save_image(&out, &path, fmt, self.settings.jpeg_quality) {
                    Ok(()) => format!("Zapisano {}", path.display()),
                    Err(e) => format!("Błąd zapisu: {e:#}"),
                };
            }
            ShotPurpose::Clipboard => {
                self.status = match export::copy_to_clipboard(&out) {
                    Ok(()) => "Skopiowano widok do schowka".into(),
                    Err(e) => format!("{e:#}"),
                };
            }
        }
    }

    fn save_dialog(&self, name: &str) -> Option<PathBuf> {
        let ext = self.settings.export_format.ext();
        rfd::FileDialog::new()
            .set_file_name(format!("{name}.{ext}"))
            .add_filter("Obraz", &[ext])
            .save_file()
    }

    fn handle_action(&mut self, a: Action, ctx: &egui::Context) {
        let series = self.active_series();
        let vi = self.active;
        match a {
            Action::OpenFolder => self.choose_folder(ctx),
            Action::OpenFile => self.choose_file(ctx),
            Action::CloseStudy => self.close_study(),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Settings => self.show_settings = !self.show_settings,
            Action::Help => self.show_help = !self.show_help,
            Action::About => self.show_about = true,
            Action::ConnectClaude => self.show_connect = true,
            Action::TogglePro => {
                self.settings.pro_mode = !self.settings.pro_mode;
                if !self.settings.pro_mode && self.tool.pro() {
                    self.tool = Tool::Window;
                }
                self.settings_dirty = true;
            }
            Action::ToggleOverlay => {
                self.settings.show_overlay = !self.settings.show_overlay;
                self.settings_dirty = true;
            }
            Action::ToggleReferenceLines => {
                self.settings.show_reference_lines = !self.settings.show_reference_lines;
                self.settings_dirty = true;
            }
            Action::ToggleSidebar => {
                self.settings.show_sidebar = !self.settings.show_sidebar;
                self.settings_dirty = true;
            }
            Action::ToggleInfoPanel => {
                self.settings.show_info_panel = !self.settings.show_info_panel;
                self.settings_dirty = true;
            }
            Action::ToggleFullscreen => {
                let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
            }
            Action::Layout1 => self.set_layout(1, ctx),
            Action::Layout2 => self.set_layout(2, ctx),
            Action::Layout4 => self.set_layout(4, ctx),
            Action::Undo => {
                if !self.store.undo() {
                    self.status = "Nie ma czego cofnąć".into();
                }
            }
            Action::Redo => {
                if !self.store.redo() {
                    self.status = "Nie ma czego ponowić".into();
                }
            }
            Action::DeleteSelected => {
                let vp = &mut self.viewports[vi];
                if let (Some(k), Some(id)) = (vp.key(), vp.selected.take()) {
                    self.store.delete(k, id);
                } else if vp.creating.take().is_some() {
                    vp.angle_stage = 0;
                } else {
                    self.status = "Najpierw kliknij pomiar, aby go zaznaczyć".into();
                }
            }
            Action::DeleteSliceAnnotations => {
                if let Some(k) = self.viewports[vi].key() {
                    self.store.clear_key(k);
                }
            }
            Action::DeleteAllAnnotations => self.store.clear_all(),
            Action::ToolWindow => self.tool = Tool::Window,
            Action::ToolPan => self.tool = Tool::Pan,
            Action::ToolZoom => self.tool = Tool::Zoom,
            Action::ToolLine => self.tool = Tool::Line,
            Action::ToolArrow => self.tool = Tool::Arrow,
            Action::ToolAngle | Action::ToolEllipse | Action::ToolRect | Action::ToolProbe | Action::ToolCursor3d => {
                if !self.settings.pro_mode {
                    self.settings.pro_mode = true;
                    self.settings_dirty = true;
                    self.status = "Włączono tryb profesjonalny".into();
                }
                self.tool = match a {
                    Action::ToolAngle => Tool::Angle,
                    Action::ToolEllipse => Tool::Ellipse,
                    Action::ToolRect => Tool::Rect,
                    Action::ToolProbe => Tool::Probe,
                    _ => Tool::Cursor3d,
                };
            }
            _ => {}
        }
        // polecenia dotyczące bieżącego obrazu
        let Some(series) = series else { return };
        let vp = &mut self.viewports[vi];
        match a {
            Action::ResetView => vp.reset_view(),
            Action::FitWindow => {
                vp.zoom = 1.0;
                vp.pan = Vec2::ZERO;
            }
            Action::Zoom100 => {
                // 1 piksel obrazu = 1 punkt ekranu
                if let Some((f, sp, _)) = self.current_frame(vi, &series) {
                    let vp = &mut self.viewports[vi];
                    let m = viewport::Mapping::new(vp.rect, &Viewport { zoom: 1.0, pan: Vec2::ZERO, xf: vp.xf, ..Viewport::new() }, f.width, f.height, sp);
                    vp.zoom = 1.0 / (m.scale * m.sx.min(m.sy)).max(1e-6);
                    vp.pan = Vec2::ZERO;
                }
            }
            Action::ZoomIn => vp.zoom = (vp.zoom * 1.25).min(40.0),
            Action::ZoomOut => vp.zoom = (vp.zoom / 1.25).max(0.1),
            Action::RotateRight => vp.xf.rot = (vp.xf.rot + 1) % 4,
            Action::RotateLeft => vp.xf.rot = (vp.xf.rot + 3) % 4,
            Action::FlipH => vp.xf.flip_h = !vp.xf.flip_h,
            Action::FlipV => vp.xf.flip_v = !vp.xf.flip_v,
            Action::Invert => vp.invert = !vp.invert,
            Action::PlayPause => vp.toggle_play(&series),
            Action::NextSlice | Action::PrevSlice | Action::FirstSlice | Action::LastSlice => {
                let d = match a {
                    Action::NextSlice => 1,
                    Action::PrevSlice => -1,
                    Action::FirstSlice => -1_000_000,
                    _ => 1_000_000,
                };
                vp.step(&series, d);
                vp.touched = true;
                self.sync_from(vi);
            }
            Action::PresetAuto => {
                if let Some((f, _, _)) = self.current_frame(vi, &series) {
                    self.viewports[vi].window = Some(f.auto_window);
                }
            }
            Action::PresetDicom => vp.window = None,
            Action::PresetFull => {
                if let Some((f, _, _)) = self.current_frame(vi, &series) {
                    self.viewports[vi].window = Some(((f.min + f.max) / 2.0, (f.max - f.min).max(1.0)));
                }
            }
            Action::SaveImage => {
                if let Some((f, sp, _)) = self.current_frame(vi, &series) {
                    let o = self.render_opts(vi);
                    let name = format!("{}_obraz{:03}", export::series_base_name(&series), self.viewports[vi].position() + 1);
                    if let Some(path) = self.save_dialog(&name) {
                        let img = export::render_image(&f, sp, &o, None);
                        self.status = match export::save_image(&img, &path, self.settings.export_format, self.settings.jpeg_quality) {
                            Ok(()) => format!("Zapisano {}", path.display()),
                            Err(e) => format!("Błąd zapisu: {e:#}"),
                        };
                    }
                }
            }
            Action::SaveView => {
                let name = format!("{}_widok{:03}", export::series_base_name(&series), vp.position() + 1);
                if let Some(path) = self.save_dialog(&name) {
                    self.request_view_shot(ctx, ShotPurpose::Save(path));
                }
            }
            Action::CopyView => self.request_view_shot(ctx, ShotPurpose::Clipboard),
            Action::ExportSeries => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Wybierz folder na obrazy serii").pick_folder() {
                    let o = self.render_opts(vi);
                    export::export_series(series, self.cache.clone(), dir, o, self.settings.export_format, self.settings.jpeg_quality, self.job.clone());
                }
            }
            Action::ExportGif => {
                let name = format!("{}.gif", export::series_base_name(&series));
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("GIF", &["gif"]).save_file() {
                    let o = self.render_opts(vi);
                    export::export_gif(series, self.cache.clone(), path, o, self.settings.cine_fps, self.job.clone());
                }
            }
            Action::ExportMontage => {
                let name = format!("{}_mozaika.png", export::series_base_name(&series));
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("PNG", &["png"]).save_file() {
                    let o = self.render_opts(vi);
                    export::export_montage(series, self.cache.clone(), path, o, self.job.clone());
                }
            }
            Action::ExportTags => {
                let idx = if vp.plane == ViewPlane::Native { vp.slice } else { series.slices.len() / 2 };
                let name = format!("{}_tagi.txt", export::series_base_name(&series));
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("Tekst", &["txt"]).save_file() {
                    self.status = match imaging::dump_tags(&series.slices[idx].path, self.settings.hide_patient)
                        .and_then(|t| Ok(std::fs::write(&path, t)?))
                    {
                        Ok(()) => format!("Zapisano {}", path.display()),
                        Err(e) => format!("Błąd zapisu: {e:#}"),
                    };
                }
            }
            _ => {}
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let mut acts = Vec::new();
        // skróty z modyfikatorami (na macOS obsługuje je menu systemowe)
        if !cfg!(target_os = "macos") {
            for a in actions::ALL {
                if let Some(s) = a.shortcut() {
                    if ctx.input_mut(|i| i.consume_shortcut(&s)) {
                        acts.push(*a);
                    }
                }
            }
        }
        if !ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                if i.modifiers.any() && !i.modifiers.shift_only() {
                    return;
                }
                for a in actions::ALL {
                    if let Some(k) = a.plain_key() {
                        if i.key_pressed(k) && !i.modifiers.shift {
                            acts.push(*a);
                        }
                    }
                }
                if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowRight) {
                    acts.push(Action::NextSlice);
                }
                if i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowLeft) {
                    acts.push(Action::PrevSlice);
                }
                if i.key_pressed(Key::PageDown) {
                    for _ in 0..10 {
                        acts.push(Action::NextSlice);
                    }
                }
                if i.key_pressed(Key::PageUp) {
                    for _ in 0..10 {
                        acts.push(Action::PrevSlice);
                    }
                }
                if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                    acts.push(Action::DeleteSelected);
                }
            });
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                let vp = &mut self.viewports[self.active];
                vp.creating = None;
                vp.angle_stage = 0;
                vp.selected = None;
                self.cursor3d = None;
            }
        }
        self.pending.extend(acts);
    }

    // --- panele ---------------------------------------------------------------------------------------

    fn patient_line(&self, st: &Study) -> String {
        if self.settings.hide_patient {
            let mut s = String::from("Pacjent ukryty");
            if !st.sex.is_empty() || !st.age.is_empty() {
                s.push_str(&format!(" ({} {})", st.sex, st.age.trim_start_matches('0')));
            }
            s
        } else if st.patient_name.is_empty() {
            String::new()
        } else {
            let mut s = st.patient_name.clone();
            if !st.age.is_empty() {
                s.push_str(&format!(", {}", st.age.trim_start_matches('0')));
            }
            s
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("Otwórz folder z badaniem…")).clicked() {
            self.pending.push(Action::OpenFolder);
        }
        if self.scan_rx.is_some() {
            let (a, b) = self.scan_progress;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if b > 0 { format!("Czytam pliki… {a}/{b}") } else { "Szukam plików…".into() });
            });
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        ui.separator();
        if lib.studies.is_empty() {
            ui.colored_label(Color32::LIGHT_RED, "W tym folderze nie znaleziono obrazów DICOM.");
            ui.small("Upewnij się, że wskazujesz folder skopiowany z płyty, a nie np. sam program do przeglądania z płyty.");
            return;
        }
        let mut clicked: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for st in &lib.studies {
                let head = format!(
                    "{}  {}",
                    fmt_date(&st.date),
                    if st.description.is_empty() { "Badanie" } else { &st.description }
                );
                egui::CollapsingHeader::new(egui::RichText::new(head).strong())
                    .id_salt(&st.uid)
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut info = self.patient_line(st);
                        if !self.settings.hide_patient && !st.institution.is_empty() {
                            info.push_str(&format!(" · {}", st.institution));
                        }
                        if !info.is_empty() {
                            ui.small(info);
                        }
                        for se in &st.series {
                            self.request_thumbnail(se, &ctx);
                            let in_views: Vec<usize> = self
                                .viewports
                                .iter()
                                .enumerate()
                                .filter(|(_, v)| v.series_id == Some(se.id))
                                .map(|(i, _)| i)
                                .collect();
                            let resp = ui
                                .horizontal(|ui| {
                                    let size = Vec2::splat(64.0);
                                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                                    ui.painter().rect_filled(rect, 3.0, Color32::BLACK);
                                    if let Some(t) = self.thumbs.get(&se.id) {
                                        let ts = t.size_vec2();
                                        let scale = (size.x / ts.x).min(size.y / ts.y);
                                        let r = Rect::from_center_size(rect.center(), ts * scale);
                                        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                                    }
                                    for (k, vi) in in_views.iter().enumerate() {
                                        let c = viewport::VIEW_COLORS[vi % viewport::VIEW_COLORS.len()];
                                        ui.painter().circle_filled(rect.min + Vec2::new(8.0 + k as f32 * 10.0, 8.0), 4.0, c);
                                    }
                                    ui.vertical(|ui| {
                                        let desc = if se.description.is_empty() { "(bez opisu)" } else { &se.description };
                                        let t = egui::RichText::new(format!(
                                            "{}{}",
                                            se.number.map(|n| format!("{n}. ")).unwrap_or_default(),
                                            desc
                                        ));
                                        ui.label(if in_views.is_empty() { t } else { t.strong() });
                                        ui.small(format!(
                                            "{} · {} {}",
                                            se.plane.pl(),
                                            se.slices.len(),
                                            library::plural_obraz(se.slices.len())
                                        ));
                                        if let Some(err) = self.errors.lock().unwrap().get(&se.id) {
                                            ui.small(egui::RichText::new(format!("Błąd: {err}")).color(Color32::LIGHT_RED));
                                        }
                                    });
                                })
                                .response;
                            let resp = ui.interact(resp.rect, ui.scope_id().with(("series", se.id)), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_stroke(resp.rect.expand(2.0), 4.0, Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Outside);
                            }
                            let resp = resp.on_hover_text(format!(
                                "{}\nKliknij, aby otworzyć w zaznaczonym oknie.\nShift + klik: otwórz w następnym oknie.",
                                se.title()
                            ));
                            if resp.clicked() {
                                clicked = Some((se.id, ui.input(|i| i.modifiers.shift)));
                            }
                            ui.add_space(3.0);
                        }
                    });
            }
            if lib.skipped_files > 0 {
                ui.add_space(6.0);
                ui.small(format!("Pominięto plików innych niż obrazy: {}", lib.skipped_files));
            }
            ui.add_space(24.0);
        });
        if let Some((id, shift)) = clicked {
            let mut target = self.active;
            if shift {
                if self.viewports.len() < 2 {
                    self.set_layout(2, &ctx);
                }
                target = (self.active + 1) % self.viewports.len();
            }
            self.assign_series(target, id, &ctx);
            self.active = target;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let pro = self.settings.pro_mode;
        ui.horizontal_wrapped(|ui| {
            let tools: &[(Tool, Action)] = &[
                (Tool::Window, Action::ToolWindow),
                (Tool::Pan, Action::ToolPan),
                (Tool::Zoom, Action::ToolZoom),
                (Tool::Line, Action::ToolLine),
                (Tool::Arrow, Action::ToolArrow),
                (Tool::Angle, Action::ToolAngle),
                (Tool::Ellipse, Action::ToolEllipse),
                (Tool::Rect, Action::ToolRect),
                (Tool::Probe, Action::ToolProbe),
                (Tool::Cursor3d, Action::ToolCursor3d),
            ];
            for (t, _) in tools {
                if t.pro() && !pro {
                    continue;
                }
                ui.selectable_value(&mut self.tool, *t, t.label()).on_hover_text(t.hint());
            }
            ui.separator();
            let has = self.viewports[self.active].series_id.is_some();
            ui.add_enabled_ui(has, |ui| {
                if ui.button("Reset").on_hover_text("Przywróć jasność, kontrast, powiększenie i obrót (R lub dwuklik)").clicked() {
                    self.pending.push(Action::ResetView);
                }
                let inv = self.viewports[self.active].invert;
                if ui.selectable_label(inv, "Negatyw").on_hover_text("Odwróć kolory (I)").clicked() {
                    self.pending.push(Action::Invert);
                }
                if ui.button("Obróć").on_hover_text("Obróć o 90° w prawo").clicked() {
                    self.pending.push(Action::RotateRight);
                }
                self.window_menu(ui);
                self.plane_menu(ui);
            });
            ui.separator();
            ui.label("Układ:");
            let n = self.viewports.len();
            let ctx = ui.ctx().clone();
            for (k, label, tip) in [(1, "1", "Jedno okno"), (2, "2", "Dwa okna obok siebie"), (4, "4", "Cztery okna")] {
                if ui.selectable_label(n == k, label).on_hover_text(tip).clicked() {
                    self.set_layout(k, &ctx);
                }
            }
            if n > 1 && ui.checkbox(&mut self.settings.sync_scroll, "Razem").on_hover_text("Przewijaj okna razem (te same miejsca w ciele)").changed() {
                self.settings_dirty = true;
            }
            ui.separator();
            if ui.selectable_label(pro, "Tryb PRO").on_hover_text("Tryb profesjonalny: więcej narzędzi i informacji").clicked() {
                self.pending.push(Action::TogglePro);
            }
            if ui.button("Ustawienia").clicked() {
                self.pending.push(Action::Settings);
            }
        });
    }

    fn window_menu(&mut self, ui: &mut egui::Ui) {
        let vi = self.active;
        let Some(series) = self.active_series() else { return };
        let frame = self.current_frame(vi, &series).map(|(f, _, _)| f);
        let pro = self.settings.pro_mode;
        ui.menu_button("Jasność ▾", |ui| {
            if ui.button("Automatyczna").on_hover_text("Dobrana do rozkładu jasności obrazu").clicked() {
                self.pending.push(Action::PresetAuto);
                ui.close();
            }
            if let Some(f) = &frame {
                for (c, w, expl) in &f.dicom_windows {
                    let label = if expl.is_empty() { format!("Z aparatu (W {w:.0} / L {c:.0})") } else { format!("Z aparatu: {expl}") };
                    if ui.button(label).clicked() {
                        self.viewports[vi].window = Some((*c, *w));
                        ui.close();
                    }
                }
                let (ac, aw) = f.auto_window;
                if ui.button("Większy kontrast").clicked() {
                    self.viewports[vi].window = Some((ac, aw * 0.6));
                    ui.close();
                }
                if ui.button("Mniejszy kontrast").clicked() {
                    self.viewports[vi].window = Some((ac, aw * 1.6));
                    ui.close();
                }
                if ui.button("Pełny zakres").clicked() {
                    self.pending.push(Action::PresetFull);
                    ui.close();
                }
                if series.modality.eq_ignore_ascii_case("CT") {
                    ui.separator();
                    for (name, c, w) in CT_PRESETS {
                        if ui.button(format!("{name} (W {w:.0} / L {c:.0})")).clicked() {
                            self.viewports[vi].window = Some((*c, *w));
                            ui.close();
                        }
                    }
                }
                if pro {
                    ui.separator();
                    let (mut c, mut w) = self.viewports[vi].window.unwrap_or(f.default_window());
                    let step = ((f.max - f.min) / 500.0).max(0.5);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("Szerokość (W):");
                        changed |= ui.add(egui::DragValue::new(&mut w).speed(step).range(1.0..=f32::MAX)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Poziom (L):");
                        changed |= ui.add(egui::DragValue::new(&mut c).speed(step)).changed();
                    });
                    if changed {
                        self.viewports[vi].window = Some((c, w));
                    }
                    ui.separator();
                    ui.label("Mapa kolorów:");
                    for cm in Colormap::ALL {
                        let sel = self.viewports[vi].colormap == cm;
                        if ui.selectable_label(sel, cm.label()).clicked() {
                            self.viewports[vi].colormap = cm;
                        }
                    }
                }
            }
        });
    }

    fn plane_menu(&mut self, ui: &mut egui::Ui) {
        let vi = self.active;
        let Some(series) = self.active_series() else { return };
        let planes = mpr::available_planes(&series);
        if planes.is_empty() {
            return;
        }
        let vp = &mut self.viewports[vi];
        let current = match vp.plane {
            ViewPlane::Native => format!("Płaszczyzna: {}", series.plane.pl()),
            ViewPlane::Reformat(p) => format!("Płaszczyzna: {} (rekonstr.)", p.pl()),
        };
        egui::ComboBox::from_id_salt("plane")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui.selectable_label(vp.plane == ViewPlane::Native, format!("oryginalna ({})", series.plane.pl())).clicked() {
                    vp.plane = ViewPlane::Native;
                    vp.tex_key = None;
                }
                for p in planes {
                    if ui.selectable_label(vp.plane == ViewPlane::Reformat(p), format!("{} – rekonstrukcja", p.pl())).clicked() {
                        vp.plane = ViewPlane::Reformat(p);
                        vp.reformat_pos = usize::MAX;
                        vp.tex_key = None;
                        vp.reformat_cache = None;
                    }
                }
            })
            .response
            .on_hover_text("Rekonstrukcja w innej płaszczyźnie z wczytanych przekrojów. Przy grubych warstwach obraz jest rozmyty – to normalne. Najlepiej działa dla serii 3D.");
    }

    fn info_panel(&mut self, ui: &mut egui::Ui) {
        let vi = self.active;
        let Some(series) = self.active_series() else {
            ui.label("Wybierz serię.");
            return;
        };
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.info_tab, InfoTab::Series, "Seria");
            ui.selectable_value(&mut self.info_tab, InfoTab::Tags, "Tagi DICOM");
            ui.selectable_value(&mut self.info_tab, InfoTab::Histogram, "Histogram");
        });
        ui.separator();
        match self.info_tab {
            InfoTab::Series => {
                let q = &series.sequence;
                let fmt = |v: Option<f64>, u: &str| v.map(|x| format!("{x:.2} {u}").replace(".00 ", " ")).unwrap_or_else(|| "—".into());
                let st = self.study_of(series.id);
                let rows: Vec<(&str, String)> = vec![
                    ("Badanie", st.as_ref().map(|s| format!("{} {}", fmt_date(&s.date), s.description)).unwrap_or_default()),
                    ("Seria", format!("{} {}", series.number.map(|n| n.to_string()).unwrap_or_default(), series.description)),
                    ("Modalność", series.modality.clone()),
                    ("Okolica", series.body_part.clone()),
                    ("Protokół", q.protocol_name.clone()),
                    ("Płaszczyzna", series.plane.pl().to_string()),
                    ("Liczba obrazów", series.slices.len().to_string()),
                    ("Matryca", format!("{} × {}", series.columns, series.rows)),
                    ("Piksel", series.pixel_spacing.map(|(r, c)| format!("{c:.3} × {r:.3} mm")).unwrap_or("—".into())),
                    ("Grubość warstwy", fmt(series.slice_thickness, "mm")),
                    ("Odstęp warstw", fmt(series.slice_gap(), "mm")),
                    ("TR", fmt(q.repetition_time, "ms")),
                    ("TE", fmt(q.echo_time, "ms")),
                    ("TI", fmt(q.inversion_time, "ms")),
                    ("Kąt odchylenia", fmt(q.flip_angle, "°")),
                    ("Pole", fmt(q.magnetic_field_strength, "T")),
                    ("Sekwencja", format!("{} {}", q.scanning_sequence, q.sequence_name)),
                    ("Kontrast", if q.contrast_agent.is_empty() { "—".into() } else { q.contrast_agent.clone() }),
                    ("Typ obrazu", q.image_type.clone()),
                ];
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("series_info").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(egui::RichText::new(k).weak());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                    ui.small("TR/TE – czasy sekwencji; krótkie TR i TE to zwykle obrazy T1-zależne, długie – T2-zależne. STIR/FLAIR tłumią sygnał tłuszczu lub płynu.");
                });
            }
            InfoTab::Tags => {
                let vp = &self.viewports[vi];
                let idx = if vp.plane == ViewPlane::Native { vp.slice } else { series.slices.len() / 2 };
                let path = series.slices[idx].path.clone();
                let hide = self.settings.hide_patient;
                let rows = self.tags_cache.entry(path.clone()).or_insert_with(|| {
                    imaging::dump_tags(&path, false)
                        .unwrap_or_default()
                        .lines()
                        .filter_map(|l| {
                            let (tag, rest) = l.split_once(' ')?;
                            let (name, val) = rest.split_once(": ").unwrap_or((rest, ""));
                            Some((tag.to_string(), name.to_string(), val.to_string()))
                        })
                        .collect()
                });
                ui.horizontal(|ui| {
                    ui.label("Szukaj:");
                    ui.text_edit_singleline(&mut self.tags_filter);
                });
                ui.small(format!("Obraz {} · {}", idx + 1, path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()));
                let filt = self.tags_filter.to_lowercase();
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("tags").num_columns(3).striped(true).show(ui, |ui| {
                        for (tag, name, val) in rows.iter() {
                            if !filt.is_empty() && !format!("{tag} {name} {val}").to_lowercase().contains(&filt) {
                                continue;
                            }
                            let t = dicom_tag_parse(tag);
                            let hidden = hide && t.is_some_and(imaging::is_identifying);
                            ui.monospace(egui::RichText::new(tag).weak());
                            ui.label(name);
                            ui.label(if hidden { "[ukryte]" } else { val });
                            ui.end_row();
                        }
                    });
                });
            }
            InfoTab::Histogram => {
                let Some((f, _, _)) = self.current_frame(vi, &series) else { return };
                let Pixels::Gray(v) = &f.pixels else {
                    ui.label("Obraz kolorowy – histogram niedostępny.");
                    return;
                };
                const BINS: usize = 128;
                let range = (f.max - f.min).max(1.0);
                let mut hist = [0u32; BINS];
                for &p in v.iter() {
                    hist[(((p - f.min) / range) * (BINS - 1) as f32) as usize] += 1;
                }
                hist[0] = 0; // tło
                let maxc = *hist.iter().max().unwrap_or(&1) as f32;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 160.0), Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 2.0, Color32::from_gray(20));
                let bw = rect.width() / BINS as f32;
                for (i, &c) in hist.iter().enumerate() {
                    let h = (c as f32 / maxc.max(1.0)).sqrt() * rect.height();
                    p.rect_filled(
                        Rect::from_min_max(Pos2::new(rect.min.x + i as f32 * bw, rect.max.y - h), Pos2::new(rect.min.x + (i + 1) as f32 * bw, rect.max.y)),
                        0.0,
                        Color32::from_gray(170),
                    );
                }
                let (c, w) = self.viewports[vi].window.unwrap_or(f.default_window());
                let x = |val: f32| rect.min.x + ((val - f.min) / range).clamp(0.0, 1.0) * rect.width();
                let col = Color32::from_rgb(80, 170, 255);
                p.line_segment([Pos2::new(x(c - w / 2.0), rect.max.y), Pos2::new(x(c + w / 2.0), rect.min.y)], Stroke::new(1.5, col));
                p.rect_filled(Rect::from_min_max(Pos2::new(x(c - w / 2.0), rect.min.y), Pos2::new(x(c + w / 2.0), rect.max.y)), 0.0, col.gamma_multiply(0.15));
                ui.small(format!("Zakres wartości {:.0} … {:.0}. Niebieski obszar to bieżące okno (W {w:.0}, L {c:.0}).", f.min, f.max));
            }
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(20.0));
            ui.heading(egui::RichText::new("Przeglądarka MRI").size(28.0));
            ui.add_space(6.0);
            ui.label("Prosty podgląd badań rezonansu, tomografii i innych obrazów DICOM.");
            ui.add_space(18.0);
            if ui.add_sized([300.0, 44.0], egui::Button::new(egui::RichText::new("Otwórz folder z badaniem…").size(17.0))).clicked() {
                self.pending.push(Action::OpenFolder);
            }
            ui.add_space(6.0);
            ui.small("Wskaż folder skopiowany z płyty albo przeciągnij go na to okno.");
            if !self.settings.recent.is_empty() {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Ostatnio otwierane").strong());
                let ctx = ui.ctx().clone();
                for r in self.settings.recent.clone() {
                    let exists = std::path::Path::new(&r).is_dir();
                    if ui.add_enabled(exists, egui::Link::new(&r)).clicked() {
                        self.open_folder(PathBuf::from(r), &ctx);
                    }
                }
            }
            ui.add_space(24.0);
            ui.small(egui::RichText::new("Narzędzie poglądowe – nie służy do diagnozy. Wiążący jest opis radiologa.").color(Color32::from_rgb(230, 170, 80)));
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.show_settings {
            let mut open = true;
            egui::Window::new("Ustawienia").pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                if self.settings.ui(ui) {
                    self.settings.apply(ctx);
                    self.settings_dirty = true;
                    if !self.settings.pro_mode && self.tool.pro() {
                        self.tool = Tool::Window;
                    }
                    for vp in &mut self.viewports {
                        vp.tex_key = None;
                    }
                }
            });
            self.show_settings = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Pomoc").pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).default_width(560.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| help_ui(ui));
            });
            self.show_help = open;
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new("O programie").pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.heading("Przeglądarka MRI");
                ui.label(format!("Wersja {}", env!("CARGO_PKG_VERSION")));
                ui.label("Prosty podgląd badań DICOM z serwerem MCP dla Claude.");
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(230, 170, 80), "Narzędzie poglądowe – nie służy do diagnozy.");
                ui.hyperlink_to("Kod źródłowy", "https://github.com/adriank1410/mri-viewer");
            });
            self.show_about = open;
        }
        if self.show_connect {
            let mut open = true;
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mri-viewer"));
            let args = export::mcp_args(self.folder.as_deref(), self.connect_show_patient);
            egui::Window::new("Podłącz do Claude (MCP)").pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).default_width(620.0).show(ctx, |ui| {
                ui.label("Dzięki temu Claude (w aplikacji Claude Desktop albo w Claude Code) może sam przeglądać serie z otwartego folderu, oglądać przekroje i mierzyć.");
                if self.folder.is_none() {
                    ui.colored_label(Color32::LIGHT_RED, "Najpierw otwórz folder z badaniem – jego ścieżka trafi do konfiguracji.");
                }
                if exe.to_string_lossy().contains("AppTranslocation") {
                    ui.colored_label(Color32::LIGHT_RED, "Program działa z tymczasowej lokalizacji. Przenieś „MRI Viewer” do folderu Programy i uruchom go ponownie.");
                }
                ui.checkbox(&mut self.connect_show_patient, "Przekazuj Claude dane osobowe pacjenta (imię, nazwisko, PESEL, placówkę)")
                    .on_hover_text("Domyślnie wyłączone: Claude widzi tylko płeć i wiek.");
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Claude Desktop").strong());
                if ui.button("Dodaj automatycznie do Claude Desktop").clicked() {
                    self.connect_msg = match export::install_into_claude_desktop(&exe, &args) {
                        Ok(p) => format!("Gotowe. Zapisano {} (kopia poprzedniej wersji: .json.bak). Zamknij i uruchom ponownie Claude Desktop.", p.display()),
                        Err(e) => format!("Nie udało się: {e:#}"),
                    };
                }
                if !self.connect_msg.is_empty() {
                    ui.label(&self.connect_msg);
                }
                ui.small("Albo wklej ręcznie w Claude Desktop → Ustawienia → Developer → Edit Config:");
                let snippet = export::desktop_snippet(&exe, &args);
                ui.add(egui::TextEdit::multiline(&mut snippet.as_str()).code_editor().desired_rows(8).desired_width(f32::INFINITY));
                if ui.button("Kopiuj").clicked() {
                    ui.ctx().copy_text(snippet.clone());
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Claude Code (terminal)").strong());
                let cmd = export::claude_code_command(&exe, &args);
                ui.add(egui::TextEdit::multiline(&mut cmd.as_str()).code_editor().desired_rows(2).desired_width(f32::INFINITY));
                if ui.button("Kopiuj polecenie").clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
                ui.add_space(6.0);
                ui.small("Obrazy oglądane przez Claude są wysyłane do Anthropic jak każda treść rozmowy. Claude może się mylić – to nie zastępuje opisu radiologa.");
            });
            self.show_connect = open;
        }
    }
}

fn dicom_tag_parse(s: &str) -> Option<dicom_object::Tag> {
    let s = s.trim_start_matches('(').trim_end_matches(')');
    let (g, e) = s.split_once(',')?;
    Some(dicom_object::Tag(u16::from_str_radix(g, 16).ok()?, u16::from_str_radix(e, 16).ok()?))
}

fn nearest_slice(s: &Series, p: [f64; 3], n: [f64; 3]) -> Option<usize> {
    let target = viewport::dot(p, n);
    s.slices
        .iter()
        .enumerate()
        .filter_map(|(i, sl)| sl.position.map(|q| (i, (viewport::dot(q, n) - target).abs())))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(i, _)| i)
}

fn help_ui(ui: &mut egui::Ui) {
    ui.heading("Jak oglądać");
    ui.label("1. Otwórz folder z badaniem (Plik → Otwórz folder albo przeciągnij folder na okno).");
    ui.label("2. Po lewej wybierz serię. Seria to zestaw zdjęć zrobionych w jednym ustawieniu aparatu (np. T1, T2, STIR) i jednej płaszczyźnie.");
    ui.label("3. Przewijaj przekroje kółkiem myszy, strzałkami albo suwakiem na dole obrazu.");
    ui.add_space(6.0);
    ui.heading("Płaszczyzny");
    ui.label("Poprzeczna – plastry oglądane od strony stóp (prawa strona pacjenta jest po lewej stronie ekranu). Strzałkowa – widok z boku. Czołowa – widok od przodu. Litery na krawędziach: R/L – prawa/lewa, A/P – przód/tył, S/I – góra/dół ciała.");
    ui.add_space(6.0);
    ui.heading("Mysz");
    egui::Grid::new("mouse").num_columns(2).striped(true).show(ui, |ui| {
        for (k, v) in [
            ("Kółko", "kolejne przekroje"),
            ("Cmd/Ctrl + kółko, gest szczypania", "powiększenie"),
            ("Lewy przycisk", "działanie wybranego narzędzia"),
            ("Prawy lub środkowy przycisk – przeciąganie", "przesuwanie obrazu"),
            ("Prawy przycisk – kliknięcie", "menu (np. usuń pomiar)"),
            ("Dwuklik", "reset widoku (na strzałce: edycja opisu)"),
            ("Przeciąganie pomiaru lub jego końca", "przesuwanie / zmiana"),
        ] {
            ui.label(egui::RichText::new(k).strong());
            ui.label(v);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.heading("Klawiatura");
    egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
        for a in actions::ALL {
            let key = a
                .shortcut()
                .map(|s| ui.ctx().format_shortcut(&s))
                .or_else(|| a.plain_key().map(|k| k.name().to_string()));
            if let Some(k) = key {
                ui.monospace(k);
                ui.label(a.label());
                ui.end_row();
            }
        }
        for (k, v) in [("↑ ↓ ← →", "poprzedni / następny przekrój"), ("PageUp / PageDown", "10 przekrojów"), ("Delete", "usuń zaznaczony pomiar"), ("Esc", "przerwij rysowanie / odznacz")] {
            ui.monospace(k);
            ui.label(v);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.heading("Pomiary");
    ui.label("Wybierz narzędzie (np. Odległość) i przeciągnij po obrazie. Kliknij pomiar, aby go zaznaczyć: można go przesunąć, przeciągnąć jego końce albo usunąć klawiszem Delete lub prawym przyciskiem. Cmd/Ctrl+Z cofa zmianę. Pomiary są zapamiętywane osobno dla każdego przekroju.");
    ui.add_space(6.0);
    ui.heading("Tryb profesjonalny");
    ui.label("Dodaje pomiar kąta, obszary ROI ze średnią jasnością i polem, sondę, wskaźnik 3D, mapy kolorów, dokładne wartości okna, panel tagów DICOM i histogram oraz współrzędne w mm.");
    ui.add_space(6.0);
    ui.colored_label(Color32::from_rgb(230, 170, 80), "To narzędzie poglądowe. Nie służy do stawiania diagnozy – wiążący jest opis radiologa.");
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // wyniki pracy w tle
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
                // na start: największa seria (zwykle najciekawsza), z pominięciem lokalizatorów
                let first = lib
                    .all_series()
                    .filter(|(_, s)| !is_localizer(s))
                    .max_by_key(|(_, s)| s.slices.len().min(60))
                    .or_else(|| lib.all_series().next())
                    .map(|(_, s)| s.id);
                self.status = format!("Znaleziono badań: {}, serii: {}", lib.studies.len(), lib.series_count());
                self.lib = Some(Arc::new(lib));
                if let Some(id) = first {
                    self.assign_series(0, id, &ctx);
                }
                let n = self.viewports.len();
                if n > 1 {
                    self.viewports.truncate(1);
                    self.set_layout(n, &ctx);
                }
            }
        }
        while let Ok((id, img)) = self.thumb_rx.try_recv() {
            self.thumbs.insert(id, ctx.load_texture(format!("thumb{id}"), img, TextureOptions::LINEAR));
        }
        while let Ok((p, r, ppp, img)) = self.shot_rx.try_recv() {
            self.handle_shot(p, r, ppp, img);
        }
        let job_line = {
            let mut j = self.job.lock().unwrap();
            match j.as_ref() {
                Some(job) if job.finished.is_some() => {
                    self.status = job.finished.clone().unwrap();
                    *j = None;
                    None
                }
                Some(job) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    Some(format!("{}… {}/{}", job.title, job.done, job.total))
                }
                None => None,
            }
        };
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.into_iter().next() {
            let dir = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
            self.open_folder(dir, &ctx);
        }

        #[cfg(target_os = "macos")]
        actions::native::poll(&mut self.pending);
        self.keyboard(&ctx);

        // --- panele ---
        if !cfg!(target_os = "macos") {
            egui::Panel::top("menu").show(ui, |ui| {
                let mut acts = Vec::new();
                actions::egui_menu(ui, self.settings.pro_mode, &mut acts);
                self.pending.extend(acts);
            });
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(3.0);
            self.toolbar(ui);
            ui.add_space(1.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small(egui::RichText::new("Narzędzie poglądowe – nie do diagnozy").color(Color32::from_rgb(230, 170, 80)));
                ui.separator();
                if let Some(j) = &job_line {
                    ui.spinner();
                    ui.small(j);
                    ui.separator();
                }
                ui.small(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(&self.hover_status);
                    let n = self.store.count();
                    if n > 0 {
                        ui.separator();
                        ui.small(format!("Pomiary: {n}"));
                    }
                });
            });
        });
        if self.settings.show_sidebar {
            egui::Panel::left("series").default_size(290.0).show(ui, |ui| self.sidebar(ui));
        }
        if self.settings.pro_mode && self.settings.show_info_panel && self.lib.is_some() {
            egui::Panel::right("info").default_size(320.0).show(ui, |ui| self.info_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.lib.is_none() && self.scan_rx.is_none() {
                self.welcome(ui);
                return;
            }
            let full = ui.available_rect_before_wrap();
            let n = self.viewports.len();
            let (cols, rows) = match n {
                1 => (1, 1),
                2 => (2, 1),
                _ => (2, 2),
            };
            let cw = full.width() / cols as f32;
            let rh = full.height() / rows as f32;
            // płaszczyzny wszystkich okon (do linii odniesienia)
            let mut planes: Vec<Option<ImagePlane>> = vec![None; n];
            let mut frames = Vec::with_capacity(n);
            for vi in 0..n {
                let series = self.viewports[vi].series_id.and_then(|id| self.series(id));
                let f = series.as_ref().and_then(|s| {
                    if let Some(l) = self.loaders.get(&s.id) {
                        if self.viewports[vi].plane == ViewPlane::Native {
                            l.target.store(self.viewports[vi].slice, Ordering::Relaxed);
                        }
                    }
                    let r = self.current_frame(vi, s);
                    if r.is_none() {
                        self.ensure_loader(s, &ctx);
                    }
                    r
                });
                planes[vi] = f.as_ref().and_then(|x| x.2);
                self.viewports[vi].plane_geom = planes[vi];
                frames.push((series, f));
            }
            self.hover_status.clear();
            for (vi, (series, f)) in frames.into_iter().enumerate() {
                let r = Rect::from_min_size(
                    Pos2::new(full.min.x + cw * (vi % cols) as f32, full.min.y + rh * (vi / cols) as f32),
                    Vec2::new(cw - 2.0, rh - 2.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(("viewport", vi)));
                let Some(series) = series else {
                    let p = child.painter_at(r);
                    p.rect_filled(r, 0.0, Color32::BLACK);
                    p.text(r.center(), egui::Align2::CENTER_CENTER, "Kliknij tu, a potem wybierz serię z listy", egui::FontId::proportional(15.0), Color32::GRAY);
                    if n > 1 {
                        let col = viewport::VIEW_COLORS[vi % 4];
                        p.rect_stroke(r.shrink(1.0), 0.0, Stroke::new(if vi == self.active { 3.0 } else { 1.0 }, col), egui::StrokeKind::Inside);
                    }
                    if child.allocate_rect(r, Sense::click()).clicked() {
                        self.active = vi;
                    }
                    continue;
                };
                let Some((frame, spacing, _)) = f else {
                    let p = child.painter_at(r);
                    p.rect_filled(r, 0.0, Color32::BLACK);
                    let msg = match self.errors.lock().unwrap().get(&series.id) {
                        Some(e) => format!("Nie udało się odczytać obrazu:\n{e}"),
                        None => format!("Wczytywanie… {}/{}", self.loaded_count(&series), series.slices.len()),
                    };
                    p.text(r.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(15.0), Color32::GRAY);
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    continue;
                };
                let study = self.study_of(series.id);
                let others: Vec<(ImagePlane, Color32)> = planes
                    .iter()
                    .enumerate()
                    .filter(|(oi, p)| *oi != vi && p.is_some())
                    .map(|(oi, p)| (p.unwrap(), viewport::VIEW_COLORS[oi % 4]))
                    .collect();
                let loaded = (self.loaded_count(&series), series.slices.len());
                let units: &'static str = if series.modality.eq_ignore_ascii_case("CT") { "HU" } else { "" };
                let patient_line = study.as_ref().map(|s| self.patient_line(s)).unwrap_or_default();
                let study_date = study.as_ref().map(|s| s.date.clone()).unwrap_or_default();
                let has_spacing = series.pixel_spacing.is_some() || self.viewports[vi].plane != ViewPlane::Native;
                let input = ViewInput {
                    series: &series,
                    frame,
                    spacing,
                    has_spacing,
                    settings: &self.settings,
                    tool: self.tool,
                    store: &mut self.store,
                    active: vi == self.active,
                    multi: n > 1,
                    index: vi,
                    others,
                    cursor3d: self.cursor3d,
                    patient_line,
                    study_date,
                    units,
                    loaded,
                };
                let out = viewport::show(&mut child, &mut self.viewports[vi], input);
                if out.clicked {
                    self.active = vi;
                }
                if out.position_changed {
                    self.sync_from(vi);
                }
                if let Some(p) = out.cursor3d {
                    self.apply_cursor3d(vi, p);
                }
                if let Some(s) = out.status {
                    self.hover_status = s;
                }
                for a in out.actions {
                    self.active = vi;
                    self.pending.push(a);
                }
            }
        });

        self.dialogs(&ctx);

        let acts: Vec<Action> = std::mem::take(&mut self.pending);
        for a in acts {
            self.handle_action(a, &ctx);
        }
        if self.settings_dirty {
            self.settings_dirty = false;
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, settings::STORAGE_KEY, &self.settings);
    }
}

pub fn run(folder: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Przeglądarka MRI")
            .with_app_id("mri-viewer")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "Przeglądarka MRI",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, folder)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Lokalizator (zwiadowczy obraz do planowania badania) – zwykle mało przydatny do oglądania.
fn is_localizer(s: &Series) -> bool {
    let d = s.description.to_lowercase();
    ["loc", "scout", "survey", "plan", "3pl"].iter().any(|k| d.contains(k))
}
