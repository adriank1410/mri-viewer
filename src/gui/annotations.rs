//! Pomiary i adnotacje na obrazach: kształty, wyniki, trafianie myszą, cofanie zmian.

use std::collections::HashMap;

use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};

use crate::imaging::{Frame, Pixels};

/// Klucz obrazu: (seria, płaszczyzna, położenie).
pub type Key = (usize, u8, usize);

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Line([Pos2; 2]),
    /// Kąt ABC z wierzchołkiem w B.
    Angle([Pos2; 3]),
    Ellipse([Pos2; 2]),
    Rect([Pos2; 2]),
    Arrow([Pos2; 2], String),
    Probe(Pos2),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ann {
    pub id: u64,
    pub shape: Shape,
}

impl Shape {
    pub fn points(&self) -> Vec<Pos2> {
        match self {
            Shape::Line(p) | Shape::Ellipse(p) | Shape::Rect(p) | Shape::Arrow(p, _) => p.to_vec(),
            Shape::Angle(p) => p.to_vec(),
            Shape::Probe(p) => vec![*p],
        }
    }

    pub fn set_point(&mut self, i: usize, v: Pos2) {
        match self {
            Shape::Line(p) | Shape::Ellipse(p) | Shape::Rect(p) | Shape::Arrow(p, _) => p[i.min(1)] = v,
            Shape::Angle(p) => p[i.min(2)] = v,
            Shape::Probe(p) => *p = v,
        }
    }

    pub fn translate(&mut self, d: Vec2) {
        let n = self.points().len();
        for i in 0..n {
            let p = self.points()[i];
            self.set_point(i, p + d);
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Shape::Line(_) => "Odległość",
            Shape::Angle(_) => "Kąt",
            Shape::Ellipse(_) => "Obszar owalny",
            Shape::Rect(_) => "Obszar prostokątny",
            Shape::Arrow(_, _) => "Strzałka",
            Shape::Probe(_) => "Sonda",
        }
    }
}

/// Odstęp pikseli w mm: (pion/wiersze, poziom/kolumny).
pub type Spacing = (f64, f64);

fn mm(a: Pos2, b: Pos2, sp: Spacing) -> f64 {
    let dx = (b.x - a.x) as f64 * sp.1;
    let dy = (b.y - a.y) as f64 * sp.0;
    (dx * dx + dy * dy).sqrt()
}

fn angle_deg(a: Pos2, b: Pos2, c: Pos2, sp: Spacing) -> f64 {
    let v1 = ((a.x - b.x) as f64 * sp.1, (a.y - b.y) as f64 * sp.0);
    let v2 = ((c.x - b.x) as f64 * sp.1, (c.y - b.y) as f64 * sp.0);
    let dot = v1.0 * v2.0 + v1.1 * v2.1;
    let n = (v1.0.hypot(v1.1) * v2.0.hypot(v2.1)).max(1e-9);
    (dot / n).clamp(-1.0, 1.0).acos().to_degrees()
}

pub struct RoiStats {
    pub mean: f64,
    pub sd: f64,
    pub min: f32,
    pub max: f32,
    pub n: usize,
}

