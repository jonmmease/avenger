//! The avenger-sql editor, which opens the sample project when it starts. Its options are
//! `--project <PATH>`, which opens the project of a directory or of a path under it in place of
//! the sample project, and a file of it in a tab, `--open <PATH>`, which opens a file of the
//! project by its path relative to the project's root in a tab, `--screenshot <PNG>`, which saves
//! the window once it settles and exits, and `--cursor <INDEX>`, `--press <KEYS>`, and `--hover`,
//! which place the cursor, press keys, and hover the pointer before the screenshot.

#[cfg(target_arch = "wasm32")]
fn main() {
    // The web build starts from the library, in `web::start`.
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use avenger_sql_editor::{EditorApp, Options, APP_ID};
    use eframe::egui;

    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("avenger-sql-editor: {error}");
            eprintln!("usage: avenger-sql-editor [--project PATH] [--open PATH] [--screenshot PNG] [--cursor INDEX [--press KEYS]... [--hover]] [--theme system|light|dark]");
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
        APP_ID,
        native,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc, options)?))),
    )
}
