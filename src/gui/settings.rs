//! Ustawienia programu (zapisywane między uruchomieniami) i okno ustawień.

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::imaging::Colormap;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum StartAt {
    Middle,
    First,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ThemeChoice {
    Dark,
    Light,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum CineMode {
    Loop,
    Bounce,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ExportFormat {
    Png,
    Jpeg,
}

impl ExportFormat {
    pub fn ext(&self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
        }
    }
}

/// Krój pisma interfejsu.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum FontChoice {
    /// Wbudowana (Ubuntu).
    Default,
    /// Systemowa (San Francisco / Helvetica na Macu, Segoe UI na Windowsie, DejaVu/Noto na Linuksie).
    System,
    /// Wbudowana o stałej szerokości znaków (Hack).
    Mono,
    /// Plik .ttf/.otf wskazany przez użytkownika.
    Custom,
}

impl FontChoice {
    pub fn label(&self) -> &'static str {
        match self {
            FontChoice::Default => t!("settings.font.default"),
            FontChoice::System => t!("settings.font.system"),
            FontChoice::Mono => t!("settings.font.mono"),
            FontChoice::Custom => t!("settings.font.custom"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    /// Kod języka interfejsu; `None` = język systemu (a gdy nieobsługiwany – angielski).
    pub language: Option<String>,
    /// Tryb dla profesjonalistów: więcej narzędzi, panel informacji, tagi DICOM.
    pub pro_mode: bool,
    pub start_at: StartAt,
    /// Ukrywa imię, nazwisko, PESEL itp. na ekranie (np. do zrzutów).
    pub hide_patient: bool,
    pub theme: ThemeChoice,
    pub ui_scale: f32,
    /// Wielkość zwykłego tekstu interfejsu (punkty).
    pub text_size: f32,
    pub font: FontChoice,
    /// Ścieżka do pliku czcionki dla [`FontChoice::Custom`].
    pub custom_font: Option<String>,
    /// Ukrywa dane osobowe pacjenta w zapisywanych i kopiowanych widokach.
    pub hide_patient_in_exports: bool,
    pub invert_scroll: bool,
    pub scroll_sensitivity: f32,
    pub cine_fps: f32,
    pub cine_mode: CineMode,
    pub smooth_interpolation: bool,
    pub show_overlay: bool,
    pub show_orientation: bool,
    pub show_scale_bar: bool,
    pub show_reference_lines: bool,
    pub sync_scroll: bool,
    pub annotation_color: [u8; 3],
    pub overlay_font: f32,
    pub cache_mb: usize,
    pub export_format: ExportFormat,
    pub jpeg_quality: u8,
    pub show_sidebar: bool,
    pub show_info_panel: bool,
    pub default_colormap: Colormap,
    pub recent: Vec<String>,
    pub last_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: None,
            pro_mode: false,
            start_at: StartAt::Middle,
            hide_patient: false,
            theme: ThemeChoice::Dark,
            ui_scale: 1.0,
            text_size: 14.5,
            font: FontChoice::Default,
            custom_font: None,
            hide_patient_in_exports: true,
            invert_scroll: false,
            scroll_sensitivity: 1.0,
            cine_fps: 8.0,
            cine_mode: CineMode::Loop,
            smooth_interpolation: true,
            show_overlay: true,
            show_orientation: true,
            show_scale_bar: true,
            show_reference_lines: true,
            sync_scroll: true,
            annotation_color: [255, 210, 60],
            overlay_font: 14.0,
            cache_mb: 2500,
            export_format: ExportFormat::Png,
            jpeg_quality: 92,
            show_sidebar: true,
            show_info_panel: false,
            default_colormap: Colormap::Gray,
            recent: Vec::new(),
            last_folder: None,
        }
    }
}

pub const STORAGE_KEY: &str = "settings_v2";

impl Settings {
    pub fn add_recent(&mut self, folder: &str) {
        self.recent.retain(|r| r != folder);
        self.recent.insert(0, folder.to_string());
        self.recent.truncate(8);
        self.last_folder = Some(folder.to_string());
    }

    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.theme {
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::System => egui::ThemePreference::System,
        });
        ctx.set_zoom_factor(self.ui_scale.clamp(0.6, 2.5));
        let ts = self.text_size.clamp(11.0, 24.0);
        ctx.all_styles_mut(|style| {
            use egui::{FontId, TextStyle};
            style.text_styles = [
                (TextStyle::Small, FontId::proportional((ts * 0.84).max(11.0))),
                (TextStyle::Body, FontId::proportional(ts)),
                (TextStyle::Button, FontId::proportional(ts)),
                (TextStyle::Heading, FontId::proportional(ts * 1.4)),
                (TextStyle::Monospace, FontId::monospace(ts * 0.92)),
            ]
            .into();
        });
        crate::i18n::set_language(self.language.as_deref());
        self.apply_fonts(ctx);
    }

    /// Ustawia krój pisma – tylko gdy się zmienił, bo przebudowa czcionek jest kosztowna.
    fn apply_fonts(&self, ctx: &egui::Context) {
        let cjk_lang = cjk_language();
        let key = format!("{:?}|{:?}|{cjk_lang:?}", self.font, self.custom_font);
        let id = egui::Id::unique("applied_font_key");
        if ctx.data(|d| d.get_temp::<String>(id)).as_deref() == Some(key.as_str()) {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(id, key));
        let mut fonts = egui::FontDefinitions::default();
        let user = match self.font {
            FontChoice::Default | FontChoice::Mono => None,
            FontChoice::System => system_font(),
            FontChoice::Custom => self.custom_font.as_deref().and_then(|p| load_font(std::path::Path::new(p))),
        };
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            if let Some(bytes) = user {
                fonts.font_data.insert("user".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                fam.insert(0, "user".into());
            } else if self.font == FontChoice::Mono {
                fam.retain(|f| f != "Hack");
                fam.insert(0, "Hack".into());
            }
            // strzałki i trójkąty są tylko w czcionce Hack – zawsze jako zapasowa
            if !fam.iter().any(|f| f == "Hack") {
                fam.push("Hack".into());
            }
            // znaki chińskie, japońskie i koreańskie: czcionka systemowa wczytywana tylko dla tych języków
            if let Some(bytes) = cjk_lang.and_then(cjk_font) {
                fonts.font_data.insert("cjk".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                fam.push("cjk".into());
            }
        }
        ctx.set_fonts(fonts);
    }

    /// Czy wybrana czcionka jest dostępna (do komunikatu w ustawieniach).
    pub fn font_ok(&self) -> bool {
        match self.font {
            FontChoice::Default | FontChoice::Mono => true,
            FontChoice::System => system_font().is_some(),
            FontChoice::Custom => self.custom_font.as_deref().and_then(|p| load_font(std::path::Path::new(p))).is_some(),
        }
    }

    pub fn color(&self) -> egui::Color32 {
        let [r, g, b] = self.annotation_color;
        egui::Color32::from_rgb(r, g, b)
    }

    /// Okno ustawień. Zwraca true, gdy coś zmieniono.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let before = format!("{self:?}");
        egui::ScrollArea::vertical().max_height(560.0).min_scrolled_height(480.0).show(ui, |ui| {
            ui.heading(t!("settings.section.general"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.language"));
                let system = crate::i18n::find(&crate::i18n::system_language()).map_or("English", |l| l.native_name);
                let system_label = t!("settings.language_system", name = system);
                let current = match &self.language {
                    None => system_label.clone(),
                    Some(code) => crate::i18n::find(code).map_or(code.clone(), |l| l.native_name.to_string()),
                };
                egui::ComboBox::from_id_salt("language").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.language, None, system_label);
                    for lang in crate::i18n::LANGS {
                        ui.selectable_value(&mut self.language, Some(lang.code.to_string()), lang.native_name);
                    }
                });
            });
            ui.checkbox(&mut self.pro_mode, t!("settings.pro_mode.label"))
                .on_hover_text(t!("settings.pro_mode.tooltip"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.start_at.label"));
                ui.radio_value(&mut self.start_at, StartAt::Middle, t!("settings.start_at.middle"));
                ui.radio_value(&mut self.start_at, StartAt::First, t!("settings.start_at.first"));
            });
            ui.checkbox(&mut self.hide_patient, t!("settings.hide_patient.label"))
                .on_hover_text(t!("settings.hide_patient.tooltip"));
            ui.checkbox(&mut self.hide_patient_in_exports, t!("settings.hide_patient_exports.label"))
                .on_hover_text(t!("settings.hide_patient_exports.tooltip"));
            ui.add_space(8.0);

            ui.heading(t!("settings.section.appearance"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.theme.label"));
                ui.radio_value(&mut self.theme, ThemeChoice::Dark, t!("settings.theme.dark"));
                ui.radio_value(&mut self.theme, ThemeChoice::Light, t!("settings.theme.light"));
                ui.radio_value(&mut self.theme, ThemeChoice::System, t!("settings.theme.system"));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.ui_scale"));
                ui.add(egui::Slider::new(&mut self.ui_scale, 0.75..=2.0).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.text_size"));
                ui.add(egui::Slider::new(&mut self.text_size, 11.0..=22.0).step_by(0.5).suffix(" pt"));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.font.label"));
                egui::ComboBox::from_id_salt("font_choice")
                    .selected_text(self.font.label())
                    .show_ui(ui, |ui| {
                        for f in [FontChoice::Default, FontChoice::System, FontChoice::Mono, FontChoice::Custom] {
                            ui.selectable_value(&mut self.font, f, f.label());
                        }
                    });
                if self.font == FontChoice::Custom && ui.button(t!("settings.font.choose_file")).clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_title(t!("settings.font.dialog_title"))
                        .add_filter(t!("settings.font.filter_name"), &["ttf", "otf", "ttc"])
                        .pick_file()
                    {
                        self.custom_font = Some(p.display().to_string());
                    }
                }
            });
            if self.font == FontChoice::Custom {
                if let Some(p) = &self.custom_font {
                    ui.small(t!("settings.font.file", path = p));
                }
            }
            if !self.font_ok() {
                ui.colored_label(egui::Color32::LIGHT_RED, t!("settings.font.load_failed"));
            }
            ui.horizontal(|ui| {
                ui.label(t!("settings.overlay_font"));
                ui.add(egui::Slider::new(&mut self.overlay_font, 10.0..=22.0).step_by(1.0));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.annotation_color"));
                ui.color_edit_button_srgb(&mut self.annotation_color);
            });
            ui.checkbox(&mut self.show_overlay, t!("settings.show_overlay"));
            ui.checkbox(&mut self.show_orientation, t!("settings.show_orientation"));
            ui.checkbox(&mut self.show_scale_bar, t!("settings.show_scale_bar"));
            ui.checkbox(&mut self.show_reference_lines, t!("settings.show_reference_lines"));
            ui.checkbox(&mut self.smooth_interpolation, t!("settings.smooth_interpolation.label"))
                .on_hover_text(t!("settings.smooth_interpolation.tooltip"));
            if self.pro_mode {
                ui.horizontal(|ui| {
                    ui.label(t!("settings.default_colormap"));
                    egui::ComboBox::from_id_salt("default_cmap")
                        .selected_text(self.default_colormap.label())
                        .show_ui(ui, |ui| {
                            for c in Colormap::ALL {
                                ui.selectable_value(&mut self.default_colormap, c, c.label());
                            }
                        });
                });
            }
            ui.add_space(8.0);

            ui.heading(t!("settings.section.mouse_playback"));
            ui.checkbox(&mut self.invert_scroll, t!("settings.invert_scroll"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.scroll_sensitivity"));
                ui.add(egui::Slider::new(&mut self.scroll_sensitivity, 0.25..=4.0).logarithmic(true));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.cine_fps.label"));
                ui.add(egui::Slider::new(&mut self.cine_fps, 1.0..=30.0).suffix(t!("settings.cine_fps.suffix")));
            });
            ui.horizontal(|ui| {
                ui.label(t!("settings.cine_mode.label"));
                ui.radio_value(&mut self.cine_mode, CineMode::Loop, t!("settings.cine_mode.loop"));
                ui.radio_value(&mut self.cine_mode, CineMode::Bounce, t!("settings.cine_mode.bounce"));
            });
            ui.checkbox(&mut self.sync_scroll, t!("settings.sync_scroll"));
            ui.add_space(8.0);

            ui.heading(t!("settings.section.export"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.export_format.label"));
                ui.radio_value(&mut self.export_format, ExportFormat::Png, t!("settings.export_format.png"));
                ui.radio_value(&mut self.export_format, ExportFormat::Jpeg, "JPEG");
            });
            if self.export_format == ExportFormat::Jpeg {
                ui.horizontal(|ui| {
                    ui.label(t!("settings.jpeg_quality"));
                    ui.add(egui::Slider::new(&mut self.jpeg_quality, 50..=100));
                });
            }
            ui.add_space(8.0);

            ui.heading(t!("settings.section.performance"));
            ui.horizontal(|ui| {
                ui.label(t!("settings.cache_mb.label"));
                ui.add(egui::Slider::new(&mut self.cache_mb, 500..=8000).step_by(250.0).suffix(" MB"));
            });
            ui.small(t!("settings.cache_mb.restart_note"));
            ui.add_space(8.0);
            if ui.button(t!("settings.reset_defaults")).clicked() {
                let recent = std::mem::take(&mut self.recent);
                let last = self.last_folder.take();
                *self = Settings::default();
                self.recent = recent;
                self.last_folder = last;
            }
        });
        before != format!("{self:?}")
    }
}

