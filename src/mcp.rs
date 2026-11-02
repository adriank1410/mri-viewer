//! Serwer MCP (stdio), dzięki któremu Claude może przeglądać badania.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use base64::Engine;
use rayon::prelude::*;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::cache::FrameCache;
use crate::imaging::{self, Frame};
use crate::library::{self, Library, Plane, Series, Study, fmt_date};
use crate::mpr;

const INSTRUCTIONS: &str = "This server gives read-only access to medical imaging studies (DICOM, typically MRI) \
stored on the user's computer. Workflow: call list_series first, then view_montage to get an overview of a series, \
then view_slice for detail. Slice numbers are 1-based. Images are rendered as PNG with a window (brightness/contrast) \
that you can change via window_center/window_width. IMPORTANT: this is a non-diagnostic viewing aid. When describing \
findings, be careful and explicit about uncertainty, and remind the user that only a radiologist's report is \
authoritative. Patient-identifying fields are hidden unless the server was started with --show-patient; \
in that mode series that may have patient data burned into the pixels (screen captures, scanned documents, \
annotated images) cannot be viewed, and file paths are not shown.";

#[derive(Clone)]
pub struct MriServer {
    lib: Arc<RwLock<Library>>,
    cache: Arc<FrameCache>,
    anonymize: bool,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OpenFolderArgs {
    /// Absolute path to a folder containing DICOM files (e.g. a copied CD with DICOMDIR). Scanned recursively.
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SeriesArgs {
    /// Series id from list_series.
    pub series_id: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WindowArgs {
    /// Window center (level). Omit to use the scanner's stored value or an automatic one.
    pub window_center: Option<f32>,
    /// Window width (contrast). Smaller = more contrast.
    pub window_width: Option<f32>,
    /// Invert grey levels.
    pub invert: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ViewSliceArgs {
    /// Series id from list_series.
    pub series_id: usize,
    /// 1-based slice number. Defaults to the middle slice.
    pub slice: Option<usize>,
    /// Longest side of the returned PNG in pixels (default 768, max 2048).
    pub size: Option<u32>,
    #[serde(flatten)]
    pub window: WindowArgs,
    /// Optional crop/zoom region in ORIGINAL image pixel coordinates: [x, y, width, height].
    pub region: Option<[u32; 4]>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MontageArgs {
    /// Series id from list_series.
    pub series_id: usize,
    /// How many slices to show (default 16, max 49). They are evenly spread between first and last.
    pub count: Option<usize>,
    /// First slice (1-based, default 1).
    pub first: Option<usize>,
    /// Last slice (1-based, default = last slice of the series).
    pub last: Option<usize>,
    /// Tile size in pixels (default 256).
    pub tile: Option<u32>,
    #[serde(flatten)]
    pub window: WindowArgs,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TagsArgs {
    /// Series id from list_series.
    pub series_id: usize,
    /// 1-based slice number (default 1).
    pub slice: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RegionArgs {
    /// Series id from list_series.
    pub series_id: usize,
    /// 1-based slice number.
    pub slice: usize,
    /// Rectangle in original image pixel coordinates.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DistanceArgs {
    /// Series id from list_series.
    pub series_id: usize,
    /// Points in original image pixel coordinates.
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReformatArgs {
    /// Series id from list_series (must be a stack of parallel slices).
    pub series_id: usize,
    /// Target plane: "axial", "coronal" or "sagittal" (must differ from the acquisition plane).
    pub plane: String,
    /// 1-based position in the new plane (default: middle). list returned in the text.
    pub position: Option<usize>,
    /// Longest side of the PNG (default 768).
    pub size: Option<u32>,
    #[serde(flatten)]
    pub window: WindowArgs,
}

fn err(msg: impl std::fmt::Display) -> McpError {
    McpError::internal_error(msg.to_string(), None)
}

fn png_block(png: Vec<u8>) -> ContentBlock {
    ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(png), "image/png")
}

fn fmt_opt(v: Option<f64>, unit: &str) -> String {
    v.map(|x| format!("{:.4}", x).trim_end_matches('0').trim_end_matches('.').to_string() + unit)
        .unwrap_or_else(|| "?".into())
}

fn pick_window(frame: &Frame, w: &WindowArgs) -> (f32, f32, bool) {
    let (c0, w0) = frame.default_window();
    (
        w.window_center.unwrap_or(c0),
        // minimum zależne od zakresu wartości (mapy parametryczne bywają rzędu 0,001)
        w.window_width.unwrap_or(w0).max(frame.window_eps()),
        w.invert.unwrap_or(false),
    )
}

/// Liczba z 3–4 cyframi znaczącymi, także dla bardzo małych wartości.
fn fmt_val(v: f32) -> String {
    let a = v.abs();
    if !a.is_finite() || a == 0.0 {
        return format!("{v:.0}");
    }
    let dec = (3 - a.log10().floor() as i32).clamp(0, 12) as usize;
    let s = format!("{v:.dec$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Nazwa strony ciała, w którą wskazuje kierunek w układzie LPS.
fn side_name(v: [f64; 3]) -> &'static str {
    let a = [v[0].abs(), v[1].abs(), v[2].abs()];
    if a[0] >= a[1] && a[0] >= a[2] {
        if v[0] > 0.0 { "patient left" } else { "patient right" }
    } else if a[1] >= a[2] {
        if v[1] > 0.0 { "posterior" } else { "anterior" }
    } else if v[2] > 0.0 {
        "head (superior)"
    } else {
        "feet (inferior)"
    }
}

/// Opis orientacji obrazu z kierunków osi (x w prawo, y w dół).
fn orientation_text(xdir: [f64; 3], ydir: [f64; 3]) -> String {
    let neg = |v: [f64; 3]| [-v[0], -v[1], -v[2]];
    format!(
        "Top of image = {}, bottom = {}, image left = {}, image right = {}.",
        side_name(neg(ydir)),
        side_name(ydir),
        side_name(neg(xdir)),
        side_name(xdir)
    )
}

impl MriServer {
    pub fn new(folder: Option<PathBuf>, anonymize: bool) -> Self {
        let lib = match &folder {
            Some(f) => library::scan(f, None),
            None => Library::default(),
        };
        Self {
            lib: Arc::new(RwLock::new(lib)),
            cache: Arc::new(FrameCache::new(1_500_000_000)),
            anonymize,
            tool_router: Self::tool_router(),
        }
    }

    fn series(&self, id: usize) -> Result<(Study, Series), McpError> {
        let lib = self.lib.read().unwrap();
        if lib.studies.is_empty() {
            return Err(err("No folder is open. Call open_folder first."));
        }
        lib.series_by_id(id)
            .map(|(st, se)| (st.clone(), se.clone()))
            .ok_or_else(|| err(format!("No series with id {id}. Call list_series.")))
    }

    /// W trybie anonimowym odmawia wysłania pikseli serii, które mogą mieć wpisane dane pacjenta.
    fn check_viewable(&self, se: &Series) -> Result<(), McpError> {
        if self.anonymize && se.may_have_burned_in_text {
            return Err(err(format!(
                "Series {} may have patient data burned into the pixels (screen capture, scanned document or \
annotated image), so its images are not shared while patient data is hidden. The user can view it in the \
MRI viewer window, or restart the server with --show-patient.",
                se.id
            )));
        }
        Ok(())
    }

    /// Błąd odczytu pliku; w trybie anonimowym bez ścieżek (nazwy folderów mogą zawierać nazwisko).
    fn load_err(&self, e: anyhow::Error) -> McpError {
        if !self.anonymize {
            return err(e);
        }
        let cause = e.root_cause().to_string();
        if cause.contains('/') || cause.contains('\\') {
            err("Cannot read the image file (unreadable, truncated or unsupported format).")
        } else {
            err(format!("Cannot read the image file: {cause}"))
        }
    }

    fn slice_index(series: &Series, slice: Option<usize>) -> Result<usize, McpError> {
        let n = series.slices.len();
        match slice {
            None => Ok(n / 2),
            Some(s) if s >= 1 && s <= n => Ok(s - 1),
            Some(s) => Err(err(format!("Slice {s} out of range 1..={n}"))),
        }
    }

    fn describe_series(&self, se: &Series) -> String {
        let q = &se.sequence;
        let mut s = format!(
            "[series_id {}] #{} \"{}\" | {} | plane: {} | {} slices | {}x{} px",
            se.id,
            se.number.map(|n| n.to_string()).unwrap_or("?".into()),
            se.description,
            se.modality,
            se.plane.en(),
            se.slices.len(),
            se.columns,
            se.rows
        );
        if let Some((r, c)) = se.pixel_spacing {
            let _ = write!(s, " | pixel {:.3}x{:.3} mm", c, r);
        }
        let _ = write!(
            s,
            " | thickness {} | slice gap {}",
            fmt_opt(se.slice_thickness, " mm"),
            fmt_opt(se.slice_gap(), " mm")
        );
        let mut seq = Vec::new();
        if q.repetition_time.is_some() {
            seq.push(format!("TR {}", fmt_opt(q.repetition_time, " ms")));
        }
        if q.echo_time.is_some() {
            seq.push(format!("TE {}", fmt_opt(q.echo_time, " ms")));
        }
        if q.inversion_time.is_some() {
            seq.push(format!("TI {}", fmt_opt(q.inversion_time, " ms")));
        }
        if q.flip_angle.is_some() {
            seq.push(format!("flip {}", fmt_opt(q.flip_angle, "°")));
        }
        if !q.scanning_sequence.is_empty() {
            seq.push(format!("seq {}", q.scanning_sequence));
        }
        if !q.sequence_name.is_empty() {
            seq.push(format!("name {}", q.sequence_name));
        }
        if !q.contrast_agent.is_empty() {
            seq.push(format!("contrast: {}", q.contrast_agent));
        }
        if !q.image_type.is_empty() {
            seq.push(format!("type {}", q.image_type));
        }
        if !seq.is_empty() {
            let _ = write!(s, " | {}", seq.join(", "));
        }
        if se.may_have_burned_in_text {
            s.push_str(if self.anonymize {
                " | [hidden: may contain burned-in patient data; images not available]"
            } else {
                " | note: may contain burned-in text (screen capture or annotated image)"
            });
        }
        s
    }

    fn render_frame_png(
        frame: &Frame,
        spacing: Option<(f64, f64)>,
        w: &WindowArgs,
        size: u32,
        region: Option<[u32; 4]>,
    ) -> Result<(Vec<u8>, String), McpError> {
        let (c, ww, inv) = pick_window(frame, w);
        let mut gray = imaging::render_gray8(frame, c, ww, inv);
        let (mut gw, mut gh) = (frame.width, frame.height);
        let mut note = String::new();
        if let Some([x, y, rw, rh]) = region {
            let x1 = (x + rw).min(gw);
            let y1 = (y + rh).min(gh);
            if x >= x1 || y >= y1 {
                return Err(err("region is outside the image"));
            }
            let mut crop = Vec::with_capacity(((x1 - x) * (y1 - y)) as usize);
            for yy in y..y1 {
                crop.extend_from_slice(&gray[(yy * gw + x) as usize..(yy * gw + x1) as usize]);
            }
            gray = crop;
            gw = x1 - x;
            gh = y1 - y;
            let _ = write!(note, " Cropped to x={x}..{x1}, y={y}..{y1}.");
        }
        let (tw, th) = imaging::target_size(gw, gh, spacing, size.clamp(64, 2048));
        let scaled = imaging::resize_gray(&gray, gw, gh, tw, th);
        let png = imaging::encode_png_gray(&scaled, tw, th).map_err(err)?;
        Ok((
            png,
            format!("Window center {}, width {}{}.{note}", fmt_val(c), fmt_val(ww), if inv { ", inverted" } else { "" }),
        ))
    }
}

#[tool_router]
impl MriServer {
    #[tool(description = "Open (scan) a folder with DICOM files, e.g. a copied MRI CD. Replaces the currently open folder.")]
    async fn open_folder(&self, Parameters(a): Parameters<OpenFolderArgs>) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(&a.path);
        if !path.is_dir() {
            return Err(err(format!("Not a folder: {}", a.path)));
        }
        let lib = tokio::task::spawn_blocking(move || library::scan(&path, None))
            .await
            .map_err(err)?;
        let msg = format!(
            "Opened {}: {} studies, {} series ({} non-image files skipped).",
            a.path,
            lib.studies.len(),
            lib.series_count(),
            lib.skipped_files
        );
        *self.lib.write().unwrap() = lib;
        self.cache.clear();
        Ok(CallToolResult::success(vec![ContentBlock::text(msg)]))
    }

    #[tool(description = "List all studies and series in the open folder, with acquisition parameters (plane, slice count, thickness, TR/TE etc.). Call this first.")]
    async fn list_series(&self) -> Result<CallToolResult, McpError> {
        let lib = self.lib.read().unwrap();
        if lib.studies.is_empty() {
            return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "No images found{}. Use open_folder with a path to the DICOM folder.",
                if lib.root.as_os_str().is_empty() || self.anonymize {
                    String::new()
                } else {
                    format!(" in {}", lib.root.display())
                }
            ))]));
        }
        // ścieżka folderu często zawiera nazwisko pacjenta albo nazwę konta
        let mut out = if self.anonymize {
            "Folder: [hidden]\n".to_string()
        } else {
            format!("Folder: {}\n", lib.root.display())
        };
        for st in &lib.studies {
            let _ = writeln!(
                out,
                "\nSTUDY {} \"{}\" | patient: {} | sex {} | age {} | institution: {}",
                fmt_date(&st.date),
                st.description,
                if self.anonymize { "[hidden]".to_string() } else { format!("{} (born {})", st.patient_name, fmt_date(&st.birth_date)) },
                if st.sex.is_empty() { "?" } else { &st.sex },
                if st.age.is_empty() { "?" } else { &st.age },
                if self.anonymize { "[hidden]" } else { &st.institution },
            );
            for se in &st.series {
                let _ = writeln!(out, "  {}", self.describe_series(se));
            }
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }

    #[tool(description = "Render one slice of a series as a PNG image. Optionally set window center/width, invert, or crop to a region (in original pixel coordinates) to zoom in.")]
    async fn view_slice(&self, Parameters(a): Parameters<ViewSliceArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        self.check_viewable(&se)?;
        let idx = Self::slice_index(&se, a.slice)?;
        let sl = se.slices[idx].clone();
        let cache = self.cache.clone();
        let frame = tokio::task::spawn_blocking(move || cache.get(&sl))
            .await
            .map_err(err)?
            .map_err(|e| self.load_err(e))?;
        let (png, wnote) =
            Self::render_frame_png(&frame, se.pixel_spacing, &a.window, a.size.unwrap_or(768), a.region)?;
        let s = &se.slices[idx];
        let mut info = format!(
            "Series {} \"{}\" ({}), slice {}/{}. Original size {}x{} px.",
            se.id,
            se.description,
            se.plane.en(),
            idx + 1,
            se.slices.len(),
            frame.width,
            frame.height
        );
        if let Some(l) = s.slice_location {
            let _ = write!(info, " Slice location {l:.1} mm.");
        }
        if let Some(p) = s.position {
            let _ = write!(info, " Position (LPS) {:.1}, {:.1}, {:.1} mm.", p[0], p[1], p[2]);
        }
        let _ = write!(
            info,
            " Pixel value range {}..{}. {wnote} Image is shown in radiological convention as stored (patient's right on image left for axial/coronal).",
            fmt_val(frame.min),
            fmt_val(frame.max)
        );
        Ok(CallToolResult::success(vec![ContentBlock::text(info), png_block(png)]))
    }

    #[tool(description = "Render a grid (montage) of evenly spaced slices from a series in one PNG, each tile labelled with its 1-based slice number in the top-left corner. Best way to get an overview.")]
    async fn view_montage(&self, Parameters(a): Parameters<MontageArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        self.check_viewable(&se)?;
        let n = se.slices.len();
        let first = a.first.unwrap_or(1).clamp(1, n) - 1;
        let last = a.last.unwrap_or(n).clamp(1, n) - 1;
        if last < first {
            return Err(err("last must be >= first"));
        }
        let span = last - first + 1;
        let count = a.count.unwrap_or(16).clamp(1, 49).min(span);
        let idxs: Vec<usize> = if count == 1 {
            vec![first + span / 2]
        } else {
            let mut v: Vec<usize> = (0..count)
                .map(|i| first + ((i as f64) * (span - 1) as f64 / (count - 1) as f64).round() as usize)
                .collect();
            v.dedup();
            v
        };
        let tile = a.tile.unwrap_or(256).clamp(64, 512);
        let cols = (idxs.len() as f64).sqrt().ceil() as u32;
        let rows = (idxs.len() as u32).div_ceil(cols);
        let cache = self.cache.clone();
        let slices: Vec<_> = idxs.iter().map(|&i| se.slices[i].clone()).collect();
        let frames = tokio::task::spawn_blocking(move || {
            slices.par_iter().map(|s| cache.get(s)).collect::<Result<Vec<_>, _>>()
        })
        .await
        .map_err(err)?
        .map_err(|e| self.load_err(e))?;
        // wspólne okno dla wszystkich kafelków (ze środkowego przekroju), chyba że podano
        let mid = &frames[frames.len() / 2];
        let (c, ww, inv) = pick_window(mid, &a.window);
        let pairs: Vec<(&Frame, usize)> = frames.iter().zip(idxs.iter()).map(|(f, &i)| (f.as_ref(), i + 1)).collect();
        let (canvas, canvas_w, canvas_h) = imaging::render_montage(&pairs, (c, ww), inv, se.pixel_spacing, tile, cols);
        let png = imaging::encode_png_gray(&canvas, canvas_w, canvas_h).map_err(err)?;
        let info = format!(
            "Series {} \"{}\" ({}), {} of {} slices: {}. Grid {}x{}, read left-to-right, top-to-bottom. Window center {}, width {}.",
            se.id,
            se.description,
            se.plane.en(),
            idxs.len(),
            n,
            idxs.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(", "),
            cols,
            rows,
            fmt_val(c),
            fmt_val(ww)
        );
        Ok(CallToolResult::success(vec![ContentBlock::text(info), png_block(png)]))
    }

    #[tool(description = "Detailed info about a series: acquisition parameters, available reconstruction planes and the position of every slice.")]
    async fn series_details(&self, Parameters(a): Parameters<SeriesArgs>) -> Result<CallToolResult, McpError> {
        let (st, se) = self.series(a.series_id)?;
        let mut out = format!("Study {} \"{}\"\n{}\n", fmt_date(&st.date), st.description, self.describe_series(&se));
        let q = &se.sequence;
        let _ = writeln!(
            out,
            "Protocol: {} | body part: {} | field strength: {} | series date/time: {} {}",
            q.protocol_name,
            se.body_part,
            fmt_opt(q.magnetic_field_strength, " T"),
            fmt_date(&se.series_date),
            se.series_time
        );
        if let Some(o) = se.orientation {
            let _ = writeln!(out, "Image orientation (row dir, col dir): {:?}", o);
        }
        let planes = mpr::available_planes(&se);
        if !planes.is_empty() {
            let _ = writeln!(
                out,
                "Reconstruction (view_reformat) possible in: {}",
                planes.iter().map(|p| p.en()).collect::<Vec<_>>().join(", ")
            );
        }
        out.push_str("Slices (number: location / position LPS mm):\n");
        for (i, s) in se.slices.iter().enumerate() {
            let _ = writeln!(
                out,
                "  {}: {}{}",
                i + 1,
                s.slice_location.map(|l| format!("loc {l:.1} ")).unwrap_or_default(),
                s.position
                    .map(|p| format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2]))
                    .unwrap_or_default()
            );
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(out)]))
    }

    #[tool(description = "Show the DICOM header (tags) of one slice as text. Patient-identifying fields are hidden by default.")]
    async fn dicom_tags(&self, Parameters(a): Parameters<TagsArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        let idx = Self::slice_index(&se, Some(a.slice.unwrap_or(1)))?;
        let txt = imaging::dump_tags(&se.slices[idx].path, self.anonymize).map_err(|e| self.load_err(e))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(txt)]))
    }

    #[tool(description = "Pixel value statistics (mean, standard deviation, min, max) inside a rectangle of a slice, in original pixel coordinates. Note: MRI signal intensities are relative, not calibrated units.")]
    async fn region_stats(&self, Parameters(a): Parameters<RegionArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        let idx = Self::slice_index(&se, Some(a.slice))?;
        let sl = se.slices[idx].clone();
        let cache = self.cache.clone();
        let frame = tokio::task::spawn_blocking(move || cache.get(&sl))
            .await
            .map_err(err)?
            .map_err(|e| self.load_err(e))?;
        let Some((mean, sd, mn, mx, n)) = imaging::region_stats(&frame, a.x, a.y, a.width, a.height) else {
            return Err(err("Region outside image or colour image"));
        };
        let area = se
            .pixel_spacing
            .map(|(r, c)| format!(", area {:.1} mm²", n as f64 * r * c))
            .unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{n} pixels{area}: mean {mean:.1}, SD {sd:.1}, min {mn:.0}, max {mx:.0}"
        ))]))
    }

    #[tool(description = "Measure the distance in millimetres between two points (original pixel coordinates) in an image of a series, using its pixel spacing.")]
    async fn measure_distance(&self, Parameters(a): Parameters<DistanceArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        let Some((sr, sc)) = se.pixel_spacing else {
            return Err(err("This series has no pixel spacing; distance in mm unknown"));
        };
        let dx = (a.x2 - a.x1) * sc;
        let dy = (a.y2 - a.y1) * sr;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Distance: {:.1} mm (dx {:.1} mm, dy {:.1} mm). Accuracy limited by pixel size {:.2}x{:.2} mm.",
            (dx * dx + dy * dy).sqrt(),
            dx.abs(),
            dy.abs(),
            sc,
            sr
        ))]))
    }

    #[tool(description = "Reconstruct an image in another plane (multiplanar reformat) from a stack of parallel slices, e.g. a coronal view from an axial series. Quality depends on slice spacing: thick slices give blurry/stair-step reformats.")]
    async fn view_reformat(&self, Parameters(a): Parameters<ReformatArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
        self.check_viewable(&se)?;
        let plane = match a.plane.to_lowercase().as_str() {
            "axial" | "transverse" => Plane::Axial,
            "coronal" => Plane::Coronal,
            "sagittal" => Plane::Sagittal,
            other => return Err(err(format!("Unknown plane '{other}'"))),
        };
        let cache = self.cache.clone();
        let slices = se.slices.clone();
        let frames = tokio::task::spawn_blocking(move || {
            slices.par_iter().map(|s| cache.get(s)).collect::<Result<Vec<_>, _>>()
        })
        .await
        .map_err(err)?
        .map_err(|e| self.load_err(e))?;
        let rf = mpr::reformat(&se, &frames, plane, a.position.map(|p| p.saturating_sub(1))).map_err(err)?;
        let (png, wnote) = Self::render_frame_png(&rf.frame, Some(rf.spacing), &a.window, a.size.unwrap_or(768), None)?;
        let pos = a.position.unwrap_or(rf.positions / 2 + 1).clamp(1, rf.positions);
        let info = format!(
            "Reformat of series {} into {} plane, position {}/{}. {} Pixel spacing {:.2} mm (vertical) x {:.2} mm (horizontal). {}",
            se.id,
            rf.plane.en(),
            pos,
            rf.positions,
            orientation_text(rf.xdir, rf.ydir),
            rf.spacing.0,
            rf.spacing.1,
            wnote
        );
        Ok(CallToolResult::success(vec![ContentBlock::text(info), png_block(png)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MriServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("mri-viewer", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

pub fn run(folder: Option<PathBuf>, anonymize: bool) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let server = MriServer::new(folder, anonymize);
        let service = server.serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_values_are_formatted_readably() {
        assert_eq!(fmt_val(1500.0), "1500");
        assert_eq!(fmt_val(12.5), "12.5");
        assert_eq!(fmt_val(0.0015), "0.0015");
        assert_eq!(fmt_val(0.0), "0");
    }

    #[test]
    fn small_window_width_is_kept() {
        let frame = Frame {
            width: 1,
            height: 1,
            pixels: imaging::Pixels::Gray(vec![0.0]),
            dicom_window: Some((0.0015, 0.003)),
            dicom_windows: Vec::new(),
            auto_window: (0.0015, 0.003),
            min: 0.0,
            max: 0.003,
            inverted: false,
        };
        let w = WindowArgs { window_center: None, window_width: Some(0.001), invert: None };
        let (_, ww, _) = pick_window(&frame, &w);
        assert!((ww - 0.001).abs() < 1e-9);
    }

    #[test]
    fn axial_orientation_text() {
        let t = orientation_text([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(
            t,
            "Top of image = anterior, bottom = posterior, image left = patient right, image right = patient left."
        );
    }
}
