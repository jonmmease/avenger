use avenger_chart::{prelude::*, render::SvgRenderer};
use datafusion::prelude::SessionContext;
use image::RgbaImage;
use std::path::Path;

const SVG_RASTER_SCALE: f32 = 2.0;
const SIMPLE_BAR_BASELINE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/baselines/bar/simple_bar_chart.png"
);
const SIMPLE_BAR_FAILURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/failures_svg/bar/simple_bar_chart.png"
);
const SIMPLE_BAR_DIFF_FAILURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/failures_svg/bar/simple_bar_chart_diff.png"
);

#[tokio::test]
async fn svg_raster_matches_simple_bar_chart_baseline() {
    let ctx = SessionContext::new();
    let df = simple_categories(&ctx).await;

    let plot = Plot::<Cartesian>::new()
        .data(df)
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
                            lit("F"),
                            lit("G"),
                            lit("H"),
                            lit("I"),
                        ])
                    })
                    .axis(|a| a.title("Category").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(1.0),
        )
        .mark(
            Rect::new()
                .x(0.0)
                .x2(1.0)
                .y(50.0)
                .y2(50.0)
                .stroke("#ff0000")
                .stroke_width(2.0)
                .opacity(0.7),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    let svg = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("Failed to render chart SVG");
    let actual = rasterize_svg(&svg);

    compare_to_baseline(SIMPLE_BAR_BASELINE, actual, 0.95)
        .expect("SVG raster did not match the simple bar chart baseline");
}

async fn simple_categories(ctx: &SessionContext) -> datafusion::prelude::DataFrame {
    ctx.sql(
        "SELECT column1 AS category, column2 AS value FROM (VALUES
            ('A', 28.0),
            ('B', 55.0),
            ('C', 43.0),
            ('D', 91.0),
            ('E', 81.0),
            ('F', 53.0),
            ('G', 19.0),
            ('H', 87.0),
            ('I', 52.0)
        )",
    )
    .await
    .expect("Failed to create simple category data")
}

fn rasterize_svg(svg: &str) -> RgbaImage {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .expect("Failed to parse generated SVG");
    let width = (tree.size().width() * SVG_RASTER_SCALE).ceil() as u32;
    let height = (tree.size().height() * SVG_RASTER_SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("Failed to allocate pixmap");

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SVG_RASTER_SCALE, SVG_RASTER_SCALE),
        &mut pixmap.as_mut(),
    );

    RgbaImage::from_raw(width, height, pixmap.data().to_vec())
        .expect("Failed to convert SVG pixmap into an image")
}

fn compare_to_baseline(
    baseline_path: &str,
    actual: RgbaImage,
    threshold: f64,
) -> Result<(), String> {
    let expected = image::open(baseline_path)
        .map_err(|e| format!("Failed to load baseline image '{baseline_path}': {e}"))?
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_failure(&actual, SIMPLE_BAR_FAILURE)?;
        return Err(format!(
            "Image dimensions differ. Expected {:?}, actual {:?}. Actual saved to {}",
            expected.dimensions(),
            actual.dimensions(),
            SIMPLE_BAR_FAILURE
        ));
    }

    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {e}"))?;

    if result.score < threshold {
        save_failure(&actual, SIMPLE_BAR_FAILURE)?;
        save_failure(
            &result.image.to_color_map().into_rgba8(),
            SIMPLE_BAR_DIFF_FAILURE,
        )?;
        Err(format!(
            "Image similarity {:.4} is below threshold {:.4}. Actual saved to {}, diff saved to {}",
            result.score, threshold, SIMPLE_BAR_FAILURE, SIMPLE_BAR_DIFF_FAILURE
        ))
    } else {
        Ok(())
    }
}

fn save_failure(image: &RgbaImage, path: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create failure directory: {e}"))?;
    }
    image
        .save(path)
        .map_err(|e| format!("Failed to save failure image '{path}': {e}"))
}
