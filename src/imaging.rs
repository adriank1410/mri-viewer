//! Dekodowanie obrazów DICOM i zamiana na piksele do wyświetlenia (okno/poziom, PNG, mozaika).

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use dicom_core::dictionary::{DataDictionary, DataDictionaryEntry};
use dicom_core::header::Header;
use dicom_core::value::Value;
use dicom_core::VR;
use dicom_dictionary_std::{StandardDataDictionary, tags};
use dicom_object::{DefaultDicomObject, open_file};
use dicom_pixeldata::PixelDecoder;

use crate::library::{floats, text};

pub enum Pixels {
    /// Wartości po przeskalowaniu (Rescale Slope/Intercept).
    Gray(Vec<f32>),
    /// Obraz kolorowy RGB, 8 bitów na kanał.
    Rgb(Vec<u8>),
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    /// Okno zapisane w pliku przez aparat (środek, szerokość).
    pub dicom_window: Option<(f32, f32)>,
    /// Wszystkie okna zapisane w pliku (środek, szerokość, opis).
    pub dicom_windows: Vec<(f32, f32, String)>,
    /// Okno wyliczone automatycznie z rozkładu jasności.
    pub auto_window: (f32, f32),
    pub min: f32,
    pub max: f32,
    /// MONOCHROME1: większa wartość = ciemniej.
    pub inverted: bool,
}

impl Frame {
    pub fn default_window(&self) -> (f32, f32) {
        self.dicom_window.unwrap_or(self.auto_window)
    }

    /// Najmniejsza sensowna szerokość okna – zależna od zakresu wartości,
    /// bo mapy parametryczne mogą mieć wartości rzędu 0,001.
    pub fn window_eps(&self) -> f32 {
        window_eps(self.min, self.max)
    }
}

pub fn window_eps(min: f32, max: f32) -> f32 {
    let range = max - min;
    if range.is_finite() { (range * 1e-3).max(f32::MIN_POSITIVE) } else { f32::MIN_POSITIVE }
}

pub fn load_frame(path: &Path, frame: u32) -> Result<Frame> {
    let obj = open_file(path).with_context(|| format!("nie można otworzyć {}", path.display()))?;
    frame_from_object(&obj, frame)
}

pub fn frame_from_object(obj: &DefaultDicomObject, frame: u32) -> Result<Frame> {
    let decoded = obj
        .decode_pixel_data_frame(frame)
        .map_err(|e| anyhow!("nie udało się zdekodować obrazu: {e}"))?;
    let width = decoded.columns();
    let height = decoded.rows();
    let photometric = text(obj, tags::PHOTOMETRIC_INTERPRETATION).to_uppercase();
    if decoded.samples_per_pixel() >= 3 {
        let img = decoded
            .to_dynamic_image(0)
            .map_err(|e| anyhow!("nie udało się zamienić obrazu kolorowego: {e}"))?
            .to_rgb8();
        return Ok(Frame {
            width,
            height,
            pixels: Pixels::Rgb(img.into_raw()),
            dicom_window: None,
            dicom_windows: Vec::new(),
            auto_window: (127.5, 255.0),
            min: 0.0,
            max: 255.0,
            inverted: false,
        });
    }
    let values: Vec<f32> = decoded
        .to_vec_frame::<f32>(0)
        .map_err(|e| anyhow!("nie udało się odczytać pikseli: {e}"))?;
    let (min, max) = values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let centers = floats(obj, tags::WINDOW_CENTER).unwrap_or_default();
    let widths = floats(obj, tags::WINDOW_WIDTH).unwrap_or_default();
    let explanations: Vec<String> = text(obj, tags::WINDOW_CENTER_WIDTH_EXPLANATION)
        .split('\\')
        .map(|s| s.trim().to_string())
        .collect();
    let dicom_windows: Vec<(f32, f32, String)> = centers
        .iter()
        .zip(widths.iter())
        .enumerate()
        .filter(|(_, (_, w))| **w > 0.0)
        .map(|(i, (c, w))| (*c as f32, *w as f32, explanations.get(i).cloned().unwrap_or_default()))
        .collect();
    let dicom_window = dicom_windows.first().map(|(c, w, _)| (*c, *w));
    let auto_window = auto_window(&values, min, max);
    Ok(Frame {
        width,
        height,
        pixels: Pixels::Gray(values),
        dicom_window,
        dicom_windows,
        auto_window,
        min,
        max,
        inverted: photometric == "MONOCHROME1",
    })
}

