//! Test HSL color function support in CSS themes

use super::datasets;
use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_hsl_primary_colors() {
    let ctx = SessionContext::new();
    let df = datasets::simple_categories();

    let css_theme = r#"
        mark {
            fill: hsl(0, 100%, 50%);     /* red */
            stroke: hsl(240, 100%, 30%); /* dark blue */
            stroke-width: 3px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Plot::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "hsl", "hsl_red_fill_blue_stroke").await;
}

#[tokio::test]
async fn test_hsl_color_wheel() {
    // Create data for color wheel test
    let categories = vec!["0°", "60°", "120°", "180°", "240°", "300°"];
    let values = vec![10.0; 6];

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(
            RecordBatch::try_from_iter(vec![
                (
                    "category",
                    Arc::new(StringArray::from(categories)) as arrow::array::ArrayRef,
                ),
                (
                    "value",
                    Arc::new(Float64Array::from(values)) as arrow::array::ArrayRef,
                ),
            ])
            .unwrap(),
        )
        .unwrap();

    let css_theme = r#"
        mark {
            fill-discrete: hsl(0, 80%, 60%), hsl(60, 80%, 60%), hsl(120, 80%, 60%), hsl(180, 80%, 60%), hsl(240, 80%, 60%), hsl(300, 80%, 60%);
            stroke: hsl(0, 0%, 20%);
            stroke-width: 1px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Plot::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .fill(col("category"))
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Hue").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "hsl", "hsl_color_wheel").await;
}

#[tokio::test]
async fn test_hsla_with_alpha() {
    let ctx = SessionContext::new();
    let df = datasets::simple_categories();

    let css_theme = r#"
        mark {
            fill: hsla(120, 100%, 50%, 0.5);  /* semi-transparent green */
            stroke: hsl(120, 100%, 30%);      /* dark green */
            stroke-width: 2px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Plot::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "hsl", "hsla_semi_transparent_green").await;
}

#[tokio::test]
async fn test_hsl_grayscale() {
    let ctx = SessionContext::new();
    let df = datasets::simple_categories();

    let css_theme = r#"
        /* Grayscale using HSL with 0% saturation */
        mark {
            fill: hsl(0, 0%, 70%);    /* light gray */
            stroke: hsl(0, 0%, 30%);  /* dark gray */
            stroke-width: 2px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Plot::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "hsl", "hsl_grayscale").await;
}
