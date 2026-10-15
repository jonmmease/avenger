//! Export the shared scene gallery as a PDF document.
use avenger_render_fixtures::scene;
use avenger_typst_label::{EngineOptions, LabelEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pdf-gallery.pdf".into());
    let engine = LabelEngine::new(EngineOptions {
        fonts: scene::fonts(),
    });
    let pdf = avenger_pdf::PdfRenderer::new().render_scene_graph(&scene::gallery(), &engine)?;
    std::fs::write(&output, &pdf)?;
    println!("Saved {output}");
    Ok(())
}
