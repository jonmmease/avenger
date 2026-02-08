// Helper functions for visual tests

use crate::tracing::try_init_tracing;
use avenger_chart::plot::CompiledPlot;
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

/// Helper to get platform-specific baseline path
pub fn get_baseline_path(category: &str, base_name: &str) -> String {
    format!("tests/baselines/{}/{}.png", category, base_name)
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
        let path = Path::new(baseline_path);
        let test_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        // Extract category from path
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

        // Create failures directory if it doesn't exist
        std::fs::create_dir_all(&failures_dir)
            .map_err(|e| format!("Failed to create failures directory: {}", e))?;

        let actual_path = format!("{}/{}.png", failures_dir, test_name);
        actual
            .save(&actual_path)
            .map_err(|e| format!("Failed to save actual image: {}", e))?;

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
        return Err(format!(
            "Image dimensions don't match. Expected: {:?}, Actual: {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }

    // Compare images using hybrid algorithm (best for visualization)
    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {}", e))?;

    // Check if similarity meets threshold
    if result.score < config.threshold {
        // Save difference image if requested
        if config.save_diff_on_failure {
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

            // Create failures directory if it doesn't exist
            std::fs::create_dir_all(&failures_dir)
                .map_err(|e| format!("Failed to create failures directory: {}", e))?;

            let diff_path = format!("{}/{}_diff.png", failures_dir, test_name);
            let actual_path = format!("{}/{}.png", failures_dir, test_name);

            // Save the actual image
            actual
                .save(&actual_path)
                .map_err(|e| format!("Failed to save actual image: {}", e))?;

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
