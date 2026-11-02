//! Polecenia programu, skróty klawiszowe i menu (systemowe na macOS, w oknie na innych systemach).

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Action {
    OpenFolder,
    OpenFile,
    CloseStudy,
    SaveImage,
    SaveView,
    CopyView,
    ExportSeries,
    ExportGif,
    ExportMontage,
    ExportTags,
    Undo,
    Redo,
    DeleteSelected,
    DeleteSliceAnnotations,
    DeleteAllAnnotations,
    Settings,
    Layout1,
    Layout2,
    Layout4,
    ResetView,
    FitWindow,
    Zoom100,
    ZoomIn,
    ZoomOut,
    RotateRight,
    RotateLeft,
    FlipH,
    FlipV,
    Invert,
    ToggleOverlay,
    ToggleReferenceLines,
    ToggleSidebar,
    ToggleInfoPanel,
    ToggleFullscreen,
    TogglePro,
    PlayPause,
    NextSlice,
    PrevSlice,
    FirstSlice,
    LastSlice,
    ToolWindow,
    ToolPan,
    ToolZoom,
    ToolLine,
    ToolAngle,
    ToolEllipse,
    ToolRect,
    ToolArrow,
    ToolProbe,
    ToolCursor3d,
    PresetAuto,
    PresetDicom,
    PresetFull,
    Help,
    ConnectClaude,
    About,
    Quit,
}

pub enum Entry {
    Item(Action),
    Sep,
    Sub(&'static str, Vec<Entry>),
}

use Action::*;
use Entry::{Item, Sep, Sub};

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl Action {
    pub fn id(&self) -> String {
        format!("{self:?}")
    }

    pub fn from_id(id: &str) -> Option<Action> {
        ALL.iter().copied().find(|a| a.id() == id)
    }

    pub fn label(&self) -> &'static str {
        match self {
            OpenFolder => "Otwórz folder z badaniem…",
            OpenFile => "Otwórz plik DICOM…",
            CloseStudy => "Zamknij badanie",
            SaveImage => "Zapisz obraz…",
            SaveView => "Zapisz widok z opisami i pomiarami…",
            CopyView => "Kopiuj widok do schowka",
            ExportSeries => "Eksportuj całą serię jako obrazy…",
            ExportGif => "Eksportuj serię jako animację GIF…",
            ExportMontage => "Eksportuj mozaikę miniatur…",
            ExportTags => "Eksportuj tagi DICOM do pliku tekstowego…",
            Undo => "Cofnij",
            Redo => "Ponów",
            DeleteSelected => "Usuń zaznaczony pomiar (Delete)",
            DeleteSliceAnnotations => "Usuń pomiary z tego obrazu",
            DeleteAllAnnotations => "Usuń wszystkie pomiary",
            Settings => "Ustawienia…",
            Layout1 => "Jedno okno",
            Layout2 => "Dwa okna obok siebie",
            Layout4 => "Cztery okna (2 × 2)",
            ResetView => "Resetuj widok (R)",
            FitWindow => "Dopasuj do okna",
            Zoom100 => "Rzeczywisty rozmiar (1:1)",
            ZoomIn => "Powiększ",
            ZoomOut => "Pomniejsz",
            RotateRight => "Obróć w prawo",
            RotateLeft => "Obróć w lewo",
            FlipH => "Odbij w poziomie",
            FlipV => "Odbij w pionie",
            Invert => "Negatyw (I)",
            ToggleOverlay => "Pokaż/ukryj opisy na obrazie",
            ToggleReferenceLines => "Pokaż/ukryj linie odniesienia",
            ToggleSidebar => "Pokaż/ukryj listę serii",
            ToggleInfoPanel => "Pokaż/ukryj panel informacji",
            ToggleFullscreen => "Pełny ekran",
            TogglePro => "Tryb profesjonalny",
            PlayPause => "Odtwarzaj / zatrzymaj (spacja)",
            NextSlice => "Następny obraz (↓)",
            PrevSlice => "Poprzedni obraz (↑)",
            FirstSlice => "Pierwszy obraz (Home)",
            LastSlice => "Ostatni obraz (End)",
            ToolWindow => "Jasność i kontrast (W)",
            ToolPan => "Przesuwanie (P)",
            ToolZoom => "Lupa / powiększanie (Z)",
            ToolLine => "Pomiar odległości (M)",
            ToolAngle => "Pomiar kąta (A)",
            ToolEllipse => "Obszar owalny – ROI (E)",
            ToolRect => "Obszar prostokątny – ROI (O)",
            ToolArrow => "Strzałka z opisem (T)",
            ToolProbe => "Sonda – wartość punktu (V)",
            ToolCursor3d => "Wskaźnik 3D – pokaż punkt w innych oknach (X)",
            PresetAuto => "Okno automatyczne",
            PresetDicom => "Okno z aparatu",
            PresetFull => "Pełny zakres",
            Help => "Pomoc i skróty klawiszowe",
            ConnectClaude => "Podłącz do Claude (MCP)…",
            About => "O programie",
            Quit => "Zakończ",
        }
    }

