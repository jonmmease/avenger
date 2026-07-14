#![cfg(feature = "svg")]

use std::{path::PathBuf, sync::Arc, time::Duration};

use avenger_chart::{
    plot::CompiledPlot,
    prelude::*,
    render::{EvaluatedEventDatumState, EvaluatedInteractionState, EvaluatedPlot, SvgRenderer},
};
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::{ImageResourceLoadOptions, ImageResourceResolver, ImageResourceState};
use avenger_resource::{
    ResourceCachePolicy, ResourceKey, ResourceKind, ResourceRequest, ResourceRequestPurpose,
    ResourceSource,
};
use avenger_scenegraph::{
    marks::image::{SceneImageMark, SceneImageResource, SceneImageSource},
    scene_graph::SceneGraph,
};
use datafusion::prelude::{SessionContext, col};

#[tokio::test]
async fn renders_compiled_chart_to_svg_string() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();

    let svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
    assert!(svg.contains("<path "));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[tokio::test]
async fn writes_compiled_chart_svg_and_creates_parent_dirs() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();
    let (test_dir, output) = temp_svg_path("chart-write");
    let expected_svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    SvgRenderer::new()
        .write_svg(&compiled, &ctx, None, &output)
        .await
        .unwrap();

    let svg = std::fs::read_to_string(&output).unwrap();
    assert_eq!(svg, expected_svg);
    assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn renders_serialized_compiled_chart_to_same_svg_string() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();
    let serialized = serde_json::to_string(&compiled).unwrap();
    let deserialized: CompiledPlot = serde_json::from_str(&serialized).unwrap();

    let direct_svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();
    let roundtrip_svg = SvgRenderer::new()
        .render(&deserialized, &ctx, None)
        .await
        .unwrap();

    assert_eq!(direct_svg, roundtrip_svg);
}

