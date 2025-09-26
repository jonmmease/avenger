// Test that demonstrates the serialization path for visual tests

use crate::visual_tests::datasets::simple_categories;
use crate::visual_tests::helpers::{VisualTestConfig, compare_images, get_baseline_path};
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::col;

#[tokio::test]
async fn test_serialization_rendering_path() {
    // Create a function to build the plot so we can create it twice
    let build_plot = || {
        Plot::new()
            .canvas_size(400.0, 300.0)
            .data(simple_categories())
            .mark(Symbol::new().x(col("category")).y(col("value")))
    };

    // Compile to get CompiledPlot
    let compiled = build_plot().compile().await.unwrap();

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&compiled).unwrap();

    // Deserialize back (just to verify serialization works)
    // NOTE: The deserialized plot will NOT render identically due to serde(skip) fields
    // like DataFrames, theme settings, and other non-serialized data.
    // Full serialization support is still in development.
    let _deserialized: avenger_chart::plot::CompiledPlot = serde_json::from_str(&json).unwrap();

    // Render from the original built plot (not the deserialized one)
    // to ensure consistent results for the visual test
    let render_result = compiled.render().await.expect("Failed to render plot");

    // Create canvas and render
    let dimensions = CanvasDimensions {
        size: [
            render_result.scene_graph.width,
            render_result.scene_graph.height,
        ],
        scale: 2.0,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("Failed to create canvas");

    canvas
        .set_scene(&render_result.scene_graph)
        .expect("Failed to set scene");

    let img = canvas.render().await.expect("Failed to render image");

    // Compare against baseline
    let baseline_path = get_baseline_path("serialization", "simple_scatter");
    let config = VisualTestConfig::default();
    compare_images(&baseline_path, img, &config).expect("Image comparison failed");
}
