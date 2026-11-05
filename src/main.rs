// No console window behind the studio on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod arrange;
mod audiofile;
mod dynlib;
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

/// The window icon, from packaging/beatstudio.png (8-bit RGBA).
fn icon() -> eframe::egui::IconData {
    let decoder = png::Decoder::new(&include_bytes!("../packaging/beatstudio.png")[..]);
    let read = decoder.read_info().ok().and_then(|mut r| {
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).ok()?;
        (info.color_type == png::ColorType::Rgba).then(|| eframe::egui::IconData { rgba: buf[..info.buffer_size()].to_vec(), width: info.width, height: info.height })
    });
    read.unwrap_or_default()
}

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
            .with_min_inner_size([900.0, 560.0])
            .with_app_id("beatstudio")
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Beat Studio",
        options,
        Box::new(move |cc| Ok(Box::new(ui::StudioApp::new(cc, screenshot)))),
    )
}
