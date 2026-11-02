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
    let dicom_window = match (
        floats(obj, tags::WINDOW_CENTER).and_then(|v| v.first().copied()),
        floats(obj, tags::WINDOW_WIDTH).and_then(|v| v.first().copied()),
    ) {
        (Some(c), Some(w)) if w > 1.0 => Some((c as f32, w as f32)),
        _ => None,
    };
    let auto_window = auto_window(&values, min, max);
    Ok(Frame {
        width,
        height,
        pixels: Pixels::Gray(values),
        dicom_window,
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
    let hi = pick(0.995).max(lo + 1.0);
    ((lo + hi) / 2.0, hi - lo)
}

/// Zamiana wartości na 0–255 zgodnie z oknem (środek, szerokość).
#[inline]
pub fn apply_window(v: f32, center: f32, width: f32, invert: bool) -> u8 {
    let lo = center - width / 2.0;
    let t = ((v - lo) / width.max(1e-6)).clamp(0.0, 1.0);
    let g = (t * 255.0 + 0.5) as u8;
    if invert { 255 - g } else { g }
}

/// Piksele w skali szarości (lub RGB) po zastosowaniu okna.
pub fn render_rgba(frame: &Frame, center: f32, width: f32, invert: bool) -> Vec<u8> {
    let n = (frame.width * frame.height) as usize;
    let mut out = Vec::with_capacity(n * 4);
    match &frame.pixels {
        Pixels::Gray(v) => {
            let inv = invert ^ frame.inverted;
            for &p in v {
                let g = apply_window(p, center, width, inv);
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        Pixels::Rgb(v) => {
            for px in v.chunks_exact(3) {
                if invert {
                    out.extend_from_slice(&[255 - px[0], 255 - px[1], 255 - px[2], 255]);
                } else {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
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

/// Lista elementów nagłówka DICOM jako tekst (bez danych binarnych i obrazu).
pub fn dump_tags(path: &Path, anonymize: bool) -> Result<String> {
    let obj = dicom_object::OpenFileOptions::new()
        .read_until(tags::PIXEL_DATA)
        .open_file(path)?;
    let dict = StandardDataDictionary;
    let mut out = String::new();
    let private_names = [
        tags::PATIENT_NAME,
        tags::PATIENT_ID,
        tags::PATIENT_BIRTH_DATE,
        tags::PATIENT_ADDRESS,
        tags::PATIENT_TELEPHONE_NUMBERS,
        tags::OTHER_PATIENT_NAMES,
        tags::PATIENT_MOTHER_BIRTH_NAME,
        tags::REFERRING_PHYSICIAN_NAME,
        tags::ACCESSION_NUMBER,
        tags::INSTITUTION_ADDRESS,
        tags::OPERATORS_NAME,
        tags::PERFORMING_PHYSICIAN_NAME,
        tags::NAME_OF_PHYSICIANS_READING_STUDY,
    ];
    for el in obj.iter() {
        let tag = el.tag();
        if tag.group() % 2 == 1 {
            continue; // prywatne tagi producenta
        }
        let name = dict
            .by_tag(tag)
            .map(|e| e.alias().to_string())
            .unwrap_or_else(|| "?".into());
        let value = if anonymize && private_names.contains(&tag) {
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

