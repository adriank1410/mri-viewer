#![allow(dead_code)]
//! Skanowanie folderu z badaniami (np. kopii płyty CD z plikiem DICOMDIR)
//! i grupowanie obrazów w badania i serie.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dicom_dictionary_std::tags;
use dicom_object::{DefaultDicomObject, OpenFileOptions, Tag};
use rayon::prelude::*;

/// Jeden obraz do wyświetlenia: plik i numer klatki (pliki wieloklatkowe dają kilka obrazów).
#[derive(Clone, Debug)]
pub struct Slice {
    pub path: PathBuf,
    pub frame: u32,
    pub instance_number: Option<i32>,
    pub position: Option<[f64; 3]>,
    pub slice_location: Option<f64>,
}

/// Seria obrazów o wspólnej orientacji i rozmiarze.
#[derive(Clone, Debug)]
pub struct Series {
    /// Numer kolejny w całej bibliotece (od 1), używany w MCP.
    pub id: usize,
    pub uid: String,
    pub number: Option<i32>,
    pub description: String,
    pub modality: String,
    pub body_part: String,
    pub plane: Plane,
    pub rows: u32,
    pub columns: u32,
    /// Odstęp pikseli w mm: (wiersze, kolumny).
    pub pixel_spacing: Option<(f64, f64)>,
    pub slice_thickness: Option<f64>,
    pub spacing_between_slices: Option<f64>,
    pub orientation: Option<[f64; 6]>,
    /// FrameOfReferenceUID – serie z tym samym UID mają wspólny układ współrzędnych pacjenta.
    pub frame_of_reference: String,
    pub sequence: SequenceInfo,
    pub series_date: String,
    pub series_time: String,
    /// Odmiana w obrębie jednej serii DICOM (np. „echo 2”, „b = 1000”, „t3” – faza dynamiczna, „#2” – kolejny stos),
    /// pusta gdy brak. Postać niezależna od języka, bo trafia też do opisu serii i do MCP.
    pub variant: String,
    /// Obrazy mogą mieć wpisane w piksele dane pacjenta (zrzut ekranu, skan dokumentu,
    /// BurnedInAnnotation = YES itp.) – nie wysyłamy ich do Claude w trybie anonimowym.
    pub may_have_burned_in_text: bool,
    pub slices: Vec<Slice>,
}

#[derive(Clone, Debug, Default)]
pub struct SequenceInfo {
    pub protocol_name: String,
    pub sequence_name: String,
    pub scanning_sequence: String,
    pub repetition_time: Option<f64>,
    pub echo_time: Option<f64>,
    pub inversion_time: Option<f64>,
    pub flip_angle: Option<f64>,
    pub magnetic_field_strength: Option<f64>,
    pub contrast_agent: String,
    pub image_type: String,
}

#[derive(Clone, Debug)]
pub struct Study {
    pub uid: String,
    pub patient_name: String,
    pub patient_id: String,
    pub birth_date: String,
    pub sex: String,
    pub age: String,
    pub date: String,
    pub description: String,
    pub institution: String,
    pub series: Vec<Series>,
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum Plane {
    Axial,
    Sagittal,
    Coronal,
    Oblique,
    Unknown,
}

impl Plane {
    /// Nazwa płaszczyzny w bieżącym języku interfejsu.
    pub fn label(&self) -> &'static str {
        match self {
            Plane::Axial => t!("lib.plane.axial"),
            Plane::Sagittal => t!("lib.plane.sagittal"),
            Plane::Coronal => t!("lib.plane.coronal"),
            Plane::Oblique => t!("lib.plane.oblique"),
            Plane::Unknown => t!("lib.plane.unknown"),
        }
    }
    /// Nazwa angielska, niezależna od języka interfejsu (serwer MCP, sortowanie).
    pub fn en(&self) -> &'static str {
        match self {
            Plane::Axial => "axial",
            Plane::Sagittal => "sagittal",
            Plane::Coronal => "coronal",
            Plane::Oblique => "oblique",
            Plane::Unknown => "unknown",
        }
    }
}

#[derive(Default)]
pub struct Library {
    pub root: PathBuf,
    pub studies: Vec<Study>,
    /// Liczba plików, których nie dało się odczytać jako obraz DICOM.
    pub skipped_files: usize,
}

impl Library {
    pub fn all_series(&self) -> impl Iterator<Item = (&Study, &Series)> {
        self.studies
            .iter()
            .flat_map(|st| st.series.iter().map(move |se| (st, se)))
    }

