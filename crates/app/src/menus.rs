//! The native menu bar (macOS only, via muda). Menu clicks and their key
//! equivalents become `Command`s that the app drains each frame.
//!
//! Linux has no equivalent: muda's Linux menus are GTK widgets that need a GTK
//! window, and eframe's windows come from winit, so there the app draws an
//! in-window menu bar instead.

use crate::app::Command;
use eframe::egui;
use std::sync::{Arc, Mutex};

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct NativeMenu {
    queue: Arc<Mutex<Vec<Command>>>,
    #[cfg(target_os = "macos")]
    mac: mac::Menus,
}

impl NativeMenu {
    /// Install the menu bar. Must run on the main thread once the event loop
    /// is running (eframe's app-creation callback is a good place).
    #[cfg(target_os = "macos")]
    pub fn install(ctx: &egui::Context) -> Option<NativeMenu> {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let mac = mac::Menus::build();
        let (q, ctx) = (queue.clone(), ctx.clone());
        let ids = mac.ids.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            if let Some((_, c)) = ids.iter().find(|(id, _)| *id == e.id) {
                q.lock().unwrap().push(*c);
                ctx.request_repaint();
            }
        }));
        Some(NativeMenu { queue, mac })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn install(_ctx: &egui::Context) -> Option<NativeMenu> {
        None
    }

    pub fn drain(&self) -> Vec<Command> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }

    /// Keep check marks and enabled states in step with the app.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub fn sync(&self, trace: bool, can_undo: bool, can_redo: bool) {
        #[cfg(target_os = "macos")]
        self.mac.sync(trace, can_undo, can_redo);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{AboutMetadata, CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    pub struct Menus {
        _bar: Menu,
        pub ids: Vec<(MenuId, Command)>,
        trace: CheckMenuItem,
        undo: MenuItem,
        redo: MenuItem,
        state: std::cell::Cell<Option<(bool, bool, bool)>>,
    }

    fn accel(c: Command) -> Option<Accelerator> {
        let s = c.shortcut()?;
        let mut m = Modifiers::empty();
        if s.modifiers.command {
            m |= Modifiers::META;
        }
        if s.modifiers.shift {
            m |= Modifiers::SHIFT;
        }
        use egui::Key as K;
        let code = match s.logical_key {
            K::N => Code::KeyN,
            K::O => Code::KeyO,
            K::S => Code::KeyS,
            K::Q => Code::KeyQ,
            K::Z => Code::KeyZ,
            K::X => Code::KeyX,
            K::C => Code::KeyC,
            K::V => Code::KeyV,
            K::D => Code::KeyD,
            K::Slash => Code::Slash,
            K::Comma => Code::Comma,
            _ => return None,
        };
        Some(Accelerator::new(m, code))
    }

    impl Menus {
        pub fn build() -> Menus {
            let mut ids = Vec::new();
            let mut item = |c: Command| {
                // F1 has no key equivalent (whether AppKit would swallow a bare
                // function key varies), so the app keeps handling it and the
                // menu just says so
                let text = if c == Command::Help { format!("{} (F1)", c.label()) } else { c.label().to_string() };
                let it = MenuItem::new(text, true, accel(c));
                ids.push((it.id().clone(), c));
                it
            };
            let (new, open, save, save_as, quit) = (item(Command::New), item(Command::Open), item(Command::Save), item(Command::SaveAs), item(Command::Quit));
            let (undo, redo, cut, copy, paste, fill) =
                (item(Command::Undo), item(Command::Redo), item(Command::Cut), item(Command::Copy), item(Command::Paste), item(Command::FillDown));
            let copy_values = item(Command::CopyValues);
            let (help, search, settings) = (item(Command::Help), item(Command::SearchHelp), item(Command::Settings));
            let trace = CheckMenuItem::new(Command::ToggleTrace.label(), true, true, None);
            ids.push((trace.id().clone(), Command::ToggleTrace));

            let about = AboutMetadata {
                name: Some("the world's best spreadsheet".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            };
            let sep = PredefinedMenuItem::separator;
            let app_menu = Submenu::with_items(
                "wbs",
                true,
                &[
                    &PredefinedMenuItem::about(None, Some(about)),
                    &sep(),
                    &settings,
                    &sep(),
                    &PredefinedMenuItem::services(None),
                    &sep(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    // ours, not the predefined Quit: that terminates without asking to save
                    &quit,
                ],
            )
            .unwrap();
            let file = Submenu::with_items("File", true, &[&new, &open, &sep(), &save, &save_as]).unwrap();
            let edit = Submenu::with_items("Edit", true, &[&undo, &redo, &sep(), &cut, &copy, &copy_values, &paste, &sep(), &fill]).unwrap();
            let view = Submenu::with_items("View", true, &[&trace]).unwrap();
            let window = Submenu::with_items(
                "Window",
                true,
                &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None), &sep(), &PredefinedMenuItem::fullscreen(None)],
            )
            .unwrap();
            let help_menu = Submenu::with_items("Help", true, &[&help, &search]).unwrap();
            let bar = Menu::with_items(&[&app_menu, &file, &edit, &view, &window, &help_menu]).unwrap();
            bar.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help_menu.set_as_help_menu_for_nsapp();
            Menus { _bar: bar, ids, trace, undo, redo, state: std::cell::Cell::new(None) }
        }

        pub fn sync(&self, trace: bool, can_undo: bool, can_redo: bool) {
            let now = Some((trace, can_undo, can_redo));
            if self.state.get() != now {
                self.trace.set_checked(trace);
                self.undo.set_enabled(can_undo);
                self.redo.set_enabled(can_redo);
                self.state.set(now);
            }
        }
    }
}
