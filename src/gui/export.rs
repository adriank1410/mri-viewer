//! Eksport obrazów, serii, animacji i podłączanie do Claude Desktop.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use image::{RgbaImage, imageops};

use super::settings::ExportFormat;
use super::viewport::ViewTransform;
use crate::cache::FrameCache;
use crate::imaging::{self, Colormap, Frame};
use crate::library::Series;

/// Postęp zadania w tle: (opis, zrobione, wszystkie, wynik końcowy).
#[derive(Default, Clone)]
pub struct Job {
    pub title: String,
    pub done: usize,
    pub total: usize,
    pub finished: Option<String>,
}

pub type JobHandle = Arc<Mutex<Option<Job>>>;

/// Wszystkie eksporty dzielą jeden wskaźnik postępu – drugi naraz nadpisałby pierwszy.
pub const BUSY_MSG: &str = "Trwa już eksport – poczekaj na jego zakończenie";

pub fn busy(job: &JobHandle) -> bool {
    job.lock().unwrap().is_some()
}

/// Zajmuje wskaźnik jeszcze w wątku interfejsu, zanim ruszy wątek eksportu.
fn start_job(job: &JobHandle, title: &str, total: usize) {
    *job.lock().unwrap() = Some(Job { title: title.into(), total, ..Default::default() });
}

#[derive(Clone, Copy)]
pub struct RenderOpts {
    pub window: Option<(f32, f32)>,
    pub invert: bool,
    pub colormap: Colormap,
    pub xf: ViewTransform,
}

/// Obraz w pełnej rozdzielczości z poprawionymi proporcjami, oknem, mapą kolorów i obrotem.
pub fn render_image(frame: &Frame, spacing: (f64, f64), o: &RenderOpts, max_side: Option<u32>) -> RgbaImage {
    let (c, w) = o.window.unwrap_or(frame.default_window());
    let rgba = imaging::render_rgba_cmap(frame, c, w, o.invert, o.colormap);
    let img = RgbaImage::from_raw(frame.width, frame.height, rgba).expect("rozmiar");
    let side = max_side.unwrap_or(frame.width.max(frame.height).max(512));
    let (tw, th) = imaging::target_size(frame.width, frame.height, Some(spacing), side);
    let mut img = imageops::resize(&img, tw, th, imageops::FilterType::CatmullRom);
    if o.xf.flip_h {
        img = imageops::flip_horizontal(&img);
    }
    if o.xf.flip_v {
        img = imageops::flip_vertical(&img);
    }
    for _ in 0..o.xf.rot % 4 {
        img = imageops::rotate90(&img);
    }
    img
}

pub fn save_image(img: &RgbaImage, path: &Path, format: ExportFormat, quality: u8) -> Result<()> {
    let mut w = BufWriter::new(std::fs::File::create(path)?);
    match format {
        ExportFormat::Png => img.write_with_encoder(image::codecs::png::PngEncoder::new(&mut w))?,
        ExportFormat::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality).encode_image(&rgb)?;
        }
    }
    // jawne opróżnienie bufora – Drop połknąłby błąd (np. pełny dysk)
    w.flush()?;
    Ok(())
}

pub fn safe_name(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "seria".into() } else { s }
}

pub fn series_base_name(series: &Series) -> String {
    format!(
        "seria{}_{}",
        series.number.unwrap_or(series.id as i32),
        safe_name(&series.description)
    )
}

fn set_job(job: &JobHandle, f: impl FnOnce(&mut Job)) {
    if let Some(j) = job.lock().unwrap().as_mut() {
        f(j);
    }
}

/// Zapis wszystkich przekrojów serii jako osobnych plików.
pub fn export_series(
    series: Series,
    cache: Arc<FrameCache>,
    dir: PathBuf,
    o: RenderOpts,
    format: ExportFormat,
    quality: u8,
    job: JobHandle,
) {
    start_job(&job, "Eksport serii", series.slices.len());
    std::thread::spawn(move || {
        let n = series.slices.len();
        let spacing = series.pixel_spacing.unwrap_or((1.0, 1.0));
        let base = series_base_name(&series);
        let sub = dir.join(&base);
        let res: Result<()> = (|| {
            std::fs::create_dir_all(&sub)?;
            for (i, s) in series.slices.iter().enumerate() {
                let f = cache.get(s)?;
                let img = render_image(&f, spacing, &o, None);
                save_image(&img, &sub.join(format!("{:03}.{}", i + 1, format.ext())), format, quality)?;
                set_job(&job, |j| j.done = i + 1);
            }
            Ok(())
        })();
        set_job(&job, |j| {
            j.finished = Some(match res {
                Ok(()) => format!("Zapisano {n} obrazów w {}", sub.display()),
                Err(e) => format!("Błąd eksportu: {e:#}"),
            })
        });
    });
}

