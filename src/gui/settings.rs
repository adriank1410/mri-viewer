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
            overlay_font: 13.0,
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
    }

    pub fn color(&self) -> egui::Color32 {
        let [r, g, b] = self.annotation_color;
        egui::Color32::from_rgb(r, g, b)
    }

    /// Okno ustawień. Zwraca true, gdy coś zmieniono.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let before = format!("{self:?}");
        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
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
