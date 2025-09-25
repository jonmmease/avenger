// Helper functions for visual tests

use avenger_chart::coords::CoordinateSystem;
use avenger_chart::plot::Plot;
use avenger_chart::render::PlotRenderer;
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
pub async fn render_plot<C: CoordinateSystem>(plot: &Plot<C>) -> RgbaImage {
    // We can demonstrate that plot.build() works to create SerializablePlotRenderer
    // But for now we still need to use PlotRenderer for actual rendering
    // This is because PlotRenderer needs access to methods on Plot that aren't
    // available on SerializablePlotRenderer yet (like get_scale, collect_channels_needing_scales, etc.)

    // Build works - this creates a SerializablePlotRenderer (we just don't use it yet)
    // let _serializable = plot.clone().build();

    // For now, continue using PlotRenderer directly
    let renderer = PlotRenderer::new(plot);
    let render_result = renderer.render().await.expect("Failed to render plot");

    // The scene graph contains the correct canvas dimensions for any mode
    let canvas_width = render_result.scene_graph.width;
    let canvas_height = render_result.scene_graph.height;

    // Create canvas with the dimensions from the scene graph
    let dimensions = CanvasDimensions {
        size: [canvas_width, canvas_height],
        scale: DEFAULT_SCALE,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // Set the already computed scene graph
    canvas
        .set_scene(&render_result.scene_graph)
        .expect("Failed to set scene");

    canvas.render().await.expect("Failed to render image")
}

/// Helper trait to make plot building more fluent for tests
pub trait PlotTestExt: Sized {
    /// Render this plot to an image using default test dimensions
    async fn to_image(self) -> RgbaImage;
}

impl<C: CoordinateSystem> PlotTestExt for Plot<C> {
    async fn to_image(self) -> RgbaImage {
        render_plot(&self).await
    }
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
pub async fn assert_visual_match<C: CoordinateSystem>(
    plot: Plot<C>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let rendered = plot.to_image().await;
    let baseline_path = get_baseline_path(category, baseline_name);

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if let Err(msg) = compare_images(&baseline_path, rendered, &config) {
        panic!("Visual test '{}' failed: {}", baseline_name, msg);
    }
}

/// Test a plot against its baseline with default tolerance (99.99%)
pub async fn assert_visual_match_default<C: CoordinateSystem>(
    plot: Plot<C>,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match(plot, category, baseline_name, 0.9999).await
}

/// Test a plot with a custom theme against its baseline
pub async fn assert_visual_match_with_theme<
    C: CoordinateSystem,
    T: avenger_chart::theme::Theme + 'static,
>(
    plot: Plot<C>,
    theme: T,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let plot_with_theme = plot.theme(theme);
    let rendered = plot_with_theme.to_image().await;
    let baseline_path = get_baseline_path(category, baseline_name);

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if let Err(msg) = compare_images(&baseline_path, rendered, &config) {
        panic!("Visual test '{}' failed: {}", baseline_name, msg);
    }
}
