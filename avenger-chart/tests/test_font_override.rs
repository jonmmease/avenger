//! Test font family overrides at different levels

use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_axis_font_override() {
    // Create test data
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create plot with theme and axis font overrides
    let plot = Plot::<Cartesian>::new()
        .with_theme(|t| {
            t.with_font_family("DefaultFont")
                .with_axis_title_font_family("AxisTitleDefault")
                .with_axis_label_font_family("AxisLabelDefault")
        })
        .title("Test Font Override")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| {
                        a.title("X Axis")
                            .title_font_family("CustomXTitle")
                            .label_font_family("CustomXLabel")
                    })
                })
                .y_with(col("y"), |c| {
                    c.axis(|a| {
                        a.title("Y Axis")
                        // Y axis doesn't override fonts, should use theme defaults
                    })
                })
                .size(100.0),
        );

    // Verify that theme is configured correctly
    let theme = plot.get_theme();
    assert_eq!(theme.base_font_family(), "DefaultFont");
    assert_eq!(theme.axis_title_font_family(), "AxisTitleDefault");
    assert_eq!(theme.axis_label_font_family(), "AxisLabelDefault");

    // The test mainly verifies that the API compiles and works
    // Actual font rendering would be tested in visual tests
}

#[test]
fn test_theme_font_override_hierarchy() {
    // Test that theme font families can be overridden at different levels
    let theme = StructTheme::default()
        .with_font_family("BaseFont")
        .with_title_font_family("ThemeTitleFont")
        .with_subtitle_font_family("ThemeSubtitleFont");

    assert_eq!(theme.base_font_family, "BaseFont");
    assert_eq!(theme.title_font_family(), "ThemeTitleFont");
    assert_eq!(theme.subtitle_font_family(), "ThemeSubtitleFont");

    // Test that clearing with set_font_family resets overrides
    let mut theme2 = theme.clone();
    theme2.set_font_family("NewBase");
    assert_eq!(theme2.base_font_family, "NewBase");
    // Overrides should be cleared
    assert_eq!(theme2.title_font_family(), "NewBase");
    assert_eq!(theme2.subtitle_font_family(), "NewBase");
}