    /// Skrót z modyfikatorem – obsługiwany przez menu systemowe (macOS) albo przez program.
    pub fn shortcut(&self) -> Option<KeyboardShortcut> {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let k = |m, key| Some(KeyboardShortcut::new(m, key));
        match self {
            OpenFolder => k(cmd, Key::O),
            OpenFile => k(cmd_shift, Key::O),
            CloseStudy => k(cmd, Key::W),
            SaveImage => k(cmd, Key::S),
            SaveView => k(cmd_shift, Key::S),
            CopyView => k(cmd_shift, Key::C),
            ExportGif => k(cmd_shift, Key::G),
            Undo => k(cmd, Key::Z),
            Redo => k(cmd_shift, Key::Z),
            Settings => k(cmd, Key::Comma),
            Layout1 => k(cmd, Key::Num1),
            Layout2 => k(cmd, Key::Num2),
            Layout4 => k(cmd, Key::Num4),
            Zoom100 => k(cmd, Key::Num0),
            ZoomIn => k(cmd, Key::Equals),
            ZoomOut => k(cmd, Key::Minus),
            RotateRight => k(cmd, Key::R),
            RotateLeft => k(cmd_shift, Key::R),
            ToggleSidebar => k(cmd, Key::B),
            ToggleInfoPanel => k(cmd, Key::I),
            TogglePro => k(cmd_shift, Key::P),
            Help => k(cmd_shift, Key::Questionmark),
            ToggleFullscreen => k(Modifiers::COMMAND | Modifiers::CTRL, Key::F),
            Quit => k(cmd, Key::Q),
            _ => None,
        }
    }

    /// Skróty jednoklawiszowe (bez modyfikatorów), działają, gdy nie wpisujemy tekstu.
    pub fn plain_key(&self) -> Option<Key> {
        Some(match self {
            ResetView => Key::R,
            Invert => Key::I,
            PlayPause => Key::Space,
            FirstSlice => Key::Home,
            LastSlice => Key::End,
            ToolWindow => Key::W,
            ToolPan => Key::P,
            ToolZoom => Key::Z,
            ToolLine => Key::M,
            ToolAngle => Key::A,
            ToolEllipse => Key::E,
            ToolRect => Key::O,
            ToolArrow => Key::T,
            ToolProbe => Key::V,
            ToolCursor3d => Key::X,
            FitWindow => Key::F,
            _ => return None,
        })
    }

    /// Skróty pomijane podczas pisania: cofanie i Ctrl+W obsługuje samo pole tekstowe,
    /// a zamknięcie badania czy obrót obrazu przy wpisywaniu tekstu byłyby zaskoczeniem.
    pub fn conflicts_with_text_edit(&self) -> bool {
        matches!(self, Undo | Redo | CloseStudy | RotateRight | RotateLeft)
    }

    pub fn pro_only(&self) -> bool {
        matches!(self, ToolAngle | ToolEllipse | ToolRect | ToolProbe | ToolCursor3d | ExportTags | ToggleInfoPanel)
    }
}

