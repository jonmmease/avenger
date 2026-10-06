//! Compares rasterized labels with PNG references rendered by upstream Typst.
//!
//! The generator puts each label in a box on an auto-sized page with a `MARGIN_PT` margin, so a
//! reference's logical size is its page size minus the margins, and the label's logical origin
//! sits at the margin. Each case runs two checks:
//!
//! - `metrics`: the label's width and height match the reference's logical size, within the
//!   half-pixel rounding of the page size.
//! - `ink`: with both images aligned at their logical origins, the ink agrees. Similarity is one
//!   minus the mean, over pixels that are ink in either image, of each pixel's largest channel
//!   difference. It is taken in every em-sized tile, and the worst tile must reach
//!   `MIN_SIMILARITY`. A shifted, resized, re-weighted, recolored or swapped glyph fails it.
//!
//! Every case must pass both checks. Failing cases write `expected.png`, `actual.png` and
//! `diff.png` to the gitignored `tests/output/upstream_png/{id}/`.

#![cfg(all(feature = "raster", feature = "upstream-png-parity"))]

mod common;

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};

use avenger_color::AbsoluteColor;
use avenger_typst_label::{
    FontWeight, LabelEngine, LabelOptions, RasterImage, RasterOptions, rasterize,
};
use common::oracle::output_dir;
use serde::Deserialize;

/// The page margin the generator puts around each label.
const MARGIN_PT: f64 = 128.0;

/// A pixel is ink when any channel is darker than this over white.
const INK_THRESHOLD: u8 = 250;

/// The lowest worst-tile ink similarity a case may have. Matching renders score 0.93 or more;
/// the mutations in `png_comparison_rejects_mutations` that keep the label's size score 0.87 or
/// less.
const MIN_SIMILARITY: f64 = 0.90;

#[derive(Debug, Deserialize)]
struct Cases {
    case: Vec<Case>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    upstream_tests: Vec<String>,
    #[allow(dead_code)]
    note: Option<String>,
    source: String,
    font_size: f32,
    #[serde(default = "default_scale")]
    scale: f32,
    font_weight: u16,
    text_font: String,
    math_font: String,
    #[serde(default)]
    requires_system_emoji: bool,
}

fn default_scale() -> f32 {
    2.0
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstream_png")
}

