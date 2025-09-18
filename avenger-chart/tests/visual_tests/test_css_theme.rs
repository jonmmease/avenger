//! Visual tests for CSS-based themes

use super::helpers::assert_visual_match;
use avenger_chart::prelude::*;
use avenger_chart::theme::css::CssTheme;
use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_css_theme_basic() {
    // Define a CSS theme with custom colors and styling
    let css = r#"
        /* Global defaults */
        :root {
            --primary-color: orange;
            --secondary-color: #10b981;
            --accent-color: #f59e0b;
            --text-color: green;
            --grid-color: yellow;
        }

        /* Mark styling */
        mark {
            fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2, #D55E00, #CC79A7, #999999;
            stroke: darkolivegreen;
            stroke-width: 2px;
        }

        mark[type="symbol"] {
            size: 300px;
        }

        mark[type="line"] {
            stroke: orange;
            stroke-width: 3px;
            fill: none;
        }

        mark[type="rect"] {
            fill: #f59e0b;
            stroke: #92400e;
            stroke-width: 1px;
        }

        /* Axis styling */
        axis {
            color: var(--text-color);
            font-size: 11px;
            font-family: "Inter", "Helvetica", sans-serif;
        }

        axis.label {
            font-weight: 400;
            /*color: #6b7280;*/
            color: darkmagenta;
        }

        axis.title {
            font-size: 1.1rem;
            font-weight: 600;
            color: darkcyan;
        }

        axis.grid {
            stroke: var(--grid-color);
            stroke-width: 0.5px;
            opacity: 0.6;
        }

        axis.tick {
            stroke: orangered;
        }

        axis.domain {
            stroke: blue;
            stroke-width: 2px;
        }

        /* Legend styling */
        legend {
            font-size: 16px;
            color: var(--text-color);
        }

        legend.title {
            font-size: 1rem;
            font-weight: 600;
            color: purple;
        }

        legend.label {
            font-size: 0.8rem;
            font-weight: 400;
        }

        /* Title styling */
        title {
            font-size: 20px;
            font-weight: 700;
            color: var(--text-color);
        }

        subtitle {
            font-size: 16px;
            font-weight: 400;
            color: #6b7280;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS theme");

    // Create a simple scatter plot with the CSS theme
    let data = vec![
        (1.0, 2.0, "A"),
        (2.0, 5.0, "A"),
        (3.0, 3.0, "B"),
        (4.0, 8.0, "B"),
        (5.0, 4.0, "A"),
        (6.0, 9.0, "B"),
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
        .title("CSS Theme Example")
        .subtitle("Using custom colors and typography")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Axis").grid(true)))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Axis").grid(true)))
                .fill(col("category")),
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_theme_basic", 0.9999).await;
}

