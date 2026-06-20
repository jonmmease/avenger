// Helper functions for visual tests

use crate::tracing::try_init_tracing;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::render::{EvaluatedPlot, EvaluationOptions};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::{SvgRenderOptions, SvgRenderer};
use avenger_text::{FontResolutionOptions, MissingFontPolicy};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::common::ScalarValue;
use image::RgbaImage;
use indexmap::IndexMap;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Default dimensions for test charts
pub const DEFAULT_SCALE: f32 = 2.0;

const SVG_BASELINES_ENV: &str = "AVENGER_CHART_SVG_BASELINES";
const BLESS_SVG_BASELINES_ENV: &str = "AVENGER_CHART_BLESS_SVG_BASELINES";
const SVG_BASELINE_RESVG_THRESHOLD: f64 = 0.99999;
const SVG_WGPU_BASELINE_THRESHOLD: f64 = 0.90;
const SVG_WGPU_LOCAL_DIFFERENCE_LIMIT: LocalDifferenceLimit = LocalDifferenceLimit {
    tile_size: 32,
    tile_mean_threshold: 0.20,
    max_bad_tiles: 32,
    max_bad_vertical_run: 7,
};

#[derive(Debug, Clone, Copy)]
struct LocalDifferenceLimit {
    tile_size: u32,
    tile_mean_threshold: f64,
    max_bad_tiles: usize,
    max_bad_vertical_run: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct LocalDifferenceSummary {
    bad_tiles: usize,
    max_bad_vertical_run: usize,
    max_bad_horizontal_run: usize,
    max_tile_mean: f64,
}

impl LocalDifferenceSummary {
    fn exceeds(self, limit: LocalDifferenceLimit) -> bool {
        self.bad_tiles > limit.max_bad_tiles
            || self.max_bad_vertical_run > limit.max_bad_vertical_run
    }
}

/// Configuration for visual tests
pub struct VisualTestConfig {
    /// Similarity threshold (0.0 to 1.0, where 1.0 is identical)
    pub threshold: f64,
    /// Whether to save difference images on failure
    pub save_diff_on_failure: bool,
}

impl Default for VisualTestConfig {
    fn default() -> Self {
        Self {
            threshold: 0.9999, // 99.99% similarity required by default
            save_diff_on_failure: true,
        }
    }
}

fn failure_image_paths(baseline_path: &str) -> (String, String) {
    let path = Path::new(baseline_path);
    let test_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Extract category from path (e.g., "tests/baselines/bar/simple_bar_chart.png" -> "bar")
    let category = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let failures_dir = if category.is_empty() {
        "tests/failures".to_string()
    } else {
        format!("tests/failures/{}", category)
    };

    (failures_dir, test_name.to_string())
}

fn save_actual_failure_image(actual: &RgbaImage, baseline_path: &str) -> Result<String, String> {
    let (failures_dir, test_name) = failure_image_paths(baseline_path);

    std::fs::create_dir_all(&failures_dir)
        .map_err(|e| format!("Failed to create failures directory: {}", e))?;

    let actual_path = format!("{}/{}.png", failures_dir, test_name);
    actual
        .save(&actual_path)
        .map_err(|e| format!("Failed to save actual image: {}", e))?;

    Ok(actual_path)
}

/// Evaluate a CompiledPlot directly and after a bincode round-trip.
async fn evaluate_compiled_plot_with_serialization(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> (EvaluatedPlot, EvaluatedPlot) {
    // Evaluate directly first
    let direct_result = compiled
        .evaluate(ctx, params.clone())
        .await
        .expect("Failed to evaluate plot directly");

    // Perform serialization round-trip through bincode
    let serialized =
        bincode::serialize(&compiled).expect("Failed to serialize CompiledPlot with bincode");

    let deserialized: CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");

    // Evaluate from the deserialized plot
    let bincode_result = deserialized
        .evaluate(ctx, params)
        .await
        .expect("Failed to evaluate plot after bincode deserialization");

    // Log if dimensions differ (but don't panic - serialization might change some aspects)
    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        tracing::warn!(
            direct_width = direct_result.scene_graph.width,
            direct_height = direct_result.scene_graph.height,
            bincode_width = bincode_result.scene_graph.width,
            bincode_height = bincode_result.scene_graph.height,
            "Serialization round-trip changed scene dimensions"
        );
    }

    (direct_result, bincode_result)
}

/// Evaluate a CompiledPlot with explicit options directly and after a bincode round-trip.
async fn evaluate_compiled_plot_with_serialization_and_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
) -> (EvaluatedPlot, EvaluatedPlot) {
    let direct_result = compiled
        .evaluate_with_options(ctx, params.clone(), options.clone())
        .await
        .expect("Failed to evaluate plot directly with options");

    let serialized =
        bincode::serialize(&compiled).expect("Failed to serialize CompiledPlot with bincode");

    let deserialized: CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");

    let bincode_result = deserialized
        .evaluate_with_options(ctx, params, options)
        .await
        .expect("Failed to evaluate plot after bincode deserialization with options");

    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        tracing::warn!(
            direct_width = direct_result.scene_graph.width,
            direct_height = direct_result.scene_graph.height,
            bincode_width = bincode_result.scene_graph.width,
            bincode_height = bincode_result.scene_graph.height,
            "Serialization round-trip changed scene dimensions with options"
        );
    }

    (direct_result, bincode_result)
}

