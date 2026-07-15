use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

// ============================================================================
// Test Data Helpers
// ============================================================================

/// Create test data for font size tests with categories for legend
fn create_test_data() -> RecordBatch {
    let categories = StringArray::from(vec!["Red", "Green", "Blue", "Red", "Green", "Blue"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap()
}

// ============================================================================
// Font Size Tests
// ============================================================================

/// Test that rem-based font sizes scale with base font size set to 18px
#[tokio::test]
async fn test_large_base_font_size() {
    let ctx = SessionContext::new();
    let batch = create_test_data();
    let df = ctx.read_batch(batch).unwrap();

    // Start with light theme and append CSS to set base font size to 18px
    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
            :root {
                font-size: 18px;
            }
        "#,
        )
        .unwrap();

    // The base font size is now immediately updated to 18px

    // Create plot with title, subtitle, axes, and legend
    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Chart with 18px Base Font")
        .subtitle("All text sizes scale with rem units")
        .theme(theme)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|scale| scale.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|scale| scale.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| {
                    c.legend(|legend| legend.title("Color"))
                })
                .size_with(lit(150.0), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "theme_font_size",
        "large_base_font_size",
    )
    .await;
}

/// Test default base font size (12px) for comparison
#[tokio::test]
async fn test_default_base_font_size() {
    let ctx = SessionContext::new();
    let batch = create_test_data();
    let df = ctx.read_batch(batch).unwrap();

    // Use default light theme (12px base)
    let theme = Theme::light();

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Chart with 12px Base Font")
        .subtitle("Default base font size")
        .theme(theme)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|scale| scale.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|scale| scale.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| {
                    c.legend(|legend| legend.title("Color"))
                })
                .size_with(lit(150.0), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "theme_font_size",
        "default_base_font_size",
    )
    .await;
}

/// Test runtime parameter for base font size
#[tokio::test]
async fn test_base_font_size_with_param() {
    let ctx = SessionContext::new();
    let batch = create_test_data();
    let df = ctx.read_batch(batch).unwrap();

    // Use default light theme (which has :root { font-size: var(--base-font-size) })
    let theme = Theme::light();

    // Override base font size to 14px using a parameter
    // NOTE: Must use string "14px" not float 14.0, so it gets parsed as Length
    let base_font_param = {
        let __avenger_param_name = "--base-font-size";
        let __avenger_param_default: datafusion::common::ScalarValue = ("14px").into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Chart with 14px Base Font")
        .subtitle("Font size controlled via parameter")
        .theme(theme)
        .param(base_font_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|scale| scale.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|scale| scale.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| {
                    c.legend(|legend| legend.title("Color"))
                })
                .size_with(lit(150.0), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "theme_font_size",
        "base_font_size_with_param_14px",
    )
    .await;

    // Now test with a larger font size (18px) to show params can be changed
    // NOTE: Must use string "18px" not float 18.0, so it gets parsed as Length
    let larger_font_param = {
        let __avenger_param_name = "--base-font-size";
        let __avenger_param_default: datafusion::common::ScalarValue = ("18px").into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Recreate the same plot with the larger param
    let categories = StringArray::from(vec!["Red", "Green", "Blue", "Red", "Green", "Blue"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let theme = Theme::light();

    let plot_larger = Chart::<Cartesian>::new()
        .data(df)
        .title("Chart with 18px Base Font")
        .subtitle("Larger font size via parameter")
        .theme(theme)
        .param(larger_font_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|scale| scale.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|scale| scale.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| {
                    c.legend(|legend| legend.title("Color"))
                })
                .size_with(lit(150.0), |c| c.no_scale()),
        );

    let compiled = plot_larger
        .compile(&ctx)
        .await
        .expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "theme_font_size",
        "base_font_size_with_param_18px",
    )
    .await;
}

#[tokio::test]
async fn test_mark_default_with_param() {
    // Test that params can override mark defaults defined in CSS
    let categories = StringArray::from(vec!["A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create theme with CSS that uses a variable for mark fill
    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
            mark[type="symbol"] {
                fill: var(--symbol-fill);
            }
            :root {
                --symbol-fill: steelblue;
            }
        "#,
        )
        .unwrap();

    // Override the symbol fill color via parameter
    let fill_param = {
        let __avenger_param_name = "--symbol-fill";
        let __avenger_param_default: datafusion::common::ScalarValue = ("orange").into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Mark Default Override via Param")
        .subtitle("Symbol fill controlled by --symbol-fill param")
        .theme(theme)
        .param(fill_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 4.0))))
                .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 5.0))))
                .size_with(lit(200.0), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "theme_font_size",
        "mark_default_with_param",
    )
    .await;
}