/// Okno z percentyli 1–99,5% (z pominięciem tła o wartości minimalnej) – dobre dla MRI.
fn auto_window(values: &[f32], min: f32, max: f32) -> (f32, f32) {
    if !(max > min) {
        return (min, 1.0);
    }
    const BINS: usize = 4096;
    let scale = (BINS - 1) as f32 / (max - min);
    let mut hist = vec![0u32; BINS];
    for &v in values {
        hist[((v - min) * scale) as usize] += 1;
    }
    hist[0] = 0; // tło
    let total: u64 = hist.iter().map(|&c| c as u64).sum();
    if total == 0 {
        return ((min + max) / 2.0, max - min);
    }
    let pick = |p: f64| {
        let target = (total as f64 * p) as u64;
        let mut acc = 0u64;
        for (i, &c) in hist.iter().enumerate() {
            acc += c as u64;
            if acc >= target {
                return min + i as f32 / scale;
            }
        }
        max
    };
    let lo = pick(0.01);
    let hi = pick(0.995).max(lo + window_eps(min, max));
    ((lo + hi) / 2.0, hi - lo)
}

/// Zamiana wartości na 0–255 zgodnie z oknem (środek, szerokość).
#[inline]
pub fn apply_window(v: f32, center: f32, width: f32, invert: bool) -> u8 {
    let lo = center - width / 2.0;
    let t = ((v - lo) / width.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    let g = (t * 255.0 + 0.5) as u8;
    if invert { 255 - g } else { g }
}

/// Mapa kolorów dla obrazów w skali szarości.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Colormap {
    #[default]
    Gray,
    Hot,
    Bone,
    Rainbow,
    Cool,
}

impl Colormap {
    pub const ALL: [Colormap; 5] = [Colormap::Gray, Colormap::Hot, Colormap::Bone, Colormap::Rainbow, Colormap::Cool];

    pub fn label(&self) -> &'static str {
        match self {
            Colormap::Gray => "Skala szarości",
            Colormap::Hot => "Gorąca (żelazo)",
            Colormap::Bone => "Kość",
            Colormap::Rainbow => "Tęcza",
            Colormap::Cool => "Chłodna",
        }
    }

    /// Tablica 256 kolorów RGB.
    pub fn lut(&self) -> [[u8; 3]; 256] {
        let mut lut = [[0u8; 3]; 256];
        for (i, e) in lut.iter_mut().enumerate() {
            let t = i as f32 / 255.0;
            let (r, g, b) = match self {
                Colormap::Gray => (t, t, t),
                Colormap::Hot => ((t * 3.0).min(1.0), (t * 3.0 - 1.0).clamp(0.0, 1.0), (t * 3.0 - 2.0).clamp(0.0, 1.0)),
                Colormap::Bone => {
                    let r = if t < 0.75 { t * 0.875 } else { 0.65625 + (t - 0.75) * 1.375 };
                    let g = if t < 0.375 { t * 0.875 } else if t < 0.75 { 0.328125 + (t - 0.375) * 1.2083 } else { 0.78125 + (t - 0.75) * 0.875 };
                    let b = if t < 0.375 { t * 1.2083 } else { 0.453125 + (t - 0.375) * 0.875 };
                    (r.min(1.0), g.min(1.0), b.min(1.0))
                }
                Colormap::Rainbow => {
                    // niebieski → cyjan → zielony → żółty → czerwony
                    let h = (1.0 - t) * 240.0;
                    hsv(h, 1.0, if t == 0.0 { 0.0 } else { 1.0 })
                }
                Colormap::Cool => (t, 1.0 - t * 0.5, 1.0),
            };
            *e = [(r * 255.0 + 0.5) as u8, (g * 255.0 + 0.5) as u8, (b * 255.0 + 0.5) as u8];
        }
        lut
    }
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