pub const ALL: &[Action] = &[
    OpenFolder, OpenFile, CloseStudy, SaveImage, SaveView, CopyView, ExportSeries, ExportGif, ExportMontage, ExportTags,
    Undo, Redo, DeleteSelected, DeleteSliceAnnotations, DeleteAllAnnotations, Settings, Layout1, Layout2, Layout4,
    ResetView, FitWindow, Zoom100, ZoomIn, ZoomOut, RotateRight, RotateLeft, FlipH, FlipV, Invert, ToggleOverlay,
    ToggleReferenceLines, ToggleSidebar, ToggleInfoPanel, ToggleFullscreen, TogglePro, PlayPause, NextSlice, PrevSlice,
    FirstSlice, LastSlice, ToolWindow, ToolPan, ToolZoom, ToolLine, ToolAngle, ToolEllipse, ToolRect, ToolArrow,
    ToolProbe, ToolCursor3d, PresetAuto, PresetDicom, PresetFull, Help, ConnectClaude, About, Quit,
];

/// Skróty z modyfikatorami, od najbardziej szczegółowych (Ctrl+Shift+Z przed Ctrl+Z),
/// bo `consume_shortcut` dopuszcza nadmiarowy Shift.
pub fn shortcuts_by_specificity() -> Vec<(Action, KeyboardShortcut)> {
    let count = |m: Modifiers| [m.alt, m.ctrl, m.shift, m.command || m.mac_cmd].iter().filter(|b| **b).count();
    let mut list: Vec<_> = ALL.iter().filter_map(|a| a.shortcut().map(|s| (*a, s))).collect();
    list.sort_by_key(|(_, s)| std::cmp::Reverse(count(s.modifiers)));
    list
}

/// Struktura menu wspólna dla menu systemowego i menu w oknie.
pub fn menu_spec() -> Vec<(&'static str, Vec<Entry>)> {
    vec![
        (
            "Plik",
            vec![
                Item(OpenFolder),
                Item(OpenFile),
                Sep,
                Item(SaveImage),
                Item(SaveView),
                Item(CopyView),
                Sep,
                Sub(
                    "Eksportuj",
                    vec![Item(ExportSeries), Item(ExportGif), Item(ExportMontage), Item(ExportTags)],
                ),
                Sep,
                Item(CloseStudy),
            ],
        ),
        (
            "Edycja",
            vec![
                Item(Undo),
                Item(Redo),
                Sep,
                Item(DeleteSelected),
                Item(DeleteSliceAnnotations),
                Item(DeleteAllAnnotations),
            ],
        ),
        (
            "Widok",
            vec![
                Item(Layout1),
                Item(Layout2),
                Item(Layout4),
                Sep,
                Item(ResetView),
                Item(FitWindow),
                Item(Zoom100),
                Item(ZoomIn),
                Item(ZoomOut),
                Sep,
                Item(RotateRight),
                Item(RotateLeft),
                Item(FlipH),
                Item(FlipV),
                Sep,
                Item(ToggleOverlay),
                Item(ToggleReferenceLines),
                Item(ToggleSidebar),
                Item(ToggleInfoPanel),
                Item(ToggleFullscreen),
            ],
        ),
        (
            "Obraz",
            vec![
                Item(PlayPause),
                Item(NextSlice),
                Item(PrevSlice),
                Item(FirstSlice),
                Item(LastSlice),
                Sep,
                Item(PresetAuto),
                Item(PresetDicom),
                Item(PresetFull),
                Item(Invert),
            ],
        ),
        (
            "Narzędzia",
            vec![
                Item(ToolWindow),
                Item(ToolPan),
                Item(ToolZoom),
                Sep,
                Item(ToolLine),
                Item(ToolAngle),
                Item(ToolEllipse),
                Item(ToolRect),
                Item(ToolArrow),
                Item(ToolProbe),
                Item(ToolCursor3d),
                Sep,
                Item(TogglePro),
            ],
        ),
        ("Pomoc", vec![Item(Help), Item(ConnectClaude), Sep, Item(About)]),
    ]
}