async fn evaluate_compiled_plot(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> EvaluatedPlot {
    compiled
        .evaluate(ctx, params)
        .await
        .expect("Failed to evaluate plot")
}

async fn evaluate_compiled_plot_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
) -> EvaluatedPlot {
    compiled
        .evaluate_with_options(ctx, params, options)
        .await
        .expect("Failed to evaluate plot with options")
}

pub async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };

    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("Failed to create visual test canvas");
    canvas
        .set_scene(scene_graph)
        .expect("Failed to set visual test scene");
    canvas
        .render()
        .await
        .expect("Failed to render visual test scene")
}

/// Helper to get platform-specific baseline path
pub fn get_baseline_path(category: &str, base_name: &str) -> String {
    format!("tests/baselines/{}/{}.png", category, base_name)
}

fn svg_baselines_enabled() -> bool {
    std::env::var_os(SVG_BASELINES_ENV).is_some()
}

fn svg_baselines_only_enabled() -> bool {
    std::env::var_os(SVG_BASELINES_ENV)
        .as_deref()
        .is_some_and(|value| value == "only")
}

fn bless_svg_baselines_enabled() -> bool {
    std::env::var_os(BLESS_SVG_BASELINES_ENV).is_some()
}

fn svg_baseline_path(category: &str, baseline_name: &str, extension: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("baselines_svg")
        .join(category)
        .join(format!("{baseline_name}.{extension}"))
}

fn svg_failure_path(category: &str, baseline_name: &str, suffix: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("failures_svg")
        .join(category)
        .join(format!("{baseline_name}{suffix}"))
}

fn write_file(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

fn save_image_to_path(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    image
        .save(path)
        .map_err(|e| format!("Failed to save {}: {e}", path.display()))
}

fn rasterize_svg(svg: &str) -> Result<RgbaImage, String> {
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(avenger_text::fonts::build_fontdb(
        &svg_visual_font_resolution(),
    ));
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|e| format!("Failed to parse generated SVG: {e}"))?;
    let width = (tree.size().width() * DEFAULT_SCALE).ceil() as u32;
    let height = (tree.size().height() * DEFAULT_SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("Failed to allocate SVG pixmap {width}x{height}"))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(DEFAULT_SCALE, DEFAULT_SCALE),
        &mut pixmap.as_mut(),
    );

    RgbaImage::from_raw(width, height, pixmap.data().to_vec())
        .ok_or_else(|| "Failed to convert SVG pixmap into an image".to_string())
}

fn save_svg_failures(
    category: &str,
    baseline_name: &str,
    svg: &str,
    image: &RgbaImage,
) -> Result<(), String> {
    write_file(
        &svg_failure_path(category, baseline_name, ".svg"),
        svg.as_bytes(),
    )?;
    save_image_to_path(image, &svg_failure_path(category, baseline_name, ".png"))
}

