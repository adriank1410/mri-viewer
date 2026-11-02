//! Jedno okno podglądu obrazu: przekształcenia widoku, obsługa myszy, rysowanie nakładek.

use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};

use super::actions::Action;
use super::annotations::{self as ann, Hit, Key, Shape, Store};
use super::settings::{CineMode, Settings};
use crate::imaging::{self, Colormap, Frame};
use crate::library::{Plane, Series, fmt_date};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Window,
    Pan,
    Zoom,
    Line,
    Angle,
    Ellipse,
    Rect,
    Arrow,
    Probe,
    Cursor3d,
}

impl Tool {
    pub fn label(&self) -> &'static str {
        match self {
            Tool::Window => "Jasność/kontrast",
            Tool::Pan => "Przesuwanie",
            Tool::Zoom => "Lupa",
            Tool::Line => "Odległość",
            Tool::Angle => "Kąt",
            Tool::Ellipse => "Owal ROI",
            Tool::Rect => "Prostokąt ROI",
            Tool::Arrow => "Strzałka",
            Tool::Probe => "Sonda",
            Tool::Cursor3d => "Wskaźnik 3D",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Tool::Window => "Przeciągaj: w poziomie kontrast, w pionie jasność (W)",
            Tool::Pan => "Przeciągaj, aby przesunąć obraz; prawy przycisk przesuwa zawsze (P)",
            Tool::Zoom => "Przeciągaj w górę lub w dół, aby powiększyć; Cmd/Ctrl + kółko też działa (Z)",
            Tool::Line => "Przeciągnij od punktu do punktu, aby zmierzyć odległość w mm (M)",
            Tool::Angle => "Kliknij trzy punkty: ramię, wierzchołek, drugie ramię (A)",
            Tool::Ellipse => "Przeciągnij, aby zaznaczyć owalny obszar – pokaże średnią jasność i pole (E)",
            Tool::Rect => "Przeciągnij, aby zaznaczyć prostokątny obszar (O)",
            Tool::Arrow => "Przeciągnij od wskazywanego miejsca, potem wpisz opis (T)",
            Tool::Probe => "Kliknij, aby odczytać wartość piksela (V)",
            Tool::Cursor3d => "Kliknij punkt – pozostałe okna przejdą do przekroju przez ten punkt (X)",
        }
    }

    pub fn pro(&self) -> bool {
        matches!(self, Tool::Angle | Tool::Ellipse | Tool::Rect | Tool::Probe | Tool::Cursor3d)
    }

    pub fn cursor(&self, dragging: bool) -> CursorIcon {
        match self {
            Tool::Window => CursorIcon::AllScroll,
            Tool::Pan => {
                if dragging {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                }
            }
            Tool::Zoom => CursorIcon::ZoomIn,
            Tool::Probe => CursorIcon::Cell,
            _ => CursorIcon::Crosshair,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ViewPlane {
    Native,
    Reformat(Plane),
}

/// Obrót (wielokrotność 90° w prawo) i odbicia widoku.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct ViewTransform {
    pub rot: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl ViewTransform {
    pub fn apply(&self, v: Vec2) -> Vec2 {
        let mut v = Vec2::new(if self.flip_h { -v.x } else { v.x }, if self.flip_v { -v.y } else { v.y });
        for _ in 0..self.rot % 4 {
            v = Vec2::new(-v.y, v.x);
        }
        v
    }
    pub fn inverse(&self, v: Vec2) -> Vec2 {
        let mut v = v;
        for _ in 0..self.rot % 4 {
            v = Vec2::new(v.y, -v.x);
        }
        Vec2::new(if self.flip_h { -v.x } else { v.x }, if self.flip_v { -v.y } else { v.y })
    }
}

/// Płaszczyzna obrazu w przestrzeni pacjenta (LPS, mm).
/// `origin` to środek piksela (0, 0) jak w DICOM; we współrzędnych obrazu (tekstury) leży on w (0,5; 0,5).
#[derive(Clone, Copy, Debug)]
pub struct ImagePlane {
    pub origin: [f64; 3],
    pub xdir: [f64; 3],
    pub ydir: [f64; 3],
    /// mm na piksel wzdłuż x (kolumny) i y (wiersze)
    pub sx: f64,
    pub sy: f64,
    pub w: u32,
    pub h: u32,
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

impl ImagePlane {
    pub fn normal(&self) -> [f64; 3] {
        cross(self.xdir, self.ydir)
    }
    pub fn to_patient(&self, p: Pos2) -> [f64; 3] {
        let (x, y) = ((p.x as f64 - 0.5) * self.sx, (p.y as f64 - 0.5) * self.sy);
        [
            self.origin[0] + x * self.xdir[0] + y * self.ydir[0],
            self.origin[1] + x * self.xdir[1] + y * self.ydir[1],
            self.origin[2] + x * self.xdir[2] + y * self.ydir[2],
        ]
    }
    /// Rzut punktu na płaszczyznę: (współrzędne obrazu, odległość od płaszczyzny w mm).
    pub fn project(&self, p: [f64; 3]) -> (Pos2, f64) {
        let d = sub(p, self.origin);
        (
            Pos2::new((dot(d, self.xdir) / self.sx + 0.5) as f32, (dot(d, self.ydir) / self.sy + 0.5) as f32),
            dot(d, self.normal()),
        )
    }
    /// Linia przecięcia z inną płaszczyzną, przycięta do obrazu (współrzędne obrazu).
    pub fn intersection(&self, other: &ImagePlane) -> Option<[Pos2; 2]> {
        let n = other.normal();
        let a = self.sx * dot(n, self.xdir);
        let b = self.sy * dot(n, self.ydir);
        // przesunięcie o pół piksela: równanie dla współrzędnych tekstury, nie indeksów pikseli
        let d = dot(n, sub(self.origin, other.origin)) - 0.5 * (a + b);
        if a.abs() < 1e-6 && b.abs() < 1e-6 {
            return None;
        }
        let (w, h) = (self.w as f64, self.h as f64);
        let mut pts: Vec<Pos2> = Vec::new();
        // a*x + b*y + d = 0 na krawędziach prostokąta
        if b.abs() > 1e-9 {
            for x in [0.0, w] {
                let y = -(a * x + d) / b;
                if (0.0..=h).contains(&y) {
                    pts.push(Pos2::new(x as f32, y as f32));
                }
            }
        }
        if a.abs() > 1e-9 {
            for y in [0.0, h] {
                let x = -(b * y + d) / a;
                if (0.0..=w).contains(&x) {
                    pts.push(Pos2::new(x as f32, y as f32));
                }
            }
        }
        pts.dedup_by(|p, q| p.distance(*q) < 0.5);
        if pts.len() >= 2 { Some([pts[0], pts[1]]) } else { None }
    }
}

/// Odstęp między kolejnymi położeniami okna (mm, prostopadle do płaszczyzny):
/// dla przekrojów natywnych odstęp przekrojów, dla rekonstrukcji odstęp wzdłuż stałej osi serii.
fn depth_step(series: &Series, plane: ViewPlane, pl: &ImagePlane) -> f64 {
    let (sr, sc) = series.pixel_spacing.unwrap_or((1.0, 1.0));
    let gap = series.slice_gap().unwrap_or(match plane {
        ViewPlane::Native => 3.0,
        ViewPlane::Reformat(_) => sr.max(sc),
    });
    match (plane, series.orientation) {
        (ViewPlane::Reformat(_), Some(o)) => step_along(pl.normal(), o, (sr, sc, gap)),
        _ => gap,
    }
}

/// Odstęp wzdłuż osi serii (kolumny, wiersze, przekroje) najbliższej kierunkowi `n`.
/// `sp` – (wiersze, kolumny, przekroje) w mm.
fn step_along(n: [f64; 3], o: [f64; 6], sp: (f64, f64, f64)) -> f64 {
    let (r, c) = ([o[0], o[1], o[2]], [o[3], o[4], o[5]]);
    [(dot(n, r).abs(), sp.1), (dot(n, c).abs(), sp.0), (dot(n, cross(r, c)).abs(), sp.2)]
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
        .unwrap_or(sp.2)
}

/// Litery stron ciała dla kierunku w układzie LPS.
pub fn dir_letters(v: [f64; 3]) -> String {
    let mut comps: Vec<(f64, char)> = vec![
        (v[0], if v[0] > 0.0 { 'L' } else { 'R' }),
        (v[1], if v[1] > 0.0 { 'P' } else { 'A' }),
        (v[2], if v[2] > 0.0 { 'S' } else { 'I' }),
    ];
    comps.sort_by(|a, b| b.0.abs().partial_cmp(&a.0.abs()).unwrap());
    comps.iter().filter(|c| c.0.abs() > 0.3).take(2).map(|c| c.1).collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Drag {
    None,
    Window,
    Pan,
    Zoom,
    Handle(u64, usize),
    Body(u64),
    Creating,
    Cursor3d,
}

pub struct Viewport {
    pub series_id: Option<usize>,
    pub slice: usize,
    pub plane: ViewPlane,
    pub reformat_pos: usize,
    pub reformat_positions: usize,
    pub window: Option<(f32, f32)>,
    pub invert: bool,
    pub colormap: Colormap,
    pub zoom: f32,
    pub pan: Vec2,
    pub xf: ViewTransform,
    pub texture: Option<TextureHandle>,
    pub tex_key: Option<(usize, u8, usize, u32, u32, bool, Colormap, bool)>,
    pub playing: bool,
    pub play_dir: i64,
    pub play_accum: f64,
    pub scroll_accum: f32,
    /// Czy użytkownik już przewijał tę serię (wpływa na start odtwarzania).
    pub touched: bool,
    pub reformat_cache: Option<((usize, u8, usize), Arc<Frame>, (f64, f64), Option<ImagePlane>)>,
    pub drag: Drag,
    pub creating: Option<Shape>,
    pub angle_stage: u8,
    pub selected: Option<u64>,
    pub editing_text: Option<u64>,
    /// Stan obrazu sprzed przeciągania pomiaru (zapis do cofnięcia tylko przy zmianie).
    pub drag_before: Option<(Key, Vec<ann::Ann>)>,
    /// Stan obrazu sprzed edycji opisu istniejącej strzałki.
    pub edit_before: Option<(Key, Vec<ann::Ann>)>,
    pub context_target: Option<u64>,
    pub plane_geom: Option<ImagePlane>,
    pub rect: Rect,
}

impl Viewport {
    pub fn new() -> Self {
        Self {
            series_id: None,
            slice: 0,
            plane: ViewPlane::Native,
            reformat_pos: 0,
            reformat_positions: 0,
            window: None,
            invert: false,
            colormap: Colormap::Gray,
            zoom: 1.0,
            pan: Vec2::ZERO,
            xf: ViewTransform::default(),
            texture: None,
            tex_key: None,
            playing: false,
            play_dir: 1,
            play_accum: 0.0,
            scroll_accum: 0.0,
            touched: false,
            reformat_cache: None,
            drag: Drag::None,
            creating: None,
            angle_stage: 0,
            selected: None,
            editing_text: None,
            drag_before: None,
            edit_before: None,
            context_target: None,
            plane_geom: None,
            rect: Rect::NOTHING,
        }
    }

    pub fn reset_view(&mut self) {
        self.window = None;
        self.invert = false;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.xf = ViewTransform::default();
    }

    pub fn plane_code(&self) -> u8 {
        match self.plane {
            ViewPlane::Native => 0,
            ViewPlane::Reformat(Plane::Axial) => 1,
            ViewPlane::Reformat(Plane::Coronal) => 2,
            ViewPlane::Reformat(Plane::Sagittal) => 3,
            ViewPlane::Reformat(_) => 4,
        }
    }

    pub fn position(&self) -> usize {
        match self.plane {
            ViewPlane::Native => self.slice,
            ViewPlane::Reformat(_) => self.reformat_pos,
        }
    }

    pub fn key(&self) -> Option<Key> {
        Some((self.series_id?, self.plane_code(), self.position()))
    }

    pub fn count(&self, series: &Series) -> usize {
        match self.plane {
            ViewPlane::Native => series.slices.len(),
            ViewPlane::Reformat(_) => self.reformat_positions.max(1),
        }
    }

    pub fn set_position(&mut self, series: &Series, p: i64) {
        let n = self.count(series) as i64;
        let p = p.clamp(0, (n - 1).max(0)) as usize;
        // ten sam przekrój (np. przewijanie na końcu serii) – nie przerywamy rysowanego pomiaru
        if p == self.position() {
            return;
        }
        match self.plane {
            ViewPlane::Native => self.slice = p,
            ViewPlane::Reformat(_) => self.reformat_pos = p,
        }
        self.creating = None;
        self.angle_stage = 0;
        self.editing_text = None;
    }

    /// Kończy edycję opisu strzałki; zmianę opisu zapisuje do cofnięcia.
    /// Trzeba ją wywołać przed każdą inną zmianą pomiarów, inaczej historia cofania się pomiesza.
    pub fn finish_text_edit(&mut self, store: &mut Store) {
        self.editing_text = None;
        if let Some((k, before)) = self.edit_before.take() {
            store.commit_if_changed(k, before);
        }
    }

    /// Rozpoczyna edycję opisu istniejącej strzałki.
    fn start_text_edit(&mut self, store: &mut Store, key: Key, id: u64) {
        self.finish_text_edit(store);
        self.edit_before = Some((key, store.get(&key).to_vec()));
        self.editing_text = Some(id);
    }

    pub fn step(&mut self, series: &Series, delta: i64) {
        self.set_position(series, self.position() as i64 + delta);
    }

    pub fn toggle_play(&mut self, series: &Series) {
        self.playing = !self.playing;
        if self.playing {
            self.play_dir = 1;
            // pierwsze odtworzenie zaczyna od początku serii
            if !self.touched || self.position() + 1 >= self.count(series) {
                self.set_position(series, 0);
            }
            self.touched = true;
        }
    }
}

/// Dane potrzebne do narysowania okna.
pub struct ViewInput<'a> {
    pub series: &'a Series,
    pub frame: Arc<Frame>,
    /// (pion, poziom) mm na piksel
    pub spacing: (f64, f64),
    pub has_spacing: bool,
    pub settings: &'a Settings,
    pub tool: Tool,
    pub store: &'a mut Store,
    pub active: bool,
    pub multi: bool,
    pub index: usize,
    pub others: Vec<(ImagePlane, Color32)>,
    pub cursor3d: Option<[f64; 3]>,
    pub patient_line: String,
    pub study_date: String,
    pub units: &'static str,
    pub loaded: (usize, usize),
}

#[derive(Default)]
pub struct ViewOutput {
    pub clicked: bool,
    pub position_changed: bool,
    pub cursor3d: Option<[f64; 3]>,
    pub actions: Vec<Action>,
    pub status: Option<String>,
}

pub const VIEW_COLORS: [Color32; 4] = [
    Color32::from_rgb(80, 170, 255),
    Color32::from_rgb(255, 120, 200),
    Color32::from_rgb(120, 230, 120),
    Color32::from_rgb(255, 170, 60),
];

pub struct Mapping {
    pub center: Pos2,
    pub scale: f32,
    pub w: f32,
    pub h: f32,
    pub sx: f32,
    pub sy: f32,
    pub xf: ViewTransform,
}

impl Mapping {
    pub fn new(rect: Rect, vp: &Viewport, w: u32, h: u32, spacing: (f64, f64)) -> Self {
        let (sy, sx) = (spacing.0 as f32, spacing.1 as f32);
        let mut ext = Vec2::new(w as f32 * sx, h as f32 * sy);
        if vp.xf.rot % 2 == 1 {
            ext = Vec2::new(ext.y, ext.x);
        }
        let fit = (rect.width() / ext.x).min(rect.height() / ext.y);
        Self {
            center: rect.center() + vp.pan,
            scale: fit * vp.zoom,
            w: w as f32,
            h: h as f32,
            sx,
            sy,
            xf: vp.xf,
        }
    }
    pub fn to_screen(&self, p: Pos2) -> Pos2 {
        let c = Vec2::new((p.x - self.w / 2.0) * self.sx, (p.y - self.h / 2.0) * self.sy);
        self.center + self.xf.apply(c) * self.scale
    }
    pub fn to_image(&self, s: Pos2) -> Pos2 {
        let c = self.xf.inverse((s - self.center) / self.scale);
        Pos2::new(c.x / self.sx + self.w / 2.0, c.y / self.sy + self.h / 2.0)
    }
}

fn text_with_shadow(painter: &egui::Painter, pos: Pos2, align: egui::Align2, text: String, size: f32, color: Color32) {
    let font = egui::FontId::proportional(size);
    painter.text(pos + Vec2::new(1.0, 1.0), align, &text, font.clone(), Color32::from_black_alpha(200));
    painter.text(pos, align, text, font, color);
}

pub fn show(ui: &mut egui::Ui, vp: &mut Viewport, inp: ViewInput<'_>) -> ViewOutput {
    let mut out = ViewOutput::default();
    let ctx = ui.ctx().clone();
    let rect = ui.available_rect_before_wrap();
    vp.rect = rect;
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
        out.clicked = true;
    }
    if vp.editing_text.is_none() && vp.edit_before.is_some() {
        vp.finish_text_edit(inp.store);
    }
    let series = inp.series;
    let frame = inp.frame.clone();
    let settings = inp.settings;
    let color = settings.color();
    let font = settings.overlay_font;
    let sp_opt = if inp.has_spacing { Some(inp.spacing) } else { None };

    // --- kółko myszy i powiększanie ---------------------------------------------------------
    let hovered = resp.hovered();
    let (zoom_delta, cmd) = ui.input(|i| (i.zoom_delta(), i.modifiers.command || i.modifiers.ctrl));
    let mut slice_delta: i64 = 0;
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
                        vp.scroll_accum += d * settings.scroll_sensitivity;
                        let s = (vp.scroll_accum / 24.0).trunc();
                        vp.scroll_accum -= s * 24.0;
                        s as i64
                    }
                    egui::MouseWheelUnit::Line => {
                        (d.signum() * (d.abs() * settings.scroll_sensitivity).round().max(1.0)) as i64
                    }
                    egui::MouseWheelUnit::Page => d.signum() as i64 * 10,
                };
                slice_delta -= if settings.invert_scroll { -steps } else { steps };
            }
        }
        if zoom_delta != 1.0 {
            let old = vp.zoom;
            vp.zoom = (vp.zoom * zoom_delta).clamp(0.1, 40.0);
            if let Some(p) = resp.hover_pos() {
                let rel = p - (rect.center() + vp.pan);
                vp.pan += rel - rel * (vp.zoom / old);
            }
        }
    }
    if slice_delta != 0 {
        vp.step(series, slice_delta);
        vp.touched = true;
        out.position_changed = true;
    }

    // --- odtwarzanie ---------------------------------------------------------------------------
    if vp.playing {
        let dt = ui.input(|i| i.stable_dt) as f64;
        vp.play_accum += dt * settings.cine_fps as f64;
        let n = vp.count(series) as i64;
        while vp.play_accum >= 1.0 && n > 1 {
            vp.play_accum -= 1.0;
            let mut p = vp.position() as i64 + vp.play_dir;
            match settings.cine_mode {
                CineMode::Loop => {
                    if p >= n {
                        p = 0;
                    }
                }
                CineMode::Bounce => {
                    if p >= n || p < 0 {
                        vp.play_dir = -vp.play_dir;
                        p = vp.position() as i64 + vp.play_dir;
                    }
                }
            }
            vp.set_position(series, p);
            out.position_changed = true;
        }
        ctx.request_repaint();
    }

    // --- obraz i odwzorowanie ------------------------------------------------------------------
    let (dc, dw) = frame.default_window();
    let (mut wc, mut ww) = vp.window.unwrap_or((dc, dw));
    let map = Mapping::new(rect, vp, frame.width, frame.height, inp.spacing);
    let key = vp.key().unwrap_or((0, 0, 0));
    let anns: Vec<ann::Ann> = inp.store.get(&key).to_vec();
    let hover_img = resp.hover_pos().map(|p| map.to_image(p));
    // wnętrze ROI chwyta mysz tylko przy narzędziach ROI (owal, prostokąt) – pozostałe narzędzia
    // działają też wewnątrz obszaru; obrys i uchwyty chwytają zawsze
    let roi_tool = matches!(inp.tool, Tool::Ellipse | Tool::Rect);
    let sel = vp.selected;
    let grab_inside = move |id: u64| roi_tool && sel == Some(id);
    let click_inside = move |_: u64| roi_tool;

    // --- mysz: przeciąganie -----------------------------------------------------------------------
    let primary_down = resp.drag_started_by(egui::PointerButton::Primary);
    if primary_down {
        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).unwrap_or(rect.center());
        if vp.editing_text.is_some() {
            vp.finish_text_edit(inp.store);
        }
        let hit = if vp.creating.is_none() { ann::hit_test(&anns, press, &|p| map.to_screen(p), &grab_inside) } else { None };
        match hit {
            Some(Hit::Handle(id, i)) => {
                vp.drag_before = Some((key, anns.clone()));
                vp.selected = Some(id);
                vp.drag = Drag::Handle(id, i);
            }
            Some(Hit::Body(id)) => {
                vp.drag_before = Some((key, anns.clone()));
                vp.selected = Some(id);
                vp.drag = Drag::Body(id);
            }
            None => {
                let ip = map.to_image(press);
                vp.drag = match inp.tool {
                    Tool::Window => Drag::Window,
                    Tool::Pan => Drag::Pan,
                    Tool::Zoom => Drag::Zoom,
                    Tool::Cursor3d => Drag::Cursor3d,
                    Tool::Line => {
                        vp.creating = Some(Shape::Line([ip, ip]));
                        Drag::Creating
                    }
                    Tool::Ellipse => {
                        vp.creating = Some(Shape::Ellipse([ip, ip]));
                        Drag::Creating
                    }
                    Tool::Rect => {
                        vp.creating = Some(Shape::Rect([ip, ip]));
                        Drag::Creating
                    }
                    Tool::Arrow => {
                        vp.creating = Some(Shape::Arrow([ip, ip], String::new()));
                        Drag::Creating
                    }
                    Tool::Angle => {
                        if vp.creating.is_none() {
                            vp.creating = Some(Shape::Angle([ip, ip, ip]));
                            vp.angle_stage = 1;
                        }
                        Drag::Creating
                    }
                    Tool::Probe => Drag::None,
                };
                if vp.drag != Drag::Creating {
                    vp.selected = None;
                }
            }
        }
    }
    if resp.drag_started_by(egui::PointerButton::Secondary) || resp.drag_started_by(egui::PointerButton::Middle) {
        vp.drag = Drag::Pan;
    }
    let dragging = resp.dragged();
    let angle_stage = vp.angle_stage;
    if dragging {
        let d = resp.drag_delta();
        let cur_img = resp.interact_pointer_pos().map(|p| map.to_image(p));
        match vp.drag {
            Drag::Window => {
                // mapy parametryczne mają zakresy rzędu 0,001 – czułość i minimum zależne od zakresu
                let range = (frame.max - frame.min).max(frame.window_eps());
                let k = range / 600.0;
                ww = (ww + d.x * k).max(frame.window_eps());
                wc -= d.y * k;
                vp.window = Some((wc, ww));
            }
            Drag::Pan => vp.pan += d,
            Drag::Zoom => {
                let old = vp.zoom;
                vp.zoom = (vp.zoom * (1.0 - d.y * 0.01)).clamp(0.1, 40.0);
                vp.pan *= vp.zoom / old;
            }
            Drag::Handle(id, i) => {
                if let (Some(a), Some(p)) = (inp.store.find_mut(&key, id), cur_img) {
                    a.shape.set_point(i, p);
                }
            }
            Drag::Body(id) => {
                if let Some(a) = inp.store.find_mut(&key, id) {
                    let dimg = map.to_image(rect.center() + d) - map.to_image(rect.center());
                    a.shape.translate(dimg);
                }
            }
            Drag::Creating => {
                if let (Some(s), Some(p)) = (vp.creating.as_mut(), cur_img) {
                    match s {
                        Shape::Angle(_) => {
                            // wierzchołek przesuwamy tylko przed jego ustaleniem
                            if angle_stage < 2 {
                                s.set_point(1, p);
                            }
                            s.set_point(2, p);
                        }
                        _ => s.set_point(1, p),
                    }
                }
            }
            Drag::Cursor3d => {
                if let (Some(pl), Some(p)) = (vp.plane_geom, cur_img) {
                    out.cursor3d = Some(pl.to_patient(p));
                }
            }
            Drag::None => {}
        }
    }
    if resp.drag_stopped() {
        match vp.drag {
            Drag::Creating => {
                if let Some(s) = vp.creating.clone() {
                    let pts = s.points();
                    let long_enough = map.to_screen(pts[0]).distance(map.to_screen(pts[1])) > 4.0;
                    match s {
                        Shape::Angle(_) => {
                            if vp.angle_stage == 2 {
                                // przeciągnięcie przy trzecim punkcie kończy kąt jak kliknięcie
                                let id = inp.store.add(key, s);
                                vp.selected = Some(id);
                                vp.creating = None;
                                vp.angle_stage = 0;
                            } else if long_enough {
                                vp.angle_stage = 2;
                            }
                        }
                        Shape::Arrow(..) if long_enough => {
                            let id = inp.store.add(key, s);
                            vp.selected = Some(id);
                            vp.editing_text = Some(id);
                            vp.creating = None;
                        }
                        _ if long_enough => {
                            let id = inp.store.add(key, s);
                            vp.selected = Some(id);
                            vp.creating = None;
                        }
                        _ => {
                            vp.creating = None;
                        }
                    }
                }
            }
            Drag::Handle(..) | Drag::Body(_) => {
                if let Some((k, before)) = vp.drag_before.take() {
                    inp.store.commit_if_changed(k, before);
                }
            }
            _ => {}
        }
        vp.drag = Drag::None;
    }

    // --- mysz: kliknięcia ---------------------------------------------------------------------------
    if resp.clicked() {
        let p = resp.interact_pointer_pos().unwrap_or(rect.center());
        let ip = map.to_image(p);
        if vp.editing_text.is_some() {
            vp.finish_text_edit(inp.store);
        }
        let hit = if vp.creating.is_none() { ann::hit_test(&anns, p, &|q| map.to_screen(q), &click_inside) } else { None };
        if let Some(Hit::Handle(id, _) | Hit::Body(id)) = hit {
            vp.selected = Some(id);
        } else {
            match inp.tool {
                Tool::Angle => match (&mut vp.creating, vp.angle_stage) {
                    (None, _) => {
                        vp.creating = Some(Shape::Angle([ip, ip, ip]));
                        vp.angle_stage = 1;
                    }
                    (Some(s), 1) => {
                        s.set_point(1, ip);
                        s.set_point(2, ip);
                        vp.angle_stage = 2;
                    }
                    (Some(s), _) => {
                        s.set_point(2, ip);
                        let s = s.clone();
                        let id = inp.store.add(key, s);
                        vp.selected = Some(id);
                        vp.creating = None;
                        vp.angle_stage = 0;
                    }
                },
                Tool::Probe => {
                    let id = inp.store.add(key, Shape::Probe(ip));
                    vp.selected = Some(id);
                }
                Tool::Cursor3d => {
                    if let Some(pl) = vp.plane_geom {
                        out.cursor3d = Some(pl.to_patient(ip));
                    }
                }
                _ => vp.selected = None,
            }
        }
    }
    // podążanie kąta za myszą między kliknięciami
    if let (Some(Shape::Angle(_)), Some(h)) = (&vp.creating, hover_img) {
        if vp.drag != Drag::Creating {
            let stage = vp.angle_stage;
            if let Some(s) = vp.creating.as_mut() {
                if stage == 1 {
                    s.set_point(1, h);
                    s.set_point(2, h);
                } else {
                    s.set_point(2, h);
                }
            }
        }
    }
    if resp.double_clicked() && vp.creating.is_none() {
        let p = resp.interact_pointer_pos().unwrap_or(rect.center());
        match ann::hit_test(&anns, p, &|q| map.to_screen(q), &grab_inside) {
            Some(Hit::Handle(id, _) | Hit::Body(id)) => {
                if matches!(inp.store.find(&key, id).map(|a| &a.shape), Some(Shape::Arrow(..))) {
                    vp.start_text_edit(inp.store, key, id);
                }
            }
            None => {
                vp.reset_view();
                (wc, ww) = (dc, dw);
            }
        }
    }
    if resp.secondary_clicked() {
        let p = resp.interact_pointer_pos().unwrap_or(rect.center());
        vp.context_target = match ann::hit_test(&anns, p, &|q| map.to_screen(q), &|_| true) {
            Some(Hit::Handle(id, _) | Hit::Body(id)) => {
                vp.selected = Some(id);
                Some(id)
            }
            None => None,
        };
    }

    // --- kursor ---------------------------------------------------------------------------------------
    if hovered || dragging {
        let icon = match vp.drag {
            Drag::Handle(..) | Drag::Body(_) => CursorIcon::Grabbing,
            Drag::Pan => CursorIcon::Grabbing,
            _ => match resp.hover_pos().and_then(|p| ann::hit_test(&anns, p, &|q| map.to_screen(q), &grab_inside)) {
                Some(Hit::Handle(..)) if vp.creating.is_none() => CursorIcon::Move,
                Some(Hit::Body(_)) if vp.creating.is_none() => CursorIcon::Grab,
                _ => inp.tool.cursor(dragging),
            },
        };
        ctx.set_cursor_icon(icon);
    }

    // --- tekstura ---------------------------------------------------------------------------------------
    let tkey = (
        series.id,
        vp.plane_code(),
        vp.position(),
        wc.to_bits(),
        ww.to_bits(),
        vp.invert,
        vp.colormap,
        settings.smooth_interpolation,
    );
    if vp.tex_key != Some(tkey) || vp.texture.is_none() {
        let rgba = imaging::render_rgba_cmap(&frame, wc, ww, vp.invert, vp.colormap);
        let img = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &rgba);
        let opts = if settings.smooth_interpolation { TextureOptions::LINEAR } else { TextureOptions::NEAREST };
        match &mut vp.texture {
            Some(t) => t.set(img, opts),
            None => vp.texture = Some(ctx.load_texture(format!("vp{}", inp.index), img, opts)),
        }
        vp.tex_key = Some(tkey);
    }
    if let Some(t) = &vp.texture {
        let corners = [
            Pos2::new(0.0, 0.0),
            Pos2::new(map.w, 0.0),
            Pos2::new(map.w, map.h),
            Pos2::new(0.0, map.h),
        ];
        let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
        let mut mesh = egui::Mesh::with_texture(t.id());
        for (c, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex { pos: map.to_screen(*c), uv, color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }

    // --- linie odniesienia i wskaźnik 3D --------------------------------------------------------------------
    if let Some(pl) = vp.plane_geom {
        if settings.show_reference_lines {
            for (other, col) in &inp.others {
                if dot(pl.normal(), other.normal()).abs() > 0.95 {
                    continue; // płaszczyzny równoległe
                }
                if let Some([a, b]) = pl.intersection(other) {
                    painter.add(egui::Shape::dashed_line(
                        &[map.to_screen(a), map.to_screen(b)],
                        Stroke::new(1.5, *col),
                        8.0,
                        5.0,
                    ));
                }
            }
        }
        if let Some(c) = inp.cursor3d {
            let (p, dist) = pl.project(c);
            let tol = (depth_step(series, vp.plane, &pl) * 0.75).max(0.25);
            let s = map.to_screen(p);
            let col = if dist.abs() <= tol { Color32::from_rgb(255, 80, 80) } else { Color32::from_rgba_unmultiplied(255, 80, 80, 90) };
            painter.circle_stroke(s, 9.0, Stroke::new(1.5, col));
            painter.line_segment([s - Vec2::new(16.0, 0.0), s - Vec2::new(4.0, 0.0)], Stroke::new(1.5, col));
            painter.line_segment([s + Vec2::new(4.0, 0.0), s + Vec2::new(16.0, 0.0)], Stroke::new(1.5, col));
            painter.line_segment([s - Vec2::new(0.0, 16.0), s - Vec2::new(0.0, 4.0)], Stroke::new(1.5, col));
            painter.line_segment([s + Vec2::new(0.0, 4.0), s + Vec2::new(0.0, 16.0)], Stroke::new(1.5, col));
        }
    }

    // --- pomiary ------------------------------------------------------------------------------------------
    let anns_now: Vec<ann::Ann> = inp.store.get(&key).to_vec();
    for a in &anns_now {
        ann::draw(&painter, &a.shape, &frame, sp_opt, inp.units, &|p| map.to_screen(p), color, vp.selected == Some(a.id), font);
    }
    if let Some(s) = &vp.creating {
        ann::draw(&painter, s, &frame, sp_opt, inp.units, &|p| map.to_screen(p), color, false, font);
    }
    // edycja opisu strzałki
    if let Some(id) = vp.editing_text {
        // gdy strzałki już nie ma, edycja też się kończy
        let mut done = true;
        if let Some(a) = inp.store.find_mut(&key, id) {
            if let Shape::Arrow(pts, text) = &mut a.shape {
                let at = map.to_screen(pts[1]) + Vec2::new(6.0, 2.0);
                done = false;
                egui::Area::new(egui::Id::unique(("arrow_text", inp.index, id)))
                    .fixed_pos(at)
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label("Opis (Enter – gotowe):");
                            let r = ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                            r.request_focus();
                            if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)) {
                                done = true;
                            }
                        });
                    });
            }
        }
        if done {
            vp.finish_text_edit(inp.store);
        }
    }

    // --- menu kontekstowe ---------------------------------------------------------------------------------
    let target = vp.context_target;
    resp.context_menu(|ui| {
        if let Some(id) = target {
            let shape = inp.store.find(&key, id).map(|a| a.shape.clone());
            if let Some(shape) = shape {
                ui.label(egui::RichText::new(shape.name()).strong());
                let result = ann::result_text(&shape, &frame, sp_opt, inp.units);
                if !result.is_empty() && ui.button("Kopiuj wynik").clicked() {
                    ui.ctx().copy_text(result.replace('\n', "; "));
                    ui.close();
                }
                if matches!(shape, Shape::Arrow(..)) && ui.button("Edytuj opis").clicked() {
                    vp.start_text_edit(&mut *inp.store, key, id);
                    ui.close();
                }
                if ui.button("Usuń ten pomiar").clicked() {
                    vp.finish_text_edit(&mut *inp.store);
                    inp.store.delete(key, id);
                    vp.selected = None;
                    ui.close();
                }
                ui.separator();
            }
        }
        for a in [
            Action::ResetView,
            Action::PresetAuto,
            Action::PresetDicom,
            Action::Invert,
            Action::RotateRight,
            Action::FlipH,
        ] {
            if ui.button(a.label()).clicked() {
                out.actions.push(a);
                ui.close();
            }
        }
        ui.separator();
        for a in [Action::DeleteSliceAnnotations, Action::SaveImage, Action::SaveView, Action::CopyView] {
            if ui.button(a.label()).clicked() {
                out.actions.push(a);
                ui.close();
            }
        }
    });

    // --- nakładki ------------------------------------------------------------------------------------------
    let white = Color32::from_rgb(235, 235, 235);
    if settings.show_overlay {
        let n = vp.count(series);
        let mut tl = String::new();
        if !inp.patient_line.is_empty() {
            tl.push_str(&inp.patient_line);
            tl.push('\n');
        }
        tl.push_str(&format!(
            "{}{}\n",
            series.number.map(|n| format!("Seria {n}: ")).unwrap_or_default(),
            if series.description.is_empty() { "(bez opisu)" } else { &series.description }
        ));
        tl.push_str(&match vp.plane {
            ViewPlane::Native => format!("Płaszczyzna {}", series.plane.pl()),
            ViewPlane::Reformat(p) => format!("Rekonstrukcja: {}", p.pl()),
        });
        tl.push_str(&format!("\nObraz {} / {}", vp.position() + 1, n));
        if vp.plane == ViewPlane::Native {
            if let Some(l) = series.slices.get(vp.slice).and_then(|s| s.slice_location) {
                tl.push_str(&format!("  ({l:.1} mm)"));
            }
        }
        text_with_shadow(&painter, rect.min + Vec2::new(8.0, 8.0), egui::Align2::LEFT_TOP, tl, font, white);

        let mut tr = format!("W {:.0}  L {:.0}", ww, wc);
        if vp.invert {
            tr.push_str("  negatyw");
        }
        if vp.colormap != Colormap::Gray {
            tr.push_str(&format!("\n{}", vp.colormap.label()));
        }
        tr.push_str(&format!("\nPowiększenie {:.0}%", vp.zoom * 100.0));
        if !inp.study_date.is_empty() {
            tr.push_str(&format!("\n{}", fmt_date(&inp.study_date)));
        }
        text_with_shadow(&painter, Pos2::new(rect.max.x - 8.0, rect.min.y + 8.0), egui::Align2::RIGHT_TOP, tr, font, white);

        let q = &series.sequence;
        let mut bl = Vec::new();
        if let Some(t) = series.slice_thickness {
            bl.push(format!("Grubość warstwy {t:.1} mm"));
        }
        if let (Some(tr), Some(te)) = (q.repetition_time, q.echo_time) {
            let mut s = format!("TR {tr:.0}  TE {te:.0}");
            if let Some(ti) = q.inversion_time {
                s.push_str(&format!("  TI {ti:.0}"));
            }
            bl.push(s);
        }
        if settings.pro_mode {
            bl.push(format!(
                "Matryca {}×{}{}",
                frame.width,
                frame.height,
                if inp.has_spacing {
                    format!(
                        "  FOV {:.0}×{:.0} mm",
                        frame.width as f64 * inp.spacing.1,
                        frame.height as f64 * inp.spacing.0
                    )
                } else {
                    String::new()
                }
            ));
            if !q.sequence_name.is_empty() || !q.scanning_sequence.is_empty() {
                bl.push(format!("{} {}", q.scanning_sequence, q.sequence_name).trim().to_string());
            }
        }
        if let Some(b) = q.magnetic_field_strength {
            bl.push(format!("{b:.1} T"));
        }
        text_with_shadow(&painter, Pos2::new(rect.min.x + 8.0, rect.max.y - 34.0), egui::Align2::LEFT_BOTTOM, bl.join("\n"), font, white);
        if inp.loaded.0 < inp.loaded.1 {
            text_with_shadow(
                &painter,
                Pos2::new(rect.max.x - 8.0, rect.max.y - 34.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("wczytano {}/{}", inp.loaded.0, inp.loaded.1),
                font,
                Color32::GRAY,
            );
        }
    }
    // oznaczenia stron ciała
    if settings.show_orientation {
        if let Some(pl) = vp.plane_geom {
            let to_patient_dir = |screen: Vec2| {
                let d = vp.xf.inverse(screen);
                [
                    pl.xdir[0] * d.x as f64 + pl.ydir[0] * d.y as f64,
                    pl.xdir[1] * d.x as f64 + pl.ydir[1] * d.y as f64,
                    pl.xdir[2] * d.x as f64 + pl.ydir[2] * d.y as f64,
                ]
            };
            let right = dir_letters(to_patient_dir(Vec2::new(1.0, 0.0)));
            let left = dir_letters(to_patient_dir(Vec2::new(-1.0, 0.0)));
            let down = dir_letters(to_patient_dir(Vec2::new(0.0, 1.0)));
            let up = dir_letters(to_patient_dir(Vec2::new(0.0, -1.0)));
            let c = Color32::from_rgb(255, 230, 140);
            let f = font + 2.0;
            text_with_shadow(&painter, Pos2::new(rect.min.x + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, left, f, c);
            text_with_shadow(&painter, Pos2::new(rect.max.x - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, right, f, c);
            text_with_shadow(&painter, Pos2::new(rect.center().x, rect.min.y + 8.0), egui::Align2::CENTER_TOP, up, f, c);
            text_with_shadow(&painter, Pos2::new(rect.center().x, rect.max.y - 34.0), egui::Align2::CENTER_BOTTOM, down, f, c);
        }
    }
    // podziałka
    if settings.show_scale_bar && inp.has_spacing {
        let px_per_mm = map.scale;
        let mut len_mm = 10.0;
        for l in [5.0, 10.0, 20.0, 50.0, 100.0, 200.0] {
            len_mm = l;
            if l * px_per_mm >= 60.0 {
                break;
            }
        }
        let len_px = len_mm * px_per_mm;
        let x1 = rect.max.x - 16.0;
        let y = rect.max.y - 56.0 - if inp.loaded.0 < inp.loaded.1 { 16.0 } else { 0.0 };
        let x0 = x1 - len_px;
        let st = Stroke::new(2.0, white);
        painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], st);
        painter.line_segment([Pos2::new(x0, y - 5.0), Pos2::new(x0, y + 5.0)], st);
        painter.line_segment([Pos2::new(x1, y - 5.0), Pos2::new(x1, y + 5.0)], st);
        text_with_shadow(&painter, Pos2::new((x0 + x1) / 2.0, y - 6.0), egui::Align2::CENTER_BOTTOM, format!("{len_mm:.0} mm"), font - 1.0, white);
    }
    // ramka aktywnego okna
    if inp.multi {
        let col = VIEW_COLORS[inp.index % VIEW_COLORS.len()];
        let w = if inp.active { 3.0 } else { 1.0 };
        painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(w, col), egui::StrokeKind::Inside);
    }

    // --- pasek przewijania na dole ---------------------------------------------------------------------------
    let n = vp.count(series);
    if n > 1 {
        let bar = Rect::from_min_max(Pos2::new(rect.min.x + 12.0, rect.max.y - 28.0), Pos2::new(rect.max.x - 12.0, rect.max.y - 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar));
        child.horizontal(|ui| {
            let resp = ui
                .add(egui::Button::new("").min_size(Vec2::new(26.0, 20.0)))
                .on_hover_text("Odtwarzaj / zatrzymaj (spacja)");
            // ikona rysowana, bo domyślne czcionki nie mają znaku pauzy
            let c = resp.rect.center();
            let col = ui.visuals().widgets.style(&resp).fg_stroke.color;
            if vp.playing {
                for dx in [-3.0, 3.0] {
                    ui.painter().rect_filled(Rect::from_center_size(c + Vec2::new(dx, 0.0), Vec2::new(3.0, 10.0)), 0.0, col);
                }
            } else {
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![c + Vec2::new(-4.0, -6.0), c + Vec2::new(6.0, 0.0), c + Vec2::new(-4.0, 6.0)],
                    col,
                    Stroke::NONE,
                ));
            }
            if resp.clicked() {
                vp.toggle_play(series);
            }
            let mut pos = vp.position() + 1;
            ui.spacing_mut().slider_width = (bar.width() - 110.0).max(60.0);
            if ui.add(egui::Slider::new(&mut pos, 1..=n).show_value(true)).changed() {
                vp.set_position(series, pos as i64 - 1);
                vp.touched = true;
                out.position_changed = true;
            }
        });
    }

    // --- pasek stanu: wartość piksela ------------------------------------------------------------------------
    if let Some(ip) = hover_img {
        if ip.x >= 0.0 && ip.y >= 0.0 && (ip.x as u32) < frame.width && (ip.y as u32) < frame.height {
            if let imaging::Pixels::Gray(v) = &frame.pixels {
                let val = v[(ip.y as u32 * frame.width + ip.x as u32) as usize];
                let mut s = format!("x {} y {}  wartość {:.0}{}", ip.x as u32, ip.y as u32, val, if inp.units.is_empty() { String::new() } else { format!(" {}", inp.units) });
                if settings.pro_mode {
                    if let Some(pl) = vp.plane_geom {
                        let p = pl.to_patient(ip);
                        s.push_str(&format!("   LPS ({:.1}, {:.1}, {:.1}) mm", p[0], p[1], p[2]));
                    }
                }
                out.status = Some(s);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axial() -> ImagePlane {
        ImagePlane { origin: [10.0, 20.0, 30.0], xdir: [1.0, 0.0, 0.0], ydir: [0.0, 1.0, 0.0], sx: 0.5, sy: 0.5, w: 100, h: 100 }
    }

    #[test]
    fn pixel_centre_maps_to_origin() {
        let pl = axial();
        // środek piksela (0, 0) to (0,5; 0,5) we współrzędnych tekstury
        let p = pl.to_patient(Pos2::new(0.5, 0.5));
        assert!((p[0] - 10.0).abs() < 1e-9 && (p[1] - 20.0).abs() < 1e-9);
        let (q, d) = pl.project([10.0 + 3.0 * 0.5, 20.0, 30.0]);
        assert!((q.x - 3.5).abs() < 1e-5 && (q.y - 0.5).abs() < 1e-5 && d.abs() < 1e-9);
        let back = pl.to_patient(q);
        assert!((back[0] - 11.5).abs() < 1e-6);
    }

    #[test]
    fn intersection_passes_through_pixel_centre() {
        let pl = axial();
        // płaszczyzna strzałkowa przez środek kolumny 4 (x = 12 mm)
        let sag = ImagePlane { origin: [12.0, 20.0, 30.0], xdir: [0.0, 1.0, 0.0], ydir: [0.0, 0.0, -1.0], sx: 0.5, sy: 1.0, w: 100, h: 50 };
        let [a, b] = pl.intersection(&sag).unwrap();
        assert!((a.x - 4.5).abs() < 1e-4 && (b.x - 4.5).abs() < 1e-4);
    }

    #[test]
    fn depth_step_uses_fixed_axis() {
        // stos osiowy: kolumny 0,9 mm, wiersze 0,8 mm, przekroje co 6 mm
        let o = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let sp = (0.8, 0.9, 6.0);
        assert_eq!(step_along([1.0, 0.0, 0.0], o, sp), 0.9); // strzałkowa
        assert_eq!(step_along([0.0, -1.0, 0.0], o, sp), 0.8); // czołowa
        assert_eq!(step_along([0.0, 0.0, 1.0], o, sp), 6.0);
    }
}
