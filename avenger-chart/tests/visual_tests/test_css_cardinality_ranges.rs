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
    // Define a CSS theme with distinct palettes for each cardinality
    // This allows us to visually verify which palette was actually selected
    let css = r#"
        /* Palette for 2 categories - red/blue */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: #e74c3c, #3498db;
        }

        /* Palette for 3 categories - warm colors (THIS SHOULD BE USED) */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #f39c12, #e67e22, #d35400;
        }

        /* Palette for 4 categories - cool colors */
        mark[type="symbol"][cardinality="4"] {
            fill-discrete: #1abc9c, #16a085, #2ecc71, #27ae60;
        }

        /* Palette for 5 categories - purple spectrum */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #9b59b6, #8e44ad, #e91e63, #c0392b, #e74c3c;
        }

        /* Base palette for other cardinalities - grayscale */
        mark[type="symbol"] {
            fill-discrete: #2c3e50, #34495e, #7f8c8d, #95a5a6, #bdc3c7, #ecf0f1;
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

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Cardinality-Based Palette: 3 Categories")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "css_cardinality",
        "cardinality_3_categories",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_cardinality_5_categories() {
    // Define a CSS theme with distinct palettes for each cardinality
    let css = r#"
        /* Palette for 2 categories - red/blue */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: #e74c3c, #3498db;
        }

        /* Palette for 3 categories - warm colors */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #f39c12, #e67e22, #d35400;
        }

        /* Palette for 4 categories - cool colors */
        mark[type="symbol"][cardinality="4"] {
            fill-discrete: #1abc9c, #16a085, #2ecc71, #27ae60;
        }

        /* Palette for 5 categories - vibrant spectrum (THIS SHOULD BE USED) */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #E91E63, #9C27B0, #673AB7, #3F51B5, #2196F3;
        }

        /* Base palette for other cardinalities - grayscale */
        mark[type="symbol"] {
            fill-discrete: #2c3e50, #34495e, #7f8c8d, #95a5a6, #bdc3c7, #ecf0f1;
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

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Cardinality-Based Palette: 5 Categories")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "css_cardinality",
        "cardinality_5_categories",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_cardinality_fallback() {
    // Define a CSS theme with cardinality-specific palettes
    // This tests the fallback logic: requesting 4 categories should use the 5-category palette
    // since no 4-category palette exists and 5 is the smallest available >= 4
    let css = r#"
        /* Palette for 2 categories - bright red/cyan */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: #FF0000, #00FFFF;
        }

        /* Palette for 3 categories - green spectrum */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #00FF00, #32CD32, #228B22;
        }

        /* Palette for 5 categories - rainbow (THIS SHOULD BE USED for 4 categories) */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #FF0000, #FF7F00, #FFFF00, #00FF00, #0000FF;
        }

        /* Base palette for other cardinalities - brown tones */
        mark[type="symbol"] {
            fill-discrete: #8B4513, #A0522D, #D2691E, #CD853F, #DEB887, #F5DEB3;
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

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Cardinality Fallback: 4 Categories → 5-color Palette")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "css_cardinality",
        "cardinality_fallback",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_cardinality_multiple_channels() {
    // Test cardinality-based ranges with multiple channels (fill and stroke)
    let css = r#"
        /* Different palettes for fill and stroke based on cardinality */

        /* 2 categories - bright primary colors for fill, dark grays for stroke */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: #FF0000, #0000FF;
            stroke-discrete: #000000, #333333;
        }

        /* 3 categories - warm spectrum for fill, grayscale for stroke (THIS SHOULD BE USED) */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: #E69F00, #56B4E9, #009E73;
            stroke-discrete: #000000, #666666, #999999;
        }

        /* 4 categories - cool colors for fill, blue tones for stroke */
        mark[type="symbol"][cardinality="4"] {
            fill-discrete: #1abc9c, #16a085, #2ecc71, #27ae60;
            stroke-discrete: #34495e, #2c3e50, #5d6d7e, #85929e;
        }

        /* 5 categories - pastels for fill, browns for stroke */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: #ffb3ba, #ffdfba, #ffffba, #baffc9, #bae1ff;
            stroke-discrete: #8b4513, #a0522d, #d2691e, #cd853f, #deb887;
        }

        /* Base palette for other cardinalities - rainbow for fill, metallics for stroke */
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

    let plot = Chart::<Cartesian>::new()
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

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "css_cardinality",
        "cardinality_multiple_channels",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn test_ggplot2_hsl_cardinality_theme() {
    // Test the ggplot2-inspired HSL theme with evenly spaced hues
    // This theme uses HSL colors with fixed saturation (65%) and lightness (60%)
    // and hues evenly distributed around the color wheel for each cardinality
    let css = r#"
        :root {
            font-family: "Inter", sans-serif;
            font-size: 12px;
        }

        canvas {
            background-color: white;
            margin: 15px;
        }

        plot {
            background-color: #f8f8f8;
        }

        chart-title {
            font-size: 1.4rem;
            font-weight: 500;
            color: #2c3e50;
        }

        axis title {
            font-size: 1.0rem;
            font-weight: 400;
            color: #34495e;
        }

        axis label {
            font-size: 0.9rem;
            color: #7f8c8d;
        }

        axis grid {
            stroke: #dfe6e9;
            opacity: 0.7;
        }

        legend title {
            font-size: 1.0rem;
            font-weight: 500;
            color: #2c3e50;
        }

        legend label {
            font-size: 0.9rem;
            color: #34495e;
        }

        /* Evenly spaced hues: 2 colors at 0° and 180° */
        mark[type="symbol"][cardinality="2"] {
            fill-discrete: hsl(15 65% 60%), hsl(195 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 3 colors at 0°, 120°, 240° */
        mark[type="symbol"][cardinality="3"] {
            fill-discrete: hsl(15 65% 60%), hsl(135 65% 60%), hsl(255 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 4 colors at 0°, 90°, 180°, 270° */
        mark[type="symbol"][cardinality="4"] {
            fill-discrete: hsl(15 65% 60%), hsl(105 65% 60%),
                           hsl(195 65% 60%), hsl(285 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 5 colors at 72° intervals */
        mark[type="symbol"][cardinality="5"] {
            fill-discrete: hsl(15 65% 60%), hsl(87 65% 60%), hsl(159 65% 60%),
                           hsl(231 65% 60%), hsl(303 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 6 colors at 60° intervals (THIS SHOULD BE USED) */
        mark[type="symbol"][cardinality="6"] {
            fill-discrete: hsl(15 65% 60%), hsl(75 65% 60%), hsl(135 65% 60%),
                           hsl(195 65% 60%), hsl(255 65% 60%), hsl(315 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 7 colors at ~51.4° intervals */
        mark[type="symbol"][cardinality="7"] {
            fill-discrete: hsl(15 65% 60%), hsl(66 65% 60%), hsl(117 65% 60%),
                           hsl(168 65% 60%), hsl(219 65% 60%), hsl(270 65% 60%),
                           hsl(321 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 8 colors at 45° intervals */
        mark[type="symbol"][cardinality="8"] {
            fill-discrete: hsl(15 65% 60%), hsl(60 65% 60%), hsl(105 65% 60%),
                           hsl(150 65% 60%), hsl(195 65% 60%), hsl(240 65% 60%),
                           hsl(285 65% 60%), hsl(330 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 9 colors at 40° intervals */
        mark[type="symbol"][cardinality="9"] {
            fill-discrete: hsl(15 65% 60%), hsl(55 65% 60%), hsl(95 65% 60%),
                           hsl(135 65% 60%), hsl(175 65% 60%), hsl(215 65% 60%),
                           hsl(255 65% 60%), hsl(295 65% 60%), hsl(335 65% 60%);
            size: 140px;
        }

        /* Evenly spaced hues: 10 colors at 36° intervals */
        mark[type="symbol"][cardinality="10"] {
            fill-discrete: hsl(15 65% 60%), hsl(51 65% 60%), hsl(87 65% 60%),
                           hsl(123 65% 60%), hsl(159 65% 60%), hsl(195 65% 60%),
                           hsl(231 65% 60%), hsl(267 65% 60%), hsl(303 65% 60%),
                           hsl(339 65% 60%);
            size: 140px;
        }

        /* Base palette - standard evenly spaced 8-color set */
        mark[type="symbol"] {
            fill-discrete: hsl(15 65% 60%), hsl(60 65% 60%), hsl(105 65% 60%),
                           hsl(150 65% 60%), hsl(195 65% 60%), hsl(240 65% 60%),
                           hsl(285 65% 60%), hsl(330 65% 60%);
            size: 140px;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

    // Create a scatter plot with exactly 6 categories to test the 6-color HSL palette
    let data = vec![
        (1.0, 2.0, "Category A"),
        (2.0, 5.0, "Category B"),
        (3.0, 3.0, "Category C"),
        (4.0, 8.0, "Category D"),
        (5.0, 4.0, "Category E"),
        (6.0, 9.0, "Category F"),
        (7.0, 6.0, "Category A"),
        (8.0, 7.0, "Category B"),
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

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("ggplot2-Style HSL Palette: 6 Categories")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "css_cardinality",
        "ggplot2_hsl_6_categories",
        0.9999,
    )
    .await;
}
