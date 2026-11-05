mod audiofile;
mod editors;
mod effects;
mod engine;
mod file;
mod midi;
mod modfx;
mod notetools;
mod plugin;
mod record;
mod settings;
mod smf;
mod tools;
mod ui;
mod widgets;
mod x11;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut screenshot: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        }
        // Child process used by the plugin scan (see plugin.rs).
        if a == "--clap-scan" {
            let Some(path) = args.next() else { std::process::exit(2) };
            let code = match plugin::scan_file_here(std::path::Path::new(&path)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            };
            std::process::exit(code);
        }
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Beat Studio")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Beat Studio",
        options,
        Box::new(move |cc| Ok(Box::new(ui::StudioApp::new(cc, screenshot)))),
    )
}