    pub fn series_by_id(&self, id: usize) -> Option<(&Study, &Series)> {
        self.all_series().find(|(_, s)| s.id == id)
    }

    pub fn series_count(&self) -> usize {
        self.studies.iter().map(|s| s.series.len()).sum()
    }
}

/// Dane odczytane z nagłówka jednego pliku.
#[derive(Clone)]
struct Header {
    path: PathBuf,
    frames: u32,
    study_uid: String,
    series_uid: String,
    study: StudyFields,
    series_number: Option<i32>,
    series_description: String,
    modality: String,
    body_part: String,
    rows: u32,
    columns: u32,
    pixel_spacing: Option<(f64, f64)>,
    slice_thickness: Option<f64>,
    spacing_between_slices: Option<f64>,
    orientation: Option<[f64; 6]>,
    frame_of_reference: String,
    position: Option<[f64; 3]>,
    slice_location: Option<f64>,
    instance_number: Option<i32>,
    sequence: SequenceInfo,
    series_date: String,
    series_time: String,
    echo_number: Option<i32>,
    b_value: Option<f64>,
    temporal_position: Option<i32>,
    acquisition_number: Option<i32>,
    burned_in: bool,
}

#[derive(Clone)]
struct StudyFields {
    patient_name: String,
    patient_id: String,
    birth_date: String,
    sex: String,
    age: String,
    date: String,
    description: String,
    institution: String,
}

pub fn text(obj: &DefaultDicomObject, tag: Tag) -> String {
    obj.element_opt(tag)
        .ok()
        .flatten()
        .and_then(|e| e.to_str().ok().map(|s| s.trim().trim_end_matches('\0').trim().to_string()))
        .unwrap_or_default()
}

pub fn float(obj: &DefaultDicomObject, tag: Tag) -> Option<f64> {
    obj.element_opt(tag)
        .ok()
        .flatten()
        .and_then(|e| e.to_float64().ok())
        .filter(|v| v.is_finite())
}

pub fn floats(obj: &DefaultDicomObject, tag: Tag) -> Option<Vec<f64>> {
    obj.element_opt(tag)
        .ok()
        .flatten()
        .and_then(|e| e.to_multi_float64().ok())
}

pub fn int(obj: &DefaultDicomObject, tag: Tag) -> Option<i32> {
    obj.element_opt(tag)
        .ok()
        .flatten()
        .and_then(|e| e.to_int::<i32>().ok())
}

/// Liczba z elementu, także prywatnego zapisanego bez VR (tekst ASCII albo 4-bajtowy float).
fn number_any(obj: &DefaultDicomObject, tag: Tag) -> Option<f64> {
    let el = obj.element_opt(tag).ok().flatten()?;
    if let Ok(v) = el.to_float64() {
        return v.is_finite().then_some(v);
    }
    let bytes = el.to_bytes().ok()?;
    let txt = String::from_utf8_lossy(&bytes);
    let first = txt.split('\\').next().unwrap_or("").trim().trim_end_matches('\0').trim();
    if let Ok(v) = first.parse::<f64>() {
        return v.is_finite().then_some(v);
    }
    if bytes.len() == 4 {
        let v = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
        return v.is_finite().then_some(v);
    }
    None
}

/// Wartość b dyfuzji: tag standardowy, a w razie braku prywatne tagi Siemens, GE i Philips.
fn b_value(obj: &DefaultDicomObject) -> Option<f64> {
    // prywatne pola producentów czytamy tylko, gdy blok należy do właściwego producenta
    let private = |group: u16, creator: &str, element: u16| {
        text(obj, Tag(group, 0x0010)).to_uppercase().contains(creator).then(|| number_any(obj, Tag(group, element))).flatten()
    };
    float(obj, tags::DIFFUSION_B_VALUE)
        .or_else(|| private(0x0019, "SIEMENS", 0x100C))
        .or_else(|| private(0x0043, "GEMS_PARM", 0x1039).map(|v| if v >= 1e9 { v - 1e9 } else { v }))
        .or_else(|| private(0x2001, "PHILIPS IMAGING", 0x1003))
        .filter(|v| (0.0..1e6).contains(v))
}

