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
authoritative. Patient-identifying fields are hidden unless the server was started with --show-patient.";

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
        w.window_width.unwrap_or(w0).max(1.0),
        w.invert.unwrap_or(false),
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
        Ok((png, format!("Window center {c:.0}, width {ww:.0}{}.{note}", if inv { ", inverted" } else { "" })))
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
                if lib.root.as_os_str().is_empty() { String::new() } else { format!(" in {}", lib.root.display()) }
            ))]));
        }
        let mut out = format!("Folder: {}\n", lib.root.display());
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
        let idx = Self::slice_index(&se, a.slice)?;
        let sl = se.slices[idx].clone();
        let cache = self.cache.clone();
        let frame = tokio::task::spawn_blocking(move || cache.get(&sl))
            .await
            .map_err(err)?
            .map_err(err)?;
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
            " Pixel value range {:.0}..{:.0}. {wnote} Image is shown in radiological convention as stored (patient's right on image left for axial/coronal).",
            frame.min, frame.max
        );
        Ok(CallToolResult::success(vec![ContentBlock::text(info), png_block(png)]))
    }

    #[tool(description = "Render a grid (montage) of evenly spaced slices from a series in one PNG, each tile labelled with its 1-based slice number in the top-left corner. Best way to get an overview.")]
    async fn view_montage(&self, Parameters(a): Parameters<MontageArgs>) -> Result<CallToolResult, McpError> {
        let (_st, se) = self.series(a.series_id)?;
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
        .map_err(err)?;
        // wspólne okno dla wszystkich kafelków (ze środkowego przekroju), chyba że podano
        let mid = &frames[frames.len() / 2];
        let (c, ww, inv) = pick_window(mid, &a.window);
        let gap = 4u32;
        let canvas_w = cols * tile + (cols - 1) * gap;
        let canvas_h = rows * tile + (rows - 1) * gap;
        let mut canvas = vec![40u8; (canvas_w * canvas_h) as usize];
        let label_scale = (tile / 64).max(2);
        for (k, (frame, &si)) in frames.iter().zip(idxs.iter()).enumerate() {
            let gray = imaging::render_gray8(frame, c, ww, inv);
            let (tw, th) = imaging::target_size(frame.width, frame.height, se.pixel_spacing, tile);
            let scaled = imaging::resize_gray(&gray, frame.width, frame.height, tw, th);
            let cx = (k as u32 % cols) * (tile + gap);
            let cy = (k as u32 / cols) * (tile + gap);
            let ox = cx + (tile - tw) / 2;
            let oy = cy + (tile - th) / 2;
            for y in 0..th {
                let dst = ((oy + y) * canvas_w + ox) as usize;
                canvas[dst..dst + tw as usize].copy_from_slice(&scaled[(y * tw) as usize..((y + 1) * tw) as usize]);
            }
            imaging::draw_number(&mut canvas, canvas_w, cx, cy, si + 1, label_scale);
        }
        let png = imaging::encode_png_gray(&canvas, canvas_w, canvas_h).map_err(err)?;
        let info = format!(
            "Series {} \"{}\" ({}), {} of {} slices: {}. Grid {}x{}, read left-to-right, top-to-bottom. Window center {:.0}, width {:.0}.",
            se.id,
            se.description,
            se.plane.en(),
            idxs.len(),
            n,
            idxs.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(", "),
            cols,
            rows,
            c,
            ww
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
        let txt = imaging::dump_tags(&se.slices[idx].path, self.anonymize).map_err(err)?;
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
            .map_err(err)?;
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
        .map_err(err)?;
        let rf = mpr::reformat(&se, &frames, plane, a.position.map(|p| p.saturating_sub(1))).map_err(err)?;
        let (png, wnote) = Self::render_frame_png(&rf.frame, Some(rf.spacing), &a.window, a.size.unwrap_or(768), None)?;
        let pos = a.position.unwrap_or(rf.positions / 2 + 1).clamp(1, rf.positions);
        let info = format!(
            "Reformat of series {} into {} plane, position {}/{}. Top of image = head (superior). Pixel spacing {:.2} mm (vertical) x {:.2} mm (horizontal). {}",
            se.id,
            rf.plane.en(),
            pos,
            rf.positions,
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