/// Statystyki pikseli wewnątrz obszaru (owal albo prostokąt).
pub fn roi_stats(frame: &Frame, shape: &Shape) -> Option<RoiStats> {
    let Pixels::Gray(v) = &frame.pixels else { return None };
    let (a, b, ellipse) = match shape {
        Shape::Ellipse([a, b]) => (*a, *b, true),
        Shape::Rect([a, b]) => (*a, *b, false),
        _ => return None,
    };
    let x0 = a.x.min(b.x).max(0.0);
    let x1 = a.x.max(b.x).min(frame.width as f32);
    let y0 = a.y.min(b.y).max(0.0);
    let y1 = a.y.max(b.y).min(frame.height as f32);
    let (cx, cy) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let (rx, ry) = (((b.x - a.x) / 2.0).abs().max(0.5), ((b.y - a.y) / 2.0).abs().max(0.5));
    let (mut sum, mut sum2, mut mn, mut mx, mut n) = (0f64, 0f64, f32::INFINITY, f32::NEG_INFINITY, 0usize);
    let mut y = y0.floor() as i64;
    while (y as f32) < y1 {
        let mut x = x0.floor() as i64;
        while (x as f32) < x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = if ellipse {
                let dx = (px - cx) / rx;
                let dy = (py - cy) / ry;
                dx * dx + dy * dy <= 1.0
            } else {
                px >= x0 && px <= x1 && py >= y0 && py <= y1
            };
            if inside && x >= 0 && y >= 0 && (x as u32) < frame.width && (y as u32) < frame.height {
                let p = v[(y as u32 * frame.width + x as u32) as usize];
                sum += p as f64;
                sum2 += (p as f64) * (p as f64);
                mn = mn.min(p);
                mx = mx.max(p);
                n += 1;
            }
            x += 1;
        }
        y += 1;
    }
    if n == 0 {
        return None;
    }
    let mean = sum / n as f64;
    Some(RoiStats {
        mean,
        sd: (sum2 / n as f64 - mean * mean).max(0.0).sqrt(),
        min: mn,
        max: mx,
        n,
    })
}

fn pixel_value(frame: &Frame, p: Pos2) -> Option<f32> {
    let Pixels::Gray(v) = &frame.pixels else { return None };
    if p.x < 0.0 || p.y < 0.0 || p.x as u32 >= frame.width || p.y as u32 >= frame.height {
        return None;
    }
    Some(v[(p.y as u32 * frame.width + p.x as u32) as usize])
}

/// Wynik pomiaru jako tekst (może mieć kilka linii).
pub fn result_text(shape: &Shape, frame: &Frame, sp: Option<Spacing>, units: &str) -> String {
    let u = if units.is_empty() { String::new() } else { format!(" {units}") };
    match shape {
        Shape::Line([a, b]) => match sp {
            Some(sp) => format!("{:.1} mm", mm(*a, *b, sp)),
            None => format!("{:.1} px", a.distance(*b)),
        },
        Shape::Angle([a, b, c]) => format!("{:.1}°", angle_deg(*a, *b, *c, sp.unwrap_or((1.0, 1.0)))),
        Shape::Ellipse(_) | Shape::Rect(_) => match roi_stats(frame, shape) {
            Some(s) => {
                let area = sp
                    .map(|(r, c)| format!("{:.1} mm²", s.n as f64 * r * c))
                    .unwrap_or_else(|| format!("{} px", s.n));
                format!(
                    "Śr. {:.1}{u} ± {:.1}\nmin {:.0}  maks. {:.0}\npole {area}",
                    s.mean, s.sd, s.min, s.max
                )
            }
            None => "—".into(),
        },
        Shape::Arrow(_, t) => t.clone(),
        Shape::Probe(p) => match pixel_value(frame, *p) {
            Some(v) => format!("{v:.0}{u}"),
            None => "—".into(),
        },
    }
}

// --- rysowanie i trafianie ---------------------------------------------------------------------

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

