//! Writes the charts of the gallery into `site/`: each chart's bytes as `charts/<NAME>.bin`, and
//! `manifest.json`, the name, the title, and the description of each chart in the gallery's
//! order.
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
    let manifest = serde_json::to_string_pretty(&manifest).expect("the manifest encodes");
    std::fs::write(site.join("manifest.json"), manifest).expect("the manifest writes");
}