fn compare_image_with_named_failures(
    baseline_path: &Path,
    actual: &RgbaImage,
    actual_failure_path: &Path,
    diff_failure_path: &Path,
    threshold: f64,
    local_difference_limit: Option<LocalDifferenceLimit>,
    label: &str,
) -> Result<(), String> {
    if !baseline_path.exists() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "No {label} baseline found at '{}'. Actual saved to '{}'.",
            baseline_path.display(),
            actual_failure_path.display()
        ));
    }

    let expected = image::open(baseline_path)
        .map_err(|e| {
            format!(
                "Failed to load {label} baseline '{}': {e}",
                baseline_path.display()
            )
        })?
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "{label} dimensions differ. Expected {:?}, actual {:?}. Actual saved to '{}'.",
            expected.dimensions(),
            actual.dimensions(),
            actual_failure_path.display()
        ));
    }

    let result = image_compare::rgba_hybrid_compare(&expected, actual)
        .map_err(|e| format!("{label} image comparison failed: {e}"))?;
    if result.score < threshold {
        save_image_to_path(actual, actual_failure_path)?;
        save_image_to_path(&result.image.to_color_map().into_rgba8(), diff_failure_path)?;
        return Err(format!(
            "{label} image similarity {:.6} is below threshold {:.6}. Actual saved to '{}', diff saved to '{}'.",
            result.score,
            threshold,
            actual_failure_path.display(),
            diff_failure_path.display()
        ));
    }

    if let Some(local_difference_limit) = local_difference_limit {
        let local_summary = local_difference_summary(&expected, actual, local_difference_limit);
        if local_summary.exceeds(local_difference_limit) {
            save_image_to_path(actual, actual_failure_path)?;
            save_image_to_path(&result.image.to_color_map().into_rgba8(), diff_failure_path)?;
            return Err(format!(
                "{label} local image difference exceeded limit. {} tiles have mean RGBA error above {:.3}; max vertical run is {}; max horizontal run is {}; max tile mean error is {:.3}. Actual saved to '{}', diff saved to '{}'.",
                local_summary.bad_tiles,
                local_difference_limit.tile_mean_threshold,
                local_summary.max_bad_vertical_run,
                local_summary.max_bad_horizontal_run,
                local_summary.max_tile_mean,
                actual_failure_path.display(),
                diff_failure_path.display()
            ));
        }
    }

    Ok(())
}

fn local_difference_summary(
    expected: &RgbaImage,
    actual: &RgbaImage,
    limit: LocalDifferenceLimit,
) -> LocalDifferenceSummary {
    let (width, height) = expected.dimensions();
    debug_assert_eq!((width, height), actual.dimensions());
    debug_assert!(limit.tile_size > 0);

    if width == 0 || height == 0 {
        return LocalDifferenceSummary::default();
    }

    let cols = width.div_ceil(limit.tile_size) as usize;
    let rows = height.div_ceil(limit.tile_size) as usize;
    let mut bad_tiles = vec![false; rows * cols];
    let expected_bytes = expected.as_raw();
    let actual_bytes = actual.as_raw();
    let mut summary = LocalDifferenceSummary::default();

    for row in 0..rows {
        let y0 = row as u32 * limit.tile_size;
        let y1 = (y0 + limit.tile_size).min(height);
        for col in 0..cols {
            let x0 = col as u32 * limit.tile_size;
            let x1 = (x0 + limit.tile_size).min(width);
            let mut diff_sum = 0u64;

            for y in y0..y1 {
                for x in x0..x1 {
                    let index = ((y * width + x) * 4) as usize;
                    diff_sum += (expected_bytes[index] as i32 - actual_bytes[index] as i32)
                        .unsigned_abs() as u64;
                    diff_sum += (expected_bytes[index + 1] as i32 - actual_bytes[index + 1] as i32)
                        .unsigned_abs() as u64;
                    diff_sum += (expected_bytes[index + 2] as i32 - actual_bytes[index + 2] as i32)
                        .unsigned_abs() as u64;
                    diff_sum += (expected_bytes[index + 3] as i32 - actual_bytes[index + 3] as i32)
                        .unsigned_abs() as u64;
                }
            }

            let tile_channels = (x1 - x0) as f64 * (y1 - y0) as f64 * 4.0;
            let tile_mean = diff_sum as f64 / (tile_channels * 255.0);
            summary.max_tile_mean = summary.max_tile_mean.max(tile_mean);

            if tile_mean > limit.tile_mean_threshold {
                summary.bad_tiles += 1;
                bad_tiles[row * cols + col] = true;
            }
        }
    }

    for row in 0..rows {
        let mut run = 0;
        for col in 0..cols {
            if bad_tiles[row * cols + col] {
                run += 1;
                summary.max_bad_horizontal_run = summary.max_bad_horizontal_run.max(run);
            } else {
                run = 0;
            }
        }
    }

    for col in 0..cols {
        let mut run = 0;
        for row in 0..rows {
            if bad_tiles[row * cols + col] {
                run += 1;
                summary.max_bad_vertical_run = summary.max_bad_vertical_run.max(run);
            } else {
                run = 0;
            }
        }
    }

    summary
}

