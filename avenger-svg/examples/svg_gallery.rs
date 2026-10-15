//! Export the shared scene gallery as an SVG document.
use avenger_render_fixtures::scene;
use avenger_typst_label::{EngineOptions, LabelEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "svg-gallery.svg".into());
    let engine = LabelEngine::new(EngineOptions {
        fonts: scene::fonts(),
    });
    let svg = avenger_svg::SvgRenderer::new().render_scene_graph(&scene::gallery(), &engine)?;
    std::fs::write(&output, &svg)?;
    println!("Saved {output}");
    Ok(())
}
