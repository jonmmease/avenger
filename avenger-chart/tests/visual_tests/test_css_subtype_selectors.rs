//! Visual tests for CSS subtype selectors (axis[type=...], legend[type=...])

use super::helpers::assert_visual_match;
use avenger_chart::prelude::*;
use avenger_chart::theme::css::CssTheme;
use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_css_axis_and_legend_subtype_selectors() {
    // Define a CSS theme that uses subtype selectors for axes and legends
    let css = r#"
        /* Base canvas */
        canvas {
            background-color: #f5f5f5;
        }

        /* Axis subtype selectors - different colors for x vs y */
        axis[type="x"] title {
            color: #2563eb;  /* Blue for x-axis */
            font-weight: 600;
        }

        axis[type="y"] title {
            color: #dc2626;  /* Red for y-axis */
            font-weight: 600;
        }

        /* General axis styling */
        axis label {
            color: #4b5563;
            font-size: 11px;
        }

        axis grid {
            stroke: #d1d5db;
            opacity: 0.5;
        }

        axis domain {
            stroke: #6b7280;
        }

        /* Legend subtype selectors - different backgrounds for different legend types */
        legend[type="symbol"] {
            background-color: #dbeafe;  /* Light blue background for symbol legends */
        }

        legend[type="symbol"] background {
            fill: #dbeafe;
            stroke: #2563eb;
            stroke-width: 1px;
            padding: 8px;
            corner-radius: 6px;
        }

        legend[type="line"] {
            background-color: #fee2e2;  /* Light red background for line legends */
        }

        legend[type="line"] background {
            fill: #fee2e2;
            stroke: #dc2626;
            stroke-width: 1px;
            padding: 8px;
            corner-radius: 6px;
        }

        /* Legend text styling */
        legend title {
            font-size: 14px;
            font-weight: 600;
            color: #111827;
        }

        legend label {
            font-size: 12px;
            color: #374151;
        }

        /* Mark styling */
        mark[type="symbol"] {
            size: 120px;
            fill-discrete: #3b82f6, #ef4444, #10b981, #f59e0b;
        }

        mark[type="line"] {
            stroke-discrete: #8b5cf6, #ec4899, #06b6d4;
            stroke-width: 2.5px;
        }

        /* Chart titles */
        chart-title {
            font-size: 18px;
            font-weight: 700;
            color: #111827;
        }

        chart-subtitle {
            font-size: 14px;
            color: #6b7280;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS theme");

    // Create data with multiple series for both symbols and lines
    let symbol_data = vec![
        (1.0, 3.0, "Category A"),
        (2.0, 5.0, "Category B"),
        (3.0, 4.0, "Category A"),
        (4.0, 7.0, "Category B"),
        (5.0, 6.0, "Category A"),
        (6.0, 8.0, "Category B"),
    ];

    let line_data: Vec<(f64, f64, &str, i32)> = vec![
        (0.5, 2.0, "Line 1", 0),
        (1.5, 4.0, "Line 1", 1),
        (2.5, 3.5, "Line 1", 2),
        (3.5, 6.0, "Line 1", 3),
        (4.5, 5.5, "Line 1", 4),
        (5.5, 7.5, "Line 1", 5),
        (6.5, 7.0, "Line 1", 6),
        (0.5, 1.5, "Line 2", 0),
        (1.5, 3.0, "Line 2", 1),
        (2.5, 2.5, "Line 2", 2),
        (3.5, 5.0, "Line 2", 3),
        (4.5, 4.5, "Line 2", 4),
        (5.5, 6.5, "Line 2", 5),
        (6.5, 6.0, "Line 2", 6),
    ];

    // Create symbol mark data
    let symbol_x = Float64Array::from(symbol_data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
    let symbol_y = Float64Array::from(symbol_data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
    let symbol_cat = StringArray::from(symbol_data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

    let symbol_schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let symbol_batch = RecordBatch::try_new(
        symbol_schema,
        vec![
            Arc::new(symbol_x),
            Arc::new(symbol_y),
            Arc::new(symbol_cat),
        ],
    )
    .expect("Failed to create symbol RecordBatch");

    let ctx = SessionContext::new();
    let symbol_df = ctx
        .read_batch(symbol_batch)
        .expect("Failed to read symbol batch");

    // Create line mark data
    let line_x = Float64Array::from(line_data.iter().map(|(x, _, _, _)| *x).collect::<Vec<_>>());
    let line_y = Float64Array::from(line_data.iter().map(|(_, y, _, _)| *y).collect::<Vec<_>>());
    let line_series = StringArray::from(line_data.iter().map(|(_, _, s, _)| *s).collect::<Vec<_>>());
    let line_index = Int32Array::from(line_data.iter().map(|(_, _, _, i)| *i).collect::<Vec<_>>());

    let line_schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("index", DataType::Int32, false),
    ]));

    let line_batch = RecordBatch::try_new(
        line_schema,
        vec![
            Arc::new(line_x),
            Arc::new(line_y),
            Arc::new(line_series),
            Arc::new(line_index),
        ],
    )
    .expect("Failed to create line RecordBatch");

    let line_df = ctx
        .read_batch(line_batch)
        .expect("Failed to read line batch");

    // Create plot with both symbol and line marks
    let plot = Plot::<Cartesian>::new()
        .title("CSS Subtype Selector Demo")
        .subtitle("axis[type=x/y] and legend[type=symbol/line]")
        .mark(
            Symbol::new()
                .data(symbol_df)
                .x_with(col("x"), |c| c.axis(|a| a.title("X Axis (Blue)").grid(true)))
                .y_with(col("y"), |c| c.axis(|a| a.title("Y Axis (Red)").grid(true)))
                .fill(col("category")),
        )
        .mark(
            Line::new()
                .data(line_df)
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .order(col("index")),
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_subtype_selectors", 0.9999).await;
}