/// Animacja GIF całej serii.
pub fn export_gif(series: Series, cache: Arc<FrameCache>, path: PathBuf, o: RenderOpts, fps: f32, job: JobHandle) {
    start_job(&job, "Animacja GIF", series.slices.len());
    std::thread::spawn(move || {
        let n = series.slices.len();
        let spacing = series.pixel_spacing.unwrap_or((1.0, 1.0));
        let res: Result<()> = (|| {
            let mut w = BufWriter::new(std::fs::File::create(&path)?);
            let mut enc = image::codecs::gif::GifEncoder::new_with_speed(&mut w, 10);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)?;
            let delay = image::Delay::from_numer_denom_ms((1000.0 / fps.max(1.0)) as u32, 1);
            // wspólne okno dla całej animacji (ze środkowego przekroju), żeby nie migotała
            let mid = cache.get(&series.slices[n / 2])?;
            let o = RenderOpts { window: Some(o.window.unwrap_or(mid.default_window())), ..o };
            for (i, s) in series.slices.iter().enumerate() {
                let f = cache.get(s)?;
                let img = render_image(&f, spacing, &o, Some(512));
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, delay))?;
                set_job(&job, |j| j.done = i + 1);
            }
            // koder dopisuje zakończenie pliku przy zwolnieniu, potem jawne opróżnienie bufora
            drop(enc);
            w.flush()?;
            Ok(())
        })();
        set_job(&job, |j| {
            j.finished = Some(match res {
                Ok(()) => format!("Zapisano animację {}", path.display()),
                Err(e) => format!("Błąd eksportu: {e:#}"),
            })
        });
    });
}

/// Mozaika miniatur (do 49 przekrojów).
pub fn export_montage(series: Series, cache: Arc<FrameCache>, path: PathBuf, o: RenderOpts, job: JobHandle) {
    let n = series.slices.len();
    let count = n.min(49);
    let idxs: Vec<usize> = if count <= 1 {
        vec![n / 2]
    } else {
        (0..count).map(|i| (i as f64 * (n - 1) as f64 / (count - 1) as f64).round() as usize).collect()
    };
    start_job(&job, "Mozaika", idxs.len());
    std::thread::spawn(move || {
        let res: Result<()> = (|| {
            let mut frames = Vec::new();
            for (k, &i) in idxs.iter().enumerate() {
                frames.push(cache.get(&series.slices[i])?);
                set_job(&job, |j| j.done = k + 1);
            }
            let mid = &frames[frames.len() / 2];
            let window = o.window.unwrap_or(mid.default_window());
            let pairs: Vec<(&Frame, usize)> = frames.iter().zip(idxs.iter()).map(|(f, &i)| (f.as_ref(), i + 1)).collect();
            let cols = (pairs.len() as f64).sqrt().ceil() as u32;
            let (canvas, w, h) = imaging::render_montage(&pairs, window, o.invert, series.pixel_spacing, 320, cols);
            std::fs::write(&path, imaging::encode_png_gray(&canvas, w, h)?)?;
            Ok(())
        })();
        set_job(&job, |j| {
            j.finished = Some(match res {
                Ok(()) => format!("Zapisano mozaikę {}", path.display()),
                Err(e) => format!("Błąd eksportu: {e:#}"),
            })
        });
    });
}

pub fn copy_to_clipboard(img: &RgbaImage) -> Result<()> {
    let mut cb = arboard::Clipboard::new().map_err(|e| anyhow!("schowek niedostępny: {e}"))?;
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Borrowed(img.as_raw()),
    })
    .map_err(|e| anyhow!("nie udało się skopiować: {e}"))
}

// --- Claude Desktop ---------------------------------------------------------------------------------

pub fn claude_desktop_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|h| PathBuf::from(h).join("Claude").join("claude_desktop_config.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/Claude/claude_desktop_config.json"))
    }
}

