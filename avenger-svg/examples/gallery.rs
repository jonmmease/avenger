//! Export a scene gallery and a PNG preview without a GPU or window.
#[path = "../../tests/render_fixtures/raster.rs"]
#[allow(dead_code)]
mod raster;
#[path = "../../tests/render_fixtures/scene.rs"]
mod scene;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "svg-gallery.svg".into());
    let svg = avenger_svg::SvgRenderer::new()
        .with_options(avenger_svg::SvgRenderOptions {
            font_resolution: scene::fonts(),
            ..Default::default()
        })
        .render_scene_graph(&scene::gallery())?;
    std::fs::write(&output, &svg)?;
    raster::svg_to_png(&svg, 2.0).save(std::path::Path::new(&output).with_extension("png"))?;
    println!("Saved {output}");
    Ok(())
}
