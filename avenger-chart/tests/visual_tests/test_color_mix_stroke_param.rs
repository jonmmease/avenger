//! Test color-mix() with parameters for stroke - reproducing documentation example

use super::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

#[tokio::test]
async fn test_color_mix_stroke_with_param() {
    // Exact reproduction of the documentation example
    let ctx = SessionContext::new();

    let batch = RecordBatch::try_from_iter(vec![
        (
            "x",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as arrow::array::ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0])) as arrow::array::ArrayRef,
        ),
    ])
    .expect("create batch");
    let df = ctx.read_batch(batch).expect("read batch");

    let accent = {
        let __avenger_param_name = "--accent";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Utf8(Some("#2563eb".into()))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
        mark[type="symbol"] {
            fill: var(--accent);
            stroke: color-mix(in srgb, var(--accent) 60%, black) !important;
            stroke-width: 1.5px !important;
        }
        "#,
        )
        .expect("append css");

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Parameter-Driven CSS Theme")
        .param(accent)
        .theme(theme)
        .mark(Symbol::new().x(col("x")).y(col("y")).size(200.0));

    let compiled = plot.compile(&ctx).await.expect("compile");

    // Render with default accent color (blue)
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "color_mix_stroke_param",
        "blue_accent",
    )
    .await;
}

#[tokio::test]
async fn test_color_mix_stroke_with_param_override() {
    // Test with overridden parameter
    let ctx = SessionContext::new();

    let batch = RecordBatch::try_from_iter(vec![
        (
            "x",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as arrow::array::ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0])) as arrow::array::ArrayRef,
        ),
    ])
    .expect("create batch");
    let df = ctx.read_batch(batch).expect("read batch");

    let accent = {
        let __avenger_param_name = "--accent";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Utf8(Some("#2563eb".into()))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
        mark[type="symbol"] {
            fill: var(--accent);
            stroke: color-mix(in srgb, var(--accent) 60%, black) !important;
            stroke-width: 1.5px !important;
        }
        "#,
        )
        .expect("append css");

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Parameter-Driven CSS Theme (Red)")
        .param(accent)
        .theme(theme)
        .mark(Symbol::new().x(col("x")).y(col("y")).size(200.0));

    let compiled = plot.compile(&ctx).await.expect("compile");

    // Render with different accent color (red)
    let mut params = indexmap::IndexMap::new();
    params.insert(
        "--accent".to_string(),
        ScalarValue::Utf8(Some("#dc2626".into())),
    );

    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params),
        "color_mix_stroke_param",
        "red_accent",
    )
    .await;
}
