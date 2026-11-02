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
            FontChoice::Default => "wbudowana (Ubuntu)",
            FontChoice::System => "systemowa",
            FontChoice::Mono => "o stałej szerokości (Hack)",
            FontChoice::Custom => "z pliku…",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
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
        self.apply_fonts(ctx);
    }

    /// Ustawia krój pisma – tylko gdy się zmienił, bo przebudowa czcionek jest kosztowna.
    fn apply_fonts(&self, ctx: &egui::Context) {
        let key = format!("{:?}|{:?}", self.font, self.custom_font);
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
            ui.heading("Ogólne");
            ui.checkbox(&mut self.pro_mode, "Tryb profesjonalny")
                .on_hover_text("Dodatkowe narzędzia (kąt, obszary ROI, sonda, wskaźnik 3D), mapy kolorów, panel tagów DICOM, histogram, dokładne wartości okna, współrzędne w mm.");
            ui.horizontal(|ui| {
                ui.label("Po otwarciu serii pokaż:");
                ui.radio_value(&mut self.start_at, StartAt::Middle, "środkowy obraz");
                ui.radio_value(&mut self.start_at, StartAt::First, "pierwszy obraz");
            });
            ui.checkbox(&mut self.hide_patient, "Ukryj dane osobowe pacjenta na ekranie")
                .on_hover_text("Przydatne przy robieniu zrzutów ekranu i pokazywaniu obrazów innym.");
            ui.checkbox(&mut self.hide_patient_in_exports, "Ukrywaj dane osobowe w zapisywanych i kopiowanych widokach")
                .on_hover_text("Imię, nazwisko, PESEL i data urodzenia nie trafią do zapisanych obrazów ani do schowka.");
            ui.add_space(8.0);

            ui.heading("Wygląd");
            ui.horizontal(|ui| {
                ui.label("Motyw:");
                ui.radio_value(&mut self.theme, ThemeChoice::Dark, "ciemny");
                ui.radio_value(&mut self.theme, ThemeChoice::Light, "jasny");
                ui.radio_value(&mut self.theme, ThemeChoice::System, "jak w systemie");
            });
            ui.horizontal(|ui| {
                ui.label("Wielkość interfejsu:");
                ui.add(egui::Slider::new(&mut self.ui_scale, 0.75..=2.0).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
            ui.horizontal(|ui| {
                ui.label("Wielkość tekstu:");
                ui.add(egui::Slider::new(&mut self.text_size, 11.0..=22.0).step_by(0.5).suffix(" pt"));
            });
            ui.horizontal(|ui| {
                ui.label("Czcionka:");
                egui::ComboBox::from_id_salt("font_choice")
                    .selected_text(self.font.label())
                    .show_ui(ui, |ui| {
                        for f in [FontChoice::Default, FontChoice::System, FontChoice::Mono, FontChoice::Custom] {
                            ui.selectable_value(&mut self.font, f, f.label());
                        }
                    });
                if self.font == FontChoice::Custom && ui.button("Wybierz plik…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_title("Wybierz plik czcionki")
                        .add_filter("Czcionki", &["ttf", "otf", "ttc"])
                        .pick_file()
                    {
                        self.custom_font = Some(p.display().to_string());
                    }
                }
            });
            if self.font == FontChoice::Custom {
                if let Some(p) = &self.custom_font {
                    ui.small(format!("Plik: {p}"));
                }
            }
            if !self.font_ok() {
                ui.colored_label(egui::Color32::LIGHT_RED, "Nie udało się wczytać tej czcionki – używam wbudowanej.");
            }
            ui.horizontal(|ui| {
                ui.label("Wielkość opisów na obrazie:");
                ui.add(egui::Slider::new(&mut self.overlay_font, 10.0..=22.0).step_by(1.0));
            });
            ui.horizontal(|ui| {
                ui.label("Kolor pomiarów:");
                ui.color_edit_button_srgb(&mut self.annotation_color);
            });
            ui.checkbox(&mut self.show_overlay, "Opisy w rogach obrazu");
            ui.checkbox(&mut self.show_orientation, "Oznaczenia stron ciała na krawędziach (R/L, A/P, S/I)");
            ui.checkbox(&mut self.show_scale_bar, "Podziałka w milimetrach");
            ui.checkbox(&mut self.show_reference_lines, "Linie odniesienia między oknami");
            ui.checkbox(&mut self.smooth_interpolation, "Wygładzanie przy powiększeniu")
                .on_hover_text("Wyłącz, aby widzieć pojedyncze piksele (ostre krawędzie).");
            if self.pro_mode {
                ui.horizontal(|ui| {
                    ui.label("Domyślna mapa kolorów:");
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

            ui.heading("Mysz i odtwarzanie");
            ui.checkbox(&mut self.invert_scroll, "Odwrotny kierunek przewijania przekrojów");
            ui.horizontal(|ui| {
                ui.label("Czułość kółka:");
                ui.add(egui::Slider::new(&mut self.scroll_sensitivity, 0.25..=4.0).logarithmic(true));
            });
            ui.horizontal(|ui| {
                ui.label("Prędkość odtwarzania:");
                ui.add(egui::Slider::new(&mut self.cine_fps, 1.0..=30.0).suffix(" obr./s"));
            });
            ui.horizontal(|ui| {
                ui.label("Odtwarzanie:");
                ui.radio_value(&mut self.cine_mode, CineMode::Loop, "w pętli");
                ui.radio_value(&mut self.cine_mode, CineMode::Bounce, "tam i z powrotem");
            });
            ui.checkbox(&mut self.sync_scroll, "W trybie porównania przewijaj okna razem (wg położenia w ciele)");
            ui.add_space(8.0);

            ui.heading("Eksport");
            ui.horizontal(|ui| {
                ui.label("Format obrazów:");
                ui.radio_value(&mut self.export_format, ExportFormat::Png, "PNG (bez strat)");
                ui.radio_value(&mut self.export_format, ExportFormat::Jpeg, "JPEG");
            });
            if self.export_format == ExportFormat::Jpeg {
                ui.horizontal(|ui| {
                    ui.label("Jakość JPEG:");
                    ui.add(egui::Slider::new(&mut self.jpeg_quality, 50..=100));
                });
            }
            ui.add_space(8.0);

            ui.heading("Wydajność");
            ui.horizontal(|ui| {
                ui.label("Pamięć na wczytane obrazy:");
                ui.add(egui::Slider::new(&mut self.cache_mb, 500..=8000).step_by(250.0).suffix(" MB"));
            });
            ui.small("Zmiana pamięci działa po ponownym uruchomieniu programu.");
            ui.add_space(8.0);
            if ui.button("Przywróć ustawienia domyślne").clicked() {
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
