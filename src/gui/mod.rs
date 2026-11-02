//! Okno programu (egui): lista serii, okna podglądu, menu, ustawienia, eksport.

mod about;
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

/// Ile przekrojów wokół bieżącego wczytywać ponownie po wypadnięciu serii z pamięci.
const RELOAD_RADIUS: usize = 12;

struct Loader {
    generation: Arc<AtomicUsize>,
    target: Arc<AtomicUsize>,
    alive: Arc<AtomicUsize>,
    /// Przekroje, których nie udało się odczytać – nie wczytujemy ich ponownie.
    failed: Arc<Mutex<HashSet<usize>>>,
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
/// Funkcja, nie stała: nazwy są tłumaczone w chwili rysowania (język może się zmienić w trakcie pracy).
fn ct_presets() -> [(&'static str, f32, f32); 7] {
    [
        (t!("app.window_menu.ct.brain"), 40.0, 80.0),
        (t!("app.window_menu.ct.subdural"), 75.0, 215.0),
        (t!("app.window_menu.ct.soft_tissue"), 40.0, 400.0),
        (t!("app.window_menu.ct.mediastinum"), 50.0, 350.0),
        (t!("app.window_menu.ct.liver"), 60.0, 150.0),
        (t!("app.window_menu.ct.lung"), -600.0, 1500.0),
        (t!("app.window_menu.ct.bone"), 400.0, 1800.0),
    ]
}

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
    /// Układ współrzędnych, w którym ustawiono wskaźnik 3D.
    cursor3d_frame: Option<FrameKey>,
    show_settings: bool,
    show_help: bool,
    show_about: bool,
    show_connect: bool,
    connect_show_patient: bool,
    /// Wynik dodania do Claude Desktop (ścieżka pliku albo błąd) – napis tworzony przy rysowaniu, w bieżącym języku.
    connect_msg: Option<Result<PathBuf, String>>,
    status: String,
    hover_status: String,
    job: JobHandle,
    shot_tx: Sender<(ShotPurpose, Rect, f32, Arc<ColorImage>)>,
    shot_rx: Receiver<(ShotPurpose, Rect, f32, Arc<ColorImage>)>,
    /// Zrzut widoku czekający, aż klatka zostanie narysowana bez danych pacjenta.
    pending_shot: Option<ShotPurpose>,
    /// Dane pacjenta ukryte na czas robienia zrzutu do zapisu lub schowka.
    shot_hide: bool,
    /// Kiedy wysłano prośbę o zrzut – gdyby zrzut nie nadszedł, po chwili znów pokazujemy dane.
    shot_sent: Option<std::time::Instant>,
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
        // tytuł okna w języku z ustawień (przy tworzeniu okna język nie był jeszcze znany)
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Title(t!("app.title").to_string()));
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
            cursor3d_frame: None,
            show_settings: false,
            show_help: false,
            show_about: false,
            show_connect: false,
            connect_show_patient: false,
            connect_msg: None,
            status: String::new(),
            hover_status: String::new(),
            job: Arc::new(Mutex::new(None)),
            shot_tx,
            shot_rx,
            pending_shot: None,
            shot_hide: false,
            shot_sent: None,
            tags_cache: HashMap::new(),
            tags_filter: String::new(),
            info_tab: InfoTab::Series,
            pending: Vec::new(),
            settings_dirty: false,
            #[cfg(target_os = "macos")]
            _native_menu: Some(actions::native::install(&cc.egui_ctx)),
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
        // przerywamy trwające szukanie plików
        self.scan_rx = None;
        self.scan_progress = (0, 0);
        // nowy kanał miniatur i nowa mapa błędów: wyniki zadań z poprzedniego badania trafiają w próżnię
        let (thumb_tx, thumb_rx) = channel();
        self.thumb_tx = thumb_tx;
        self.thumb_rx = thumb_rx;
        self.thumbs.clear();
        self.thumbs_requested.clear();
        self.errors = Arc::new(Mutex::new(HashMap::new()));
        self.tags_cache.clear();
        let n = self.viewports.len();
        self.viewports = (0..n).map(|_| Viewport::new()).collect();
        self.active = 0;
        self.lib = None;
        self.folder = None;
        self.store.reset();
        self.cursor3d = None;
        self.cursor3d_frame = None;
    }

    fn choose_folder(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new().set_title(t!("app.dialog.choose_study_folder"));
        if let Some(f) = self.folder.clone().or_else(|| self.settings.last_folder.clone().map(PathBuf::from)) {
            dlg = dlg.set_directory(f);
        }
        if let Some(f) = dlg.pick_folder() {
            self.open_folder(f, ctx);
        }
    }

    fn choose_file(&mut self, ctx: &egui::Context) {
        if let Some(f) = rfd::FileDialog::new().set_title(t!("app.dialog.choose_dicom_file")).pick_file() {
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
        let mut failed = Arc::new(Mutex::new(HashSet::new()));
        let reload = self.loaders.contains_key(&series.id);
        if let Some(l) = self.loaders.get(&series.id) {
            if l.alive.load(Ordering::SeqCst) > 0 {
                return;
            }
            {
                let bad = l.failed.lock().unwrap();
                if series.slices.iter().enumerate().all(|(i, s)| bad.contains(&i) || self.cache.peek(s).is_some()) {
                    return;
                }
            }
            // wątki skończyły, ale część obrazów wypadła z pamięci – wczytujemy ponownie (bez uszkodzonych)
            if let Some(old) = self.loaders.remove(&series.id) {
                failed = old.failed;
            }
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
        let start = self
            .viewports
            .iter()
            .find(|v| v.series_id == Some(series.id) && v.plane == ViewPlane::Native)
            .map_or(series.slices.len() / 2, |v| v.slice);
        let target = Arc::new(AtomicUsize::new(start));
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6);
        let alive = Arc::new(AtomicUsize::new(workers));
        self.loaders.insert(
            series.id,
            Loader { generation: generation.clone(), target: target.clone(), alive: alive.clone(), failed: failed.clone() },
        );
        let slices = series.slices.clone();
        let sid = series.id;
        let done: Arc<Mutex<Vec<bool>>> = {
            let bad = failed.lock().unwrap();
            Arc::new(Mutex::new(
                // ponowne wczytanie (obrazy wypadły z pamięci, bo otwarte serie się w niej nie mieszczą):
                // tylko okolica bieżącego przekroju, żeby serie nie wypychały się nawzajem bez końca
                slices
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        bad.contains(&i) || self.cache.peek(s).is_some() || (reload && i.abs_diff(start) > RELOAD_RADIUS)
                    })
                    .collect(),
            ))
        };
        for _ in 0..workers {
            let slices = slices.clone();
            let cache = self.cache.clone();
            let errors = self.errors.clone();
            let generation = generation.clone();
            let target = target.clone();
            let ctx = ctx.clone();
            let done = done.clone();
            let alive = alive.clone();
            let failed = failed.clone();
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
                        failed.lock().unwrap().insert(i);
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
                        self.status = t!("app.status.reformat_failed", error = e);
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

    /// Układ współrzędnych serii (FrameOfReferenceUID i badanie).
    fn frame_key(&self, s: &Series) -> FrameKey {
        let study = self.lib.as_ref().and_then(|l| l.series_by_id(s.id)).map(|(st, _)| st.uid.clone()).unwrap_or_default();
        FrameKey { frame_of_reference: s.frame_of_reference.clone(), study }
    }

    /// Układ współrzędnych serii pokazanej w oknie `vi`.
    fn viewport_frame_key(&self, vi: usize) -> Option<FrameKey> {
        let id = self.viewports.get(vi)?.series_id?;
        let (st, s) = self.lib.as_ref()?.series_by_id(id)?;
        Some(FrameKey { frame_of_reference: s.frame_of_reference.clone(), study: st.uid.clone() })
    }

    /// Po zmianie przekroju w oknie `vi` ustawia pozostałe okna z równoległą płaszczyzną na to samo miejsce.
    fn sync_from(&mut self, vi: usize) {
        if self.viewports.len() < 2 || !self.settings.sync_scroll {
            return;
        }
        let Some(src_series) = self.viewports[vi].series_id.and_then(|id| self.series(id)) else { return };
        let Some((_, _, Some(src_plane))) = self.current_frame(vi, &src_series) else { return };
        let src_key = self.frame_key(&src_series);
        let center = src_plane.to_patient(Pos2::new(src_plane.w as f32 / 2.0, src_plane.h as f32 / 2.0));
        let n_src = src_plane.normal();
        for oi in 0..self.viewports.len() {
            if oi == vi || self.viewports[oi].plane != ViewPlane::Native {
                continue;
            }
            let Some(s) = self.viewports[oi].series_id.and_then(|id| self.series(id)) else { continue };
            // inne badanie / układ współrzędnych – położenia w mm nic nie mówią
            if !self.frame_key(&s).matches(&src_key) {
                continue;
            }
            let Some(o) = s.orientation else { continue };
            let n = viewport::cross([o[0], o[1], o[2]], [o[3], o[4], o[5]]);
            if viewport::dot(n, n_src).abs() < 0.95 {
                continue;
            }
            if let Some(best) = nearest_slice(&s, center, n) {
                // set_position przerywa rysowanie pomiaru – tylko gdy przekrój naprawdę się zmienia
                if best != self.viewports[oi].position() {
                    self.viewports[oi].set_position(&s, best as i64);
                }
            }
        }
    }

    fn apply_cursor3d(&mut self, src: usize, p: [f64; 3]) {
        let Some(src_key) = self.viewport_frame_key(src) else { return };
        self.cursor3d = Some(p);
        self.cursor3d_frame = Some(src_key.clone());
        for oi in 0..self.viewports.len() {
            if oi == src {
                continue;
            }
            let Some(s) = self.viewports[oi].series_id.and_then(|id| self.series(id)) else { continue };
            if !self.frame_key(&s).matches(&src_key) {
                continue;
            }
            let vp = &mut self.viewports[oi];
            match vp.plane {
                ViewPlane::Native => {
                    let Some(o) = s.orientation else { continue };
                    let n = viewport::cross([o[0], o[1], o[2]], [o[3], o[4], o[5]]);
                    if let Some(best) = nearest_slice(&s, p, n) {
                        if best != vp.position() {
                            vp.set_position(&s, best as i64);
                        }
                    }
                }
                ViewPlane::Reformat(plane) => {
                    // bez set_position: przed pierwszą rekonstrukcją liczba położeń nie jest jeszcze znana
                    if let Some(idx) = reformat_index_for_point(&s, plane, p) {
                        if idx != vp.reformat_pos {
                            vp.reformat_pos = idx;
                            vp.creating = None;
                            vp.angle_stage = 0;
                            vp.editing_text = None;
                        }
                    }
                }
            }
        }
    }

    // --- polecenia ------------------------------------------------------------------------------------

    fn render_opts(&self, vi: usize) -> RenderOpts {
        let vp = &self.viewports[vi];
        RenderOpts { window: vp.window, invert: vp.invert, colormap: vp.colormap, xf: vp.xf }
    }

    fn request_view_shot(&mut self, ctx: &egui::Context, purpose: ShotPurpose) {
        if self.settings.hide_patient_in_exports && !self.settings.hide_patient {
            // najpierw jedna klatka bez danych pacjenta, zrzut dopiero po niej (koniec `ui`)
            self.shot_hide = true;
            self.pending_shot = Some(purpose);
            ctx.request_repaint();
            return;
        }
        self.send_view_shot(ctx, purpose);
    }

    fn send_view_shot(&mut self, ctx: &egui::Context, purpose: ShotPurpose) {
        self.shot_sent = Some(std::time::Instant::now());
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
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
        if self.pending_shot.is_none() {
            self.shot_hide = false;
        }
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
                    Ok(()) => t!("app.status.saved", path = path.display()),
                    Err(e) => t!("app.status.save_error", error = format!("{e:#}")),
                };
            }
            ShotPurpose::Clipboard => {
                self.status = match export::copy_to_clipboard(&out) {
                    Ok(()) => t!("app.status.view_copied").into(),
                    Err(e) => format!("{e:#}"),
                };
            }
        }
    }

    fn save_dialog(&self, name: &str) -> Option<PathBuf> {
        let ext = self.settings.export_format.ext();
        rfd::FileDialog::new()
            .set_file_name(format!("{name}.{ext}"))
            .add_filter(t!("app.dialog.filter_image"), &[ext])
            .save_file()
    }

    fn handle_action(&mut self, a: Action, ctx: &egui::Context) {
        let vi = self.active;
        // zmiana pomiarów w trakcie edycji opisu strzałki: najpierw zapisujemy edycję do historii
        if matches!(
            a,
            Action::Undo | Action::Redo | Action::DeleteSelected | Action::DeleteSliceAnnotations | Action::DeleteAllAnnotations
        ) {
            for vp in &mut self.viewports {
                vp.finish_text_edit(&mut self.store);
            }
        }
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
                    self.status = t!("app.status.nothing_to_undo").into();
                }
            }
            Action::Redo => {
                if !self.store.redo() {
                    self.status = t!("app.status.nothing_to_redo").into();
                }
            }
            Action::DeleteSelected => {
                let vp = &mut self.viewports[vi];
                if let (Some(k), Some(id)) = (vp.key(), vp.selected.take()) {
                    self.store.delete(k, id);
                } else if vp.creating.take().is_some() {
                    vp.angle_stage = 0;
                } else {
                    self.status = t!("app.status.select_measurement_first").into();
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
                    self.status = t!("app.status.pro_enabled").into();
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
        // układ okien albo badanie mogły się właśnie zmienić (np. „Jedno okno”) – czytamy aktywne okno ponownie
        let vi = self.active;
        let Some(series) = self.active_series() else { return };
        let Some(vp) = self.viewports.get_mut(vi) else { return };
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
                    self.viewports[vi].window = Some(((f.min + f.max) / 2.0, (f.max - f.min).max(f.window_eps())));
                }
            }
            Action::SaveImage => {
                if let Some((f, sp, _)) = self.current_frame(vi, &series) {
                    let o = self.render_opts(vi);
                    let name = t!("app.file.image", series = export::series_base_name(&series), n = format!("{:03}", self.viewports[vi].position() + 1));
                    if let Some(path) = self.save_dialog(&name) {
                        let img = export::render_image(&f, sp, &o, None);
                        self.status = match export::save_image(&img, &path, self.settings.export_format, self.settings.jpeg_quality) {
                            Ok(()) => t!("app.status.saved", path = path.display()),
                            Err(e) => t!("app.status.save_error", error = format!("{e:#}")),
                        };
                    }
                }
            }
            Action::SaveView => {
                let name = t!("app.file.view", series = export::series_base_name(&series), n = format!("{:03}", vp.position() + 1));
                if let Some(path) = self.save_dialog(&name) {
                    self.request_view_shot(ctx, ShotPurpose::Save(path));
                }
            }
            Action::CopyView => self.request_view_shot(ctx, ShotPurpose::Clipboard),
            Action::ExportSeries => {
                if export::busy(&self.job) {
                    self.status = export::busy_msg().into();
                } else if let Some(dir) = rfd::FileDialog::new().set_title(t!("app.dialog.choose_export_folder")).pick_folder() {
                    let o = self.render_opts(vi);
                    export::export_series(series, self.cache.clone(), dir, o, self.settings.export_format, self.settings.jpeg_quality, self.job.clone());
                }
            }
            Action::ExportGif => {
                let name = format!("{}.gif", export::series_base_name(&series));
                if export::busy(&self.job) {
                    self.status = export::busy_msg().into();
                } else if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("GIF", &["gif"]).save_file() {
                    let o = self.render_opts(vi);
                    export::export_gif(series, self.cache.clone(), path, o, self.settings.cine_fps, self.job.clone());
                }
            }
            Action::ExportMontage => {
                let name = format!("{}.png", t!("app.file.montage", series = export::series_base_name(&series)));
                if export::busy(&self.job) {
                    self.status = export::busy_msg().into();
                } else if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("PNG", &["png"]).save_file() {
                    let o = self.render_opts(vi);
                    export::export_montage(series, self.cache.clone(), path, o, self.job.clone());
                }
            }
            Action::ExportTags => {
                let idx = if vp.plane == ViewPlane::Native { vp.slice } else { series.slices.len() / 2 };
                let name = format!("{}.txt", t!("app.file.tags", series = export::series_base_name(&series)));
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(t!("app.dialog.filter_text"), &["txt"]).save_file() {
                    self.status = match imaging::dump_tags(&series.slices[idx].path, self.settings.hide_patient)
                        .and_then(|t| Ok(std::fs::write(&path, t)?))
                    {
                        Ok(()) => t!("app.status.saved", path = path.display()),
                        Err(e) => t!("app.status.save_error", error = format!("{e:#}")),
                    };
                }
            }
            _ => {}
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let mut acts = Vec::new();
        // pole tekstowe ma fokus – cofanie i Ctrl+W należą do niego
        let typing = ctx.text_edit_focused();
        // skróty z modyfikatorami (na macOS obsługuje je menu systemowe)
        if !cfg!(target_os = "macos") {
            for (a, s) in actions::shortcuts_by_specificity() {
                if typing && a.conflicts_with_text_edit() {
                    continue;
                }
                if ctx.input_mut(|i| i.consume_shortcut(&s)) {
                    acts.push(a);
                }
            }
        } else if typing {
            // menu systemowe przechwytuje Cmd+Z – oddajemy cofanie polu tekstowemu,
            // a pozostałe skróty kolidujące z pisaniem (Cmd+W, Cmd+R) pomijamy
            self.pending.retain(|a| {
                let shift = match a {
                    Action::Undo => false,
                    Action::Redo => true,
                    a if a.conflicts_with_text_edit() => return false,
                    _ => return true,
                };
                let mut modifiers = egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND;
                modifiers.shift = shift;
                ctx.input_mut(|i| {
                    i.events.push(egui::Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers })
                });
                false
            });
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
        if self.settings.hide_patient || self.shot_hide {
            if !st.sex.is_empty() || !st.age.is_empty() {
                t!("app.patient.hidden_sex_age", sex = st.sex, age = st.age.trim_start_matches('0'))
            } else {
                t!("app.patient.hidden").to_string()
            }
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
        if ui.add_sized([ui.available_width(), 30.0], egui::Button::new(t!("app.sidebar.open_folder"))).clicked() {
            self.pending.push(Action::OpenFolder);
        }
        if self.scan_rx.is_some() {
            let (a, b) = self.scan_progress;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if b > 0 { t!("app.sidebar.reading_files", done = a, total = b) } else { t!("app.sidebar.searching_files").into() });
            });
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        ui.separator();
        if lib.studies.is_empty() {
            ui.colored_label(Color32::LIGHT_RED, t!("app.sidebar.no_dicom"));
            ui.small(t!("app.sidebar.no_dicom_hint"));
            return;
        }
        let mut clicked: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for st in &lib.studies {
                let head = format!(
                    "{}  {}",
                    fmt_date(&st.date),
                    if st.description.is_empty() { t!("app.sidebar.study_untitled") } else { &st.description }
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
                                        let desc = if se.description.is_empty() { t!("app.sidebar.series_untitled") } else { &se.description };
                                        let t = egui::RichText::new(format!(
                                            "{}{}",
                                            se.number.map(|n| format!("{n}. ")).unwrap_or_default(),
                                            desc
                                        ));
                                        ui.label(if in_views.is_empty() { t } else { t.strong() });
                                        ui.small(crate::i18n::trn_args(
                                            "app.sidebar.series_info",
                                            se.slices.len() as u64,
                                            &[("plane", &se.plane.label())],
                                        ));
                                        if se.may_have_burned_in_text {
                                            ui.small(egui::RichText::new(t!("app.sidebar.burned_in")).color(Color32::from_rgb(230, 170, 80)))
                                                .on_hover_text(t!("app.sidebar.burned_in_tip"));
                                        }
                                        if let Some(err) = self.errors.lock().unwrap_or_else(|e| e.into_inner()).get(&se.id) {
                                            ui.small(egui::RichText::new(t!("app.sidebar.error", error = err)).color(Color32::LIGHT_RED));
                                        }
                                    });
                                })
                                .response;
                            let resp = ui.interact(resp.rect, ui.scope_id().with(("series", se.id)), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_stroke(resp.rect.expand(2.0), 4.0, Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Outside);
                            }
                            let resp = resp.on_hover_text(t!("app.sidebar.series_tip", title = se.title()));
                            if resp.clicked() {
                                clicked = Some((se.id, ui.input(|i| i.modifiers.shift)));
                            }
                            ui.add_space(3.0);
                        }
                    });
            }
            if lib.skipped_files > 0 {
                ui.add_space(6.0);
                ui.small(t!("app.sidebar.skipped_files", count = lib.skipped_files));
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

    /// Pasek narzędzi w jednej linii. Gdy brakuje miejsca (wąskie okno, tryb PRO, duży tekst),
    /// zwija kolejno: pomiary do listy, przyciski obrazu do menu „Obraz”, a na końcu wszystkie narzędzia.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let pro = self.settings.pro_mode;
        let tools: Vec<Tool> = [
            Tool::Window,
            Tool::Pan,
            Tool::Zoom,
            Tool::Line,
            Tool::Arrow,
            Tool::Angle,
            Tool::Ellipse,
            Tool::Rect,
            Tool::Probe,
            Tool::Cursor3d,
        ]
        .into_iter()
        .filter(|t| pro || !t.pro())
        .collect();
        let basic = [Tool::Window, Tool::Pan, Tool::Zoom];
        let measure: Vec<Tool> = tools.iter().copied().filter(|t| !basic.contains(t)).collect();
        let has = self.viewports[self.active].series_id.is_some();
        let n = self.viewports.len();

        // szerokości w punktach, liczone z prawdziwej czcionki przycisków
        let font = egui::TextStyle::Button.resolve(ui.style());
        let pad = ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x;
        let ctx = ui.ctx().clone();
        let tw = |s: &str| -> f32 { ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x) + pad };
        let sep = ui.spacing().item_spacing.x * 2.0 + 1.0;
        // napisy przycisków w bieżącym języku – mierzone są dokładnie te same, które potem rysujemy
        let l_reset = t!("app.toolbar.reset");
        let l_invert = t!("app.toolbar.invert");
        let l_rotate = t!("app.toolbar.rotate");
        let l_window = format!("{} ▾", t!("app.toolbar.window_menu"));
        let l_image = format!("{} ▾", t!("app.toolbar.image_menu"));
        let l_measure = format!("{} ▾", t!("app.toolbar.measurements_menu"));
        let l_sync = t!("app.toolbar.sync");
        let l_layout = t!("app.toolbar.layout");
        let l_pro = t!("app.toolbar.pro_mode");
        let l_pro_short = t!("app.toolbar.pro_mode_short");
        let l_settings = t!("app.toolbar.settings");
        let longest_measure = measure.iter().map(|t| tw(t.label())).fold(0.0, f32::max);
        let w_tools_full: f32 = tools.iter().map(|t| tw(t.label())).sum();
        let w_tools_mid: f32 = basic.iter().map(|t| tw(t.label())).sum::<f32>() + longest_measure.max(tw(t!("app.toolbar.measurements_menu"))) + tw(" ▾");
        let plane_w = self.plane_label(false).map(|l| tw(&l) + 16.0).unwrap_or(0.0);
        let plane_w_short = self.plane_label(true).map(|l| tw(&l) + 16.0).unwrap_or(0.0);
        let w_view_full = if has { tw(l_reset) + tw(l_invert) + tw(l_rotate) + tw(&l_window) + plane_w } else { tw(l_reset) + tw(l_invert) + tw(l_rotate) };
        let w_view_min = tw(&l_image) + if has { tw(&l_window) + plane_w_short } else { 0.0 };
        let w_layout = tw("1") + tw("2") + tw("4") + if n > 1 { tw(l_sync) + 20.0 } else { 0.0 };
        let w_right = tw(l_layout) + w_layout + tw(l_pro) + tw(l_settings) + sep * 2.0;
        // poziom 4 (najwęższy): bez napisu „Układ:”, „PRO” zamiast „Tryb PRO”, ustawienia tylko w menu
        let w_tools_min = tools.iter().map(|t| tw(t.label())).fold(0.0, f32::max) + tw(" ▾");
        let avail = ui.available_width() - 8.0;
        let level = if w_tools_full + sep + w_view_full + w_right <= avail {
            0
        } else if w_tools_mid + sep + w_view_full + w_right <= avail {
            1
        } else if w_tools_mid + sep + w_view_min + w_right <= avail {
            2
        } else if w_tools_min + sep + w_view_min + w_right <= avail {
            3
        } else {
            4
        };

        ui.horizontal(|ui| {
            // --- narzędzia ---
            match level {
                0 => {
                    for t in &tools {
                        ui.selectable_value(&mut self.tool, *t, t.label()).on_hover_text(t.hint());
                    }
                }
                1 | 2 => {
                    for t in basic {
                        ui.selectable_value(&mut self.tool, t, t.label()).on_hover_text(t.hint());
                    }
                    let active = measure.contains(&self.tool);
                    let title = if active { format!("{} ▾", self.tool.label()) } else { l_measure.clone() };
                    self.tool_menu(ui, &measure, title, active);
                }
                _ => {
                    let title = format!("{} ▾", self.tool.label());
                    self.tool_menu(ui, &tools, title, true);
                }
            }
            ui.separator();
            // --- obraz ---
            ui.add_enabled_ui(has, |ui| {
                if level < 2 {
                    if ui.button(l_reset).on_hover_text(t!("app.toolbar.reset_tip")).clicked() {
                        self.pending.push(Action::ResetView);
                    }
                    let inv = self.viewports[self.active].invert;
                    if ui.selectable_label(inv, l_invert).on_hover_text(t!("app.toolbar.invert_tip")).clicked() {
                        self.pending.push(Action::Invert);
                    }
                    if ui.button(l_rotate).on_hover_text(t!("app.toolbar.rotate_tip")).clicked() {
                        self.pending.push(Action::RotateRight);
                    }
                } else {
                    ui.menu_button(l_image.as_str(), |ui| {
                        for a in [Action::ResetView, Action::Invert, Action::RotateRight, Action::RotateLeft, Action::FlipH, Action::FlipV] {
                            if ui.button(a.label()).clicked() {
                                self.pending.push(a);
                                ui.close();
                            }
                        }
                    });
                }
                self.window_menu(ui, &l_window);
                self.plane_menu(ui, level >= 2);
            });
            // --- układ i ustawienia, dosunięte do prawej ---
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if level < 4 && ui.button(l_settings).clicked() {
                    self.pending.push(Action::Settings);
                }
                let pro_label = if level < 4 { l_pro } else { l_pro_short };
                if ui.selectable_label(pro, pro_label).on_hover_text(t!("app.toolbar.pro_mode_tip")).clicked() {
                    self.pending.push(Action::TogglePro);
                }
                ui.separator();
                if n > 1 && ui.checkbox(&mut self.settings.sync_scroll, l_sync).on_hover_text(t!("app.toolbar.sync_tip")).changed() {
                    self.settings_dirty = true;
                }
                let ctx = ui.ctx().clone();
                for (k, label, tip) in [
                    (4, "4", t!("app.toolbar.layout_4_tip")),
                    (2, "2", t!("app.toolbar.layout_2_tip")),
                    (1, "1", t!("app.toolbar.layout_1_tip")),
                ] {
                    if ui.selectable_label(n == k, label).on_hover_text(tip).clicked() {
                        self.set_layout(k, &ctx);
                    }
                }
                if level < 4 {
                    ui.label(l_layout);
                }
            });
        });
    }

    /// Napis na liście płaszczyzn (None, gdy lista nie jest pokazywana).
    fn plane_label(&self, compact: bool) -> Option<String> {
        let series = self.active_series()?;
        if mpr::available_planes(&series).is_empty() {
            return None;
        }
        Some(match (self.viewports[self.active].plane, compact) {
            (ViewPlane::Native, true) => series.plane.label().to_string(),
            (ViewPlane::Native, false) => t!("app.toolbar.plane", plane = series.plane.label()),
            (ViewPlane::Reformat(p), true) => t!("app.toolbar.plane_reformat_short", plane = p.label()),
            (ViewPlane::Reformat(p), false) => t!("app.toolbar.plane_reformat", plane = p.label()),
        })
    }

    /// Lista rozwijana z narzędziami (gdy nie mieszczą się na pasku).
    fn tool_menu(&mut self, ui: &mut egui::Ui, tools: &[Tool], title: String, highlight: bool) {
        let text = if highlight { egui::RichText::new(title).color(ui.visuals().selection.stroke.color) } else { egui::RichText::new(title) };
        let resp = ui.menu_button(text, |ui| {
            for t in tools {
                if ui.selectable_label(self.tool == *t, t.label()).on_hover_text(t.hint()).clicked() {
                    self.tool = *t;
                    ui.close();
                }
            }
        });
        resp.response.on_hover_text(t!("app.toolbar.choose_tool"));
    }

    /// `title` – napis przycisku menu (ten sam, który zmierzył pasek narzędzi).
    fn window_menu(&mut self, ui: &mut egui::Ui, title: &str) {
        let vi = self.active;
        let Some(series) = self.active_series() else { return };
        let frame = self.current_frame(vi, &series).map(|(f, _, _)| f);
        let pro = self.settings.pro_mode;
        ui.menu_button(title, |ui| {
            if ui.button(t!("app.window_menu.auto")).on_hover_text(t!("app.window_menu.auto_tip")).clicked() {
                self.pending.push(Action::PresetAuto);
                ui.close();
            }
            if let Some(f) = &frame {
                for (c, w, expl) in &f.dicom_windows {
                    let label = if expl.is_empty() {
                        t!("app.window_menu.from_scanner", w = format!("{w:.0}"), c = format!("{c:.0}"))
                    } else {
                        t!("app.window_menu.from_scanner_named", name = expl)
                    };
                    if ui.button(label).clicked() {
                        self.viewports[vi].window = Some((*c, *w));
                        ui.close();
                    }
                }
                let (ac, aw) = f.auto_window;
                if ui.button(t!("app.window_menu.more_contrast")).clicked() {
                    self.viewports[vi].window = Some((ac, aw * 0.6));
                    ui.close();
                }
                if ui.button(t!("app.window_menu.less_contrast")).clicked() {
                    self.viewports[vi].window = Some((ac, aw * 1.6));
                    ui.close();
                }
                if ui.button(t!("app.window_menu.full_range")).clicked() {
                    self.pending.push(Action::PresetFull);
                    ui.close();
                }
                if series.modality.eq_ignore_ascii_case("CT") {
                    ui.separator();
                    for (name, c, w) in ct_presets() {
                        if ui.button(format!("{name} (W {w:.0} / L {c:.0})")).clicked() {
                            self.viewports[vi].window = Some((c, w));
                            ui.close();
                        }
                    }
                }
                if pro {
                    ui.separator();
                    let (mut c, mut w) = self.viewports[vi].window.unwrap_or(f.default_window());
                    let step = ((f.max - f.min) / 500.0).max(f.window_eps());
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label(t!("app.window_menu.width"));
                        changed |= ui.add(egui::DragValue::new(&mut w).speed(step).range(f.window_eps()..=f32::MAX)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label(t!("app.window_menu.level"));
                        changed |= ui.add(egui::DragValue::new(&mut c).speed(step)).changed();
                    });
                    if changed {
                        self.viewports[vi].window = Some((c, w));
                    }
                    ui.separator();
                    ui.label(t!("app.window_menu.colormap"));
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

    fn plane_menu(&mut self, ui: &mut egui::Ui, compact: bool) {
        let vi = self.active;
        let Some(series) = self.active_series() else { return };
        let planes = mpr::available_planes(&series);
        if planes.is_empty() {
            return;
        }
        let current = self.plane_label(compact).unwrap_or_default();
        let vp = &mut self.viewports[vi];
        egui::ComboBox::from_id_salt("plane")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui.selectable_label(vp.plane == ViewPlane::Native, t!("app.toolbar.plane_original", plane = series.plane.label())).clicked() {
                    vp.plane = ViewPlane::Native;
                    vp.tex_key = None;
                }
                for p in planes {
                    if ui.selectable_label(vp.plane == ViewPlane::Reformat(p), t!("app.toolbar.plane_reformat_item", plane = p.label())).clicked() {
                        vp.plane = ViewPlane::Reformat(p);
                        vp.reformat_pos = usize::MAX;
                        vp.tex_key = None;
                        vp.reformat_cache = None;
                    }
                }
            })
            .response
            .on_hover_text(t!("app.toolbar.plane_tip"));
    }

    fn info_panel(&mut self, ui: &mut egui::Ui) {
        let vi = self.active;
        let Some(series) = self.active_series() else {
            ui.label(t!("app.info.select_series"));
            return;
        };
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.info_tab, InfoTab::Series, t!("app.info.tab.series"));
            ui.selectable_value(&mut self.info_tab, InfoTab::Tags, t!("app.info.tab.tags"));
            ui.selectable_value(&mut self.info_tab, InfoTab::Histogram, t!("app.info.tab.histogram"));
        });
        ui.separator();
        match self.info_tab {
            InfoTab::Series => {
                let q = &series.sequence;
                let fmt = |v: Option<f64>, u: &str| v.map(|x| format!("{x:.2} {u}").replace(".00 ", " ")).unwrap_or_else(|| "—".into());
                let st = self.study_of(series.id);
                let rows: Vec<(&str, String)> = vec![
                    (t!("app.info.row.study"), st.as_ref().map(|s| format!("{} {}", fmt_date(&s.date), s.description)).unwrap_or_default()),
                    (t!("app.info.row.series"), format!("{} {}", series.number.map(|n| n.to_string()).unwrap_or_default(), series.description)),
                    (t!("app.info.row.modality"), series.modality.clone()),
                    (t!("app.info.row.body_part"), series.body_part.clone()),
                    (t!("app.info.row.protocol"), q.protocol_name.clone()),
                    (t!("app.info.row.plane"), series.plane.label().to_string()),
                    (t!("app.info.row.image_count"), series.slices.len().to_string()),
                    (t!("app.info.row.matrix"), format!("{} × {}", series.columns, series.rows)),
                    (t!("app.info.row.pixel"), series.pixel_spacing.map(|(r, c)| format!("{c:.3} × {r:.3} mm")).unwrap_or("—".into())),
                    (t!("app.info.row.slice_thickness"), fmt(series.slice_thickness, "mm")),
                    (t!("app.info.row.slice_spacing"), fmt(series.slice_gap(), "mm")),
                    ("TR", fmt(q.repetition_time, "ms")),
                    ("TE", fmt(q.echo_time, "ms")),
                    ("TI", fmt(q.inversion_time, "ms")),
                    (t!("app.info.row.flip_angle"), fmt(q.flip_angle, "°")),
                    (t!("app.info.row.field_strength"), fmt(q.magnetic_field_strength, "T")),
                    (t!("app.info.row.sequence"), format!("{} {}", q.scanning_sequence, q.sequence_name)),
                    (t!("app.info.row.contrast_agent"), if q.contrast_agent.is_empty() { "—".into() } else { q.contrast_agent.clone() }),
                    (t!("app.info.row.image_type"), q.image_type.clone()),
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
                    ui.small(t!("app.info.sequence_note"));
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
                    ui.label(t!("app.info.tags_search"));
                    ui.text_edit_singleline(&mut self.tags_filter);
                });
                ui.small(t!(
                    "app.info.tags_image",
                    n = idx + 1,
                    file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
                ));
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
                            ui.label(if hidden { t!("app.info.tags_hidden") } else { val });
                            ui.end_row();
                        }
                    });
                });
            }
            InfoTab::Histogram => {
                let Some((f, _, _)) = self.current_frame(vi, &series) else { return };
                let Pixels::Gray(v) = &f.pixels else {
                    ui.label(t!("app.info.histogram_color"));
                    return;
                };
                const BINS: usize = 128;
                let range = (f.max - f.min).max(f32::MIN_POSITIVE);
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
                ui.small(t!(
                    "app.info.histogram_note",
                    min = format!("{:.0}", f.min),
                    max = format!("{:.0}", f.max),
                    w = format!("{w:.0}"),
                    c = format!("{c:.0}")
                ));
            }
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(20.0));
            ui.heading(egui::RichText::new(t!("app.title")).size(28.0));
            ui.add_space(6.0);
            ui.label(t!("app.start.subtitle"));
            ui.add_space(18.0);
            if ui.add_sized([300.0, 44.0], egui::Button::new(egui::RichText::new(t!("app.start.open_folder")).size(17.0))).clicked() {
                self.pending.push(Action::OpenFolder);
            }
            ui.add_space(6.0);
            ui.small(t!("app.start.open_folder_hint"));
            if !self.settings.recent.is_empty() {
                ui.add_space(20.0);
                ui.label(egui::RichText::new(t!("app.start.recent")).strong());
                let ctx = ui.ctx().clone();
                for r in self.settings.recent.clone() {
                    let exists = std::path::Path::new(&r).is_dir();
                    if ui.add_enabled(exists, egui::Link::new(&r)).clicked() {
                        self.open_folder(PathBuf::from(r), &ctx);
                    }
                }
            }
            ui.add_space(24.0);
            ui.small(egui::RichText::new(t!("app.start.disclaimer")).color(Color32::from_rgb(230, 170, 80)));
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // okna mają stałe id, bo tytuł zmienia się razem z językiem
        if self.show_settings {
            let mut open = true;
            egui::Window::new(t!("app.dialog.settings_title")).id(egui::Id::unique("settings_window")).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                if self.settings.ui(ui) {
                    let lang_before = crate::i18n::current();
                    self.settings.apply(ctx);
                    if crate::i18n::current() != lang_before {
                        // nowy język: tytuł okna i (na macOS) menu systemowe trzeba zbudować ponownie
                        ctx.send_viewport_cmd(egui::ViewportCommand::Title(t!("app.title").to_string()));
                        #[cfg(target_os = "macos")]
                        actions::native::rebuild_menu();
                    }
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
            egui::Window::new(t!("app.dialog.help_title")).id(egui::Id::unique("help_window")).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).default_width(560.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| help_ui(ui));
            });
            self.show_help = open;
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new(t!("app.dialog.about_title")).id(egui::Id::unique("about_window")).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                about::ui(ui);
            });
            self.show_about = open;
        }
        if self.show_connect {
            let mut open = true;
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mri-viewer"));
            let args = export::mcp_args(self.folder.as_deref(), self.connect_show_patient);
            egui::Window::new(t!("app.connect.title")).id(egui::Id::unique("connect_window")).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).open(&mut open).collapsible(false).default_width(620.0).show(ctx, |ui| {
                ui.label(t!("app.connect.intro"));
                if self.folder.is_none() {
                    ui.colored_label(Color32::LIGHT_RED, t!("app.connect.no_folder"));
                }
                if exe.to_string_lossy().contains("AppTranslocation") {
                    ui.colored_label(Color32::LIGHT_RED, t!("app.connect.translocated"));
                }
                ui.checkbox(&mut self.connect_show_patient, t!("app.connect.share_patient"))
                    .on_hover_text(t!("app.connect.share_patient_tip"));
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Claude Desktop").strong());
                if ui.button(t!("app.connect.add_to_desktop")).clicked() {
                    self.connect_msg = Some(export::install_into_claude_desktop(&exe, &args).map_err(|e| format!("{e:#}")));
                }
                match &self.connect_msg {
                    Some(Ok(p)) => {
                        ui.label(t!("app.connect.added", path = p.display()));
                    }
                    Some(Err(e)) => {
                        ui.label(t!("app.connect.add_failed", error = e));
                    }
                    None => {}
                }
                ui.small(t!("app.connect.manual"));
                let snippet = export::desktop_snippet(&exe, &args);
                ui.add(egui::TextEdit::multiline(&mut snippet.as_str()).code_editor().desired_rows(8).desired_width(f32::INFINITY));
                if ui.button(t!("app.connect.copy")).clicked() {
                    ui.ctx().copy_text(snippet.clone());
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(if cfg!(windows) { t!("app.connect.claude_code_powershell") } else { t!("app.connect.claude_code_terminal") }).strong());
                let cmd = export::claude_code_command(&exe, &args);
                ui.add(egui::TextEdit::multiline(&mut cmd.as_str()).code_editor().desired_rows(2).desired_width(f32::INFINITY));
                if ui.button(t!("app.connect.copy_command")).clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
                ui.add_space(6.0);
                ui.small(t!("app.connect.privacy_note"));
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

/// Układ współrzędnych pacjenta serii: punkty w mm można porównywać tylko w obrębie jednego układu.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FrameKey {
    frame_of_reference: String,
    study: String,
}

impl FrameKey {
    /// Ten sam FrameOfReferenceUID; gdy brak go w którejś serii – to samo badanie.
    fn matches(&self, other: &FrameKey) -> bool {
        if !self.frame_of_reference.is_empty() && !other.frame_of_reference.is_empty() {
            self.frame_of_reference == other.frame_of_reference
        } else {
            self.study == other.study
        }
    }
}

/// Główna oś wektora (0 – x, 1 – y, 2 – z), jak w `mpr`.
fn dominant_axis(v: &[f64; 3]) -> usize {
    let a = [v[0].abs(), v[1].abs(), v[2].abs()];
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn plane_from_normal(v: &[f64; 3]) -> library::Plane {
    match dominant_axis(v) {
        0 => library::Plane::Sagittal,
        1 => library::Plane::Coronal,
        _ => library::Plane::Axial,
    }
}

/// Położenie rekonstrukcji `plane` przechodzącej przez punkt `p` (mm, LPS) – ta sama oś stała co w `mpr::reformat`.
fn reformat_index_for_point(series: &Series, plane: library::Plane, p: [f64; 3]) -> Option<usize> {
    let o = series.orientation?;
    let p0 = series.slices.first()?.position?;
    let r = [o[0], o[1], o[2]];
    let c = [o[3], o[4], o[5]];
    let (sr, sc) = series.pixel_spacing.unwrap_or((1.0, 1.0));
    let d = [p[0] - p0[0], p[1] - p0[1], p[2] - p0[2]];
    let (axis, sp, positions) = if plane_from_normal(&c) == plane {
        (c, sr, series.rows as usize)
    } else if plane_from_normal(&r) == plane {
        (r, sc, series.columns as usize)
    } else {
        return None;
    };
    if positions == 0 || sp <= 0.0 {
        return None;
    }
    let idx = (viewport::dot(d, axis) / sp).round();
    Some(idx.clamp(0.0, (positions - 1) as f64) as usize)
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
    ui.heading(t!("app.help.how_to_view"));
    ui.label(t!("app.help.step_open"));
    ui.label(t!("app.help.step_series"));
    ui.label(t!("app.help.step_scroll"));
    ui.add_space(6.0);
    ui.heading(t!("app.help.planes"));
    ui.label(t!("app.help.planes_text"));
    ui.add_space(6.0);
    ui.heading(t!("app.help.mouse"));
    egui::Grid::new("mouse").num_columns(2).striped(true).show(ui, |ui| {
        for (k, v) in [
            (t!("app.help.mouse.wheel"), t!("app.help.mouse.wheel_action")),
            (t!("app.help.mouse.zoom"), t!("app.help.mouse.zoom_action")),
            (t!("app.help.mouse.left"), t!("app.help.mouse.left_action")),
            (t!("app.help.mouse.drag_pan"), t!("app.help.mouse.drag_pan_action")),
            (t!("app.help.mouse.right_click"), t!("app.help.mouse.right_click_action")),
            (t!("app.help.mouse.double_click"), t!("app.help.mouse.double_click_action")),
            (t!("app.help.mouse.drag_measurement"), t!("app.help.mouse.drag_measurement_action")),
        ] {
            ui.label(egui::RichText::new(k).strong());
            ui.label(v);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.heading(t!("app.help.keyboard"));
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
        for (k, v) in [
            ("↑ ↓ ← →", t!("app.help.key.arrows")),
            ("PageUp / PageDown", t!("app.help.key.page")),
            ("Delete", t!("app.help.key.delete")),
            ("Esc", t!("app.help.key.escape")),
        ] {
            ui.monospace(k);
            ui.label(v);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.heading(t!("app.help.measurements"));
    ui.label(t!("app.help.measurements_text", tool = Tool::Line.label()));
    ui.add_space(6.0);
    ui.heading(t!("app.help.pro_mode"));
    ui.label(t!("app.help.pro_mode_text"));
    ui.add_space(6.0);
    ui.colored_label(Color32::from_rgb(230, 170, 80), t!("app.help.disclaimer"));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // ta klatka jest rysowana bez danych pacjenta – po jej narysowaniu można zrobić zrzut
        if self.shot_hide && self.pending_shot.is_none() && self.shot_sent.is_none_or(|t| t.elapsed().as_secs() >= 3) {
            self.shot_hide = false;
        }
        let drawn_hidden = self.shot_hide;
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
                self.status = t!("app.status.scan_done", studies = lib.studies.len(), series = lib.series_count());
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
            let mut j = self.job.lock().unwrap_or_else(|e| e.into_inner());
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
                ui.small(egui::RichText::new(t!("app.status.disclaimer")).color(Color32::from_rgb(230, 170, 80)));
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
                        ui.small(t!("app.status.measurement_count", count = n));
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
            let frame_keys: Vec<Option<FrameKey>> = (0..n).map(|vi| self.viewport_frame_key(vi)).collect();
            // przekrój, dla którego pobrano obraz okna
            let mut fetched: Vec<(u8, usize)> = Vec::with_capacity(n);
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
                fetched.push((self.viewports[vi].plane_code(), self.viewports[vi].position()));
                frames.push((series, f));
            }
            self.hover_status.clear();
            for (vi, (series, mut f)) in frames.into_iter().enumerate() {
                let r = Rect::from_min_size(
                    Pos2::new(full.min.x + cw * (vi % cols) as f32, full.min.y + rh * (vi / cols) as f32),
                    Vec2::new(cw - 2.0, rh - 2.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(("viewport", vi)));
                let Some(series) = series else {
                    let p = child.painter_at(r);
                    p.rect_filled(r, 0.0, Color32::BLACK);
                    p.text(r.center(), egui::Align2::CENTER_CENTER, t!("app.view.empty"), egui::FontId::proportional(15.0), Color32::GRAY);
                    if n > 1 {
                        let col = viewport::VIEW_COLORS[vi % 4];
                        p.rect_stroke(r.shrink(1.0), 0.0, Stroke::new(if vi == self.active { 3.0 } else { 1.0 }, col), egui::StrokeKind::Inside);
                    }
                    if child.allocate_rect(r, Sense::click()).clicked() {
                        self.active = vi;
                    }
                    continue;
                };
                // okno przesunięte przez wcześniejsze okno (synchronizacja, wskaźnik 3D) – pobieramy obraz ponownie
                if (self.viewports[vi].plane_code(), self.viewports[vi].position()) != fetched[vi] {
                    f = self.current_frame(vi, &series);
                    self.viewports[vi].plane_geom = f.as_ref().and_then(|x| x.2);
                    planes[vi] = self.viewports[vi].plane_geom;
                    fetched[vi] = (self.viewports[vi].plane_code(), self.viewports[vi].position());
                }
                let Some((frame, spacing, _)) = f else {
                    let p = child.painter_at(r);
                    p.rect_filled(r, 0.0, Color32::BLACK);
                    let msg = match self.errors.lock().unwrap().get(&series.id) {
                        Some(e) => t!("app.view.read_error", error = e),
                        None => t!("app.view.loading", done = self.loaded_count(&series), total = series.slices.len()),
                    };
                    p.text(r.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(15.0), Color32::GRAY);
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    continue;
                };
                let study = self.study_of(series.id);
                // linie odniesienia tylko z okien w tym samym układzie współrzędnych
                let same_frame = |oi: usize| match (&frame_keys[vi], &frame_keys[oi]) {
                    (Some(a), Some(b)) => a.matches(b),
                    _ => false,
                };
                let others: Vec<(ImagePlane, Color32)> = planes
                    .iter()
                    .enumerate()
                    .filter(|(oi, p)| *oi != vi && p.is_some() && same_frame(*oi))
                    .map(|(oi, p)| (p.unwrap(), viewport::VIEW_COLORS[oi % 4]))
                    .collect();
                let cursor3d = match (&self.cursor3d_frame, &frame_keys[vi]) {
                    (Some(a), Some(b)) if a.matches(b) => self.cursor3d,
                    _ => None,
                };
                let loaded = (self.loaded_count(&series), series.slices.len());
                let units: &'static str = if series.modality.eq_ignore_ascii_case("CT") { "HU" } else { "" };
                let patient_line = study.as_ref().map(|s| self.patient_line(s)).unwrap_or_default();
                let study_date = study.as_ref().map(|s| s.date.clone()).unwrap_or_default();
                // rekonstrukcja ma milimetry tylko przy znanym odstępie pikseli i przekrojów
                let has_spacing = series.pixel_spacing.is_some()
                    && (self.viewports[vi].plane == ViewPlane::Native || series.slice_gap().is_some());
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
                    cursor3d,
                    patient_line,
                    study_date,
                    units,
                    loaded,
                };
                let out = viewport::show(&mut child, &mut self.viewports[vi], input);
                // przekrój zmienił się w trakcie rysowania – tekstura pochodzi z poprzedniego obrazu, odświeżamy ją
                if (self.viewports[vi].plane_code(), self.viewports[vi].position()) != fetched[vi] {
                    self.viewports[vi].tex_key = None;
                    ctx.request_repaint();
                }
                if out.clicked {
                    self.active = vi;
                }
                if out.position_changed {
                    self.sync_from(vi);
                }
                if let Some(p) = out.cursor3d {
                    self.apply_cursor3d(vi, p);
                    // okna narysowane wcześniej w tej klatce pokażą nowy przekrój dopiero w następnej
                    ctx.request_repaint();
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
        if drawn_hidden {
            if let Some(purpose) = self.pending_shot.take() {
                self.send_view_shot(&ctx, purpose);
            }
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
            // język z ustawień nie jest jeszcze znany – App::new ustawia tytuł ponownie
            .with_title(t!("app.title"))
            .with_app_id("mri-viewer")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(app_icon())),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        // nazwa programu dla eframe wyznacza folder z zapisanymi ustawieniami – nie tłumaczymy jej
        "Przeglądarka MRI",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, folder)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Ikona programu (okno, Dock, pasek zadań).
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icon-256.png");

pub fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_default()
}

/// Lokalizator (zwiadowczy obraz do planowania badania) – zwykle mało przydatny do oglądania.
fn is_localizer(s: &Series) -> bool {
    let d = s.description.to_lowercase();
    ["loc", "scout", "survey", "plan", "3pl"].iter().any(|k| d.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Plane, SequenceInfo, Slice};

    fn key(fr: &str, study: &str) -> FrameKey {
        FrameKey { frame_of_reference: fr.into(), study: study.into() }
    }

    #[test]
    fn frame_key_compares_frame_of_reference_then_study() {
        assert!(key("1.2", "A").matches(&key("1.2", "B")));
        assert!(!key("1.2", "A").matches(&key("1.3", "A")));
        // brak UID w jednej z serii – decyduje badanie
        assert!(key("", "A").matches(&key("1.3", "A")));
        assert!(!key("", "A").matches(&key("", "B")));
    }

    /// Seria poprzeczna: wiersze wzdłuż y, kolumny wzdłuż x, przekroje co 2 mm wzdłuż z.
    fn axial_series(n: usize) -> Series {
        Series {
            id: 1,
            uid: "s".into(),
            number: None,
            description: String::new(),
            modality: "MR".into(),
            body_part: String::new(),
            plane: Plane::Axial,
            rows: 12,
            columns: 10,
            pixel_spacing: Some((0.5, 0.8)),
            slice_thickness: None,
            spacing_between_slices: None,
            orientation: Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            frame_of_reference: String::new(),
            variant: String::new(),
            may_have_burned_in_text: false,
            sequence: SequenceInfo::default(),
            series_date: String::new(),
            series_time: String::new(),
            slices: (0..n)
                .map(|k| Slice {
                    path: PathBuf::from(format!("{k}.dcm")),
                    frame: 0,
                    instance_number: None,
                    position: Some([-50.0, -60.0, 10.0 + 2.0 * k as f64]),
                    slice_location: None,
                })
                .collect(),
        }
    }

    #[test]
    fn reformat_index_for_point_matches_mpr_origin() {
        let s = axial_series(8);
        let frames: Vec<Arc<Frame>> = (0..8)
            .map(|_| {
                Arc::new(Frame {
                    width: 10,
                    height: 12,
                    pixels: Pixels::Gray(vec![0.0; 120]),
                    dicom_window: None,
                    dicom_windows: Vec::new(),
                    auto_window: (0.0, 1.0),
                    min: 0.0,
                    max: 0.0,
                    inverted: false,
                })
            })
            .collect();
        for plane in [Plane::Coronal, Plane::Sagittal] {
            for idx in [0, 3, 7] {
                let rf = mpr::reformat(&s, &frames, plane, Some(idx)).unwrap();
                let o = rf.origin.unwrap();
                assert_eq!(reformat_index_for_point(&s, plane, o), Some(idx), "{plane:?} {idx}");
            }
        }
        // płaszczyzna akwizycji nie jest rekonstrukcją
        assert_eq!(reformat_index_for_point(&s, Plane::Axial, [0.0; 3]), None);
        // punkt poza serią – skrajne położenie
        assert_eq!(reformat_index_for_point(&s, Plane::Coronal, [0.0, -500.0, 0.0]), Some(0));
        assert_eq!(reformat_index_for_point(&s, Plane::Coronal, [0.0, 500.0, 0.0]), Some(11));
        assert_eq!(reformat_index_for_point(&s, Plane::Sagittal, [-50.0 + 0.8 * 4.0, 0.0, 0.0]), Some(4));
    }
}
