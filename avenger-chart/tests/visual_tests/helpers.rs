// Helper functions for visual tests

use crate::tracing::try_init_tracing;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::render::EvaluationOptions;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::common::ScalarValue;
use image::RgbaImage;
use indexmap::IndexMap;
use std::path::Path;

/// Default dimensions for test charts
pub const DEFAULT_SCALE: f32 = 2.0;

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

/// Render a CompiledPlot with params to an image
/// This version performs a serialization round-trip through bincode to test serialization
async fn render_compiled_plot_with_serialization(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> (RgbaImage, RgbaImage) {
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

    // Render both to images
    let dimensions_direct = CanvasDimensions {
        size: [
            direct_result.scene_graph.width,
            direct_result.scene_graph.height,
        ],
        scale: DEFAULT_SCALE,
    };

    let dimensions_serialized = CanvasDimensions {
        size: [
            bincode_result.scene_graph.width,
            bincode_result.scene_graph.height,
        ],
        scale: DEFAULT_SCALE,
    };

    // Create direct image
    let mut canvas_direct = PngCanvas::new(dimensions_direct, CanvasConfig::default())
        .await
        .expect("Failed to create direct canvas");
    canvas_direct
        .set_scene(&direct_result.scene_graph)
        .expect("Failed to set direct scene");
    let direct_image = canvas_direct
        .render()
        .await
        .expect("Failed to render direct image");

    // Create serialized image
    let mut canvas_serialized = PngCanvas::new(dimensions_serialized, CanvasConfig::default())
        .await
        .expect("Failed to create serialized canvas");
    canvas_serialized
        .set_scene(&bincode_result.scene_graph)
        .expect("Failed to set serialized scene");
    let serialized_image = canvas_serialized
        .render()
        .await
        .expect("Failed to render serialized image");

    (direct_image, serialized_image)
}

/// Render a CompiledPlot with params and explicit evaluation options to an image.
/// This version performs a serialization round-trip through bincode to test serialization.
async fn render_compiled_plot_with_serialization_and_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
) -> (RgbaImage, RgbaImage) {
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

    let dimensions_direct = CanvasDimensions {
        size: [
            direct_result.scene_graph.width,
            direct_result.scene_graph.height,
        ],
        scale: DEFAULT_SCALE,
    };

    let dimensions_serialized = CanvasDimensions {
        size: [
            bincode_result.scene_graph.width,
            bincode_result.scene_graph.height,
        ],
        scale: DEFAULT_SCALE,
    };

    let mut canvas_direct = PngCanvas::new(dimensions_direct, CanvasConfig::default())
        .await
        .expect("Failed to create direct canvas");
    canvas_direct
        .set_scene(&direct_result.scene_graph)
        .expect("Failed to set direct scene");
    let direct_image = canvas_direct
        .render()
        .await
        .expect("Failed to render direct image");

    let mut canvas_serialized = PngCanvas::new(dimensions_serialized, CanvasConfig::default())
        .await
        .expect("Failed to create serialized canvas");
    canvas_serialized
        .set_scene(&bincode_result.scene_graph)
        .expect("Failed to set serialized scene");
    let serialized_image = canvas_serialized
        .render()
        .await
        .expect("Failed to render serialized image");

    (direct_image, serialized_image)
}

/// Helper to get platform-specific baseline path
pub fn get_baseline_path(category: &str, base_name: &str) -> String {
    format!("tests/baselines/{}/{}.png", category, base_name)
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

    let (direct_image, serialized_image) =
        render_compiled_plot_with_serialization(compiled, ctx, params).await;
    let baseline_path = get_baseline_path(category, baseline_name);

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

    let (direct_image, serialized_image) =
        render_compiled_plot_with_serialization_and_options(compiled, ctx, params, options).await;
    let baseline_path = get_baseline_path(category, baseline_name);

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
}