pub fn ellipse_points(a: Pos2, b: Pos2, n: usize) -> Vec<Pos2> {
    let c = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let (rx, ry) = ((b.x - a.x) / 2.0, (b.y - a.y) / 2.0);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            Pos2::new(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Handle(u64, usize),
    Body(u64),
}

/// Szuka pomiaru pod kursorem (współrzędne ekranowe).
/// `inside(id)` – czy wnętrze obszaru ROI też chwyta mysz (inaczej tylko obrys i uchwyty).
pub fn hit_test(anns: &[Ann], p: Pos2, to_screen: &dyn Fn(Pos2) -> Pos2, inside: &dyn Fn(u64) -> bool) -> Option<Hit> {
    for a in anns.iter().rev() {
        for (i, h) in a.shape.points().iter().enumerate() {
            if to_screen(*h).distance(p) <= 8.0 {
                return Some(Hit::Handle(a.id, i));
            }
        }
    }
    for a in anns.iter().rev() {
        let hit = match &a.shape {
            Shape::Line([x, y]) | Shape::Arrow([x, y], _) => dist_to_segment(p, to_screen(*x), to_screen(*y)) <= 6.0,
            Shape::Angle([x, y, z]) => {
                dist_to_segment(p, to_screen(*x), to_screen(*y)) <= 6.0 || dist_to_segment(p, to_screen(*y), to_screen(*z)) <= 6.0
            }
            Shape::Rect([x, y]) | Shape::Ellipse([x, y]) => {
                let pts: Vec<Pos2> = if matches!(a.shape, Shape::Rect(_)) {
                    vec![*x, Pos2::new(y.x, x.y), *y, Pos2::new(x.x, y.y)]
                } else {
                    ellipse_points(*x, *y, 48)
                };
                let sp: Vec<Pos2> = pts.iter().map(|q| to_screen(*q)).collect();
                let near_edge = (0..sp.len()).any(|i| dist_to_segment(p, sp[i], sp[(i + 1) % sp.len()]) <= 6.0);
                near_edge || (inside(a.id) && point_in_polygon(p, &sp))
            }
            Shape::Probe(q) => to_screen(*q).distance(p) <= 8.0,
        };
        if hit {
            return Some(Hit::Body(a.id));
        }
    }
    None
}

fn point_in_polygon(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn label(painter: &egui::Painter, at: Pos2, text: &str, color: Color32, size: f32) {
    if text.is_empty() {
        return;
    }
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(size), color);
    let rect = egui::Rect::from_min_size(at, galley.size()).expand(3.0);
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(170));
    painter.galley(at, galley, color);
}

/// Rysuje pomiar. `selected` – pogrubienie i uchwyty.
pub fn draw(
    painter: &egui::Painter,
    ann: &Shape,
    frame: &Frame,
    sp: Option<Spacing>,
    units: &str,
    to_screen: &dyn Fn(Pos2) -> Pos2,
    color: Color32,
    selected: bool,
    font: f32,
) {
    let w = if selected { 2.5 } else { 1.6 };
    let stroke = Stroke::new(w, color);
    let pts: Vec<Pos2> = ann.points().iter().map(|p| to_screen(*p)).collect();
    let text = result_text(ann, frame, sp, units);
    match ann {
        Shape::Line(_) => {
            painter.line_segment([pts[0], pts[1]], stroke);
            for p in &pts {
                // krótkie poprzeczki na końcach
                let d = (pts[1] - pts[0]).normalized().rot90() * 5.0;
                painter.line_segment([*p - d, *p + d], stroke);
            }
            label(painter, pts[1] + Vec2::new(8.0, -8.0), &text, color, font);
        }
        Shape::Angle(_) => {
            painter.line_segment([pts[0], pts[1]], stroke);
            painter.line_segment([pts[1], pts[2]], stroke);
            label(painter, pts[1] + Vec2::new(10.0, 4.0), &text, color, font);
        }
        Shape::Rect(_) => {
            let r = egui::Rect::from_two_pos(pts[0], pts[1]);
            let corners = [ann.points()[0], Pos2::new(ann.points()[1].x, ann.points()[0].y), ann.points()[1], Pos2::new(ann.points()[0].x, ann.points()[1].y)];
            let sc: Vec<Pos2> = corners.iter().map(|p| to_screen(*p)).collect();
            painter.add(egui::Shape::closed_line(sc, stroke));
            label(painter, Pos2::new(r.max.x + 8.0, r.min.y), &text, color, font);
        }
        Shape::Ellipse(_) => {
            let p = ann.points();
            let sc: Vec<Pos2> = ellipse_points(p[0], p[1], 64).iter().map(|q| to_screen(*q)).collect();
            let r = egui::Rect::from_points(&sc);
            painter.add(egui::Shape::closed_line(sc, stroke));
            label(painter, Pos2::new(r.max.x + 8.0, r.min.y), &text, color, font);
        }
        Shape::Arrow(_, _) => {
            // grot w punkcie pierwszym
            let (tip, tail) = (pts[0], pts[1]);
            painter.line_segment([tail, tip], stroke);
            let dir = (tip - tail).normalized();
            let side = dir.rot90();
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip - dir * 12.0 + side * 6.0, tip - dir * 12.0 - side * 6.0],
                color,
                Stroke::NONE,
            ));
            label(painter, tail + Vec2::new(6.0, 2.0), &text, color, font);
        }
        Shape::Probe(_) => {
            let p = pts[0];
            painter.line_segment([p - Vec2::new(7.0, 0.0), p + Vec2::new(7.0, 0.0)], stroke);
            painter.line_segment([p - Vec2::new(0.0, 7.0), p + Vec2::new(0.0, 7.0)], stroke);
            label(painter, p + Vec2::new(8.0, 4.0), &text, color, font);
        }
    }
    if selected {
        for p in &pts {
            painter.rect_filled(egui::Rect::from_center_size(*p, Vec2::splat(7.0)), 1.0, Color32::WHITE);
            painter.rect_stroke(egui::Rect::from_center_size(*p, Vec2::splat(7.0)), 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
        }
    } else {
        for p in &pts {
            painter.circle_filled(*p, 2.5, color);
        }
    }
}