#[tokio::test]
async fn test_css_theme_scale_ranges() {
    // Define a CSS theme with custom ranges using discrete/continuous properties
    let css = r#"
        /* Apply Okabe-Ito colors for discrete fills */
        mark {
            fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2, #D55E00, #CC79A7, #999999;
            shape-discrete: circle, square, cross, diamond, triangle, star;
        }

        /* Linear gradient for continuous color scales */
        mark {
            fill-continuous: #2c7bb6, #d7191c;
        }

        /* Basic styling */
        title {
            font-size: 18px;
            font-weight: 600;
            color: #333;
        }

        mark[type="symbol"] {
            size: 150px;
            stroke: white;
            stroke-width: 1px;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS theme");

    // Create a scatter plot that will use the Okabe-Ito colors
    let data = vec![
        (1.0, 2.0, "Type A"),
        (2.0, 5.0, "Type B"),
        (3.0, 3.0, "Type C"),
        (4.0, 8.0, "Type D"),
        (5.0, 4.0, "Type E"),
        (6.0, 9.0, "Type F"),
        (7.0, 6.0, "Type G"),
        (8.0, 7.0, "Type H"),
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
        .title("Okabe-Ito Color Palette via CSS")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")),
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_theme_scale_ranges", 0.9999).await;
}

#[tokio::test]
async fn test_css_theme_dark_mode() {
    // Define a dark mode CSS theme
    let css = r#"
        /* Dark mode theme */
        :root {
            --bg-color: #1a1a1a;
            --fg-color: #e0e0e0;
            --primary: #60a5fa;
            --secondary: #34d399;
            --accent: #fbbf24;
            --grid: #374151;
        }

        /* Canvas background */
        canvas {
            background-color: var(--bg-color);
        }

        /* Marks */
        mark {
            fill: var(--primary);
            stroke: var(--bg-color);
            stroke-width: 1.5px;
        }

        mark[type="symbol"] {
            size: 60px;
        }

        mark[type="line"] {
            stroke: var(--accent);
            stroke-width: 2.5px;
            fill: none;
        }

        mark.area {
            fill: var(--secondary);
            opacity: 0.7;
        }

        /* Axes */
        axis {
            color: var(--fg-color);
            font-size: 10px;
        }

        axis.domain {
            stroke: var(--fg-color);
            stroke-width: 1px;
        }

        axis.tick {
            stroke: var(--fg-color);
            stroke-width: 0.5px;
        }

        axis.grid {
            stroke: var(--grid);
            stroke-width: 0.5px;
            opacity: 0.4;
        }

        axis.title {
            font-size: 12px;
            font-weight: 500;
        }

        /* Legends */
        legend {
            color: var(--fg-color);
        }

        legend.title {
            font-size: 13px;
            font-weight: 600;
        }

        /* Titles */
        title {
            color: var(--fg-color);
            font-size: 16px;
            font-weight: 700;
        }

        subtitle {
            color: #9ca3af;
            font-size: 13px;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS dark theme");

    // Create a line chart with the dark theme
    let x_data: Vec<f64> = (0..20).map(|i| i as f64 * 0.5).collect();
    let y1_data: Vec<f64> = x_data.iter().map(|x| (x * 0.5).sin() * 3.0 + 5.0).collect();
    let y2_data: Vec<f64> = x_data.iter().map(|x| (x * 0.3).cos() * 2.0 + 5.0).collect();

    let mut series = Vec::new();
    for (i, (x, y)) in x_data.iter().zip(y1_data.iter()).enumerate() {
        series.push((*x, *y, "Series A", i as i32));
    }
    for (i, (x, y)) in x_data.iter().zip(y2_data.iter()).enumerate() {
        series.push((*x, *y, "Series B", i as i32));
    }

    let x_array = Float64Array::from(series.iter().map(|(x, _, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(series.iter().map(|(_, y, _, _)| *y).collect::<Vec<_>>());
    let series_array = StringArray::from(series.iter().map(|(_, _, s, _)| *s).collect::<Vec<_>>());
    let index_array = Int32Array::from(series.iter().map(|(_, _, _, i)| *i).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("index", DataType::Int32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(series_array),
            Arc::new(index_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Dark Mode CSS Theme")
        .subtitle("Line chart with custom dark styling")
        .mark(
            Line::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("Time").grid(true)))
                .y_with(col("y"), |c| c.axis(|a| a.title("Value").grid(true)))
                .stroke(col("series"))
                .stroke_width(2.5)
                .order(col("index")),
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_theme_dark_mode", 0.9999).await;
}

#[tokio::test]
async fn test_css_theme_discrete_continuous_properties() {
    // CSS theme using discrete and continuous property variants
    let css = r#"
        /* Define discrete and continuous ranges for different channels */
        mark {
            /* Discrete color palette for categorical data */
            fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2, #D55E00, #CC79A7;
            stroke-discrete: #D55E00, #CC79A7, #E69F00, #56B4E9;

            /* Continuous color gradients */
            fill-continuous: #deebf7, #08306b;
            stroke-continuous: #fee6ce, #a63603;

            /* Size ranges */
            size-discrete: 50, 100, 150, 200, 250;
            size-continuous: 20, 400;

            /* Opacity ranges */
            opacity-discrete: 0.3, 0.5, 0.7, 0.9, 1.0;
            opacity-continuous: 0.1, 1.0;
        }

        /* Mark-specific overrides */
        mark[type="symbol"] {
            fill-discrete: #ff7f00, #377eb8, #4daf4a, #984ea3, #ff7f00;
            size-discrete: 100, 200, 300;
        }

        mark[type="rect"] {
            fill-discrete: #a6cee3, #1f78b4, #b2df8a, #33a02c;
            opacity-discrete: 0.8, 0.9, 1.0;
        }

        /* Basic styling */
        title {
            font-size: 18px;
            font-weight: 600;
        }

        axis {
            font-size: 11px;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS theme");

    // Create a scatter plot with both categorical and continuous data
    let data = vec![
        (1.0, 2.0, "Type A", 0.3),
        (2.0, 5.0, "Type B", 0.5),
        (3.0, 3.0, "Type C", 0.7),
        (4.0, 8.0, "Type A", 0.9),
        (5.0, 4.0, "Type B", 0.4),
        (6.0, 9.0, "Type C", 0.8),
    ];

    let x_array = Float64Array::from(data.iter().map(|(x, _, _, _)| *x).collect::<Vec<_>>());
    let y_array = Float64Array::from(data.iter().map(|(_, y, _, _)| *y).collect::<Vec<_>>());
    let category_array = StringArray::from(data.iter().map(|(_, _, c, _)| *c).collect::<Vec<_>>());
    let value_array = Float64Array::from(data.iter().map(|(_, _, _, v)| *v).collect::<Vec<_>>());

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_array),
            Arc::new(y_array),
            Arc::new(category_array),
            Arc::new(value_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .title("CSS Discrete/Continuous Properties")
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
                .fill(col("category")) // Uses fill-discrete
                .size(col("value")), // Uses size-continuous
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_theme_discrete_continuous", 0.9999).await;
}
