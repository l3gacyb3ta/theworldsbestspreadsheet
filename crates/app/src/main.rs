mod app;
mod chart_view;
mod demo;
mod help_view;
mod syntax;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("sheet.wbs.json"));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title("the world's best spreadsheet"),
        ..Default::default()
    };
    eframe::run_native("wbs", options, Box::new(move |_cc| Ok(Box::new(app::App::new(path).with_welcome()))))
}
