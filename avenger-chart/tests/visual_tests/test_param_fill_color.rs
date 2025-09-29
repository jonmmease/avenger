use super::datasets;
use super::helpers::{get_baseline_path, compare_images, VisualTestConfig, DEFAULT_SCALE};
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::scalar::ScalarValue;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use avenger_common::canvas::CanvasDimensions;
use indexmap::IndexMap;
use image::RgbaImage;

/// Helper function to render compiled plot with specific params
async fn render_compiled_plot(
    compiled: &avenger_chart::plot::CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> RgbaImage {
    // Render with the specified params
    let result = compiled.render(ctx, params).await.expect("Failed to render plot");

    // Create canvas and render to image
    let dimensions = CanvasDimensions {
        size: [result.scene_graph.width, result.scene_graph.height],
        scale: DEFAULT_SCALE,
    };

    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("Failed to create canvas");

    canvas.set_scene(&result.scene_graph)
        .expect("Failed to set scene");

    canvas.render().await.expect("Failed to render image")
}

#[tokio::test]
async fn test_param_fill_color() {
    let ctx = datafusion::prelude::SessionContext::new();
    let df = datasets::simple_categories();

    // Create a parameter for fill color with blue as default
    let fill_color_param = Param::new("fill_color", ScalarValue::Utf8(Some("#4682b4".to_string())));

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .add_param(fill_color_param.clone())
        .mark(
            Rect::new()
                .x_with(col("category"), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![
                            lit("A"),
                            lit("B"),
                            lit("C"),
                            lit("D"),
                            lit("E"),
                        ])
                    })
                    .axis(|a| a.title("Category").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(0.8))
                .y_with(lit(0.0), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                // Use the parameter for fill color - no scale needed for color strings
                .fill_with(fill_color_param.expr(), |c| c.no_scale())
                .stroke("#333333")
                .stroke_width(1.0),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Render with default params (blue)
    let image_default = render_compiled_plot(&compiled, &ctx, None).await;
    let baseline_path_default = get_baseline_path("param", "fill_color_default");
    let config = VisualTestConfig::default();
    compare_images(&baseline_path_default, image_default, &config)
        .expect("Visual comparison failed for default param fill color");

    // Render with override params (red)
    let mut override_params = IndexMap::new();
    override_params.insert("fill_color".to_string(), ScalarValue::Utf8(Some("#ff6b6b".to_string())));
    let image_override = render_compiled_plot(&compiled, &ctx, Some(override_params)).await;
    let baseline_path_override = get_baseline_path("param", "fill_color_override");
    compare_images(&baseline_path_override, image_override, &config)
        .expect("Visual comparison failed for override param fill color");
}
