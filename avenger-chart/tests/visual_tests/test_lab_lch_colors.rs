//! Test Lab/Lch/Oklab/Oklch CSS color functions

use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_oklch_perceptual_lightness_scale() {
    // Demonstrate perceptually uniform lightness scale using Oklch
    // All colors have same chroma and hue, only lightness varies
    let categories = vec![
        "L=0.3", "L=0.4", "L=0.5", "L=0.6", "L=0.7", "L=0.8", "L=0.9",
    ];
    let values = vec![10.0; 7];

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
            fill-discrete:
                oklch(0.3 0.15 250),
                oklch(0.4 0.15 250),
                oklch(0.5 0.15 250),
                oklch(0.6 0.15 250),
                oklch(0.7 0.15 250),
                oklch(0.8 0.15 250),
                oklch(0.9 0.15 250);
            stroke: black;
            stroke-width: 1px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Chart::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .fill(col("category"))
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Lightness").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "lab_lch_colors",
        "oklch_lightness_scale",
    )
    .await;
}

#[tokio::test]
async fn test_oklch_hue_wheel() {
    // Demonstrate hue variation using Oklch
    // All colors have same lightness and chroma, only hue varies
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
            fill-discrete:
                oklch(0.65 0.2 0),
                oklch(0.65 0.2 60),
                oklch(0.65 0.2 120),
                oklch(0.65 0.2 180),
                oklch(0.65 0.2 240),
                oklch(0.65 0.2 300);
            stroke: black;
            stroke-width: 1px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Chart::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .fill(col("category"))
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Hue Angle").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "lab_lch_colors", "oklch_hue_wheel").await;
}

#[tokio::test]
async fn test_all_lab_color_spaces() {
    // Test all four color space functions with similar colors
    let categories = vec!["oklab", "oklch", "lab", "lch"];
    let values = vec![10.0; 4];

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
            fill-discrete:
                oklab(0.6 0.1 -0.1),
                oklch(0.6 0.14 315),
                lab(60 20 -30),
                lch(60 36 303);
            stroke: black;
            stroke-width: 1px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Chart::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .fill(col("category"))
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Color Space").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "lab_lch_colors", "all_color_spaces").await;
}
