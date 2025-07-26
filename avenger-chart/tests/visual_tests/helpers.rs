//! Helper functions for visual tests

use avenger_chart::coords::CoordinateSystem;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
use image::RgbaImage;
use std::path::Path;

/// Default dimensions for test charts
pub const DEFAULT_SIZE: (f32, f32) = (400.0, 300.0);
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
            threshold: 0.95, // 95% similarity required by default
            save_diff_on_failure: true,
        }
    }
}

/// Render a plot to an image with default dimensions
pub async fn render_plot<C: CoordinateSystem>(plot: &Plot<C>) -> RgbaImage {
    render_plot_with_size(plot, DEFAULT_SIZE, DEFAULT_SCALE).await
}

/// Render a plot to an image with custom dimensions
pub async fn render_plot_with_size<C: CoordinateSystem>(
    plot: &Plot<C>,
    size: (f32, f32),
    scale: f32,
) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [size.0, size.1],
        scale,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    canvas
        .render_plot(plot)
        .await
        .expect("Failed to render plot");

    canvas.render().await.expect("Failed to render image")
}

/// Helper trait to make plot building more fluent for tests
pub trait PlotTestExt: Sized {
    /// Render this plot to an image using default test dimensions
    async fn to_image(self) -> RgbaImage;

    /// Render this plot to an image with custom dimensions
    async fn to_image_with_size(self, size: (f32, f32), scale: f32) -> RgbaImage;
}

impl<C: CoordinateSystem> PlotTestExt for Plot<C> {
    async fn to_image(self) -> RgbaImage {
        render_plot(&self).await
    }

    async fn to_image_with_size(self, size: (f32, f32), scale: f32) -> RgbaImage {
        render_plot_with_size(&self, size, scale).await
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

        let actual_path = format!("{}/{}_actual.png", failures_dir, test_name);
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

            let diff_path = format!("{}/{}_diff.png", failures_dir, test_name);
            let actual_path = format!("{}/{}_actual.png", failures_dir, test_name);

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

/// Test a plot against its baseline with default tolerance (95%)
pub async fn assert_visual_match_default<C: CoordinateSystem>(
    plot: Plot<C>,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match(plot, category, baseline_name, 0.95).await
}