/// Menu w oknie programu (Windows, Linux).
pub fn egui_menu(ui: &mut egui::Ui, pro: bool, out: &mut Vec<Action>) {
    egui::MenuBar::new().ui(ui, |ui| {
        for (title, entries) in menu_spec() {
            ui.menu_button(title, |ui| {
                entries_ui(ui, &entries, pro, out);
                if title == "Edycja" {
                    ui.separator();
                    item_ui(ui, Settings, out);
                }
                if title == "Plik" {
                    ui.separator();
                    item_ui(ui, Quit, out);
                }
            });
        }
    });
}

fn item_ui(ui: &mut egui::Ui, a: Action, out: &mut Vec<Action>) {
    let mut b = egui::Button::new(a.label());
    if let Some(s) = a.shortcut() {
        b = b.shortcut_text(ui.ctx().format_shortcut(&s));
    }
    if ui.add(b).clicked() {
        out.push(a);
        ui.close();
    }
}

fn entries_ui(ui: &mut egui::Ui, entries: &[Entry], pro: bool, out: &mut Vec<Action>) {
    for e in entries {
        match e {
            Item(a) => {
                if a.pro_only() && !pro {
                    continue;
                }
                item_ui(ui, *a, out);
            }
            Sep => {
                ui.separator();
            }
            Sub(title, items) => {
                ui.menu_button(*title, |ui| entries_ui(ui, items, pro, out));
            }
        }
    }
}