pub fn mcp_args(folder: Option<&Path>, show_patient: bool) -> Vec<String> {
    let mut a = vec!["mcp".to_string()];
    if let Some(f) = folder {
        a.push(f.display().to_string());
    }
    if show_patient {
        a.push("--show-patient".into());
    }
    a
}

pub fn desktop_snippet(exe: &Path, args: &[String]) -> String {
    let v = serde_json::json!({ "mcpServers": { "mri": { "command": exe.display().to_string(), "args": args } } });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Cytowanie argumentu dla powłoki: sh/zsh na macOS i Linuksie, PowerShell/cmd na Windows.
fn shell_quote(s: &str, windows: bool) -> String {
    let extra = if windows { "/._:-\\" } else { "/._:-+,@%" };
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    if safe {
        s.to_string()
    } else if windows {
        // końcowy „\” przed cudzysłowem trzeba podwoić, inaczej zjada cudzysłów
        let tail = if s.ends_with('\\') { "\\" } else { "" };
        format!("\"{s}{tail}\"")
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn claude_code_command(exe: &Path, args: &[String]) -> String {
    let windows = cfg!(windows);
    // zakres „user” – serwer widoczny w każdym katalogu, nie tylko w bieżącym
    let mut parts = vec![shell_quote(&exe.display().to_string(), windows)];
    parts.extend(args.iter().map(|a| shell_quote(a, windows)));
    format!("claude mcp add --scope user mri -- {}", parts.join(" "))
}

/// Dopisuje serwer „mri” do konfiguracji Claude Desktop (z kopią zapasową).
pub fn install_into_claude_desktop(exe: &Path, args: &[String]) -> Result<PathBuf> {
    let path = claude_desktop_config_path().ok_or_else(|| anyhow!("nie znaleziono katalogu domowego"))?;
    let mut root: serde_json::Value = if path.exists() {
        let text = std::fs::read_to_string(&path).context("odczyt konfiguracji")?;
        std::fs::write(path.with_extension("json.bak"), &text).context("kopia zapasowa")?;
        if text.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(&text).context("konfiguracja nie jest poprawnym JSON-em")? }
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        serde_json::json!({})
    };
    let obj = root.as_object_mut().ok_or_else(|| anyhow!("nieoczekiwany format konfiguracji"))?;
    let servers = obj.entry("mcpServers").or_insert_with(|| serde_json::json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| anyhow!("mcpServers nie jest obiektem"))?;
    servers.insert(
        "mri".into(),
        serde_json::json!({ "command": exe.display().to_string(), "args": args }),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_unix() {
        assert_eq!(shell_quote("/Users/a/mri", false), "/Users/a/mri");
        assert_eq!(shell_quote("--show-patient", false), "--show-patient");
        assert_eq!(shell_quote("/a/MRI(2)", false), "'/a/MRI(2)'");
        assert_eq!(shell_quote("/a/Jan's scan", false), "'/a/Jan'\\''s scan'");
        assert_eq!(shell_quote("/a/$HOME`x`", false), "'/a/$HOME`x`'");
        assert_eq!(shell_quote("", false), "''");
    }

    #[test]
    fn quote_windows() {
        assert_eq!(shell_quote(r"C:\mri\mri-viewer.exe", true), r"C:\mri\mri-viewer.exe");
        assert_eq!(shell_quote(r"D:\Badania&Wyniki", true), r#""D:\Badania&Wyniki""#);
        assert_eq!(shell_quote(r"C:\Program Files\", true), r#""C:\Program Files\\""#);
        assert_eq!(shell_quote("a,b", true), "\"a,b\"");
    }

    #[test]
    fn command_has_user_scope() {
        let c = claude_code_command(Path::new("/opt/mri"), &mcp_args(None, false));
        assert!(c.starts_with("claude mcp add --scope user mri -- "), "{c}");
    }

    #[test]
    fn busy_after_start() {
        let job: JobHandle = Arc::new(Mutex::new(None));
        assert!(!busy(&job));
        start_job(&job, "x", 3);
        assert!(busy(&job));
    }

    #[test]
    fn save_png_and_jpeg() {
        let dir = std::env::temp_dir().join(format!("mri-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]));
        for (f, ext) in [(ExportFormat::Png, "png"), (ExportFormat::Jpeg, "jpg")] {
            let p = dir.join(format!("t.{ext}"));
            save_image(&img, &p, f, 90).unwrap();
            let back = image::open(&p).unwrap();
            assert_eq!((back.width(), back.height()), (4, 3));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