/// Piksele RGBA po zastosowaniu okna (i ewentualnie mapy kolorów).

pub fn render_rgba_cmap(frame: &Frame, center: f32, width: f32, invert: bool, cmap: Colormap) -> Vec<u8> {
    use rayon::prelude::*;
    let n = (frame.width * frame.height) as usize;
    let mut out = vec![255u8; n * 4];
    match &frame.pixels {
        Pixels::Gray(v) => {
            let inv = invert ^ frame.inverted;
            let lut = cmap.lut();
            out.par_chunks_mut(4 * 4096)
                .zip(v.par_chunks(4096))
                .for_each(|(o, src)| {
                    for (px, &p) in o.chunks_exact_mut(4).zip(src) {
                        let c = lut[apply_window(p, center, width, inv) as usize];
                        px[0] = c[0];
                        px[1] = c[1];
                        px[2] = c[2];
                    }
                });
        }
        Pixels::Rgb(v) => {
            for (px, s) in out.chunks_exact_mut(4).zip(v.chunks_exact(3)) {
                for k in 0..3 {
                    px[k] = if invert { 255 - s[k] } else { s[k] };
                }
            }
        }
    }
    out
}

pub fn render_gray8(frame: &Frame, center: f32, width: f32, invert: bool) -> Vec<u8> {
    match &frame.pixels {
        Pixels::Gray(v) => {
            let inv = invert ^ frame.inverted;
            v.iter().map(|&p| apply_window(p, center, width, inv)).collect()
        }
        Pixels::Rgb(v) => v
            .chunks_exact(3)
            .map(|p| {
                let g = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8;
                if invert { 255 - g } else { g }
            })
            .collect(),
    }
}

/// Skalowanie obrazu w skali szarości (dwuliniowe), z uwzględnieniem różnych odstępów pikseli.
pub fn resize_gray(src: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let img = image::GrayImage::from_raw(w, h, src.to_vec()).expect("rozmiar bufora");
    image::imageops::resize(&img, nw.max(1), nh.max(1), image::imageops::FilterType::Triangle).into_raw()
}

pub fn encode_png_gray(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let img = image::GrayImage::from_raw(w, h, data.to_vec()).ok_or_else(|| anyhow!("zły rozmiar obrazu"))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}


/// Wymiary docelowe tak, by dłuższy bok miał `max_side` pikseli, z korektą proporcji
/// dla pikseli niekwadratowych (`pixel_spacing` = (wiersze, kolumny) w mm).
pub fn target_size(w: u32, h: u32, pixel_spacing: Option<(f64, f64)>, max_side: u32) -> (u32, u32) {
    let (sr, sc) = pixel_spacing.unwrap_or((1.0, 1.0));
    let phys_w = w as f64 * sc;
    let phys_h = h as f64 * sr;
    let scale = max_side as f64 / phys_w.max(phys_h);
    (
        ((phys_w * scale).round() as u32).max(1),
        ((phys_h * scale).round() as u32).max(1),
    )
}

