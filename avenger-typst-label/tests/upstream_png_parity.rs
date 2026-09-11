#![cfg(all(feature = "raster", feature = "upstream-png-parity"))]

mod common;

use std::{
    fs,
    fs::File,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};

use avenger_typst_label::{
    FontWeight, LabelEngine, LabelOptions, MathFontSpec, RasterOptions, rasterize,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Cases {
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    source: String,
    font_size: f32,
    font_weight: u16,
    text_font: String,
    math_font: String,
    pixel_tolerance: u32,
    min_similarity: f64,
    #[serde(default)]
    requires_system_emoji: bool,
}

#[test]
fn upstream_png_parity() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures_dir = manifest_dir.join("tests/fixtures/upstream_png");
    let cases_path = fixtures_dir.join("cases.toml");
    let mut cases: Cases = toml::from_str(
        &fs::read_to_string(&cases_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", cases_path.display())),
    )
    .unwrap_or_else(|err| panic!("failed to parse {}: {err}", cases_path.display()));
    cases.case.sort_by(|left, right| left.id.cmp(&right.id));

    let mut engine_options = common::engine_options();
    engine_options.fonts.load_system_fonts = false;
    let engine = LabelEngine::new(engine_options).expect("label engine should initialize");

    let mut failures = Vec::new();
    for case in &cases.case {
        if case.requires_system_emoji
            && !Path::new("/System/Library/Fonts/Apple Color Emoji.ttc").is_file()
        {
            eprintln!(
                "skipping {} because Apple Color Emoji is unavailable",
                case.id
            );
            continue;
        }

        if let Err(err) = run_case(&engine, &fixtures_dir, case) {
            failures.push(err);
        }
    }

    if !failures.is_empty() {
        panic!("upstream PNG parity failures:\n{}", failures.join("\n\n"));
    }
}

fn run_case(engine: &LabelEngine, fixtures_dir: &Path, case: &Case) -> Result<(), String> {
    let source_path = fixtures_dir.join("src").join(&case.source);
    let source = read_label_source(&source_path)?;
    let expected_path = fixtures_dir.join("ref").join(format!("{}.png", case.id));
    if !expected_path.is_file() {
        return Err(format!(
            "missing upstream PNG reference for {}; run:\n  cargo run --release -p avenger-typst-label --features upstream-png-parity --bin generate_upstream_png_refs",
            case.id
        ));
    }

    let mut options = LabelOptions::default();
    options.text.font_family = case.text_font.clone();
    options.text.font_size = case.font_size;
    options.text.font_weight = FontWeight::Number(case.font_weight);
    options.math.font = math_font_spec(&case.math_font);
    options.math.font_size = case.font_size;
    options.math.font_weight = FontWeight::Number(case.font_weight);

    let compiled = engine
        .compile(&source, &options)
        .map_err(|err| format!("{}: failed to compile Avenger label: {err}", case.id))?;
    let actual = rasterize(&compiled, &RasterOptions { scale: 1.0 })
        .map_err(|err| format!("{}: failed to rasterize Avenger label: {err}", case.id))?;

    let expected = read_png_rgba(&expected_path).map_err(|err| {
        format!(
            "{}: failed to open {}: {err}",
            case.id,
            expected_path.display()
        )
    })?;
    let actual =
        TestRgbaImage::from_vec(actual.image.width, actual.image.height, actual.image.data)
            .ok_or_else(|| {
                format!(
                    "{}: raster buffer dimensions do not match data length",
                    case.id
                )
            })?;

    let expected = crop_to_content(&composite_over_white(&expected), 4)
        .ok_or_else(|| format!("{}: expected reference is blank", case.id))?;
    let actual = crop_to_content(&composite_over_white(&actual), 4)
        .ok_or_else(|| format!("{}: actual render is blank", case.id))?;

    let report = compare_images(&expected, &actual);
    let width_delta = expected.width.abs_diff(actual.width);
    let height_delta = expected.height.abs_diff(actual.height);
    let dimension_failed =
        width_delta > case.pixel_tolerance || height_delta > case.pixel_tolerance;
    let similarity_failed = report.similarity < case.min_similarity;
    if dimension_failed || similarity_failed {
        let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/tests/upstream_png_parity")
            .join(&case.id);
        fs::create_dir_all(&output_dir)
            .map_err(|err| format!("{}: failed to create failure dir: {err}", case.id))?;
        write_png_rgba(&output_dir.join("expected.png"), &expected)
            .map_err(|err| format!("{}: failed to save expected artifact: {err}", case.id))?;
        write_png_rgba(&output_dir.join("actual.png"), &actual)
            .map_err(|err| format!("{}: failed to save actual artifact: {err}", case.id))?;
        write_png_rgba(&output_dir.join("diff.png"), &report.diff)
            .map_err(|err| format!("{}: failed to save diff artifact: {err}", case.id))?;

        return Err(format!(
            "{}: dimension_failed={}, similarity_failed={}; similarity {:.6} (min {:.6}); dimensions differ by {}x{} px (allowed {}); max delta {}, mean delta {:.4}; artifacts: {}",
            case.id,
            dimension_failed,
            similarity_failed,
            report.similarity,
            case.min_similarity,
            width_delta,
            height_delta,
            case.pixel_tolerance,
            report.max_channel_delta,
            report.mean_channel_delta,
            output_dir.display()
        ));
    }

    Ok(())
}

fn read_label_source(path: &Path) -> Result<String, String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(source.trim_end_matches(['\r', '\n']).to_string())
}