/// Natywne menu macOS (pasek na górze ekranu).
#[cfg(target_os = "macos")]
pub mod native {
    use super::*;
    use muda::accelerator::{Accelerator, Code, Modifiers as MM};
    use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Mutex, OnceLock};

    fn code(key: Key) -> Option<Code> {
        Some(match key {
            Key::O => Code::KeyO,
            Key::W => Code::KeyW,
            Key::S => Code::KeyS,
            Key::C => Code::KeyC,
            Key::G => Code::KeyG,
            Key::Z => Code::KeyZ,
            Key::Comma => Code::Comma,
            Key::Num0 => Code::Digit0,
            Key::Num1 => Code::Digit1,
            Key::Num2 => Code::Digit2,
            Key::Num4 => Code::Digit4,
            Key::Equals => Code::Equal,
            Key::Minus => Code::Minus,
            Key::R => Code::KeyR,
            Key::B => Code::KeyB,
            Key::I => Code::KeyI,
            Key::P => Code::KeyP,
            Key::F => Code::KeyF,
            Key::Q => Code::KeyQ,
            Key::Questionmark => Code::Slash,
            _ => return None,
        })
    }

    fn accel(a: Action) -> Option<Accelerator> {
        let s = a.shortcut()?;
        let mut m = MM::empty();
        if s.modifiers.command || s.modifiers.mac_cmd {
            m |= MM::META;
        }
        if s.modifiers.shift {
            m |= MM::SHIFT;
        }
        if s.modifiers.ctrl {
            m |= MM::CONTROL;
        }
        if s.modifiers.alt {
            m |= MM::ALT;
        }
        Some(Accelerator::new(m, code(s.logical_key)?))
    }

    fn item(a: Action) -> MenuItem {
        MenuItem::with_id(a.id(), a.label(), true, accel(a))
    }

    fn build(entries: &[Entry], sub: &Submenu, keep: &mut Vec<Box<dyn std::any::Any>>) {
        for e in entries {
            match e {
                Entry::Item(a) => {
                    let it = item(*a);
                    let _ = sub.append(&it);
                    keep.push(Box::new(it));
                }
                Entry::Sep => {
                    let _ = sub.append(&PredefinedMenuItem::separator());
                }
                Entry::Sub(title, items) => {
                    let s = Submenu::new(*title, true);
                    build(items, &s, keep);
                    let _ = sub.append(&s);
                    keep.push(Box::new(s));
                }
            }
        }
    }

    pub struct NativeMenu {
        _menu: Menu,
        _keep: Vec<Box<dyn std::any::Any>>,
    }

    fn about_icon() -> Option<muda::Icon> {
        let img = image::load_from_memory(crate::gui::ICON_PNG).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        muda::Icon::from_rgba(img.into_raw(), w, h).ok()
    }

    static EVENTS: OnceLock<Mutex<Receiver<MenuEvent>>> = OnceLock::new();

    pub fn install(ctx: &egui::Context) -> NativeMenu {
        // klik w menu nie jest zdarzeniem okna – trzeba obudzić egui, inaczej czeka na ruch myszy
        let (tx, rx) = channel();
        if EVENTS.set(Mutex::new(rx)).is_ok() {
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let _ = tx.send(e);
                ctx.request_repaint();
            }));
        }
        let menu = Menu::new();
        let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        let app = Submenu::new("MRI Viewer", true);
        let about = PredefinedMenuItem::about(
            Some("O programie Przeglądarka MRI"),
            Some(AboutMetadata {
                name: Some("Przeglądarka MRI".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                // pusty numer kompilacji – inaczej macOS pokazuje „0.2.0 (0.2.0)”
                short_version: Some(String::new()),
                copyright: Some(format!("© 2026 {}", crate::gui::about::AUTHOR)),
                credits: Some(crate::gui::about::credits()),
                icon: about_icon(),
                ..Default::default()
            }),
        );
        let settings = item(Action::Settings);
        let _ = app.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(Some("Usługi")),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some("Ukryj Przeglądarkę MRI")),
            &PredefinedMenuItem::hide_others(Some("Ukryj pozostałe")),
            &PredefinedMenuItem::show_all(Some("Pokaż wszystkie")),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some("Zakończ Przeglądarkę MRI")),
        ]);
        let _ = menu.append(&app);
        keep.push(Box::new(settings));
        keep.push(Box::new(app));
        for (title, entries) in menu_spec() {
            let sub = Submenu::new(title, true);
            build(&entries, &sub, &mut keep);
            if title == "Widok" {
                // okno systemowe: minimalizacja
            }
            let _ = menu.append(&sub);
            if title == "Pomoc" {
                sub.set_as_help_menu_for_nsapp();
            }
            keep.push(Box::new(sub));
        }
        let window = Submenu::new("Okno", true);
        let _ = window.append_items(&[
            &PredefinedMenuItem::minimize(Some("Minimalizuj")),
            &PredefinedMenuItem::fullscreen(Some("Pełny ekran")),
        ]);
        let _ = menu.append(&window);
        window.set_as_windows_menu_for_nsapp();
        keep.push(Box::new(window));
        menu.init_for_nsapp();
        NativeMenu { _menu: menu, _keep: keep }
    }

    pub fn poll(out: &mut Vec<Action>) {
        let Some(rx) = EVENTS.get() else { return };
        let rx = rx.lock().unwrap();
        while let Ok(ev) = rx.try_recv() {
            if let Some(a) = Action::from_id(ev.id.as_ref()) {
                out.push(a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(list: &[(Action, KeyboardShortcut)], a: Action) -> usize {
        list.iter().position(|(x, _)| *x == a).unwrap()
    }

    #[test]
    fn shift_variants_checked_first() {
        let list = shortcuts_by_specificity();
        for (more, less) in [(Redo, Undo), (OpenFile, OpenFolder), (SaveView, SaveImage), (RotateLeft, RotateRight)] {
            assert!(pos(&list, more) < pos(&list, less), "{more:?} przed {less:?}");
        }
        assert_eq!(list.len(), ALL.iter().filter(|a| a.shortcut().is_some()).count());
    }

    #[test]
    fn redo_not_taken_by_undo() {
        // jak consume_shortcut: Ctrl+Shift+Z ma trafić w Ponów, nie w Cofnij
        let pressed = Modifiers::COMMAND | Modifiers::SHIFT;
        let hit = shortcuts_by_specificity()
            .into_iter()
            .find(|(_, s)| s.logical_key == Key::Z && pressed.matches_logically(s.modifiers))
            .map(|(a, _)| a);
        assert_eq!(hit, Some(Redo));
    }
}