fn assert_svg_scene_graph_match(scene_graph: &SceneGraph, category: &str, baseline_name: &str) {
    if !svg_baselines_enabled() {
        return;
    }

    let svg = SvgRenderer::new()
        .with_options(SvgRenderOptions {
            font_resolution: svg_visual_font_resolution(),
            ..Default::default()
        })
        .render_scene_graph(scene_graph)
        .expect("Failed to render SVG visual baseline");
    let svg_image = rasterize_svg(&svg).expect("Failed to rasterize SVG visual baseline");

    let svg_path = svg_baseline_path(category, baseline_name, "svg");
    let svg_png_path = svg_baseline_path(category, baseline_name, "png");
    let svg_failure = svg_failure_path(category, baseline_name, ".svg");
    let svg_png_failure = svg_failure_path(category, baseline_name, ".png");
    let svg_diff_failure = svg_failure_path(category, baseline_name, "_vs_svg_baseline_diff.png");
    let wgpu_diff_failure = svg_failure_path(category, baseline_name, "_vs_wgpu_baseline_diff.png");
    let wgpu_baseline_path = PathBuf::from(get_baseline_path(category, baseline_name));

    if bless_svg_baselines_enabled() {
        write_file(&svg_path, svg.as_bytes()).expect("Failed to write SVG baseline");
        save_image_to_path(&svg_image, &svg_png_path).expect("Failed to write SVG PNG baseline");
    } else {
        if !svg_path.exists() {
            save_svg_failures(category, baseline_name, &svg, &svg_image)
                .expect("Failed to save missing SVG baseline failure");
            panic!(
                "No SVG baseline found at '{}'. Generated SVG saved to '{}'. Generate baselines with {SVG_BASELINES_ENV}=only {BLESS_SVG_BASELINES_ENV}=1 cargo test -p avenger-chart --test visual_regression -- --nocapture",
                svg_path.display(),
                svg_failure.display()
            );
        }

        let expected_svg = fs::read_to_string(&svg_path).unwrap_or_else(|e| {
            panic!("Failed to read SVG baseline '{}': {e}", svg_path.display())
        });
        if expected_svg != svg {
            save_svg_failures(category, baseline_name, &svg, &svg_image)
                .expect("Failed to save SVG mismatch failure");
            panic!(
                "SVG baseline '{}' differs from generated SVG. Generated SVG saved to '{}'.",
                svg_path.display(),
                svg_failure.display()
            );
        }

        if let Err(msg) = compare_image_with_named_failures(
            &svg_png_path,
            &svg_image,
            &svg_png_failure,
            &svg_diff_failure,
            SVG_BASELINE_RESVG_THRESHOLD,
            None,
            "SVG/resvg",
        ) {
            panic!("SVG baseline '{}' failed: {msg}", baseline_name);
        }
    }

    if let Err(msg) = compare_image_with_named_failures(
        &wgpu_baseline_path,
        &svg_image,
        &svg_png_failure,
        &wgpu_diff_failure,
        SVG_WGPU_BASELINE_THRESHOLD,
        Some(SVG_WGPU_LOCAL_DIFFERENCE_LIMIT),
        "SVG/WGPU",
    ) {
        panic!("SVG/WGPU baseline '{}' failed: {msg}", baseline_name);
    }
}