fn load_cases() -> Vec<Case> {
    let path = fixtures_dir().join("cases.toml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let mut cases: Cases =
        toml::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    cases.case.sort_by(|left, right| left.id.cmp(&right.id));
    cases.case
}

fn engine() -> LabelEngine {
    let mut options = common::engine_options();
    options.fonts.load_system_fonts = false;
    LabelEngine::new(options)
}

fn emoji_available() -> bool {
    Path::new("/System/Library/Fonts/Apple Color Emoji.ttc").is_file()
}

#[test]
fn upstream_png_parity() {
    let engine = engine();
    let out_dir = output_dir("upstream_png");
    fs::remove_dir_all(&out_dir).ok();

    let mut failures = vec![];
    for case in load_cases() {
        if case.requires_system_emoji && !emoji_available() {
            eprintln!("skipping {} because Apple Color Emoji is unavailable", case.id);
            continue;
        }
        let reference = Reference::load(&case);
        let source = read_label_source(&fixtures_dir().join("src").join(&case.source));
        match render(&engine, &source, &label_options(&case), case.scale) {
            Ok(actual) => {
                let comparison = compare(&reference, &actual, case.scale, case.font_size);
                let failed = comparison.failed_checks();
                if !failed.is_empty() {
                    comparison.write_artifacts(&out_dir.join(&case.id));
                    failures.push(format!(
                        "{}: fails {failed:?}\n{}",
                        case.id,
                        comparison.describe()
                    ));
                }
            }
            Err(err) => failures.push(format!("{}: {err}", case.id)),
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Mutated renders must fail the comparison: the suite has to notice a 3% size change, bold
/// instead of medium, a recolor, a one-pixel shift, and one-glyph swaps.
#[test]
fn png_comparison_rejects_mutations() {
    let engine = engine();
    let mut accepted = Vec::new();
    let mut tried = 0;
    for case in load_cases() {
        if case.requires_system_emoji {
            continue;
        }
        let reference = Reference::load(&case);
        let source = read_label_source(&fixtures_dir().join("src").join(&case.source));
        let Ok(unmutated) = render(&engine, &source, &label_options(&case), case.scale)
        else {
            continue;
        };
        for mutation in Mutation::ALL {
            let Some((source, options)) = mutation.apply(&case, &source) else {
                continue;
            };
            let Ok(mut actual) = render(&engine, &source, &options, case.scale) else {
                continue;
            };
            if let Mutation::Shift = mutation {
                actual.origin_x += 1.0 / f64::from(case.scale);
            }
            if actual.same_as(&unmutated) {
                // The mutation changed nothing, for example bold text that is already bold.
                continue;
            }
            tried += 1;
            let c = compare(&reference, &actual, case.scale, case.font_size);
            if c.failed_checks().is_empty() {
                accepted.push(format!("{} with {mutation:?}", case.id));
            }
        }
    }
    assert!(tried > 200, "only {tried} mutations ran");
    assert!(
        accepted.is_empty(),
        "{} of {tried} mutated renders passed:\n{}",
        accepted.len(),
        accepted.join("\n")
    );
}

#[derive(Debug, Clone, Copy)]
enum Mutation {
    /// Text and math 3% larger.
    Larger,
    /// Weight 700 instead of the case's weight.
    Bold,
    /// A dark red fill instead of black.
    Recolor,
    /// The raster moved one pixel to the right.
    Shift,
    /// The first `^2` replaced by `^3`.
    Script,
    /// The first `e` replaced by `c`.
    Letter,
    /// The first `hat` replaced by `tilde`.
    Accent,
}

impl Mutation {
    const ALL: [Self; 7] = [
        Self::Larger,
        Self::Bold,
        Self::Recolor,
        Self::Shift,
        Self::Script,
        Self::Letter,
        Self::Accent,
    ];

    fn apply(self, case: &Case, source: &str) -> Option<(String, LabelOptions)> {
        let mut options = label_options(case);
        let source = match self {
            Self::Larger => {
                options.text.font_size *= 1.03;
                source.to_string()
            }
            Self::Bold => {
                if case.font_weight >= 700 {
                    return None;
                }
                options.text.font_weight = FontWeight::BOLD;
                options.math.font_weight = Some(FontWeight::BOLD);
                source.to_string()
            }
            Self::Recolor => {
                options.text.fill = AbsoluteColor::from_srgb(0.6, 0.0, 0.0, 1.0);
                source.to_string()
            }
            Self::Shift => source.to_string(),
            Self::Script => {
                source.contains("^2").then(|| source.replacen("^2", "^3", 1))?
            }
            Self::Letter => source.contains('e').then(|| source.replacen('e', "c", 1))?,
            Self::Accent => source
                .contains("hat(")
                .then(|| source.replacen("hat(", "tilde(", 1))?,
        };
        Some((source, options))
    }
}

fn label_options(case: &Case) -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = case.text_font.clone();
    options.text.font_size = case.font_size;
    options.text.font_weight = FontWeight::from_number(case.font_weight);
    // The generator's `#show math.equation: set text(font: .., weight: ..)`.
    options.math.font_family = case.math_font.clone();
    options.math.font_weight = Some(FontWeight::from_number(case.font_weight));
    options
}

/// A label's raster with its logical size and origin, in points.
struct Rendered {
    image: TestRgbaImage,
    width: f64,
    height: f64,
    /// Where the raster's top-left pixel sits relative to the logical origin.
    origin_x: f64,
    origin_y: f64,
}

fn render(
    engine: &LabelEngine,
    source: &str,
    options: &LabelOptions,
    scale: f32,
) -> Result<Rendered, String> {
    let compiled = engine
        .compile(source, options)
        .map_err(|err| format!("failed to compile: {err}"))?;
    let RasterImage { image, origin_x, origin_y, .. } =
        rasterize(&compiled, &RasterOptions { scale })
            .map_err(|err| format!("failed to rasterize: {err}"))?;
    let image = TestRgbaImage::from_vec(image.width, image.height, image.data)
        .ok_or("raster buffer dimensions do not match data length")?;
    Ok(Rendered {
        image: composite_over_white(&image),
        width: f64::from(compiled.metrics.width),
        height: f64::from(compiled.metrics.height),
        origin_x: f64::from(origin_x),
        origin_y: f64::from(origin_y),
    })
}

impl Rendered {
    fn same_as(&self, other: &Self) -> bool {
        self.image.data == other.image.data
            && self.origin_x == other.origin_x
            && self.origin_y == other.origin_y
    }
}

/// An upstream reference and its logical size.
struct Reference {
    image: TestRgbaImage,
    width: f64,
    height: f64,
}

impl Reference {
    fn load(case: &Case) -> Self {
        let path = fixtures_dir().join("ref").join(format!("{}.png", case.id));
        let image = read_png_rgba(&path).unwrap_or_else(|err| {
            panic!(
                "{}: failed to open {} ({err}); regenerate with generate_upstream_png_refs",
                case.id,
                path.display()
            )
        });
        let scale = f64::from(case.scale);
        Self {
            width: f64::from(image.width) / scale - 2.0 * MARGIN_PT,
            height: f64::from(image.height) / scale - 2.0 * MARGIN_PT,
            image: composite_over_white(&image),
        }
    }
}

struct Comparison {
    width_delta: f64,
    height_delta: f64,
    /// The allowed logical size difference: half a pixel, from the page size's rounding.
    size_tolerance: f64,
    /// Ink similarity over the whole label.
    similarity: f64,
    /// The worst ink similarity of any em-sized tile.
    local: f64,
    expected: TestRgbaImage,
    actual: TestRgbaImage,
    diff: TestRgbaImage,
}

/// Aligns both images at the label's logical origin and compares the ink.
fn compare(
    reference: &Reference,
    actual: &Rendered,
    scale: f32,
    font_size: f32,
) -> Comparison {
    let scale = f64::from(scale);
    // Pixel offset of the actual raster inside the reference image.
    let dx = (MARGIN_PT * scale + actual.origin_x * scale).round() as i64;
    let dy = (MARGIN_PT * scale + actual.origin_y * scale).round() as i64;

    let expected_box = ink_bounds(&reference.image, 0, 0);
    let actual_box = ink_bounds(&actual.image, dx, dy);
    let bounds = match (expected_box, actual_box) {
        (Some(a), Some(b)) => {
            Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
        }
        (a, b) => a.or(b),
    };
    let [x0, y0, x1, y1] =
        bounds.map_or([0, 0, 0, 0], |[x0, y0, x1, y1]| [x0 - 4, y0 - 4, x1 + 5, y1 + 5]);
    let width = (x1 - x0).max(1) as u32;
    let height = (y1 - y0).max(1) as u32;

    let sample = |image: &TestRgbaImage, ox: i64, oy: i64, x: i64, y: i64| -> [u8; 4] {
        let (ix, iy) = (x - ox, y - oy);
        if ix < 0
            || iy < 0
            || ix >= i64::from(image.width)
            || iy >= i64::from(image.height)
        {
            [255; 4]
        } else {
            image.pixel(ix as u32, iy as u32)
        }
    };

    let mut expected = TestRgbaImage::new(width, height);
    let mut aligned = TestRgbaImage::new(width, height);
    let mut diff = TestRgbaImage::new(width, height);
    let mut total = 0u64;
    let mut ink_pixels = 0u64;
    for y in 0..height {
        for x in 0..width {
            let (px, py) = (x0 + i64::from(x), y0 + i64::from(y));
            let e = sample(&reference.image, 0, 0, px, py);
            let a = sample(&actual.image, dx, dy, px, py);
            expected.put_pixel(x, y, e);
            aligned.put_pixel(x, y, a);
            let delta = (0..3).map(|c| e[c].abs_diff(a[c])).max().unwrap();
            let shade = 255 - delta.saturating_mul(4);
            diff.put_pixel(x, y, [255, shade, shade, 255]);
            if is_ink(e) || is_ink(a) {
                ink_pixels += 1;
                total += u64::from(delta);
            }
        }
    }
    let similarity = if ink_pixels == 0 {
        1.0
    } else {
        1.0 - total as f64 / (ink_pixels as f64 * 255.0)
    };
    let tile = (f64::from(font_size) * scale).round().max(4.0) as u32;
    let local = worst_tile_similarity(&expected, &aligned, tile);

    Comparison {
        width_delta: actual.width - reference.width,
        height_delta: actual.height - reference.height,
        size_tolerance: 0.5 / scale + 1e-3,
        similarity,
        local,
        expected,
        actual: aligned,
        diff,
    }
}

/// The lowest ink similarity over em-sized tiles at half-tile steps. A local defect such as a
/// swapped glyph barely moves the label-wide mean but dominates its tile.
fn worst_tile_similarity(
    expected: &TestRgbaImage,
    actual: &TestRgbaImage,
    tile: u32,
) -> f64 {
    let step = (tile / 2).max(1);
    let mut worst = 1.0f64;
    let mut y = 0;
    loop {
        let mut x = 0;
        loop {
            let (mut total, mut ink) = (0u64, 0u64);
            for py in y..(y + tile).min(expected.height) {
                for px in x..(x + tile).min(expected.width) {
                    let (e, a) = (expected.pixel(px, py), actual.pixel(px, py));
                    if is_ink(e) || is_ink(a) {
                        ink += 1;
                        total +=
                            u64::from((0..3).map(|c| e[c].abs_diff(a[c])).max().unwrap());
                    }
                }
            }
            // Ignore tiles that only clip the edge of a stroke.
            if ink * 50 >= u64::from(tile) * u64::from(tile) {
                worst = worst.min(1.0 - total as f64 / (ink as f64 * 255.0));
            }
            if x + tile >= expected.width {
                break;
            }
            x += step;
        }
        if y + tile >= expected.height {
            break;
        }
        y += step;
    }
    worst
}

impl Comparison {
    fn failed_checks(&self) -> BTreeSet<String> {
        let mut failed = BTreeSet::new();
        if self.width_delta.abs() > self.size_tolerance
            || self.height_delta.abs() > self.size_tolerance
        {
            failed.insert("metrics".to_string());
        }
        if self.local < MIN_SIMILARITY {
            failed.insert("ink".to_string());
        }
        failed
    }

    fn describe(&self) -> String {
        format!(
            "    size differs by {:.3} x {:.3} pt (allowed {:.3}); ink similarity {:.4}, worst tile {:.4} (min {MIN_SIMILARITY})\n",
            self.width_delta,
            self.height_delta,
            self.size_tolerance,
            self.similarity,
            self.local
        )
    }

    fn write_artifacts(&self, dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for (name, image) in
            [("expected", &self.expected), ("actual", &self.actual), ("diff", &self.diff)]
        {
            write_png_rgba(&dir.join(format!("{name}.png")), image).unwrap();
        }
    }
}

fn is_ink(pixel: [u8; 4]) -> bool {
    pixel[0] < INK_THRESHOLD || pixel[1] < INK_THRESHOLD || pixel[2] < INK_THRESHOLD
}

/// The ink's bounds `[x0, y0, x1, y1)` after offsetting the image by `(dx, dy)`.
fn ink_bounds(image: &TestRgbaImage, dx: i64, dy: i64) -> Option<[i64; 4]> {
    let mut bounds: Option<[i64; 4]> = None;
    for y in 0..image.height {
        for x in 0..image.width {
            if is_ink(image.pixel(x, y)) {
                let (px, py) = (i64::from(x) + dx, i64::from(y) + dy);
                let b = bounds.get_or_insert([px, py, px + 1, py + 1]);
                *b = [b[0].min(px), b[1].min(py), b[2].max(px + 1), b[3].max(py + 1)];
            }
        }
    }
    bounds
}

fn read_label_source(path: &Path) -> String {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    source.trim_end_matches(['\r', '\n']).to_string()
}

fn composite_over_white(image: &TestRgbaImage) -> TestRgbaImage {
    let mut output = TestRgbaImage::new(image.width, image.height);
    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.pixel(x, y);
            let alpha = f32::from(pixel[3]) / 255.0;
            let channel =
                |c: u8| (f32::from(c) * alpha + 255.0 * (1.0 - alpha)).round() as u8;
            output.put_pixel(
                x,
                y,
                [channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), 255],
            );
        }
    }
    output
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
            data: vec![255; width as usize * height as usize * 4],
        }
    }

    fn from_vec(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (data.len() == width as usize * height as usize * 4).then_some(Self {
            width,
            height,
            data,
        })
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

    fn index(&self, x: u32, y: u32) -> usize {
        ((y as usize * self.width as usize) + x as usize) * 4
    }
}

fn read_png_rgba(path: &Path) -> Result<TestRgbaImage, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(
        png::Transformations::ALPHA | png::Transformations::STRIP_16,
    );
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output buffer size overflowed".to_string())?;
    let mut buffer = vec![0; buffer_size];
    let info = reader.next_frame(&mut buffer).map_err(|err| err.to_string())?;
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
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Grayscale => {
            data.iter().flat_map(|gray| [*gray, *gray, *gray, 255]).collect()
        }
        png::ColorType::Indexed => {
            return Err("unsupported indexed PNG output".to_string());
        }
    })
}
