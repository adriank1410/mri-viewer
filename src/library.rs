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
    pub fn pl(&self) -> &'static str {
        match self {
            Plane::Axial => "poprzeczna",
            Plane::Sagittal => "strzałkowa",
            Plane::Coronal => "czołowa",
            Plane::Oblique => "skośna",
            Plane::Unknown => "nieznana",
        }
    }
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
        modality: text(&obj, tags::MODALITY),
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
            image_type: text(&obj, tags::IMAGE_TYPE),
        },
        series_date: text(&obj, tags::SERIES_DATE),
        series_time: text(&obj, tags::SERIES_TIME),
        frame_of_reference: text(&obj, tags::FRAME_OF_REFERENCE_UID),
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
        let mut series_list: Vec<Series> = series_map
            .into_values()
            .map(|mut hs| {
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
                Series {
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
                    slices,
                }
            })
            .collect();
        series_list.sort_by(|a, b| {
            (a.number.unwrap_or(i32::MAX), a.series_time.clone(), a.description.clone(), a.plane.en())
                .cmp(&(b.number.unwrap_or(i32::MAX), b.series_time.clone(), b.description.clone(), b.plane.en()))
        });
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

/// Data DICOM (RRRRMMDD) w czytelnej postaci.
pub fn fmt_date(d: &str) -> String {
    if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
        format!("{}.{}.{}", &d[6..8], &d[4..6], &d[0..4])
    } else {
        d.to_string()
    }
}

impl Series {
    /// Krótki opis serii po polsku, np. „T2 TSE (strzałkowa, 24 obrazy)”.
    pub fn title(&self) -> String {
        let desc = if self.description.is_empty() {
            "(bez opisu)".to_string()
        } else {
            self.description.clone()
        };
        format!(
            "{}{} · {} · {} {}",
            self.number.map(|n| format!("{}. ", n)).unwrap_or_default(),
            desc,
            self.plane.pl(),
            self.slices.len(),
            plural_obraz(self.slices.len())
        )
    }

    /// Odległość między kolejnymi przekrojami w mm (z pozycji, w razie braku z tagów).
    pub fn slice_gap(&self) -> Option<f64> {
        if let (Some(o), true) = (self.orientation.as_ref(), self.slices.len() > 1) {
            let n = normal(o);
            let ps: Vec<f64> = self
                .slices
                .iter()
                .filter_map(|s| s.position.map(|p| p[0] * n[0] + p[1] * n[1] + p[2] * n[2]))
                .collect();
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

pub fn plural_obraz(n: usize) -> &'static str {
    if n == 1 {
        "obraz"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "obrazy"
    } else {
        "obrazów"
    }
}