fn svg_visual_font_resolution() -> FontResolutionOptions {
    FontResolutionOptions {
        missing_font: MissingFontPolicy::Fallback,
        ..Default::default()
    }
}

const DERIVED_PLOT_SIZE_BASELINES_ENV: &str = "AVENGER_CHART_DERIVED_PLOT_SIZE_BASELINES";

fn canvas_derived_plot_size_enabled() -> bool {
    std::env::var_os(DERIVED_PLOT_SIZE_BASELINES_ENV).is_some()
}

fn canvas_derived_plot_size_only_enabled() -> bool {
    std::env::var_os(DERIVED_PLOT_SIZE_BASELINES_ENV)
        .as_deref()
        .is_some_and(|value| value == "only")
}

fn canvas_derived_plot_size_category(category: &str) -> Option<String> {
    if !canvas_derived_plot_size_enabled() {
        return None;
    }

    match category {
        "facet"
        | "facet_legend_sharing"
        | "facet_legends"
        | "nested_grid"
        | "nested_grid_empty_subplot" => Some(format!("{category}_plot_size_from_canvas")),
        _ => None,
    }
}

/// Compare a rendered image against a baseline
pub fn compare_images(
    baseline_path: &str,
    actual: RgbaImage,
    config: &VisualTestConfig,
) -> Result<(), String> {
    // Check if baseline exists
    if !std::path::Path::new(baseline_path).exists() {
        // Save the actual image to failures directory for review
        let actual_path = save_actual_failure_image(&actual, baseline_path)?;

        // Ensure baseline directory exists for the copy command
        if let Some(baseline_dir) = Path::new(baseline_path).parent() {
            let mkdir_cmd = format!("mkdir -p {}", baseline_dir.display());
            return Err(format!(
                "No baseline image found at '{}'. Generated image saved to '{}'. \
                To accept this as the baseline, run:\n  {}\n  cp {} {}",
                baseline_path, actual_path, mkdir_cmd, actual_path, baseline_path
            ));
        } else {
            return Err(format!(
                "No baseline image found at '{}'. Generated image saved to '{}'. \
                To accept this as the baseline, run: cp {} {}",
                baseline_path, actual_path, actual_path, baseline_path
            ));
        }
    }

    // Load baseline image
    let expected = image::open(baseline_path)
        .map_err(|e| format!("Failed to load baseline image '{}': {}", baseline_path, e))?
        .into_rgba8();

    // Ensure dimensions match
    if expected.dimensions() != actual.dimensions() {
        if config.save_diff_on_failure {
            let actual_path = save_actual_failure_image(&actual, baseline_path)?;
            return Err(format!(
                "Image dimensions don't match. Expected: {:?}, Actual: {:?}. Actual saved to: {}",
                expected.dimensions(),
                actual.dimensions(),
                actual_path
            ));
        } else {
            return Err(format!(
                "Image dimensions don't match. Expected: {:?}, Actual: {:?}",
                expected.dimensions(),
                actual.dimensions()
            ));
        }
    }

    // Compare images using hybrid algorithm (best for visualization)
    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {}", e))?;

    // Check if similarity meets threshold
    if result.score < config.threshold {
        // Save difference image if requested
        if config.save_diff_on_failure {
            let (failures_dir, test_name) = failure_image_paths(baseline_path);
            let diff_path = format!("{}/{}_diff.png", failures_dir, test_name);

            // Save the actual image
            let actual_path = save_actual_failure_image(&actual, baseline_path)?;

            // Save the difference map
            result
                .image
                .to_color_map()
                .save(&diff_path)
                .map_err(|e| format!("Failed to save diff image: {}", e))?;

            Err(format!(
                "Image similarity {:.4} is below threshold {:.4}. Diff saved to: {}, Actual saved to: {}",
                result.score, config.threshold, diff_path, actual_path
            ))
        } else {
            Err(format!(
                "Image similarity {:.4} is below threshold {:.4}",
                result.score, config.threshold
            ))
        }
    } else {
        Ok(())
    }
}

