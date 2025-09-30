// Helper functions for visual tests

use avenger_chart::coords::CoordinateSystem;
use avenger_chart::plot::Plot;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use image::RgbaImage;
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

/// Render a plot to an image, automatically handling canvas sizing based on layout spec
/// This version performs a serialization round-trip through bincode to test serialization
pub async fn render_plot<C: CoordinateSystem + Clone>(
    plot: Plot<C>,
    ctx: &datafusion::prelude::SessionContext,
) -> RgbaImage {
    // Clone the plot so we can test both paths
    let plot_for_direct = plot.clone();
    let plot_for_bincode = plot;

    // Compile and render directly first
    let compiled_direct = plot_for_direct
        .compile(ctx)
        .await
        .expect("Failed to compile plot");
    let direct_result = compiled_direct
        .render(ctx, None)
        .await
        .expect("Failed to render plot directly");

    // Compile, serialize, deserialize, and render
    let compiled_for_serialization = plot_for_bincode
        .compile(ctx)
        .await
        .expect("Failed to compile plot for serialization");

    // Perform serialization round-trip through bincode
    let serialized = bincode::serialize(&compiled_for_serialization)
        .expect("Failed to serialize CompiledPlot with bincode");

    let deserialized: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");

    // Render from the deserialized plot
    let bincode_result = deserialized
        .render(ctx, None)
        .await
        .expect("Failed to render plot after bincode deserialization");

    // Log if dimensions differ (but don't panic - serialization might change some aspects)
    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        eprintln!(
            "Warning: Serialization round-trip changed dimensions! Direct: {}x{}, After bincode: {}x{}",
            direct_result.scene_graph.width,
            direct_result.scene_graph.height,
            bincode_result.scene_graph.width,
            bincode_result.scene_graph.height
        );
    }

    // Create canvas with the dimensions from the bincode result (this is what we're testing)
    let dimensions = CanvasDimensions {
        size: [
            bincode_result.scene_graph.width,
            bincode_result.scene_graph.height,
        ],
        scale: DEFAULT_SCALE,
    };
    let config = CanvasConfig::default();

    // Render the bincode version
    let mut canvas_bincode = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create bincode canvas");
    canvas_bincode
        .set_scene(&bincode_result.scene_graph)
        .expect("Failed to set bincode scene");
    let bincode_image = canvas_bincode
        .render()
        .await
        .expect("Failed to render bincode image");

    // Also render the direct version for comparison (if dimensions match)
    if direct_result.scene_graph.width == bincode_result.scene_graph.width
        && direct_result.scene_graph.height == bincode_result.scene_graph.height
    {
        let mut canvas_direct = PngCanvas::new(dimensions, CanvasConfig::default())
            .await
            .expect("Failed to create direct canvas");
        canvas_direct
            .set_scene(&direct_result.scene_graph)
            .expect("Failed to set direct scene");
        let direct_image = canvas_direct
            .render()
            .await
            .expect("Failed to render direct image");

        // Compare and log similarity (but don't fail if they differ)
        let result = image_compare::rgba_hybrid_compare(&direct_image, &bincode_image)
            .expect("Failed to compare direct and bincode-serialized renders");

        if result.score < 0.99999 {
            eprintln!(
                "Info: Serialization round-trip produced different rendering. Similarity: {:.6}",
                result.score
            );
        } else {
            eprintln!(
                "Info: Serialization round-trip produced identical rendering (score: {:.6})",
                result.score
            );
        }
    }

    // Return the bincode version to ensure all tests use the serialization path
    bincode_image
}

/// Helper trait to make plot building more fluent for tests
pub trait PlotTestExt: Sized {
    /// Render this plot to an image using default test dimensions
    async fn to_image(self) -> RgbaImage;
}

impl<C: CoordinateSystem + Clone> PlotTestExt for Plot<C> {
    async fn to_image(self) -> RgbaImage {
        let ctx = datafusion::prelude::SessionContext::new();
        render_plot(self, &ctx).await
    }
}

/// Render a plot both directly and after bincode serialization
/// Returns (direct_image, serialized_image)
pub async fn render_plot_with_serialization_test<C: CoordinateSystem + Clone>(
    plot: Plot<C>,
    ctx: &datafusion::prelude::SessionContext,
) -> (RgbaImage, RgbaImage) {
    // Clone for both paths
    let plot_for_direct = plot.clone();
    let plot_for_serialized = plot;

    // Compile and render directly
    let compiled_direct = plot_for_direct
        .compile(ctx)
        .await
        .expect("Failed to compile plot directly");
    let direct_result = compiled_direct
        .render(ctx, None)
        .await
        .expect("Failed to render plot directly");

    // Compile, serialize, deserialize, and render
    let compiled_for_serialization = plot_for_serialized
        .compile(ctx)
        .await
        .expect("Failed to compile plot for serialization");

    let serialized = bincode::serialize(&compiled_for_serialization)
        .expect("Failed to serialize CompiledPlot with bincode");

    let deserialized: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");

    let serialized_result = deserialized
        .render(ctx, None)
        .await
        .expect("Failed to render plot after bincode deserialization");

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
            serialized_result.scene_graph.width,
            serialized_result.scene_graph.height,
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
        .set_scene(&serialized_result.scene_graph)
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

/// Test a plot against its baseline with a given name and tolerance
pub async fn assert_visual_match<C: CoordinateSystem + Clone>(
    plot: Plot<C>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let ctx = datafusion::prelude::SessionContext::new();
    let (direct_image, serialized_image) = render_plot_with_serialization_test(plot, &ctx).await;
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
        eprintln!(
            "Warning: Serialization round-trip changed rendering for '{}'. Similarity: {:.6}",
            baseline_name, comparison.score
        );
    }
}

/// Test a plot against its baseline with default tolerance (99.99%)
pub async fn assert_visual_match_default<C: CoordinateSystem + Clone>(
    plot: Plot<C>,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match(plot, category, baseline_name, 0.9999).await
}

/// Test a plot with a custom theme against its baseline
pub async fn assert_visual_match_with_theme<
    C: CoordinateSystem + Clone,
    T: avenger_chart::theme::Theme + 'static,
>(
    plot: Plot<C>,
    theme: T,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let plot_with_theme = plot.theme(theme);
    let ctx = datafusion::prelude::SessionContext::new();
    let (direct_image, serialized_image) =
        render_plot_with_serialization_test(plot_with_theme, &ctx).await;
    let baseline_path = get_baseline_path(category, baseline_name);

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    // Test direct rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct rendering with theme): {}",
            baseline_name, msg
        );
    }

    // Test serialized rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (after serialization with theme): {}",
            baseline_name, msg
        );
    }

    // Also verify that direct and serialized produce identical results
    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized renders");

    if comparison.score < 0.99999 {
        eprintln!(
            "Warning: Serialization round-trip changed rendering for '{}' with theme. Similarity: {:.6}",
            baseline_name, comparison.score
        );
    }
}
