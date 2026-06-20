use std::path::PathBuf;

use avenger_chart::{
    plot::CompiledPlot,
    prelude::*,
    render::{PdfRenderer, SvgRenderer},
};
use datafusion::prelude::{SessionContext, col};

#[tokio::test]
async fn renders_compiled_chart_to_pdf_bytes() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();

    let pdf = PdfRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert!(pdf.starts_with(b"%PDF-"));
}

#[tokio::test]
async fn writes_compiled_chart_pdf_and_creates_parent_dirs() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();
    let (test_dir, output) = temp_pdf_path("chart-write");

    PdfRenderer::new()
        .write_pdf(&compiled, &ctx, None, &output)
        .await
        .unwrap();

    let pdf = std::fs::read(&output).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    std::fs::remove_dir_all(test_dir).unwrap();
}

#[tokio::test]
async fn renders_serialized_compiled_chart_to_pdf_bytes() {
    let ctx = SessionContext::new();
    let compiled = rect_plot(&ctx).await.compile(&ctx).await.unwrap();
    let serialized = serde_json::to_string(&compiled).unwrap();
    let deserialized: CompiledPlot = serde_json::from_str(&serialized).unwrap();

    let direct_pdf = PdfRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .unwrap();
    let roundtrip_pdf = PdfRenderer::new()
        .render(&deserialized, &ctx, None)
        .await
        .unwrap();

    assert!(direct_pdf.starts_with(b"%PDF-"));
    assert!(roundtrip_pdf.starts_with(b"%PDF-"));
}

#[tokio::test]
async fn embeds_bundled_font_for_text_chart_pdf() {
    let ctx = SessionContext::new();
    let compiled = text_plot(&ctx, "Atkinson Hyperlegible Next")
        .await
        .compile(&ctx)
        .await
        .unwrap();

    let pdf = PdfRenderer::new()
        .with_options(avenger_pdf::PdfRenderOptions {
            compress: false,
            ..Default::default()
        })
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert_embedded_pdf_font(&pdf);
}

#[tokio::test]
async fn pdf_font_embedding_is_independent_of_svg_font_subset_embedding() {
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
    assert!(svg.contains("data:font/woff2;base64,"));

    let pdf = PdfRenderer::new()
        .with_options(avenger_pdf::PdfRenderOptions {
            compress: false,
            ..Default::default()
        })
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert_embedded_pdf_font(&pdf);
    assert!(!pdf_contains(&pdf, b"data:font/woff2;base64,"));
}

#[tokio::test]
async fn embeds_extra_font_dir_for_text_chart_pdf() {
    let ctx = SessionContext::new();
    let compiled = text_plot(&ctx, "Caveat").await.compile(&ctx).await.unwrap();
    let caveat_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-vega-test-data/fonts/Caveat/static");

    let pdf = PdfRenderer::new()
        .with_options(avenger_pdf::PdfRenderOptions {
            compress: false,
            font_resolution: avenger_text::FontResolutionOptions {
                extra_font_dirs: vec![caveat_dir],
                ..Default::default()
            },
            ..Default::default()
        })
        .render(&compiled, &ctx, None)
        .await
        .unwrap();

    assert_embedded_pdf_font(&pdf);
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
        .sql("SELECT 18.0 AS x, 36.0 AS y, 'PDF text' AS label")
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

fn temp_pdf_path(name: &str) -> (PathBuf, PathBuf) {
    let test_dir = std::env::temp_dir().join(format!(
        "avenger-chart-pdf-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = test_dir.join("nested").join("chart.pdf");
    (test_dir, output)
}

fn assert_embedded_pdf_font(pdf: &[u8]) {
    assert!(pdf_contains(pdf, b"/FontFile2") || pdf_contains(pdf, b"/FontFile3"));
    assert!(pdf_contains(pdf, b"/ToUnicode"));
}

fn pdf_contains(pdf: &[u8], needle: &[u8]) -> bool {
    pdf.windows(needle.len()).any(|window| window == needle)
}
