//! Helper functions for visual tests

use avenger_chart::coords::CoordinateSystem;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
use image::RgbaImage;
use super::{compare_images, get_baseline_path, VisualTestConfig};

/// Default dimensions for test charts
pub const DEFAULT_SIZE: (f32, f32) = (400.0, 300.0);
pub const DEFAULT_SCALE: f32 = 2.0;

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

/// Test a plot against its baseline with a given name and tolerance
pub async fn assert_visual_match<C: CoordinateSystem>(
    plot: Plot<C>,
    baseline_name: &str,
    tolerance: f64,
) -> Result<(), String> {
    let rendered = plot.to_image().await;
    let baseline_path = get_baseline_path(baseline_name);
    
    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };
    
    compare_images(&baseline_path, rendered, &config)
}

/// Test a plot against its baseline with default tolerance (95%)
pub async fn assert_visual_match_default<C: CoordinateSystem>(
    plot: Plot<C>,
    baseline_name: &str,
) -> Result<(), String> {
    assert_visual_match(plot, baseline_name, 0.95).await
}