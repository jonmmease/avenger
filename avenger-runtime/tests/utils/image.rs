use anyhow::{Result, bail};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use image::RgbaImage;
use pixelmatch;
use std::path::Path;

/// Compare a runtime-generated image against a baseline image
pub async fn assert_runtime_image_equal(
    scene_graph: &SceneGraph,
    path: &str,
    scale: f32,
    save_baseline: bool,
) -> Result<()> {
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [scene_graph.width, scene_graph.height],
            scale,
        },
        CanvasConfig::default(),
    )
    .await
    .expect("Failed to create canvas");

    canvas.set_scene(&scene_graph).expect("Failed to set scene");
    let generated_image = canvas.render().await?;
    let generated_buffer = image_to_png(&generated_image, path, save_baseline)?;
    let (baseline_image, baseline_buffer) = load_baseline_image(path)?;
    assert_images_equal(
        &generated_image,
        &baseline_image,
        &generated_buffer,
        &baseline_buffer,
    )?;
    Ok(())
}

/// Save an image to a PNG file and return it as a PNG-encoded buffer
pub fn image_to_png(image: &RgbaImage, path: &str, save_baseline: bool) -> Result<Vec<u8>> {
    let path = format!("tests/baselines/{}/App.png", path);
    let mut buffer = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut buffer),
        image::ImageFormat::Png,
    )?;

    if save_baseline {
        // Ensure the directory exists
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        image.save(path)?;
    }
    Ok(buffer)
}

/// Load a PNG image from a file and return it as a PNG-encoded buffer
pub fn load_baseline_image(path: &str) -> Result<(RgbaImage, Vec<u8>)> {
    let path = format!("tests/baselines/{}/App.png", path);
    let image = image::open(path)?.to_rgba8();

    let mut buffer = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut buffer),
        image::ImageFormat::Png,
    )?;

    Ok((image, buffer))
}

/// Compare two PNG-encoded images and return the number of different pixels
pub fn compare_images(generated_buffer: &[u8], baseline_buffer: &[u8]) -> Result<usize> {
    let mut diff_output = Vec::new();
    let Ok(diff_pixels) = pixelmatch::pixelmatch(
        std::io::Cursor::new(generated_buffer),
        std::io::Cursor::new(baseline_buffer),
        Some(&mut diff_output),
        None,
        None,
        None,
    ) else {
        bail!("Pixelmatch failed");
    };

    Ok(diff_pixels)
}

/// Assert that two images are the same, with helpful error messages
pub fn assert_images_equal(
    generated_image: &RgbaImage,
    baseline_image: &RgbaImage,
    generated_buffer: &[u8],
    baseline_buffer: &[u8],
) -> Result<()> {
    assert_eq!(
        generated_image.dimensions(),
        baseline_image.dimensions(),
        "Image dimensions do not match"
    );

    let diff_pixels = compare_images(generated_buffer, baseline_buffer)?;
    assert_eq!(
        diff_pixels, 0,
        "Generated image does not match baseline ({} pixel differences)",
        diff_pixels
    );
    Ok(())
}