fn math_font_spec(font: &str) -> MathFontSpec {
    match font {
        "Lete Sans Math" | "LeteSansMath" => MathFontSpec::LeteSansMath,
        "New Computer Modern Math" | "NewComputerModernMath" => MathFontSpec::NewComputerModernMath,
        family => MathFontSpec::Family(family.to_string()),
    }
}

fn composite_over_white(image: &TestRgbaImage) -> TestRgbaImage {
    let mut output = TestRgbaImage::new(image.width, image.height);
    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.pixel(x, y);
            let alpha = f32::from(pixel[3]) / 255.0;
            let red = composite_channel(pixel[0], alpha);
            let green = composite_channel(pixel[1], alpha);
            let blue = composite_channel(pixel[2], alpha);
            output.put_pixel(x, y, [red, green, blue, 255]);
        }
    }
    output
}

fn composite_channel(channel: u8, alpha: f32) -> u8 {
    (f32::from(channel) * alpha + 255.0 * (1.0 - alpha)).round() as u8
}

fn crop_to_content(image: &TestRgbaImage, padding: u32) -> Option<TestRgbaImage> {
    let mut min_x = image.width;
    let mut min_y = image.height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.pixel(x, y);
            if pixel[0] < 250 || pixel[1] < 250 || pixel[2] < 250 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if !found {
        return None;
    }

    min_x = min_x.saturating_sub(padding);
    min_y = min_y.saturating_sub(padding);
    max_x = (max_x + padding).min(image.width.saturating_sub(1));
    max_y = (max_y + padding).min(image.height.saturating_sub(1));

    Some(image.crop(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

struct CompareReport {
    similarity: f64,
    mean_channel_delta: f64,
    max_channel_delta: u8,
    diff: TestRgbaImage,
}

fn compare_images(expected: &TestRgbaImage, actual: &TestRgbaImage) -> CompareReport {
    let width = expected.width.max(actual.width);
    let height = expected.height.max(actual.height);
    let expected = pad_to(expected, width, height);
    let actual = pad_to(actual, width, height);
    let mut diff = TestRgbaImage::new(width, height);
    let mut max_channel_delta = 0u8;
    let mut total_delta = 0u64;
    let mut channels = 0u64;

    for y in 0..height {
        for x in 0..width {
            let expected_pixel = expected.pixel(x, y);
            let actual_pixel = actual.pixel(x, y);
            let red = expected_pixel[0].abs_diff(actual_pixel[0]);
            let green = expected_pixel[1].abs_diff(actual_pixel[1]);
            let blue = expected_pixel[2].abs_diff(actual_pixel[2]);
            max_channel_delta = max_channel_delta.max(red).max(green).max(blue);
            total_delta += u64::from(red) + u64::from(green) + u64::from(blue);
            channels += 3;
            diff.put_pixel(
                x,
                y,
                [
                    red.saturating_mul(4),
                    green.saturating_mul(4),
                    blue.saturating_mul(4),
                    255,
                ],
            );
        }
    }

    let mean_channel_delta = total_delta as f64 / channels as f64;
    let similarity = 1.0 - total_delta as f64 / (channels as f64 * 255.0);
    CompareReport {
        similarity,
        mean_channel_delta,
        max_channel_delta,
        diff,
    }
}

fn pad_to(image: &TestRgbaImage, width: u32, height: u32) -> TestRgbaImage {
    let mut padded = TestRgbaImage::from_pixel(width, height, [255, 255, 255, 255]);
    for y in 0..image.height {
        for x in 0..image.width {
            padded.put_pixel(x, y, image.pixel(x, y));
        }
    }
    padded
}

#[derive(Clone)]
struct TestRgbaImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl TestRgbaImage {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    fn from_pixel(width: u32, height: u32, pixel: [u8; 4]) -> Self {
        let mut image = Self::new(width, height);
        for chunk in image.data.chunks_mut(4) {
            chunk.copy_from_slice(&pixel);
        }
        image
    }

    fn from_vec(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        if data.len() == width as usize * height as usize * 4 {
            Some(Self {
                width,
                height,
                data,
            })
        } else {
            None
        }
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let index = self.index(x, y);
        [
            self.data[index],
            self.data[index + 1],
            self.data[index + 2],
            self.data[index + 3],
        ]
    }

    fn put_pixel(&mut self, x: u32, y: u32, pixel: [u8; 4]) {
        let index = self.index(x, y);
        self.data[index..index + 4].copy_from_slice(&pixel);
    }

    fn crop(&self, left: u32, top: u32, width: u32, height: u32) -> Self {
        let mut cropped = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                cropped.put_pixel(x, y, self.pixel(left + x, top + y));
            }
        }
        cropped
    }

    fn index(&self, x: u32, y: u32) -> usize {
        ((y as usize * self.width as usize) + x as usize) * 4
    }
}

fn read_png_rgba(path: &Path) -> Result<TestRgbaImage, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output buffer size overflowed".to_string())?;
    let mut buffer = vec![0; buffer_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|err| err.to_string())?;
    let decoded = &buffer[..info.buffer_size()];
    let rgba = convert_png_to_rgba(info.color_type, info.bit_depth, decoded)?;
    TestRgbaImage::from_vec(info.width, info.height, rgba)
        .ok_or_else(|| "PNG RGBA data did not match dimensions".to_string())
}

fn write_png_rgba(path: &Path, image: &TestRgbaImage) -> Result<(), String> {
    let file = File::create(path).map_err(|err| err.to_string())?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&image.data))
        .map_err(|err| err.to_string())
}

fn convert_png_to_rgba(
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    if bit_depth != png::BitDepth::Eight {
        return Err(format!("unsupported PNG bit depth: {bit_depth:?}"));
    }

    Ok(match color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(data.len() / 3 * 4);
            for pixel in data.chunks_exact(3) {
                out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(data.len() / 2 * 4);
            for pixel in data.chunks_exact(2) {
                out.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(data.len() * 4);
            for gray in data {
                out.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
            out
        }
        png::ColorType::Indexed => {
            return Err("unsupported indexed PNG output".to_string());
        }
    })
}