#[tokio::test]
async fn embeds_bundled_font_for_text_chart_svg() {
    let ctx = SessionContext::new();
    let compiled = text_plot(&ctx, "Lato").await.compile(&ctx).await.unwrap();

    let svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert!(svg.contains("<style><![CDATA[\n@font-face"));
    assert!(svg.contains(r#"font-family: "Lato";"#));
    assert!(svg.contains("data:font/woff2;base64,"));
    assert!(svg.contains("<text "));
    assert!(svg.contains("SVG text</text>"));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[tokio::test]
async fn render_with_options_respects_debug_overlay() {
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        return;
    }

    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();
    let renderer = SvgRenderer::new();

    let base_svg = renderer
        .render_with_options(
            &compiled,
            &ctx,
            None,
            EvaluationOptions {
                debug_layout_overlay: LayoutDebugOverlayMode::Off,
                ..EvaluationOptions::default()
            },
        )
        .await
        .unwrap();
    let debug_svg = renderer
        .render_with_options(
            &compiled,
            &ctx,
            None,
            EvaluationOptions {
                debug_layout_overlay: LayoutDebugOverlayMode::Components,
                ..EvaluationOptions::default()
            },
        )
        .await
        .unwrap();

    assert_ne!(debug_svg, base_svg);
    assert!(!base_svg.contains(">plot-area</text>"));
    assert!(debug_svg.contains(">plot-area</text>"));
    assert!(usvg::Tree::from_str(&debug_svg, &usvg::Options::default()).is_ok());
}

#[tokio::test]
async fn applies_text_limit_to_chart_svg_text() {
    let ctx = SessionContext::new();
    let compiled = limited_text_plot(&ctx).await.compile(&ctx).await.unwrap();

    let svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert!(svg.contains("\u{2026}</text>"));
    assert!(!svg.contains("Long label text</text>"));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn render_evaluated_plot_loads_resource_images_for_svg() {
    let evaluated = resource_evaluated_plot(true);
    let svg = SvgRenderer::new()
        .render_evaluated_plot(&evaluated)
        .unwrap();

    assert!(svg.contains("data:image/png;base64"));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn render_evaluated_plot_errors_for_resource_without_request() {
    let evaluated = resource_evaluated_plot(false);
    let err = SvgRenderer::new()
        .render_evaluated_plot(&evaluated)
        .unwrap_err();
    let message = err.to_string();

    assert!(message.contains("failed to resolve image resources"));
    assert!(message.contains("tile/0/0/0"));
}

#[test]
fn render_evaluated_plot_errors_when_resource_loading_times_out() {
    let evaluated = resource_evaluated_plot(true);
    let resolver: Arc<dyn ImageResourceResolver> = Arc::new(PendingResolver);
    let err = SvgRenderer::new()
        .with_image_resource_resolver(resolver)
        .with_image_resource_load_options(ImageResourceLoadOptions {
            timeout: Some(Duration::from_millis(1)),
            poll_interval: Duration::from_millis(1),
        })
        .render_evaluated_plot(&evaluated)
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("timed out waiting for image resource")
    );
}

async fn rect_plot(ctx: &SessionContext) -> Chart<Cartesian> {
    let df = ctx
        .sql("SELECT 10.0 AS x, 90.0 AS x2, 12.0 AS y, 52.0 AS y2")
        .await
        .unwrap();

    Chart::<Cartesian>::new()
        .canvas_size(120.0, 80.0)
        .data(df)
        .mark(
            Rect::new()
                .x(col("x"))
                .x2(col("x2"))
                .y(col("y"))
                .y2(col("y2"))
                .fill("#3366cc"),
        )
}

async fn text_plot(ctx: &SessionContext, font: &str) -> Chart<Cartesian> {
    let df = ctx
        .sql("SELECT 18.0 AS x, 36.0 AS y, 'SVG text' AS label")
        .await
        .unwrap();

    Chart::<Cartesian>::new()
        .canvas_size(140.0, 80.0)
        .data(df)
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .font(font)
                .font_size(16.0),
        )
}

async fn limited_text_plot(ctx: &SessionContext) -> Chart<Cartesian> {
    let df = ctx
        .sql("SELECT 18.0 AS x, 36.0 AS y, 'Long label text' AS label")
        .await
        .unwrap();

    Chart::<Cartesian>::new()
        .canvas_size(140.0, 80.0)
        .data(df)
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .font("Lato")
                .font_size(10.0)
                .limit(35.0),
        )
}

fn temp_svg_path(name: &str) -> (PathBuf, PathBuf) {
    let test_dir = std::env::temp_dir().join(format!(
        "avenger-chart-svg-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = test_dir.join("nested").join("chart.svg");
    (test_dir, output)
}

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

fn resource_evaluated_plot(include_request: bool) -> EvaluatedPlot {
    let key = ResourceKey::new("tile/0/0/0");
    let scene_graph = SceneGraph {
        width: 8.0,
        height: 8.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(SceneImageSource::Resource(SceneImageResource {
                    key: key.clone(),
                    intrinsic_width: 2,
                    intrinsic_height: 2,
                    fallback_key: None,
                })),
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: ScalarOrArray::new_scalar(8.0),
                height: ScalarOrArray::new_scalar(8.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                ..Default::default()
            }
            .into(),
        ],
    };
    let resource_requests = include_request
        .then(|| ResourceRequest {
            key,
            kind: ResourceKind::new("image"),
            source: ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
            priority: 0.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        })
        .into_iter()
        .collect();

    EvaluatedPlot {
        scene_graph,
        resource_requests,
        materialization_requests: Vec::new(),
        rtree: None,
        interaction: EvaluatedInteractionState { scopes: Vec::new() },
        event_datums: EvaluatedEventDatumState { rows: Vec::new() },
        widget_frames: Default::default(),
        native_widgets: Default::default(),
        prefetch_planners: Vec::new(),
    }
}

struct PendingResolver;

impl ImageResourceResolver for PendingResolver {
    fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
        ImageResourceState::Pending
    }

    fn request_image(&self, _request: &ResourceRequest) {}
}