/// Test a CompiledPlot against its baseline with params and tolerance
pub async fn assert_visual_match(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let params_for_derived_plot_size = params.clone();
    let derived_category = canvas_derived_plot_size_category(category);
    if canvas_derived_plot_size_only_enabled() {
        if let Some(derived_category) = derived_category {
            assert_canvas_derived_plot_size_visual_match(
                compiled,
                ctx,
                params_for_derived_plot_size,
                &derived_category,
                baseline_name,
                tolerance,
            )
            .await;
        }
        return;
    }

    assert_visual_match_baseline_only(compiled, ctx, params, category, baseline_name, tolerance)
        .await;

    if let Some(derived_category) = derived_category {
        assert_canvas_derived_plot_size_visual_match(
            compiled,
            ctx,
            params_for_derived_plot_size,
            &derived_category,
            baseline_name,
            tolerance,
        )
        .await;
    }
}

async fn assert_visual_match_baseline_only(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    try_init_tracing();

    let baseline_path = get_baseline_path(category, baseline_name);

    if svg_baselines_only_enabled() {
        let direct_result = evaluate_compiled_plot(compiled, ctx, params).await;
        assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        return;
    }

    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization(compiled, ctx, params).await;
    let direct_image = render_scene_graph_to_wgpu_image(&direct_result.scene_graph).await;
    let serialized_image = render_scene_graph_to_wgpu_image(&serialized_result.scene_graph).await;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    // Test direct rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct rendering): {}",
            baseline_name, msg
        );
    }

    // Test serialized rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (after serialization): {}",
            baseline_name, msg
        );
    }

    // Also verify that direct and serialized produce identical results
    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized renders");

    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed rendering"
        );
    }

    assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
}

/// Test a CompiledPlot against its baseline with default tolerance (99.99%)
pub async fn assert_visual_match_default(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match(compiled, ctx, params, category, baseline_name, 0.9999).await
}

pub async fn assert_scene_graph_visual_match(
    scene_graph: &SceneGraph,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    try_init_tracing();
    let baseline_path = get_baseline_path(category, baseline_name);

    if !svg_baselines_only_enabled() {
        let image = render_scene_graph_to_wgpu_image(scene_graph).await;
        let config = VisualTestConfig {
            threshold: tolerance,
            save_diff_on_failure: true,
        };
        if let Err(msg) = compare_images(&baseline_path, image, &config) {
            panic!(
                "Visual test '{}' failed (scene graph rendering): {}",
                baseline_name, msg
            );
        }
    }

    assert_svg_scene_graph_match(scene_graph, category, baseline_name);
}

pub async fn assert_scene_graph_visual_match_default(
    scene_graph: &SceneGraph,
    category: &str,
    baseline_name: &str,
) {
    assert_scene_graph_visual_match(scene_graph, category, baseline_name, 0.9999).await
}

/// Test a CompiledPlot against its baseline with explicit evaluation options.
pub async fn assert_visual_match_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    if canvas_derived_plot_size_only_enabled() {
        return;
    }

    try_init_tracing();

    let baseline_path = get_baseline_path(category, baseline_name);

    if svg_baselines_only_enabled() {
        let direct_result =
            evaluate_compiled_plot_with_options(compiled, ctx, params, options).await;
        assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        return;
    }

    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization_and_options(compiled, ctx, params, options).await;
    let direct_image = render_scene_graph_to_wgpu_image(&direct_result.scene_graph).await;
    let serialized_image = render_scene_graph_to_wgpu_image(&serialized_result.scene_graph).await;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct rendering): {}",
            baseline_name, msg
        );
    }

    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (after serialization): {}",
            baseline_name, msg
        );
    }

    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized renders");

    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed rendering with options"
        );
    }

    assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
}