/// Czy obraz może zawierać dane pacjenta wpisane w piksele.
pub fn may_have_burned_in_text(
    burned_in_annotation: &str,
    sop_class: &str,
    modality: &str,
    image_type: &str,
    samples_per_pixel: i32,
) -> bool {
    if burned_in_annotation.trim().eq_ignore_ascii_case("YES") {
        return true;
    }
    let sop = sop_class.trim().trim_end_matches('\0');
    // Secondary Capture (1.1.7.x) oraz zdjęcia VL (1.1.77.x)
    if sop.starts_with("1.2.840.10008.5.1.4.1.1.7") {
        return true;
    }
    if matches!(modality.trim().to_uppercase().as_str(), "SC" | "OT" | "DOC" | "XC") {
        return true;
    }
    let it = image_type.to_uppercase();
    if ["SCREEN SAVE", "SCREENSAVE", "SCREEN_SAVE", "SCREENSHOT", "SCREEN CAPTURE"]
        .iter()
        .any(|k| it.contains(k))
    {
        return true;
    }
    // obrazy kolorowe z MRI to zwykle zrzuty ekranu albo raporty
    if samples_per_pixel >= 3 {
        return true;
    }
    // DERIVED\SECONDARY poza klasami MR/CT (rekonstrukcje z aparatu zwykle są bez napisów)
    let in_class = |base: &str| sop == base || sop.strip_prefix(base).is_some_and(|r| r.starts_with('.'));
    let scanner_image = in_class("1.2.840.10008.5.1.4.1.1.4") || in_class("1.2.840.10008.5.1.4.1.1.2");
    it.split('\\').any(|t| t.trim() == "SECONDARY") && !scanner_image
}

fn read_header(path: &Path) -> Option<Header> {
    let obj = OpenFileOptions::new()
        .read_until(tags::PIXEL_DATA)
        .open_file(path)
        .ok()?;
    // pomijamy sam DICOMDIR i obiekty bez obrazu (raporty, protokoły dawki itp.)
    if obj.meta().media_storage_sop_class_uid().trim_end_matches('\0') == "1.2.840.10008.1.3.10" {
        return None;
    }
    let rows = int(&obj, tags::ROWS)? as u32;
    let columns = int(&obj, tags::COLUMNS)? as u32;
    if rows == 0 || columns == 0 {
        return None;
    }
    let frames = int(&obj, tags::NUMBER_OF_FRAMES).unwrap_or(1).max(1) as u32;
    let ps = floats(&obj, tags::PIXEL_SPACING)
        .or_else(|| floats(&obj, tags::IMAGER_PIXEL_SPACING))
        .filter(|v| v.len() >= 2 && v[0] > 0.0 && v[1] > 0.0)
        .map(|v| (v[0], v[1]));
    let orientation = floats(&obj, tags::IMAGE_ORIENTATION_PATIENT)
        .filter(|v| v.len() == 6)
        .map(|v| [v[0], v[1], v[2], v[3], v[4], v[5]]);
    let position = floats(&obj, tags::IMAGE_POSITION_PATIENT)
        .filter(|v| v.len() == 3)
        .map(|v| [v[0], v[1], v[2]]);
    let sop_class = {
        let m = obj.meta().media_storage_sop_class_uid().trim_end_matches('\0').trim().to_string();
        if m.is_empty() { text(&obj, tags::SOP_CLASS_UID) } else { m }
    };
    let modality = text(&obj, tags::MODALITY);
    let image_type = text(&obj, tags::IMAGE_TYPE);
    let burned_in = may_have_burned_in_text(
        &text(&obj, tags::BURNED_IN_ANNOTATION),
        &sop_class,
        &modality,
        &image_type,
        int(&obj, tags::SAMPLES_PER_PIXEL).unwrap_or(1),
    );
    let mut study_uid = text(&obj, tags::STUDY_INSTANCE_UID);
    if study_uid.is_empty() {
        study_uid = "?".into();
    }
    let mut series_uid = text(&obj, tags::SERIES_INSTANCE_UID);
    if series_uid.is_empty() {
        series_uid = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    }
    Some(Header {
        path: path.to_path_buf(),
        frames,
        study_uid,
        series_uid,
        study: StudyFields {
            patient_name: text(&obj, tags::PATIENT_NAME).replace('^', " ").trim().to_string(),
            patient_id: text(&obj, tags::PATIENT_ID),
            birth_date: text(&obj, tags::PATIENT_BIRTH_DATE),
            sex: text(&obj, tags::PATIENT_SEX),
            age: text(&obj, tags::PATIENT_AGE),
            date: text(&obj, tags::STUDY_DATE),
            description: text(&obj, tags::STUDY_DESCRIPTION),
            institution: text(&obj, tags::INSTITUTION_NAME),
        },
        series_number: int(&obj, tags::SERIES_NUMBER),
        series_description: {
            let d = text(&obj, tags::SERIES_DESCRIPTION);
            if d.is_empty() { text(&obj, tags::PROTOCOL_NAME) } else { d }
        },
        modality,
        body_part: text(&obj, tags::BODY_PART_EXAMINED),
        rows,
        columns,
        pixel_spacing: ps,
        slice_thickness: float(&obj, tags::SLICE_THICKNESS),
        spacing_between_slices: float(&obj, tags::SPACING_BETWEEN_SLICES),
        orientation,
        position,
        slice_location: float(&obj, tags::SLICE_LOCATION),
        instance_number: int(&obj, tags::INSTANCE_NUMBER),
        sequence: SequenceInfo {
            protocol_name: text(&obj, tags::PROTOCOL_NAME),
            sequence_name: text(&obj, tags::SEQUENCE_NAME),
            scanning_sequence: text(&obj, tags::SCANNING_SEQUENCE),
            repetition_time: float(&obj, tags::REPETITION_TIME),
            echo_time: float(&obj, tags::ECHO_TIME),
            inversion_time: float(&obj, tags::INVERSION_TIME),
            flip_angle: float(&obj, tags::FLIP_ANGLE),
            magnetic_field_strength: float(&obj, tags::MAGNETIC_FIELD_STRENGTH),
            contrast_agent: text(&obj, tags::CONTRAST_BOLUS_AGENT),
            image_type,
        },
        series_date: text(&obj, tags::SERIES_DATE),
        series_time: text(&obj, tags::SERIES_TIME),
        frame_of_reference: text(&obj, tags::FRAME_OF_REFERENCE_UID),
        echo_number: int(&obj, tags::ECHO_NUMBERS),
        b_value: b_value(&obj),
        temporal_position: int(&obj, tags::TEMPORAL_POSITION_IDENTIFIER),
        acquisition_number: int(&obj, tags::ACQUISITION_NUMBER),
        burned_in,
    })
}

