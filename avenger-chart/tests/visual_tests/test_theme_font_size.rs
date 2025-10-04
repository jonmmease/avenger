use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_large_base_font_size() {
    // Create test data with categories for legend
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
    let plot = Plot::<Cartesian>::new()
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

    assert_visual_match_default(plot, "theme_font_size", "large_base_font_size").await;
}

#[tokio::test]
async fn test_default_base_font_size() {
    // Create same plot but with default 12px base font for comparison
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

    // Use default light theme (12px base)
    let theme = Theme::light();

    let plot = Plot::<Cartesian>::new()
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

    assert_visual_match_default(plot, "theme_font_size", "default_base_font_size").await;
}

#[tokio::test]
async fn test_base_font_size_with_param() {
    // Create test data with categories for legend
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

    // Use default light theme (which has :root { font-size: var(--base-font-size) })
    let theme = Theme::light();

    // Override base font size to 18px using a parameter
    let base_font_param = Param::new("base-font-size", 18.0f32);

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Chart with 18px Base Font (Param)")
        .subtitle("Font size controlled via parameter")
        .theme(theme)
        .add_param(base_font_param)
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

    assert_visual_match_default(plot, "theme_font_size", "base_font_size_with_param").await;

    // Now test with a larger font size (24px) to show params can be changed
    let larger_font_param = Param::new("base-font-size", 24.0f32);

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

    let plot_larger = Plot::<Cartesian>::new()
        .data(df)
        .title("Chart with 24px Base Font (Param)")
        .subtitle("Larger font size via parameter")
        .theme(theme)
        .add_param(larger_font_param)
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

    assert_visual_match_default(plot_larger, "theme_font_size", "base_font_size_with_param_24px").await;
}
