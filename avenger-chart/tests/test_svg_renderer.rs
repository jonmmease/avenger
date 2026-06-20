use std::path::PathBuf;

use avenger_chart::{plot::CompiledPlot, prelude::*, render::SvgRenderer};
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
    let compiled = text_plot(&ctx, "Atkinson Hyperlegible Next")
        .await
        .compile(&ctx)
        .await
        .unwrap();

    let svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert!(svg.contains("<style><![CDATA[\n@font-face"));
    assert!(svg.contains(r#"font-family: "Atkinson Hyperlegible Next";"#));
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

async fn rect_plot(ctx: &SessionContext) -> Plot<Cartesian> {
    let df = ctx
        .sql("SELECT 10.0 AS x, 90.0 AS x2, 12.0 AS y, 52.0 AS y2")
        .await
        .unwrap();

    Plot::<Cartesian>::new()
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

async fn text_plot(ctx: &SessionContext, font: &str) -> Plot<Cartesian> {
    let df = ctx
        .sql("SELECT 18.0 AS x, 36.0 AS y, 'SVG text' AS label")
        .await
        .unwrap();

    Plot::<Cartesian>::new()
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

async fn limited_text_plot(ctx: &SessionContext) -> Plot<Cartesian> {
    let df = ctx
        .sql("SELECT 18.0 AS x, 36.0 AS y, 'Long label text' AS label")
        .await
        .unwrap();

    Plot::<Cartesian>::new()
        .canvas_size(140.0, 80.0)
        .data(df)
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .font("Atkinson Hyperlegible Next")
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
