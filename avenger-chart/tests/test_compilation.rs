// rustfmt::skip

use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart::error::AvengerChartError;
use avenger_chart::param::Param;
use avenger_chart::runtime::scale::scale_expr;
use avenger_chart::{
    runtime::AvengerRuntime,
    types::{
        group::Group,
        mark::Mark,
        scales::{Scale, ScaleDomain, ScaleRange},
    },
};
use avenger_scales::scales::linear::LinearScale;
use datafusion::prelude::*;
use image::{self, RgbaImage};
use palette::Srgba;
use pixelmatch;

#[tokio::test]
async fn test_arcs() -> Result<(), AvengerChartError> {
    // runtime
    let runtime = AvengerRuntime::new(SessionContext::new());

    // params
    let stroke_color = Param::new("stroke_color", "cyan");
    let width = Param::new("width", 300.0);

    // Load dataframe
    let schema = Schema::new(vec![Field::new("a", DataType::Float32, true)]);
    let columns = vec![Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as ArrayRef];
    let batch = RecordBatch::try_new(Arc::new(schema), columns).unwrap();
    let data_0 = runtime.ctx().read_batch(batch).unwrap();

    // scales
    let x_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(data_0.clone()), "a")
        .range(ScaleRange::new_interval(lit(0.0), &width));

    let y_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));

    let color_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    let chart = Group::new()
        .x(10.0)
        .y(10.0)
        .mark(
            Mark::arc()
                .from(data_0)
                .x(scale_expr(&x_scale, col("a")).unwrap())
                .y(scale_expr(&y_scale, lit(5.0)).unwrap())
                .start_angle(lit(0.0))
                .end_angle(lit(PI / 2.0))
                .outer_radius(lit(50.0))
                .inner_radius(lit(20.0))
                .fill(scale_expr(&color_scale, col("a")).unwrap())
                .stroke(&stroke_color)
                .stroke_width(lit(3.0)),
        )
        .param(width)
        .param(stroke_color);

    let generated_image = runtime.to_image(chart, 2.0).await?;

    // Save generated image and get its buffer
    let generated_buffer = image_to_png(&generated_image, "arcs", false)?;

    // Load baseline image and get its buffer
    let (baseline_image, baseline_buffer) = load_baseline_image("arcs")?;

    // Compare the images
    assert_images_equal(
        &generated_image,
        &baseline_image,
        &generated_buffer,
        &baseline_buffer,
    )?;

    Ok(())
}

/// Save an image to a PNG file and return it as a PNG-encoded buffer
fn image_to_png(
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
fn load_baseline_image(name: &str) -> Result<(RgbaImage, Vec<u8>), AvengerChartError> {
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
fn compare_images(
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
fn assert_images_equal(
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
