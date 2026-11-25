//! Writes the charts of the gallery into `site/`: each chart's bytes as `charts/<NAME>.bin`, the
//! faces that `avenger-fonts` bundles as `fonts/<FILE>`, which the page adds, since the library
//! embeds no fonts, and `manifest.json`: the name, the title, and the description of each chart in
//! the gallery's order, and the fonts' files in the order that the page adds them. It copies the
//! library's `<avenger-chart>` element beside them, as `avenger-chart.js`.
//!
//! Usage: `build-charts [SITE_DIR]`, where `SITE_DIR` is the example's `site` by default.

use std::path::PathBuf;

fn main() {
    let site = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("site"));
    let charts = site.join("charts");
    std::fs::create_dir_all(&charts).expect("the charts directory");
    let mut manifest = Vec::new();
    for entry in chart_web::gallery() {
        let bytes = entry.chart.to_bytes().expect("the chart encodes");
        std::fs::write(charts.join(format!("{}.bin", entry.name)), &bytes)
            .expect("the chart's file writes");
        println!("{}: {} bytes", entry.name, bytes.len());
        manifest.push(serde_json::json!({
            "name": entry.name,
            "title": entry.title,
            "description": entry.description,
        }));
    }
    let fonts = site.join("fonts");
    std::fs::create_dir_all(&fonts).expect("the fonts directory");
    let mut font_files = Vec::new();
    for (file, face) in avenger_fonts::FACES {
        std::fs::write(fonts.join(file), avenger_fonts::decompress(face))
            .expect("the font's file writes");
        font_files.push(format!("fonts/{file}"));
    }
    let manifest = serde_json::json!({ "charts": manifest, "fonts": font_files });
    std::fs::write(
        site.join("avenger-chart.js"),
        include_str!("../../../avenger-chart-web/element/avenger-chart.js"),
    )
    .expect("the element writes");
    let manifest = serde_json::to_string_pretty(&manifest).expect("the manifest encodes");
    std::fs::write(site.join("manifest.json"), manifest).expect("the manifest writes");
}