/// Test a CompiledPlot against its baseline with options and default tolerance (99.99%).
pub async fn assert_visual_match_default_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match_with_options(
        compiled,
        ctx,
        params,
        options,
        category,
        baseline_name,
        0.9999,
    )
    .await
}

/// Render a canvas-sized faceted chart as a plot-area-sized chart.
///
/// The leaf plot-area size is derived from the final canvas solution. This is useful
/// for checking whether plot-size mode can reproduce the canvas-fit layout when
/// both paths start from the same leaf subplot dimensions.
pub async fn assert_canvas_derived_plot_size_visual_match(
    compiled_canvas: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    try_init_tracing();

    let (leaf_plot_width, leaf_plot_height) = compiled_canvas
        .uniform_facet_leaf_plot_area_from_canvas_for_testing(
            ctx,
            params.clone(),
            EvaluationOptions::default(),
        )
        .await
        .expect("Failed to derive uniform leaf plot-area size from canvas solution");

    tracing::info!(
        baseline_name,
        leaf_plot_width,
        leaf_plot_height,
        "Derived leaf plot-area size from canvas solution"
    );

    let serialized =
        bincode::serialize(compiled_canvas).expect("Failed to serialize CompiledPlot with bincode");
    let plot_size_compiled: CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");
    let plot_size_compiled =
        plot_size_compiled.with_facet_leaf_plot_area_for_testing(leaf_plot_width, leaf_plot_height);

    assert_visual_match_baseline_only(
        &plot_size_compiled,
        ctx,
        params,
        category,
        baseline_name,
        tolerance,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use std::fs;

    #[test]
    fn compare_images_saves_actual_on_dimension_mismatch() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let baseline_dir = temp_dir.path().join("baselines/dimension_mismatch");
        fs::create_dir_all(&baseline_dir).expect("failed to create test baseline dir");
        let baseline_path = baseline_dir.join("size_case.png");
        let failure_path = Path::new("tests/failures/dimension_mismatch/size_case.png");

        if let Some(parent) = failure_path.parent() {
            fs::create_dir_all(parent).expect("failed to create failure parent dir");
        }
        let _ = fs::remove_file(failure_path);

        RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
            .save(&baseline_path)
            .expect("failed to save test baseline");
        let actual = RgbaImage::from_pixel(3, 2, Rgba([255, 0, 0, 255]));

        let err = compare_images(
            baseline_path.to_str().expect("non-utf8 test baseline path"),
            actual,
            &VisualTestConfig::default(),
        )
        .expect_err("dimension mismatch should fail");

        assert!(err.contains("Image dimensions don't match"));
        assert!(err.contains("Actual saved to"));
        assert!(failure_path.exists());

        let saved = image::open(failure_path)
            .expect("saved actual image should be readable")
            .into_rgba8();
        assert_eq!(saved.dimensions(), (3, 2));

        fs::remove_file(failure_path).expect("failed to remove test failure image");
    }

    #[test]
    fn local_difference_summary_detects_sustained_vertical_error() {
        let limit = SVG_WGPU_LOCAL_DIFFERENCE_LIMIT;
        let expected = RgbaImage::from_pixel(256, 512, Rgba([255, 255, 255, 255]));
        let mut actual = expected.clone();

        for y in 0..512 {
            for x in 64..96 {
                actual.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }

        let summary = local_difference_summary(&expected, &actual, limit);

        assert!(summary.max_bad_vertical_run > limit.max_bad_vertical_run);
        assert!(summary.exceeds(limit));
    }

    #[test]
    fn local_difference_summary_allows_short_horizontal_error() {
        let limit = SVG_WGPU_LOCAL_DIFFERENCE_LIMIT;
        let expected = RgbaImage::from_pixel(256, 512, Rgba([255, 255, 255, 255]));
        let mut actual = expected.clone();

        for y in 64..96 {
            for x in 0..256 {
                actual.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }

        let summary = local_difference_summary(&expected, &actual, limit);

        assert!(summary.max_bad_horizontal_run > limit.max_bad_vertical_run);
        assert!(!summary.exceeds(limit));
    }
}
