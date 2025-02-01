use avenger_chart::error::AvengerChartError;
use avenger_chart::runtime::AvengerRuntime;
use avenger_chart::types::group::Group;
use datafusion::scalar::ScalarValue;
use image::RgbaImage;
use pixelmatch;
use std::path::Path;

/// Compare a runtime-generated image against a baseline image
pub async fn assert_runtime_image_equal(
    runtime: &AvengerRuntime,
    chart: Group,
    name: &str,
    save_baseline: bool,
    param_values: Vec<(&str, ScalarValue)>,
) -> Result<(), AvengerChartError> {
    let generated_image = runtime.to_image(chart, 2.0, param_values).await?;
    let generated_buffer = image_to_png(&generated_image, name, save_baseline)?;
    let (baseline_image, baseline_buffer) = load_baseline_image(name)?;
    assert_images_equal(
        &generated_image,
        &baseline_image,
        &generated_buffer,
        &baseline_buffer,
    )?;
    Ok(())
}

/// Save an image to a PNG file and return it as a PNG-encoded buffer
pub fn image_to_png(
    image: &RgbaImage,
    name: &str,
    save_baseline: bool,
) -> Result<Vec<u8>, AvengerChartError> {
    let path = format!("tests/baselines/{}.png", name);
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
pub fn load_baseline_image(name: &str) -> Result<(RgbaImage, Vec<u8>), AvengerChartError> {
    let path = format!("tests/baselines/{}.png", name);
    let image = image::open(path)
        .map_err(|e| AvengerChartError::InternalError(format!("Failed to load image: {}", e)))?
        .to_rgba8();

    let mut buffer = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut buffer),
        image::ImageFormat::Png,
    )?;

    Ok((image, buffer))
}

/// Compare two PNG-encoded images and return the number of different pixels
pub fn compare_images(
    generated_buffer: &[u8],
    baseline_buffer: &[u8],
) -> Result<usize, AvengerChartError> {
    let mut diff_output = Vec::new();
    let diff_pixels = pixelmatch::pixelmatch(
        std::io::Cursor::new(generated_buffer),
        std::io::Cursor::new(baseline_buffer),
        Some(&mut diff_output),
        None,
        None,
        None,
    )
    .map_err(|e| AvengerChartError::InternalError(format!("Pixelmatch failed: {}", e)))?;

    Ok(diff_pixels)
}

/// Assert that two images are the same, with helpful error messages
pub fn assert_images_equal(
    generated_image: &RgbaImage,
    baseline_image: &RgbaImage,
    generated_buffer: &[u8],
    baseline_buffer: &[u8],
) -> Result<(), AvengerChartError> {
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
