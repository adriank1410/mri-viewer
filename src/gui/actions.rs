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
    Sub(MenuTitle, Vec<Entry>),
}

/// Nazwy menu i podmenu. Tekst jest tłumaczony przy każdym użyciu, bo język może się zmienić w trakcie pracy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuTitle {
    File,
    Edit,
    View,
    Image,
    Tools,
    Help,
    Export,
}

impl MenuTitle {
    pub fn text(&self) -> &'static str {
        match self {
            MenuTitle::File => t!("menu.file"),
            MenuTitle::Edit => t!("menu.edit"),
            MenuTitle::View => t!("menu.view"),
            MenuTitle::Image => t!("menu.image"),
            MenuTitle::Tools => t!("menu.tools"),
            MenuTitle::Help => t!("menu.help"),
            MenuTitle::Export => t!("menu.export"),
        }
    }
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
            OpenFolder => t!("menu.action.open_folder"),
            OpenFile => t!("menu.action.open_file"),
            CloseStudy => t!("menu.action.close_study"),
            SaveImage => t!("menu.action.save_image"),
            SaveView => t!("menu.action.save_view"),
            CopyView => t!("menu.action.copy_view"),
            ExportSeries => t!("menu.action.export_series"),
            ExportGif => t!("menu.action.export_gif"),
            ExportMontage => t!("menu.action.export_montage"),
            ExportTags => t!("menu.action.export_tags"),
            Undo => t!("menu.action.undo"),
            Redo => t!("menu.action.redo"),
            DeleteSelected => t!("menu.action.delete_selected"),
            DeleteSliceAnnotations => t!("menu.action.delete_slice_annotations"),
            DeleteAllAnnotations => t!("menu.action.delete_all_annotations"),
            Settings => t!("menu.action.settings"),
            Layout1 => t!("menu.action.layout_1"),
            Layout2 => t!("menu.action.layout_2"),
            Layout4 => t!("menu.action.layout_4"),
            ResetView => t!("menu.action.reset_view"),
            FitWindow => t!("menu.action.fit_window"),
            Zoom100 => t!("menu.action.zoom_100"),
            ZoomIn => t!("menu.action.zoom_in"),
            ZoomOut => t!("menu.action.zoom_out"),
            RotateRight => t!("menu.action.rotate_right"),
            RotateLeft => t!("menu.action.rotate_left"),
            FlipH => t!("menu.action.flip_h"),
            FlipV => t!("menu.action.flip_v"),
            Invert => t!("menu.action.invert"),
            ToggleOverlay => t!("menu.action.toggle_overlay"),
            ToggleReferenceLines => t!("menu.action.toggle_reference_lines"),
            ToggleSidebar => t!("menu.action.toggle_sidebar"),
            ToggleInfoPanel => t!("menu.action.toggle_info_panel"),
            ToggleFullscreen => t!("menu.action.toggle_fullscreen"),
            TogglePro => t!("menu.action.toggle_pro"),
            PlayPause => t!("menu.action.play_pause"),
            NextSlice => t!("menu.action.next_slice"),
            PrevSlice => t!("menu.action.prev_slice"),
            FirstSlice => t!("menu.action.first_slice"),
            LastSlice => t!("menu.action.last_slice"),
            ToolWindow => t!("menu.action.tool_window"),
            ToolPan => t!("menu.action.tool_pan"),
            ToolZoom => t!("menu.action.tool_zoom"),
            ToolLine => t!("menu.action.tool_line"),
            ToolAngle => t!("menu.action.tool_angle"),
            ToolEllipse => t!("menu.action.tool_ellipse"),
            ToolRect => t!("menu.action.tool_rect"),
            ToolArrow => t!("menu.action.tool_arrow"),
            ToolProbe => t!("menu.action.tool_probe"),
            ToolCursor3d => t!("menu.action.tool_cursor_3d"),
            PresetAuto => t!("menu.action.preset_auto"),
            PresetDicom => t!("menu.action.preset_dicom"),
            PresetFull => t!("menu.action.preset_full"),
            Help => t!("menu.action.help"),
            ConnectClaude => t!("menu.action.connect_claude"),
            About => t!("menu.action.about"),
            Quit => t!("menu.action.quit"),
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
pub fn menu_spec() -> Vec<(MenuTitle, Vec<Entry>)> {
    vec![
        (
            MenuTitle::File,
            vec![
                Item(OpenFolder),
                Item(OpenFile),
                Sep,
                Item(SaveImage),
                Item(SaveView),
                Item(CopyView),
                Sep,
                Sub(
                    MenuTitle::Export,
                    vec![Item(ExportSeries), Item(ExportGif), Item(ExportMontage), Item(ExportTags)],
                ),
                Sep,
                Item(CloseStudy),
            ],
        ),
        (
            MenuTitle::Edit,
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
            MenuTitle::View,
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
            MenuTitle::Image,
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
            MenuTitle::Tools,
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
        (MenuTitle::Help, vec![Item(Help), Item(ConnectClaude), Sep, Item(About)]),
    ]
}

/// Menu w oknie programu (Windows, Linux).
pub fn egui_menu(ui: &mut egui::Ui, pro: bool, out: &mut Vec<Action>) {
    egui::MenuBar::new().ui(ui, |ui| {
        for (title, entries) in menu_spec() {
            ui.menu_button(title.text(), |ui| {
                entries_ui(ui, &entries, pro, out);
                if title == MenuTitle::Edit {
                    ui.separator();
                    item_ui(ui, Settings, out);
                }
                if title == MenuTitle::File {
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
                ui.menu_button(title.text(), |ui| entries_ui(ui, items, pro, out));
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
    use std::cell::RefCell;
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
                    let s = Submenu::new(title.text(), true);
                    build(items, &s, keep);
                    let _ = sub.append(&s);
                    keep.push(Box::new(s));
                }
            }
        }
    }

    /// Znacznik zainstalowanego menu. Samo menu (z elementami) żyje w `CURRENT`,
    /// żeby [`rebuild_menu`] mógł je podmienić po zmianie języka.
    pub struct NativeMenu {
        _private: (),
    }

    /// Zbudowane menu wraz z elementami, które muszą żyć tak długo jak ono.
    struct Built {
        _menu: Menu,
        _keep: Vec<Box<dyn std::any::Any>>,
    }

    thread_local! {
        // muda::Menu nie jest Send – menu tworzymy i trzymamy w wątku głównym
        static CURRENT: RefCell<Option<Built>> = const { RefCell::new(None) };
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
        rebuild_menu();
        NativeMenu { _private: () }
    }

    /// Buduje menu od nowa w bieżącym języku (razem z danymi okna „O programie”) i podmienia je w NSApp.
    /// Identyfikatory poleceń i kanał zdarzeń zostają te same, więc [`poll`] działa dalej.
    /// Wywoływać z wątku głównego, np. z `update` po zmianie języka.
    pub fn rebuild_menu() {
        let built = build_menu();
        // stare menu zwalniamy dopiero po podmianie w NSApp
        let old = CURRENT.with(|c| c.borrow_mut().replace(built));
        drop(old);
    }

    fn build_menu() -> Built {
        let menu = Menu::new();
        let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        // nazwa programu w pasku menu – macOS i tak pokazuje tu nazwę z pakietu aplikacji
        let app = Submenu::new("MRI Viewer", true);
        let about = PredefinedMenuItem::about(
            Some(t!("menu.app.about")),
            Some(AboutMetadata {
                name: Some(t!("menu.app.name").into()),
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
            &PredefinedMenuItem::services(Some(t!("menu.app.services"))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(t!("menu.app.hide"))),
            &PredefinedMenuItem::hide_others(Some(t!("menu.app.hide_others"))),
            &PredefinedMenuItem::show_all(Some(t!("menu.app.show_all"))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some(t!("menu.app.quit"))),
        ]);
        let _ = menu.append(&app);
        keep.push(Box::new(settings));
        keep.push(Box::new(app));
        for (title, entries) in menu_spec() {
            let sub = Submenu::new(title.text(), true);
            build(&entries, &sub, &mut keep);
            if title == MenuTitle::View {
                // okno systemowe: minimalizacja
            }
            let _ = menu.append(&sub);
            if title == MenuTitle::Help {
                sub.set_as_help_menu_for_nsapp();
            }
            keep.push(Box::new(sub));
        }
        let window = Submenu::new(t!("menu.window"), true);
        let _ = window.append_items(&[
            &PredefinedMenuItem::minimize(Some(t!("menu.window.minimize"))),
            &PredefinedMenuItem::fullscreen(Some(t!("menu.window.fullscreen"))),
        ]);
        let _ = menu.append(&window);
        window.set_as_windows_menu_for_nsapp();
        keep.push(Box::new(window));
        menu.init_for_nsapp();
        Built { _menu: menu, _keep: keep }
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