/// Sprawdza, czy plik wygląda na czcionkę TrueType/OpenType, i zwraca jego zawartość.
fn load_font(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let magic = bytes.get(0..4)?;
    let ok = matches!(magic, [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf");
    ok.then_some(bytes)
}

/// Bieżący język, jeśli potrzebuje czcionki z pismem CJK.
fn cjk_language() -> Option<&'static str> {
    let code = crate::i18n::current();
    matches!(code, "zh" | "ja" | "ko").then_some(code)
}

/// Systemowa czcionka z pismem danego języka (chiński, japoński, koreański).
fn cjk_font(lang: &str) -> Option<Vec<u8>> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        match lang {
            "ja" => &[
                "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            ],
            "ko" => &["/System/Library/Fonts/AppleSDGothicNeo.ttc", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"],
            _ => &[
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/System/Library/Fonts/STHeiti Light.ttc",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            ],
        }
    } else if cfg!(target_os = "windows") {
        match lang {
            "ja" => &["C:\\Windows\\Fonts\\YuGothM.ttc", "C:\\Windows\\Fonts\\meiryo.ttc", "C:\\Windows\\Fonts\\msgothic.ttc"],
            "ko" => &["C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\gulim.ttc"],
            _ => &["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\simsun.ttc"],
        }
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/OTF/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        ]
    };
    candidates.iter().find_map(|p| load_font(std::path::Path::new(p)))
}

fn system_font() -> Option<Vec<u8>> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
    } else if cfg!(target_os = "windows") {
        &["C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
    } else {
        &[
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ]
    };
    candidates.iter().find_map(|p| load_font(std::path::Path::new(p)))
}