// --- magazyn z cofaniem --------------------------------------------------------------------------

type Snapshot = Vec<(Key, Vec<Ann>)>;

#[derive(Default)]
pub struct Store {
    map: HashMap<Key, Vec<Ann>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    next_id: u64,
}

impl Store {
    pub fn get(&self, key: &Key) -> &[Ann] {
        self.map.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn find(&self, key: &Key, id: u64) -> Option<&Ann> {
        self.get(key).iter().find(|a| a.id == id)
    }

    pub fn find_mut(&mut self, key: &Key, id: u64) -> Option<&mut Ann> {
        self.map.get_mut(key)?.iter_mut().find(|a| a.id == id)
    }

    /// Zapamiętuje stan obrazu przed zmianą (do cofnięcia).
    pub fn checkpoint(&mut self, key: Key) {
        self.push_snapshot(key, self.get(&key).to_vec());
    }

    /// Zapisuje do cofnięcia stan obrazu sprzed zmiany, zapamiętany wcześniej.
    pub fn push_snapshot(&mut self, key: Key, before: Vec<Ann>) {
        self.undo.push(vec![(key, before)]);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Jak `push_snapshot`, ale tylko gdy obraz rzeczywiście się zmienił. Zwraca, czy zapisano.
    pub fn commit_if_changed(&mut self, key: Key, before: Vec<Ann>) -> bool {
        if self.get(&key) == before.as_slice() {
            return false;
        }
        self.push_snapshot(key, before);
        true
    }

    pub fn add(&mut self, key: Key, shape: Shape) -> u64 {
        self.checkpoint(key);
        self.next_id += 1;
        let id = self.next_id;
        self.map.entry(key).or_default().push(Ann { id, shape });
        id
    }

    pub fn delete(&mut self, key: Key, id: u64) -> bool {
        if self.find(&key, id).is_none() {
            return false;
        }
        self.checkpoint(key);
        if let Some(v) = self.map.get_mut(&key) {
            v.retain(|a| a.id != id);
        }
        true
    }

    pub fn clear_key(&mut self, key: Key) {
        if self.get(&key).is_empty() {
            return;
        }
        self.checkpoint(key);
        self.map.remove(&key);
    }

    pub fn clear_all(&mut self) {
        if self.map.values().all(|v| v.is_empty()) {
            return;
        }
        let snap: Snapshot = self.map.drain().collect();
        self.undo.push(snap);
        self.redo.clear();
    }

    /// Usuwa wszystko bez możliwości cofnięcia (np. przy zamknięciu badania).
    pub fn reset(&mut self) {
        self.map.clear();
        self.undo.clear();
        self.redo.clear();
    }

    fn swap(&mut self, snap: Snapshot) -> Snapshot {
        let mut current: Snapshot = Vec::new();
        for (k, v) in snap {
            current.push((k, self.map.remove(&k).unwrap_or_default()));
            if !v.is_empty() {
                self.map.insert(k, v);
            }
        }
        current
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        let cur = self.swap(s);
        self.redo.push(cur);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        let cur = self.swap(s);
        self.undo.push(cur);
        true
    }

    pub fn count(&self) -> usize {
        self.map.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_add_delete() {
        let mut s = Store::default();
        let k = (1, 0, 5);
        let id = s.add(k, Shape::Line([Pos2::ZERO, Pos2::new(10.0, 0.0)]));
        assert_eq!(s.get(&k).len(), 1);
        assert!(s.delete(k, id));
        assert_eq!(s.get(&k).len(), 0);
        assert!(s.undo());
        assert_eq!(s.get(&k).len(), 1);
        assert!(s.undo());
        assert_eq!(s.get(&k).len(), 0);
        assert!(s.redo());
        assert_eq!(s.get(&k).len(), 1);
    }

    #[test]
    fn undo_clear_all() {
        let mut s = Store::default();
        s.add((1, 0, 1), Shape::Probe(Pos2::ZERO));
        s.add((2, 0, 3), Shape::Probe(Pos2::ZERO));
        s.clear_all();
        assert_eq!(s.count(), 0);
        assert!(s.undo());
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn commit_only_when_changed() {
        let mut s = Store::default();
        let k = (1, 0, 0);
        let id = s.add(k, Shape::Line([Pos2::ZERO, Pos2::new(10.0, 0.0)]));
        assert!(s.undo());
        assert!(s.redo());
        // przeciągnięcie bez zmiany nie dotyka historii (redo zostaje)
        assert!(s.undo());
        let before = s.get(&k).to_vec();
        assert!(!s.commit_if_changed(k, before));
        assert!(s.redo());
        // zmiana jest zapisywana i daje się cofnąć
        let before = s.get(&k).to_vec();
        s.find_mut(&k, id).unwrap().shape.set_point(1, Pos2::new(20.0, 0.0));
        assert!(s.commit_if_changed(k, before));
        assert!(s.undo());
        assert_eq!(s.get(&k)[0].shape.points()[1], Pos2::new(10.0, 0.0));
    }

    #[test]
    fn roi_interior_hit_only_when_allowed() {
        let anns = vec![Ann { id: 7, shape: Shape::Ellipse([Pos2::ZERO, Pos2::new(100.0, 100.0)]) }];
        let id = |p: Pos2| p;
        let centre = Pos2::new(50.0, 50.0);
        assert_eq!(hit_test(&anns, centre, &id, &|_| false), None);
        assert_eq!(hit_test(&anns, centre, &id, &|i| i == 7), Some(Hit::Body(7)));
        // obrys chwyta zawsze
        assert_eq!(hit_test(&anns, Pos2::new(50.0, 1.0), &id, &|_| false), Some(Hit::Body(7)));
    }

    #[test]
    fn distances_and_angles() {
        assert!((mm(Pos2::ZERO, Pos2::new(10.0, 0.0), (0.5, 0.7)) - 7.0).abs() < 1e-6);
        let a = angle_deg(Pos2::new(10.0, 0.0), Pos2::ZERO, Pos2::new(0.0, 10.0), (1.0, 1.0));
        assert!((a - 90.0).abs() < 1e-6);
    }
}
