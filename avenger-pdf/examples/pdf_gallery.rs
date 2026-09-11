//! Export the shared scene gallery to PDF. Set AVENGER_PDFIUM_LIBRARY_PATH for a PNG preview.
#[path = "../tests/support/pdf_raster.rs"]
mod pdf_raster;
#[path = "../../tests/render_fixtures/scene.rs"]
mod scene;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pdf-gallery.pdf".into());
    let scene = scene::gallery();
    let pdf = avenger_pdf::PdfRenderer::new()
        .with_options(avenger_pdf::PdfRenderOptions {
            font_resolution: scene::fonts(),
            ..Default::default()
        })
        .render_scene_graph(&scene)?;
    std::fs::write(&output, &pdf)?;
    if std::env::var_os("AVENGER_PDFIUM_LIBRARY_PATH").is_some() {
        pdf_raster::pdf_to_png(&pdf, scene.width, scene.height)
            .save(std::path::Path::new(&output).with_extension("png"))?;
    }
    println!("Saved {output}");
    Ok(())
}
