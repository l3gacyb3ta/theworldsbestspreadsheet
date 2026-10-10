mod app;
mod chart_view;
mod demo;
mod fonts;
mod help_view;
mod menus;
mod prefs;
mod settings_view;
mod syntax;

use std::path::PathBuf;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> eframe::Result {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title("the world's best spreadsheet"),
        ..Default::default()
    };
    eframe::run_native(
        "wbs",
        options,
        Box::new(move |cc| {
            // the file named on the command line, else the last one used, else ./sheet.wbs.json
            let last = cc.storage.and_then(|s| s.get_string(app::LAST_FILE_KEY)).map(PathBuf::from).filter(|p| p.exists());
            let path = arg.or(last).unwrap_or_else(|| PathBuf::from("sheet.wbs.json"));
            let help = cc.storage.and_then(|s| s.get_string(app::HELP_WINDOW_KEY));
            let prefs = prefs::Prefs::default_path().map(prefs::Prefs::load).unwrap_or_else(prefs::Prefs::in_memory);
            Ok(Box::new(app::App::new(path).with_prefs(prefs).with_help_geometry(help).with_welcome().with_native_menu(&cc.egui_ctx)))
        }),
    )
}
