//! Menu commands. The native menu bar (macOS), the in-window menu bar (other
//! platforms) and the keyboard shortcuts all end up in `App::run`.

use super::*;
use eframe::egui::KeyboardShortcut;
use files::Pending;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    FillDown,
    ToggleTrace,
    Help,
    SearchHelp,
}

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Command::New => "New",
            Command::Open => "Open…",
            Command::Save => "Save",
            Command::SaveAs => "Save As…",
            Command::Quit => "Quit",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::Cut => "Cut",
            Command::Copy => "Copy",
            Command::Paste => "Paste",
            Command::FillDown => "Fill Down",
            Command::ToggleTrace => "Trace Precedents & Dependents",
            Command::Help => "Help for Selection",
            Command::SearchHelp => "Search Help",
        }
    }

    /// The shortcut, as egui sees it (⌘ is `COMMAND`: Cmd on macOS, Ctrl elsewhere).
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let (m, k) = match self {
            Command::New => (cmd, Key::N),
            Command::Open => (cmd, Key::O),
            Command::Save => (cmd, Key::S),
            Command::SaveAs => (cmd_shift, Key::S),
            Command::Quit => (cmd, Key::Q),
            Command::Undo => (cmd, Key::Z),
            Command::Redo => (cmd_shift, Key::Z),
            Command::Cut => (cmd, Key::X),
            Command::Copy => (cmd, Key::C),
            Command::Paste => (cmd, Key::V),
            Command::FillDown => (cmd, Key::D),
            Command::ToggleTrace => return None,
            Command::Help => (Modifiers::NONE, Key::F1),
            Command::SearchHelp => (cmd, Key::Slash),
        };
        Some(KeyboardShortcut::new(m, k))
    }
}

impl App {
    /// Queue a command for the next frame (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn queue(&mut self, c: Command) {
        self.queued.push(c);
    }

    pub(super) fn run_queued(&mut self, ctx: &egui::Context) {
        let mut cmds = std::mem::take(&mut self.queued);
        if let Some(m) = &self.native_menu {
            cmds.extend(m.drain());
        }
        for c in cmds {
            self.run(ctx, c);
        }
    }

    pub(super) fn run(&mut self, ctx: &egui::Context, c: Command) {
        self.dirty_stale = true;
        ctx.request_repaint();
        // While a text field has focus, edit commands belong to it: replay
        // them as the keystroke so the field's own undo, copy etc. run.
        let replay = |app: &mut App| {
            let s = c.shortcut().unwrap();
            app.inject.push(Event::Key { key: s.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: s.modifiers });
        };
        match c {
            Command::New => self.guarded(ctx, Pending::New),
            Command::Open => self.guarded(ctx, Pending::Open),
            Command::Save => {
                self.commit();
                self.save();
            }
            Command::SaveAs => {
                self.commit();
                self.save_as();
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            // the native menu works whichever window is in front: edits go to the help window's fields
            Command::Undo | Command::Redo | Command::FillDown | Command::Cut | Command::Copy | Command::Paste if self.help.has_focus() => {
                self.help.forward(c)
            }
            Command::Undo | Command::Redo | Command::FillDown if ctx.egui_wants_keyboard_input() => replay(self),
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::FillDown => self.fill_down(),
            // The integration reads/writes the clipboard and hands us (or the
            // focused text field) the usual Copy/Cut/Paste events.
            Command::Cut => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
            Command::Copy => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
            Command::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Command::ToggleTrace => self.trace = !self.trace,
            Command::Help => self.context_help(),
            Command::SearchHelp => self.help.focus_search(),
        }
    }

    /// Document shortcuts handled here unless the native menu owns them (its
    /// key equivalents fire first and would otherwise run twice).
    /// F1 stays ours everywhere: the native menu only shows it in the title.
    pub(super) fn command_keys(&mut self, ctx: &egui::Context) {
        for &c in self.owned_keys() {
            let s = c.shortcut().unwrap();
            if ctx.input_mut(|i| i.consume_shortcut(&s)) {
                self.run(ctx, c);
            }
        }
    }

    /// The shortcuts `command_keys` handles (the help window handles them too).
    pub(super) fn owned_keys(&self) -> &'static [Command] {
        if self.native_menu.is_some() {
            &[Command::Help]
        } else {
            &[Command::SaveAs, Command::Save, Command::New, Command::Open, Command::Quit, Command::Help, Command::SearchHelp]
        }
    }

    /// In-window menu bar for platforms without a native one.
    pub(super) fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut picked = None;
        let item = |ui: &mut Ui, c: Command, enabled: bool, picked: &mut Option<Command>| {
            let mut b = egui::Button::new(c.label());
            if let Some(s) = c.shortcut() {
                b = b.shortcut_text(ctx.format_shortcut(&s));
            }
            if ui.add_enabled(enabled, b).clicked() {
                *picked = Some(c);
            }
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                for c in [Command::New, Command::Open, Command::Save, Command::SaveAs] {
                    item(ui, c, true, &mut picked);
                }
                ui.separator();
                item(ui, Command::Quit, true, &mut picked);
            });
            ui.menu_button("Edit", |ui| {
                item(ui, Command::Undo, !self.undo.is_empty(), &mut picked);
                item(ui, Command::Redo, !self.redo.is_empty(), &mut picked);
                ui.separator();
                for c in [Command::Cut, Command::Copy, Command::Paste] {
                    item(ui, c, true, &mut picked);
                }
                ui.separator();
                item(ui, Command::FillDown, true, &mut picked);
            });
            ui.menu_button("View", |ui| {
                if ui.checkbox(&mut self.trace.clone(), Command::ToggleTrace.label()).clicked() {
                    picked = Some(Command::ToggleTrace);
                }
            });
            ui.menu_button("Help", |ui| {
                item(ui, Command::Help, true, &mut picked);
                item(ui, Command::SearchHelp, true, &mut picked);
            });
        });
        if let Some(c) = picked {
            self.run(&ctx, c);
        }
    }
}
