//! Proste rekonstrukcje w innych płaszczyznach (MPR) ze stosu równoległych przekrojów.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::imaging::{Frame, Pixels};
use crate::library::{Plane, Series};

/// Wynik rekonstrukcji: obraz i odstęp pikseli (pion, poziom) w mm.
pub struct Reformat {
    pub frame: Frame,
    pub spacing: (f64, f64),
    /// Liczba możliwych położeń w tej płaszczyźnie.
    pub positions: usize,
    pub plane: Plane,
    /// Kierunki osi obrazu w układzie pacjenta (x rośnie w prawo, y w dół).
    pub xdir: [f64; 3],
    pub ydir: [f64; 3],
    /// Położenie piksela (0, 0) w mm (układ LPS), jeśli znane.
    pub origin: Option<[f64; 3]>,
}

#[derive(Clone, Copy)]
enum Axis {
    Col,   // indeks kolumny i (wektor r)
    Row,   // indeks wiersza j (wektor c)
    Slice, // indeks przekroju k (wektor n)
}

fn dominant(v: &[f64; 3]) -> usize {
    let a = [v[0].abs(), v[1].abs(), v[2].abs()];
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn plane_from_normal(v: &[f64; 3]) -> Plane {
    match dominant(v) {
        0 => Plane::Sagittal,
        1 => Plane::Coronal,
        _ => Plane::Axial,
    }
}

/// Płaszczyzny dostępne do rekonstrukcji dla danej serii (poza płaszczyzną akwizycji).
pub fn available_planes(series: &Series) -> Vec<Plane> {
    let Some(o) = series.orientation else { return vec![] };
    if series.slices.len() < 8 {
        return vec![];
    }
    let r = [o[0], o[1], o[2]];
    let c = [o[3], o[4], o[5]];
    vec![plane_from_normal(&c), plane_from_normal(&r)]
}

/// Rekonstrukcja: `index` – położenie w nowej płaszczyźnie (0..positions).
/// Gdy `index` jest None, bierzemy środek.
pub fn reformat(series: &Series, frames: &[Arc<Frame>], plane: Plane, index: Option<usize>) -> Result<Reformat> {
    let o = series
        .orientation
        .ok_or_else(|| anyhow!("seria nie ma informacji o orientacji, rekonstrukcja niemożliwa"))?;
    if frames.len() < 2 {
        bail!("za mało przekrojów do rekonstrukcji");
    }
    let r = [o[0], o[1], o[2]];
    let c = [o[3], o[4], o[5]];
    let n = [
        r[1] * c[2] - r[2] * c[1],
        r[2] * c[0] - r[0] * c[2],
        r[0] * c[1] - r[1] * c[0],
    ];
    let w = frames[0].width as usize;
    let h = frames[0].height as usize;
    let nslices = frames.len();
    let mut vols: Vec<&[f32]> = Vec::with_capacity(nslices);
    for f in frames {
        match &f.pixels {
            Pixels::Gray(v) if f.width as usize == w && f.height as usize == h => vols.push(v),
            _ => bail!("przekroje mają różne rozmiary albo są kolorowe"),
        }
    }
    let (sr, sc) = series.pixel_spacing.unwrap_or((1.0, 1.0));
    let gap = series.slice_gap().unwrap_or(sr.max(sc));

    // która oś jest stała
    let fixed = if plane_from_normal(&c) == plane {
        Axis::Row
    } else if plane_from_normal(&r) == plane {
        Axis::Col
    } else {
        bail!(
            "płaszczyzna {} to płaszczyzna akwizycji tej serii albo jest niedostępna – użyj zwykłego przeglądania przekrojów",
            plane.pl()
        );
    };
    let (a1, a2) = match fixed {
        Axis::Row => (Axis::Col, Axis::Slice),
        Axis::Col => (Axis::Row, Axis::Slice),
        Axis::Slice => unreachable!(),
    };
    let vec_of = |a: Axis| match a {
        Axis::Col => r,
        Axis::Row => c,
        Axis::Slice => n,
    };
    let len_of = |a: Axis| match a {
        Axis::Col => w,
        Axis::Row => h,
        Axis::Slice => nslices,
    };
    let sp_of = |a: Axis| match a {
        Axis::Col => sc,
        Axis::Row => sr,
        Axis::Slice => gap,
    };
    // pion: oś z największą składową głowa–stopy (z); góra = głowa
    let (vert, horiz) = if vec_of(a1)[2].abs() >= vec_of(a2)[2].abs() { (a1, a2) } else { (a2, a1) };
    let vflip = vec_of(vert)[2] > 0.0; // rosnący indeks idzie w górę ciała → odwracamy
    let hdir = vec_of(horiz);
    let hflip = hdir[dominant(&hdir)] < 0.0;

    let positions = len_of(fixed);
    let idx = index.unwrap_or(positions / 2).min(positions - 1);
    let ow = len_of(horiz);
    let oh = len_of(vert);
    let mut out = vec![0f32; ow * oh];
    for y in 0..oh {
        let vy = if vflip { oh - 1 - y } else { y };
        for x in 0..ow {
            let hx = if hflip { ow - 1 - x } else { x };
            let mut ijk = [0usize; 3];
            let mut set = |a: Axis, v: usize| match a {
                Axis::Col => ijk[0] = v,
                Axis::Row => ijk[1] = v,
                Axis::Slice => ijk[2] = v,
            };
            set(fixed, idx);
            set(vert, vy);
            set(horiz, hx);
            out[y * ow + x] = vols[ijk[2]][ijk[1] * w + ijk[0]];
        }
    }
    let (min, max) = out
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let base = &frames[frames.len() / 2];
    let sgn = |v: [f64; 3], neg: bool| if neg { [-v[0], -v[1], -v[2]] } else { v };
    let xdir = sgn(vec_of(horiz), hflip);
    let ydir = sgn(vec_of(vert), vflip);
    let origin = {
        let mut ijk = [0usize; 3];
        let mut set = |a: Axis, v: usize| match a {
            Axis::Col => ijk[0] = v,
            Axis::Row => ijk[1] = v,
            Axis::Slice => ijk[2] = v,
        };
        set(fixed, idx);
        set(vert, if vflip { oh - 1 } else { 0 });
        set(horiz, if hflip { ow - 1 } else { 0 });
        series.slices[ijk[2]].position.map(|p| {
            [
                p[0] + ijk[0] as f64 * sc * r[0] + ijk[1] as f64 * sr * c[0],
                p[1] + ijk[0] as f64 * sc * r[1] + ijk[1] as f64 * sr * c[1],
                p[2] + ijk[0] as f64 * sc * r[2] + ijk[1] as f64 * sr * c[2],
            ]
        })
    };
    Ok(Reformat {
        frame: Frame {
            width: ow as u32,
            height: oh as u32,
            pixels: Pixels::Gray(out),
            dicom_window: base.dicom_window,
            dicom_windows: base.dicom_windows.clone(),
            auto_window: base.auto_window,
            min,
            max,
            inverted: base.inverted,
        },
        spacing: (sp_of(vert), sp_of(horiz)),
        positions,
        plane,
        xdir,
        ydir,
        origin,
    })
}
