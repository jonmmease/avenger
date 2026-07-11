//! Test color-mix() CSS function support

use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_color_mix_combinations() {
    let ctx = SessionContext::new();

    // Create data for different color mix combinations
    let categories = vec![
        "sRGB 50/50",
        "sRGB 75/25",
        "sRGB 25/75",
        "Oklab 50/50",
        "Oklch 50/50",
        "HSL 50/50",
    ];
    let values = vec![10.0; 6];

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
                color-mix(in srgb, red, blue),
                color-mix(in srgb, red 75%, blue 25%),
                color-mix(in srgb, red 25%, blue 75%),
                color-mix(in oklab, red, blue),
                color-mix(in oklch, red, blue),
                color-mix(in hsl, red, blue);
            stroke: black;
            stroke-width: 1px;
        }
    "#;

    let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");

    let plot = Chart::<Cartesian>::new().data(df).theme(theme).mark(
        Rect::new()
            .fill(col("category"))
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Mix Type").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "color_mix", "color_mix_combinations").await;
}