fn normal(o: &[f64; 6]) -> [f64; 3] {
    [
        o[1] * o[5] - o[2] * o[4],
        o[2] * o[3] - o[0] * o[5],
        o[0] * o[4] - o[1] * o[3],
    ]
}

pub fn plane_of(o: Option<&[f64; 6]>) -> Plane {
    let Some(o) = o else { return Plane::Unknown };
    let n = normal(o);
    let a = [n[0].abs(), n[1].abs(), n[2].abs()];
    let (idx, max) = a
        .iter()
        .enumerate()
        .fold((0, 0.0), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
    if max < 0.8 {
        return Plane::Oblique;
    }
    match idx {
        0 => Plane::Sagittal,
        1 => Plane::Coronal,
        _ => Plane::Axial,
    }
}

/// Odczytuje wszystkie pliki DICOM w folderze (rekurencyjnie) i układa je w badania i serie.
pub fn scan(root: &Path, progress: Option<&(dyn Fn(usize, usize) + Sync)>) -> Library {
    let files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            // pomijamy oczywiste pliki niebędące obrazami
            !(name.starts_with('.')
                || name.ends_with(".exe")
                || name.ends_with(".dll")
                || name.ends_with(".txt")
                || name.ends_with(".htm")
                || name.ends_with(".html")
                || name.ends_with(".inf")
                || name.ends_with(".ini")
                || name.ends_with(".xml")
                || name.ends_with(".pdf")
                || name.ends_with(".jar")
                || name.ends_with(".js")
                || name.ends_with(".css")
                || name.ends_with(".png")
                || name.ends_with(".jpg")
                || name.ends_with(".gif")
                || name.ends_with(".zip")
                || name.ends_with(".app")
                || name.ends_with(".plist")
                || name.ends_with(".dylib")
                || name == "dicomdir")
        })
        .map(|e| e.into_path())
        .collect();
    let total = files.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let headers: Vec<Option<Header>> = files
        .par_iter()
        .map(|p| {
            let h = read_header(p);
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if let Some(cb) = progress {
                if n % 25 == 0 || n == total {
                    cb(n, total);
                }
            }
            h
        })
        .collect();
    let skipped_files = headers.iter().filter(|h| h.is_none()).count();
    build_library(root, headers.into_iter().flatten().collect(), skipped_files)
}

