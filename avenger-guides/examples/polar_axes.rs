//! Polar axes on radar charts, rose charts, and parts of circles, rendered to PNG sheets that
//! compare their options.
//!
//! ```bash
//! cargo run --release -p avenger-guides --example polar_axes -- <OUTPUT_DIR>
//! ```

use std::sync::Arc;

use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::PolarLabelOrient;
use avenger_render_fixtures::{
    polar::{halo, radar, rose, sector, sheet, Panel},
    raster::svg_to_png,
};
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;

/// Renders `panels` in a row to `<name>.png` in `out`.
fn render(name: &str, panels: Vec<Panel>, out: &std::path::Path, engine: &LabelEngine) {
    let scene = sheet(&panels, panels.len(), engine);
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, engine)
        .unwrap();
    svg_to_png(&svg, 2.0)
        .save(out.join(format!("{name}.png")))
        .unwrap();
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("an output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let engine = avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
    render(
        "radius-angle",
        vec![
            radar("Start spoke (0°)", None, None),
            radar("22.5° off the start", Some(22.5), None),
            radar("Half a step (30°)", Some(30.0), None),
            radar("Start spoke, halo", None, Some(halo(2.0))),
        ],
        &out,
        &engine,
    );
    render(
        "orientations",
        vec![
            rose("Horizontal", PolarLabelOrient::Horizontal, None),
            rose("Radial", PolarLabelOrient::Radial, None),
            rose("Tangential", PolarLabelOrient::Tangential, None),
            rose(
                "Horizontal, halo",
                PolarLabelOrient::Horizontal,
                Some(halo(2.0)),
            ),
        ],
        &out,
        &engine,
    );
    render(
        "partial-circles",
        vec![
            sector("Half donut, -90° to 90°", -90.0, 90.0, 0.5),
            sector("Quarter, 0° to 90°", 0.0, 90.0, 0.0),
            sector("Sector, 30° to 150°", 30.0, 150.0, 0.0),
            sector("Whole circle", 0.0, 360.0, 0.0),
        ],
        &out,
        &engine,
    );
}