// --- małe cyfry do podpisywania mozaiki -------------------------------------------------

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Rysuje liczbę w lewym górnym rogu (biały tekst na czarnym tle).
pub fn draw_number(buf: &mut [u8], stride: u32, x0: u32, y0: u32, n: usize, scale: u32) {
    let s = n.to_string();
    let cw = 4 * scale;
    let box_w = s.len() as u32 * cw + scale;
    let box_h = 7 * scale;
    for y in y0..y0 + box_h {
        for x in x0..x0 + box_w {
            if let Some(p) = buf.get_mut((y * stride + x) as usize) {
                *p = 0;
            }
        }
    }
    for (i, ch) in s.chars().enumerate() {
        let d = ch.to_digit(10).unwrap() as usize;
        for (row, bits) in DIGITS[d].iter().enumerate() {
            for col in 0..3u32 {
                if bits & (0b100 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let x = x0 + scale + i as u32 * cw + col * scale + dx;
                            let y = y0 + scale + row as u32 * scale + dy;
                            if let Some(p) = buf.get_mut((y * stride + x) as usize) {
                                *p = 255;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Mozaika przekrojów w skali szarości z numerami w rogach. Zwraca (piksele, szerokość, wysokość).
pub fn render_montage(
    frames: &[(&Frame, usize)],
    window: (f32, f32),
    invert: bool,
    spacing: Option<(f64, f64)>,
    tile: u32,
    cols: u32,
) -> (Vec<u8>, u32, u32) {
    let cols = cols.max(1);
    let rows = (frames.len() as u32).div_ceil(cols).max(1);
    let gap = 4u32;
    let canvas_w = cols * tile + (cols - 1) * gap;
    let canvas_h = rows * tile + (rows - 1) * gap;
    let mut canvas = vec![40u8; (canvas_w * canvas_h) as usize];
    let label_scale = (tile / 64).max(2);
    for (k, (frame, number)) in frames.iter().enumerate() {
        let gray = render_gray8(frame, window.0, window.1, invert);
        let (tw, th) = target_size(frame.width, frame.height, spacing, tile);
        let scaled = resize_gray(&gray, frame.width, frame.height, tw, th);
        let cx = (k as u32 % cols) * (tile + gap);
        let cy = (k as u32 / cols) * (tile + gap);
        let ox = cx + (tile - tw) / 2;
        let oy = cy + (tile - th) / 2;
        for y in 0..th {
            let dst = ((oy + y) * canvas_w + ox) as usize;
            canvas[dst..dst + tw as usize].copy_from_slice(&scaled[(y * tw) as usize..((y + 1) * tw) as usize]);
        }
        draw_number(&mut canvas, canvas_w, cx, cy, *number, label_scale);
    }
    (canvas, canvas_w, canvas_h)
}

/// Lista elementów nagłówka DICOM jako tekst (bez danych binarnych i obrazu).
pub fn dump_tags(path: &Path, anonymize: bool) -> Result<String> {
    let obj = dicom_object::OpenFileOptions::new()
        .read_until(tags::PIXEL_DATA)
        .open_file(path)?;
    let dict = StandardDataDictionary;
    let mut out = String::new();
    for el in obj.iter() {
        let tag = el.tag();
        if tag.group() % 2 == 1 {
            continue; // prywatne tagi producenta
        }
        let name = dict
            .by_tag(tag)
            .map(|e| e.alias().to_string())
            .unwrap_or_else(|| "?".into());
        let value = if anonymize && (is_identifying(tag) || el.vr() == VR::PN) {
            "[ukryte]".to_string()
        } else {
            match el.value() {
                Value::Sequence(seq) => format!("<sekwencja, {} elem.>", seq.items().len()),
                Value::PixelSequence(_) => continue,
                Value::Primitive(_) => {
                    if matches!(el.vr(), VR::OB | VR::OW | VR::OF | VR::OD | VR::UN) {
                        continue;
                    }
                    let s = el.to_str().map(|s| s.to_string()).unwrap_or_default();
                    if s.len() > 200 { format!("{}…", &s[..s.char_indices().nth(200).map(|x| x.0).unwrap_or(s.len())]) } else { s }
                }
            }
        };
        out.push_str(&format!(
            "({:04X},{:04X}) {}: {}\n",
            tag.group(),
            tag.element(),
            name,
            value.trim()
        ));
    }
    Ok(out)
}

/// Czy element nagłówka może identyfikować pacjenta, personel albo placówkę.
/// Z grupy pacjenta (0010) zostawiamy tylko płeć, wiek, wzrost i wagę.
pub fn is_identifying(tag: dicom_object::Tag) -> bool {
    let (g, e) = (tag.group(), tag.element());
    match g {
        0x0010 => !matches!(e, 0x0040 | 0x1010 | 0x1020 | 0x1030),
        0x0008 => matches!(
            e,
            0x0050 // AccessionNumber
                | 0x0080 // InstitutionName
                | 0x0081 // InstitutionAddress
                | 0x0082 // InstitutionCodeSequence
                | 0x0090 // ReferringPhysicianName
                | 0x0092 // ReferringPhysicianAddress
                | 0x0094 // ReferringPhysicianTelephoneNumbers
                | 0x0096 // ReferringPhysicianIdentificationSequence
                | 0x1010 // StationName
                | 0x1040 // InstitutionalDepartmentName
                | 0x1048 // PhysiciansOfRecord
                | 0x1049
                | 0x1050 // PerformingPhysicianName
                | 0x1052
                | 0x1060 // NameOfPhysiciansReadingStudy
                | 0x1062
                | 0x1070 // OperatorsName
                | 0x1072
        ),
        0x0020 => e == 0x0010, // StudyID
        // zlecenie, rejestracja wizyty, kroki procedury – numery zleceń, nazwiska
        0x0032 | 0x0038 | 0x0040 => true,
        _ => false,
    }
}

/// Statystyki wartości pikseli w prostokącie (współrzędne w pikselach obrazu).
pub fn region_stats(frame: &Frame, x: u32, y: u32, w: u32, h: u32) -> Option<(f64, f64, f32, f32, usize)> {
    let Pixels::Gray(v) = &frame.pixels else { return None };
    let x1 = (x + w).min(frame.width);
    let y1 = (y + h).min(frame.height);
    if x >= x1 || y >= y1 {
        return None;
    }
    let mut sum = 0f64;
    let mut sum2 = 0f64;
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    let mut n = 0usize;
    for yy in y..y1 {
        for xx in x..x1 {
            let p = v[(yy * frame.width + xx) as usize];
            sum += p as f64;
            sum2 += (p as f64) * (p as f64);
            mn = mn.min(p);
            mx = mx.max(p);
            n += 1;
        }
    }
    let mean = sum / n as f64;
    let sd = (sum2 / n as f64 - mean * mean).max(0.0).sqrt();
    Some((mean, sd, mn, mx, n))
}


#[cfg(test)]
mod tests {
    use super::*;
    use dicom_object::Tag;

    #[test]
    fn auto_window_handles_small_value_range() {
        let values: Vec<f32> = (0..1000).map(|i| i as f32 * 3e-6).collect();
        let (c, w) = auto_window(&values, 0.0, values[999]);
        assert!(w > 0.0 && w < 0.004, "szerokość {w}");
        assert!(c > 0.0 && c < 0.003, "środek {c}");
        // piksel w środku zakresu nie może być czarny
        assert!(apply_window(0.0015, c, w, false) > 100);
    }

    #[test]
    fn identifying_tags_are_hidden() {
        for t in [
            tags::PATIENT_NAME,
            tags::PATIENT_ID,
            tags::PATIENT_BIRTH_DATE,
            Tag(0x0010, 0x1000), // OtherPatientIDs (np. PESEL)
            Tag(0x0010, 0x1002), // OtherPatientIDsSequence
            tags::ISSUER_OF_PATIENT_ID,
            tags::PATIENT_ADDRESS,
            tags::INSTITUTION_NAME,
            tags::INSTITUTION_ADDRESS,
            tags::REFERRING_PHYSICIAN_NAME,
            tags::OPERATORS_NAME,
            tags::ACCESSION_NUMBER,
            tags::STATION_NAME,
            tags::STUDY_ID,
            tags::REQUESTING_PHYSICIAN,
        ] {
            assert!(is_identifying(t), "{t} powinien być ukryty");
        }
        for t in [
            tags::PATIENT_SEX,
            tags::PATIENT_AGE,
            tags::PATIENT_WEIGHT,
            tags::STUDY_DATE,
            tags::SERIES_DESCRIPTION,
            tags::REPETITION_TIME,
            tags::PIXEL_SPACING,
        ] {
            assert!(!is_identifying(t), "{t} nie powinien być ukryty");
        }
    }
}
