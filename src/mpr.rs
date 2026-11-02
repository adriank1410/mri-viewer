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
    // stos z powtórzonymi położeniami (np. kilka ech) dałby prążkowany obraz
    if series.slices.len() < 8 || series.has_duplicate_positions() {
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
    if series.has_duplicate_positions() {
        bail!("kilka przekrojów leży w tym samym miejscu (np. różne echa) – rekonstrukcja niemożliwa");
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
    // pion: w przekroju poprzecznym oś przód–tył (y), góra = przód;
    // w pozostałych oś z największą składową głowa–stopy (z), góra = głowa
    let (vert, horiz, vflip) = if plane == Plane::Axial {
        let (v, h) = if vec_of(a1)[1].abs() >= vec_of(a2)[1].abs() { (a1, a2) } else { (a2, a1) };
        (v, h, vec_of(v)[1] < 0.0) // rosnący indeks idzie do przodu → odwracamy
    } else {
        let (v, h) = if vec_of(a1)[2].abs() >= vec_of(a2)[2].abs() { (a1, a2) } else { (a2, a1) };
        (v, h, vec_of(v)[2] > 0.0) // rosnący indeks idzie w górę ciała → odwracamy
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{SequenceInfo, Slice};
    use std::path::PathBuf;

    fn series(o: [f64; 6], n: usize) -> Series {
        let nrm = [o[1] * o[5] - o[2] * o[4], o[2] * o[3] - o[0] * o[5], o[0] * o[4] - o[1] * o[3]];
        Series {
            id: 1,
            uid: String::new(),
            number: None,
            description: String::new(),
            modality: "MR".into(),
            body_part: String::new(),
            plane: Plane::Unknown,
            rows: 4,
            columns: 4,
            pixel_spacing: Some((1.0, 1.0)),
            slice_thickness: None,
            spacing_between_slices: None,
            orientation: Some(o),
            frame_of_reference: String::new(),
            sequence: SequenceInfo::default(),
            series_date: String::new(),
            series_time: String::new(),
            variant: String::new(),
            may_have_burned_in_text: false,
            slices: (0..n)
                .map(|k| Slice {
                    path: PathBuf::new(),
                    frame: 0,
                    instance_number: None,
                    position: Some([nrm[0] * k as f64, nrm[1] * k as f64, nrm[2] * k as f64]),
                    slice_location: None,
                })
                .collect(),
        }
    }

    fn frames(n: usize) -> Vec<Arc<Frame>> {
        (0..n)
            .map(|_| {
                Arc::new(Frame {
                    width: 4,
                    height: 4,
                    pixels: Pixels::Gray(vec![0.0; 16]),
                    dicom_window: None,
                    dicom_windows: Vec::new(),
                    auto_window: (0.5, 1.0),
                    min: 0.0,
                    max: 1.0,
                    inverted: false,
                })
            })
            .collect()
    }

    fn assert_axial(o: [f64; 6]) {
        let se = series(o, 8);
        let rf = reformat(&se, &frames(8), Plane::Axial, None).unwrap();
        // góra = przód (w dół rośnie y), prawa strona obrazu = lewa pacjenta (+x)
        assert_eq!(rf.ydir, [0.0, 1.0, 0.0], "ydir dla {o:?}");
        assert_eq!(rf.xdir, [1.0, 0.0, 0.0], "xdir dla {o:?}");
    }

    #[test]
    fn axial_reformat_is_in_radiological_orientation() {
        assert_axial([1.0, 0.0, 0.0, 0.0, 0.0, -1.0]); // czołowa
        assert_axial([0.0, 1.0, 0.0, 0.0, 0.0, -1.0]); // strzałkowa
        assert_axial([0.0, -1.0, 0.0, 0.0, 0.0, -1.0]); // strzałkowa, odwrócone wiersze
    }

    #[test]
    fn coronal_reformat_has_head_on_top() {
        let se = series([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 8); // poprzeczna
        let rf = reformat(&se, &frames(8), Plane::Coronal, None).unwrap();
        assert_eq!(rf.ydir, [0.0, 0.0, -1.0]);
        assert_eq!(rf.xdir, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn duplicate_positions_block_reformat() {
        let mut se = series([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 8);
        let p = se.slices[0].position;
        se.slices[1].position = p;
        assert!(available_planes(&se).is_empty());
        assert!(reformat(&se, &frames(8), Plane::Coronal, None).is_err());
    }
}
