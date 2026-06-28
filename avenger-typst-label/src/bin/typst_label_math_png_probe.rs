use std::{error::Error, fs::File, io::BufWriter, path::PathBuf};

use avenger_typst_label::{FontWeight, LabelEngine, LabelOptions, RasterOptions, rasterize};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/typst-label-math-png-probe/math-label.png"));

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let engine = LabelEngine::new(Default::default())?;
    let mut options = LabelOptions::default();
    options.text.font_size = 36.0;
    options.text.font_weight = FontWeight::Number(500);
    options.math.font_size = 36.0;
    options.math.font_weight = FontWeight::Number(500);

    let label = engine.compile(r"$y = sqrt(x) / (1 + x^2)$", &options)?;
    let raster = rasterize(&label, &RasterOptions { scale: 2.0 })?;
    let width = raster.image.width;
    let height = raster.image.height;
    let expected_len = width as usize * height as usize * 4;
    if raster.image.data.len() != expected_len {
        return Err("raster dimensions did not match RGBA data length".into());
    }
    let file = File::create(&output)?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(&raster.image.data)?;

    let png_bytes = std::fs::metadata(&output)?.len();
    println!(
        "{} {}x{} png_bytes={} metrics={:.2}x{:.2}",
        output.display(),
        width,
        height,
        png_bytes,
        label.metrics.width,
        label.metrics.height,
    );
    Ok(())
}
