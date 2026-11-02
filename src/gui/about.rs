//! Informacje o programie i autorze (okno „O programie” i panel macOS).

use eframe::egui::{self, Color32, RichText};

pub const AUTHOR: &str = "Adrian Kwiatkowski";
pub const WEBSITE: &str = "https://adriankwiatkowski.eu";
pub const EMAIL: &str = "kontakt@adriankwiatkowski.eu";
pub const REPO: &str = "https://github.com/adriank1410/mri-viewer";

/// Tekst do natywnego okna „O programie” na macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn credits() -> String {
    format!(
        "Przeglądarka badań MRI i innych obrazów DICOM z serwerem MCP dla Claude.\n\n\
         Autor: {AUTHOR}\n{}\n{EMAIL}\n\n\
         Narzędzie poglądowe – nie służy do diagnozy.\n\
         Licencja MIT · {}",
        WEBSITE.trim_start_matches("https://"),
        REPO.trim_start_matches("https://"),
    )
}

/// Zawartość okna „O programie” (Windows i Linux, a na macOS z menu Pomoc).
pub fn ui(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        if let Some(tex) = icon_texture(ui.ctx()) {
            ui.add(egui::Image::new((tex.id(), egui::vec2(96.0, 96.0))));
        }
        ui.add_space(4.0);
        ui.heading(RichText::new("Przeglądarka MRI").strong());
        ui.label(format!("Wersja {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(6.0);
        ui.label("Prosty podgląd badań MRI i innych obrazów DICOM z serwerem MCP dla Claude.");
        ui.add_space(8.0);
        ui.label(format!("Autor: {AUTHOR}"));
        ui.hyperlink_to(WEBSITE.trim_start_matches("https://"), WEBSITE);
        ui.hyperlink_to(EMAIL, format!("mailto:{EMAIL}"));
        ui.hyperlink_to("Kod źródłowy (licencja MIT)", REPO);
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(230, 170, 80), "Narzędzie poglądowe – nie służy do diagnozy.");
    });
}

fn icon_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique("about_icon_texture");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let img = image::load_from_memory(super::ICON_PNG).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let tex = ctx.load_texture("about_icon", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}
