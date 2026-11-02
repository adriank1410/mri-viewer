//! Informacje o programie i autorze (okno „O programie” i panel macOS).

use eframe::egui::{self, Color32, RichText};

pub const AUTHOR: &str = "Adrian Kwiatkowski";
pub const WEBSITE: &str = "https://adriankwiatkowski.eu";
pub const REPO: &str = "https://github.com/adriank1410/mri-viewer";

/// Adres kontaktowy – zależy od języka (po polsku „kontakt@…”, w innych językach „contact@…”).
pub fn email() -> &'static str {
    t!("about.email")
}

/// Tekst do natywnego okna „O programie” na macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn credits() -> String {
    format!(
        "{}\n\n{}\n{}\n{}\n\n{}\n{} · {}",
        t!("about.tagline_long"),
        t!("about.author", name = AUTHOR),
        WEBSITE.trim_start_matches("https://"),
        email(),
        t!("about.disclaimer"),
        t!("about.license"),
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
        ui.heading(RichText::new(t!("about.app_name")).strong());
        ui.label(t!("about.version", version = env!("CARGO_PKG_VERSION")));
        ui.add_space(6.0);
        ui.label(t!("about.tagline"));
        ui.add_space(8.0);
        ui.label(t!("about.author", name = AUTHOR));
        ui.hyperlink_to(WEBSITE.trim_start_matches("https://"), WEBSITE);
        ui.hyperlink_to(email(), format!("mailto:{}", email()));
        ui.hyperlink_to(t!("about.source_code"), REPO);
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(230, 170, 80), t!("about.disclaimer"));
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