fn build_library(root: &Path, headers: Vec<Header>, skipped_files: usize) -> Library {
    // badanie -> (klucz serii) -> nagłówki
    let mut studies: HashMap<String, (StudyFields, HashMap<String, Vec<Header>>)> = HashMap::new();
    for h in headers {
        // ta sama seria może zawierać kilka orientacji (np. lokalizator 3-płaszczyznowy)
        // albo różne rozmiary obrazów – rozdzielamy je na osobne stosy
        let orient_key = h
            .orientation
            .map(|o| {
                o.iter()
                    .map(|v| format!("{:.2}", v))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let key = format!("{}|{}|{}x{}", h.series_uid, orient_key, h.rows, h.columns);
        let entry = studies
            .entry(h.study_uid.clone())
            .or_insert_with(|| (h.study.clone(), HashMap::new()));
        entry.1.entry(key).or_default().push(h);
    }

    let mut out: Vec<Study> = Vec::new();
    for (uid, (fields, series_map)) in studies {
        let mut series_list: Vec<(Series, Vec<f64>)> = series_map
            .into_values()
            .flat_map(|hs| {
                let n = hs[0].orientation.as_ref().map(normal);
                split_variants(hs, n.as_ref())
            })
            .map(|(variant, order, mut hs)| {
                let first = &hs[0];
                let orientation = first.orientation;
                let n = orientation.as_ref().map(normal);
                // sortowanie przekrojów wzdłuż normalnej do płaszczyzny obrazu
                hs.sort_by(|a, b| {
                    let ka = sort_key(a, n.as_ref());
                    let kb = sort_key(b, n.as_ref());
                    ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                });
                let first = &hs[0];
                let mut slices = Vec::new();
                for h in &hs {
                    for f in 0..h.frames {
                        slices.push(Slice {
                            path: h.path.clone(),
                            frame: f,
                            instance_number: h.instance_number,
                            position: if h.frames == 1 { h.position } else { None },
                            slice_location: if h.frames == 1 { h.slice_location } else { None },
                        });
                    }
                }
                let series = Series {
                    id: 0,
                    uid: first.series_uid.clone(),
                    number: first.series_number,
                    description: first.series_description.clone(),
                    modality: first.modality.clone(),
                    body_part: first.body_part.clone(),
                    plane: plane_of(orientation.as_ref()),
                    rows: first.rows,
                    columns: first.columns,
                    pixel_spacing: first.pixel_spacing,
                    slice_thickness: first.slice_thickness,
                    spacing_between_slices: first.spacing_between_slices,
                    orientation,
                    frame_of_reference: first.frame_of_reference.clone(),
                    sequence: first.sequence.clone(),
                    series_date: first.series_date.clone(),
                    series_time: first.series_time.clone(),
                    variant,
                    may_have_burned_in_text: hs.iter().any(|h| h.burned_in),
                    slices,
                };
                (series, order)
            })
            .collect();
        series_list.sort_by(|(a, oa), (b, ob)| {
            (a.number.unwrap_or(i32::MAX), &a.series_time, &a.description)
                .cmp(&(b.number.unwrap_or(i32::MAX), &b.series_time, &b.description))
                .then_with(|| oa.partial_cmp(ob).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.plane.en().cmp(b.plane.en()))
        });
        // odmianę dopisujemy do opisu dopiero po sortowaniu, np. „T2 FSE – echo 2”
        let series_list: Vec<Series> = series_list
            .into_iter()
            .map(|(mut se, _)| {
                if !se.variant.is_empty() {
                    se.description = if se.description.is_empty() {
                        se.variant.clone()
                    } else {
                        format!("{} – {}", se.description, se.variant)
                    };
                }
                se
            })
            .collect();
        out.push(Study {
            uid,
            patient_name: fields.patient_name,
            patient_id: fields.patient_id,
            birth_date: fields.birth_date,
            sex: fields.sex,
            age: fields.age,
            date: fields.date,
            description: fields.description,
            institution: fields.institution,
            series: series_list,
        });
    }
    out.sort_by(|a, b| (a.date.clone(), a.uid.clone()).cmp(&(b.date.clone(), b.uid.clone())));
    let mut id = 1;
    for st in &mut out {
        for se in &mut st.series {
            se.id = id;
            id += 1;
        }
    }
    Library {
        root: root.to_path_buf(),
        studies: out,
        skipped_files,
    }
}

fn sort_key(h: &Header, n: Option<&[f64; 3]>) -> (f64, f64) {
    let inst = h.instance_number.unwrap_or(0) as f64;
    match (h.position, n) {
        (Some(p), Some(n)) => (p[0] * n[0] + p[1] * n[1] + p[2] * n[2], inst),
        _ => match h.slice_location {
            Some(l) => (l, inst),
            None => (inst, 0.0),
        },
    }
}

/// Położenie przekroju (w setnych mm) do wykrywania powtórzeń; None dla plików wieloklatkowych.
fn position_key(h: &Header, n: Option<&[f64; 3]>) -> Option<i64> {
    if h.frames != 1 {
        return None;
    }
    let v = match (h.position, n) {
        (Some(p), Some(n)) => p[0] * n[0] + p[1] * n[1] + p[2] * n[2],
        _ => h.slice_location?,
    };
    Some((v * 100.0).round() as i64)
}

/// Liczba obrazów leżących w tym samym miejscu co wcześniejszy obraz oraz największa liczba obrazów w jednym miejscu.
fn duplicates(hs: &[Header], n: Option<&[f64; 3]>) -> (usize, usize) {
    let mut count: HashMap<i64, usize> = HashMap::new();
    for h in hs {
        if let Some(k) = position_key(h, n) {
            *count.entry(k).or_default() += 1;
        }
    }
    let dup = count.values().map(|c| c - 1).sum();
    (dup, count.values().copied().max().unwrap_or(0))
}

type VariantDim = (fn(&Header) -> Option<f64>, fn(f64) -> String);

fn fmt_num(v: f64) -> String {
    format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Wymiary, które w jednej serii DICOM rozróżniają obrazy w tym samym położeniu.
/// Opisy są zapisywane w danych serii (także dla MCP), więc mają postać niezależną od języka.
const VARIANT_DIMS: [VariantDim; 5] = [
    (|h| h.echo_number.map(f64::from), |v| format!("echo {}", fmt_num(v))),
    (|h| h.sequence.echo_time, |v| format!("TE {} ms", fmt_num(v))),
    (|h| h.b_value, |v| format!("b = {}", fmt_num(v))),
    // faza dynamiczna (TemporalPositionIdentifier)
    (|h| h.temporal_position.map(f64::from), |v| format!("t{}", fmt_num(v))),
    // numer akwizycji (AcquisitionNumber)
    (|h| h.acquisition_number.map(f64::from), |v| format!("acq {}", fmt_num(v))),
];

/// Rozdziela stos z powtarzającymi się położeniami (wiele ech, wartości b, faz dynamicznych)
/// na osobne stosy. Zwraca (opis odmiany, klucz sortowania, nagłówki).
fn split_variants(hs: Vec<Header>, n: Option<&[f64; 3]>) -> Vec<(String, Vec<f64>, Vec<Header>)> {
    let (dup, max_at_pos) = duplicates(&hs, n);
    if dup == 0 {
        return vec![(String::new(), Vec::new(), hs)];
    }
    for (get, label) in VARIANT_DIMS {
        let Some(vals) = hs.iter().map(get).collect::<Option<Vec<f64>>>() else { continue };
        let mut distinct = vals.clone();
        distinct.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        distinct.dedup();
        // wymiar musi rozróżniać obrazy w tym samym miejscu, a nie np. numerować każdy przekrój osobno
        if distinct.len() < 2 || distinct.len() > max_at_pos {
            continue;
        }
        let mut groups: Vec<Vec<Header>> = distinct.iter().map(|_| Vec::new()).collect();
        for (h, v) in hs.iter().zip(&vals) {
            let i = distinct.iter().position(|d| d == v).unwrap_or(0);
            groups[i].push(h.clone());
        }
        let after: usize = groups.iter().map(|g| duplicates(g, n).0).sum();
        if after >= dup {
            continue;
        }
        return groups
            .into_iter()
            .zip(distinct)
            .flat_map(|(g, v)| {
                split_variants(g, n).into_iter().map(move |(sub, mut order, g)| {
                    order.insert(0, v);
                    let name = if sub.is_empty() { label(v) } else { format!("{}, {}", label(v), sub) };
                    (name, order, g)
                })
            })
            .collect();
    }
    // w ostateczności: k-te wystąpienie danego położenia trafia do k-tego stosu
    let mut hs = hs;
    hs.sort_by(|a, b| {
        let ka = sort_key(a, n);
        let kb = sort_key(b, n);
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen: HashMap<i64, usize> = HashMap::new();
    let mut groups: Vec<Vec<Header>> = Vec::new();
    for h in hs {
        let k = match position_key(&h, n) {
            Some(p) => {
                let c = seen.entry(p).or_default();
                *c += 1;
                *c - 1
            }
            None => 0,
        };
        if groups.len() <= k {
            groups.resize_with(k + 1, Vec::new);
        }
        groups[k].push(h);
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(i, g)| (format!("#{}", i + 1), vec![i as f64], g))
        .collect()
}

/// Data DICOM (RRRRMMDD) w czytelnej postaci.
pub fn fmt_date(d: &str) -> String {
    if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
        format!("{}.{}.{}", &d[6..8], &d[4..6], &d[0..4])
    } else {
        d.to_string()
    }
}

impl Series {
    /// Krótki opis serii w języku interfejsu, np. „3. T2 TSE · strzałkowa · 24 obrazy”.
    pub fn title(&self) -> String {
        t!(
            "lib.series.title",
            number = self.number.map(|n| format!("{}. ", n)).unwrap_or_default(),
            description = self.display_description(),
            plane = self.plane.label(),
            images = images_count(self.slices.len())
        )
    }

    /// Opis serii do wyświetlenia – „(bez opisu)”, gdy plik go nie podaje.
    pub fn display_description(&self) -> &str {
        if self.description.is_empty() { t!("lib.series.no_description") } else { &self.description }
    }

    /// Czy dwa przekroje leżą w tym samym miejscu (stos nie nadaje się do rekonstrukcji).
    pub fn has_duplicate_positions(&self) -> bool {
        let Some(o) = self.orientation.as_ref() else { return false };
        let n = normal(o);
        let mut ps: Vec<i64> = self
            .slices
            .iter()
            .filter_map(|s| s.position.map(|p| ((p[0] * n[0] + p[1] * n[1] + p[2] * n[2]) * 100.0).round() as i64))
            .collect();
        let len = ps.len();
        ps.sort_unstable();
        ps.dedup();
        ps.len() < len
    }

    /// Odległość między kolejnymi przekrojami w mm (z pozycji, w razie braku z tagów).
    pub fn slice_gap(&self) -> Option<f64> {
        if let (Some(o), true) = (self.orientation.as_ref(), self.slices.len() > 1) {
            let n = normal(o);
            // tylko różne położenia – powtórzenia w tym samym miejscu zaniżałyby odstęp
            let mut ps: Vec<f64> = self
                .slices
                .iter()
                .filter_map(|s| s.position.map(|p| p[0] * n[0] + p[1] * n[1] + p[2] * n[2]))
                .collect();
            ps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            ps.dedup_by(|a, b| (*a - *b).abs() < 0.01);
            if ps.len() > 1 {
                let gap = (ps[ps.len() - 1] - ps[0]).abs() / (ps.len() - 1) as f64;
                if gap > 0.0 {
                    return Some(gap);
                }
            }
        }
        self.spacing_between_slices.or(self.slice_thickness)
    }
}

/// Liczba obrazów z właściwą odmianą, np. „24 obrazy” / „24 images”.
pub fn images_count(n: usize) -> String {
    crate::i18n::trn("lib.images", n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(z: f64, inst: i32) -> Header {
        Header {
            path: PathBuf::from(format!("{inst}.dcm")),
            frames: 1,
            study_uid: "1".into(),
            series_uid: "1.1".into(),
            study: StudyFields {
                patient_name: String::new(),
                patient_id: String::new(),
                birth_date: String::new(),
                sex: String::new(),
                age: String::new(),
                date: String::new(),
                description: String::new(),
                institution: String::new(),
            },
            series_number: Some(3),
            series_description: "PD+T2 FSE".into(),
            modality: "MR".into(),
            body_part: String::new(),
            rows: 4,
            columns: 4,
            pixel_spacing: Some((1.0, 1.0)),
            slice_thickness: Some(3.0),
            spacing_between_slices: None,
            orientation: Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            frame_of_reference: String::new(),
            position: Some([0.0, 0.0, z]),
            slice_location: Some(z),
            instance_number: Some(inst),
            sequence: SequenceInfo::default(),
            series_date: String::new(),
            series_time: String::new(),
            echo_number: None,
            b_value: None,
            temporal_position: None,
            acquisition_number: None,
            burned_in: false,
        }
    }

    #[test]
    fn plain_stack_is_not_split() {
        let hs: Vec<Header> = (0..10)
            .map(|k| {
                let mut h = header(k as f64 * 4.0, k);
                h.acquisition_number = Some(k); // numer akwizycji na każdy przekrój
                h
            })
            .collect();
        let lib = build_library(Path::new("/x"), hs, 0);
        assert_eq!(lib.series_count(), 1);
        let se = &lib.studies[0].series[0];
        assert_eq!(se.slices.len(), 10);
        assert_eq!(se.description, "PD+T2 FSE");
        assert!(!se.has_duplicate_positions());
    }

    #[test]
    fn dual_echo_is_split_per_echo() {
        let mut hs = Vec::new();
        for k in 0..10 {
            for e in 1..=2 {
                let mut h = header(k as f64 * 4.0, k * 2 + e);
                h.echo_number = Some(e);
                h.acquisition_number = Some(k * 2 + e);
                h.sequence.echo_time = Some(if e == 1 { 12.0 } else { 90.0 });
                hs.push(h);
            }
        }
        let lib = build_library(Path::new("/x"), hs, 0);
        let titles: Vec<&str> = lib.studies[0].series.iter().map(|s| s.description.as_str()).collect();
        assert_eq!(titles, ["PD+T2 FSE – echo 1", "PD+T2 FSE – echo 2"]);
        for se in &lib.studies[0].series {
            assert_eq!(se.slices.len(), 10);
            assert!(!se.has_duplicate_positions());
            assert!((se.slice_gap().unwrap() - 4.0).abs() < 1e-9);
        }
    }

    #[test]
    fn diffusion_is_split_by_b_value_in_numeric_order() {
        let mut hs = Vec::new();
        for k in 0..10 {
            for (i, b) in [1000.0, 0.0, 500.0].into_iter().enumerate() {
                let mut h = header(k as f64 * 5.0, k * 3 + i as i32);
                h.b_value = Some(b);
                hs.push(h);
            }
        }
        let lib = build_library(Path::new("/x"), hs, 0);
        let v: Vec<&str> = lib.studies[0].series.iter().map(|s| s.variant.as_str()).collect();
        assert_eq!(v, ["b = 0", "b = 500", "b = 1000"]);
    }

    #[test]
    fn unknown_repeats_fall_back_to_occurrence() {
        let mut hs = Vec::new();
        for k in 0..6 {
            hs.push(header(k as f64 * 4.0, k));
            hs.push(header(k as f64 * 4.0, 100 + k));
        }
        let lib = build_library(Path::new("/x"), hs, 0);
        assert_eq!(lib.series_count(), 2);
        for se in &lib.studies[0].series {
            assert!(!se.has_duplicate_positions());
            assert_eq!(se.slices.len(), 6);
        }
    }

    #[test]
    fn burned_in_detection() {
        let mr = "1.2.840.10008.5.1.4.1.1.4";
        assert!(!may_have_burned_in_text("", mr, "MR", "ORIGINAL\\PRIMARY\\M\\ND", 1));
        assert!(!may_have_burned_in_text("NO", mr, "MR", "DERIVED\\SECONDARY\\MPR", 1));
        assert!(!may_have_burned_in_text("", "1.2.840.10008.5.1.4.1.1.4.1", "MR", "DERIVED\\SECONDARY", 1));
        assert!(may_have_burned_in_text("YES", mr, "MR", "ORIGINAL\\PRIMARY", 1));
        assert!(may_have_burned_in_text("", "1.2.840.10008.5.1.4.1.1.7", "MR", "", 1));
        assert!(may_have_burned_in_text("", "1.2.840.10008.5.1.4.1.1.7.4", "MR", "", 1));
        assert!(may_have_burned_in_text("", mr, "OT", "", 1));
        assert!(may_have_burned_in_text("", mr, "MR", "DERIVED\\SECONDARY\\SCREEN SAVE", 1));
        assert!(may_have_burned_in_text("", mr, "MR", "ORIGINAL\\PRIMARY", 3));
        assert!(may_have_burned_in_text("", "", "MR", "DERIVED\\SECONDARY", 1));
        // klasa CT 1.1.2 nie może łapać np. 1.1.20 (NM)
        assert!(may_have_burned_in_text("", "1.2.840.10008.5.1.4.1.1.20", "NM", "DERIVED\\SECONDARY", 1));
    }
}
