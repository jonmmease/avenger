//! The avenger-sql editor. Arguments are the CSV and Parquet files to open as tables, and the
//! options `--query <FILE>`, which opens a query and runs it, `--screenshot <PNG>`, which saves the
//! window once it settles and exits, and `--cursor <INDEX>`, `--press <KEYS>`, and `--hover`, which
//! place the cursor, press keys, and hover the pointer before the screenshot.

#[cfg(target_arch = "wasm32")]
fn main() {
    // The web build starts from the library, in `web::start`.
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use avenger_sql_editor::{EditorApp, Options};
    use eframe::egui;

    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("avenger-sql-editor: {error}");
            eprintln!("usage: avenger-sql-editor [FILE ...] [--query FILE] [--screenshot PNG] [--cursor INDEX [--press KEYS]... [--hover]]");
            std::process::exit(2);
        }
    };
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("avenger-sql editor")
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "avenger-sql editor",
        native,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc, options)?))),
    )
}
