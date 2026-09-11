//! Render the shared export gallery for comparison with SVG and PDF output.
#[path = "../../tests/render_fixtures/scene.rs"]
mod scene;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "wgpu-gallery.png".into());
    let scene = scene::gallery();
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig {
            font_resolution: scene::fonts(),
            ..Default::default()
        },
    ))?;
    canvas.set_scene(&scene)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
