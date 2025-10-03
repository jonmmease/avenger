//! Visual tests for CSS cardinality-based scale ranges

use super::helpers::assert_visual_match;
use avenger_chart::prelude::*;
use avenger_chart::theme::Theme;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_cardinality_3_categories() {
    // Define a CSS theme with cardinality-specific palettes
    let css = r#"
        /* Specific palette for 3 categories - blue spectrum */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #1f77b4, #ff7f0e, #2ca02c;
        }

        /* Base palette for other cardinalities */
        mark[type="symbol"] {
            fill-discrete: red, blue, green, yellow, purple, orange;
            size: 150px;
        }

        chart-title {
            font-size: 18px;
            font-weight: 600;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

    // Create a scatter plot with exactly 3 categories
    let data = vec![
        (1.0, 2.0, "Type A"),
        (2.0, 5.0, "Type B"),
        (3.0, 3.0, "Type C"),
        (4.0, 8.0, "Type A"),
        (5.0, 4.0, "Type B"),
        (6.0, 9.0, "Type C"),
        (7.0, 6.0, "Type A"),
        (8.0, 7.0, "Type B"),
    ];

    let x_array = Float64Array::from(data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
    let category_array = StringArray::from(data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(category_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Cardinality-Based Palette: 3 Categories")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    assert_visual_match(
        plot,
        "css_cardinality",
        "cardinality_3_categories",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_cardinality_5_categories() {
    // Define a CSS theme with cardinality-specific palettes
    let css = r#"
        /* Specific palette for 5 categories - Okabe-Ito colors */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2;
        }

        /* Base palette for other cardinalities */
        mark[type="symbol"] {
            fill-discrete: red, blue, green, yellow, purple, orange;
            size: 150px;
        }

        chart-title {
            font-size: 18px;
            font-weight: 600;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

    // Create a scatter plot with exactly 5 categories
    let data = vec![
        (1.0, 2.0, "Alpha"),
        (2.0, 5.0, "Beta"),
        (3.0, 3.0, "Gamma"),
        (4.0, 8.0, "Delta"),
        (5.0, 4.0, "Epsilon"),
        (6.0, 9.0, "Alpha"),
        (7.0, 6.0, "Beta"),
        (8.0, 7.0, "Gamma"),
        (9.0, 5.0, "Delta"),
        (10.0, 8.0, "Epsilon"),
    ];

    let x_array = Float64Array::from(data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
    let category_array = StringArray::from(data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(category_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Cardinality-Based Palette: 5 Categories")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    assert_visual_match(
        plot,
        "css_cardinality",
        "cardinality_5_categories",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_cardinality_fallback() {
    // Define a CSS theme with cardinality-specific palettes
    // This tests the fallback logic: requesting 4 categories should use the 3-category palette
    let css = r#"
        /* Specific palette for 2 categories */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: #1f77b4, #ff7f0e;
        }

        /* Specific palette for 3 categories */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #1f77b4, #ff7f0e, #2ca02c;
        }

        /* Specific palette for 5 categories */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2;
        }

        /* Base palette for other cardinalities */
        mark[type="symbol"] {
            fill-discrete: red, blue, green, yellow, purple, orange;
            size: 150px;
        }

        chart-title {
            font-size: 18px;
            font-weight: 600;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

    // Create a scatter plot with 4 categories
    // Should fall back to the 3-category palette (largest < 4)
    let data = vec![
        (1.0, 2.0, "Type A"),
        (2.0, 5.0, "Type B"),
        (3.0, 3.0, "Type C"),
        (4.0, 8.0, "Type D"),
        (5.0, 4.0, "Type A"),
        (6.0, 9.0, "Type B"),
        (7.0, 6.0, "Type C"),
        (8.0, 7.0, "Type D"),
    ];

    let x_array = Float64Array::from(data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
    let category_array = StringArray::from(data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(category_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Cardinality Fallback: 4 Categories → 3-color Palette")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    assert_visual_match(plot, "css_cardinality", "cardinality_fallback", 0.9999).await;
}

#[tokio::test]
async fn test_cardinality_multiple_channels() {
    // Test cardinality-based ranges with multiple channels (fill and stroke)
    let css = r#"
        /* Different palettes for fill and stroke based on cardinality */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #E69F00, #56B4E9, #009E73;
            stroke-discrete: #000000, #666666, #999999;
        }

        mark[type="symbol"] {
            fill-discrete: red, blue, green, yellow, purple, orange;
            stroke-discrete: black, gray, silver;
            size: 150px;
            stroke-width: 2px;
        }

        chart-title {
            font-size: 18px;
            font-weight: 600;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

    // Create a scatter plot with 3 categories for both fill and stroke
    let data = vec![
        (1.0, 2.0, "Type A", "Group 1"),
        (2.0, 5.0, "Type B", "Group 2"),
        (3.0, 3.0, "Type C", "Group 3"),
        (4.0, 8.0, "Type A", "Group 1"),
        (5.0, 4.0, "Type B", "Group 2"),
        (6.0, 9.0, "Type C", "Group 3"),
    ];

    let x_array = Float64Array::from(data.iter().map(|(x, _, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(data.iter().map(|(_, y, _, _)| *y).collect::<Vec<_>>());
    let fill_array = StringArray::from(data.iter().map(|(_, _, f, _)| *f).collect::<Vec<_>>());
    let stroke_array = StringArray::from(data.iter().map(|(_, _, _, s)| *s).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("fill_cat", DataType::Utf8, false),
        Field::new("stroke_cat", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(fill_array),
            Arc::new(stroke_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Cardinality-Based Palettes: Multiple Channels")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("fill_cat"))
                .stroke(col("stroke_cat")),
        )
        .theme(theme);

    assert_visual_match(
        plot,
        "css_cardinality",
        "cardinality_multiple_channels",
        0.9999,
    )
    .await;
}
