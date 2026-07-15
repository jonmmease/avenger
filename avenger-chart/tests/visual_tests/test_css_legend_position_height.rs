//! Visual tests for CSS Media Query support with height-based legend positioning
//!
//! Tests that legend position can change based on canvas height via CSS media queries.

use super::helpers::assert_visual_match;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use avenger_chart::theme::Theme;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

/// Test responsive legend positioning via height-based media queries
///
/// Demonstrates using media queries to change legend position based on canvas height:
/// - Tall layouts (height >= 300px): Legend positioned at the bottom
/// - Short layouts (height < 300px): Legend positioned on the right
///
/// This is useful for creating responsive visualizations that adapt to different
/// viewport heights while maintaining good layout and readability.
#[tokio::test]
async fn test_media_query_legend_position_height() {
    let css = r#"
        :root {
            font-family: "Inter", sans-serif;
            font-size: 12px;
        }

        canvas {
            background-color: #ffffff;
            margin: 10px;
        }

        /* Default: legend on the right for short layouts */
        legend {
            position: right;
            spacing: 8;
            label-padding: 4;
        }

        /* Tall canvases (>= 300px): legend at bottom for better vertical space usage */
        @media (height >= 300px) {
            legend {
                position: bottom;
                spacing: 12;
                label-padding: 6;
            }
        }

        legend title {
            color: #374151;
            font-weight: 500;
            font-size: 1.0rem;
        }

        legend label {
            color: #6b7280;
            font-weight: 400;
            font-size: 0.9rem;
        }

        legend background {
            fill: rgba(255, 255, 255, 0.95);
            stroke: #e5e7eb;
            stroke-width: 1.0;
            corner-radius: 4;
            padding: 8;
        }

        mark[type="line"] {
            stroke-width: 2.5;
            stroke-cap: round;
        }

        axis title {
            font-size: 14px;
            font-weight: 600;
            color: #1f2937;
        }

        axis label {
            font-size: 11px;
            color: #6b7280;
        }

        axis grid {
            stroke: #f3f4f6;
            stroke-width: 1px;
        }

        axis domain {
            stroke: #d1d5db;
            stroke-width: 1.5px;
        }
    "#;

    let ctx = SessionContext::new();
    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create sample stock price data
    let dates = vec![
        "2020-01-01",
        "2020-02-01",
        "2020-03-01",
        "2020-04-01",
        "2020-05-01",
        "2020-06-01",
        "2020-07-01",
        "2020-08-01",
    ];

    let aapl_prices = vec![75.0, 73.0, 68.0, 71.0, 77.0, 79.0, 91.0, 95.0];
    let goog_prices = vec![68.0, 71.0, 69.0, 73.0, 78.0, 82.0, 85.0, 88.0];
    let msft_prices = vec![160.0, 170.0, 165.0, 175.0, 183.0, 189.0, 202.0, 210.0];

    // Create flattened data for all stocks
    let mut all_dates = Vec::new();
    let mut all_prices = Vec::new();
    let mut all_symbols = Vec::new();

    for (i, date) in dates.iter().enumerate() {
        // AAPL
        all_dates.push(date.to_string());
        all_prices.push(aapl_prices[i]);
        all_symbols.push("AAPL".to_string());

        // GOOG
        all_dates.push(date.to_string());
        all_prices.push(goog_prices[i]);
        all_symbols.push("GOOG".to_string());

        // MSFT
        all_dates.push(date.to_string());
        all_prices.push(msft_prices[i]);
        all_symbols.push("MSFT".to_string());
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("date", DataType::Utf8, false),
        Field::new("price", DataType::Float64, false),
        Field::new("symbol", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(all_dates)),
            Arc::new(Float64Array::from(all_prices)),
            Arc::new(StringArray::from(all_symbols)),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create width and height parameters
    let width_param = {
        let __avenger_param_name = "width";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Float32(Some(600.0))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };
    let height_param = {
        let __avenger_param_name = "height";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Float32(Some(400.0))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Create title that indicates the current layout
    let title_expr = when(
        height_param.expr().gt_eq(lit(300)),
        lit("Tall Canvas: Legend at Bottom"),
    )
    .otherwise(lit("Short Canvas: Legend on Right"))
    .unwrap();

    let subtitle_expr = when(
        height_param.expr().gt_eq(lit(300)),
        lit("Height ≥ 300px triggers bottom legend position"),
    )
    .otherwise(lit("Height < 300px uses default right legend position"))
    .unwrap();

    // Create a SINGLE plot with responsive legend positioning based on height
    let plot = Chart::<Cartesian>::new()
        .canvas_size(width_param.expr(), height_param.expr())
        .title(title_expr)
        .subtitle(subtitle_expr)
        .data(df)
        .param(width_param)
        .param(height_param)
        .mark(
            Line::new()
                .x_with(col("date"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .axis(|a| a.title("Date").label_angle(-45.0))
                })
                .y_with(col("price"), |c| {
                    c.scale(|s| s.domain((60.0, 220.0)))
                        .axis(|a| a.grid(true).title("Stock Price ($)"))
                })
                .stroke_with(col("symbol"), |c| {
                    c.scale_with::<Ordinal>(|s| s).legend(|l| l.title("Stock"))
                })
                .stroke_width(2.5),
        )
        .theme(theme);

    // Compile ONCE
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: Tall layout (height=400px) - legend should be at bottom
    let mut params_tall = IndexMap::new();
    params_tall.insert("width".to_string(), ScalarValue::Float32(Some(600.0)));
    params_tall.insert("height".to_string(), ScalarValue::Float32(Some(400.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_tall),
        "media_query",
        "legend_position_height_tall_400px",
        0.9999,
    )
    .await;

    // Test 2: Short layout (height=250px) - legend should be on right
    let mut params_short = IndexMap::new();
    params_short.insert("width".to_string(), ScalarValue::Float32(Some(600.0)));
    params_short.insert("height".to_string(), ScalarValue::Float32(Some(250.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_short),
        "media_query",
        "legend_position_height_short_250px",
        0.9999,
    )
    .await;

    // Test 3: Boundary case (height=300px) - should trigger bottom position
    let mut params_boundary = IndexMap::new();
    params_boundary.insert("width".to_string(), ScalarValue::Float32(Some(600.0)));
    params_boundary.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_boundary),
        "media_query",
        "legend_position_height_boundary_300px",
        0.9999,
    )
    .await;
}

/// Test combined width and height media queries for legend positioning
///
/// Demonstrates using both width and height media queries to create a fully
/// responsive legend layout:
/// - Wide and tall (w >= 600px, h >= 400px): Legend at bottom
/// - Wide and short (w >= 600px, h < 400px): Legend at right
/// - Narrow and tall (w < 600px, h >= 400px): Legend at top
/// - Narrow and short (w < 600px, h < 400px): Legend at right
///
/// This creates an adaptive layout that optimizes legend placement based on
/// both dimensions of the available canvas space.
#[tokio::test]
async fn test_media_query_legend_position_combined() {
    let css = r#"
        :root {
            font-family: "Inter", sans-serif;
            font-size: 12px;
        }

        canvas {
            background-color: #ffffff;
            margin: 10px;
        }

        /* Default: legend on the right */
        legend {
            position: right;
        }

        /* Narrow layouts: legend at top */
        @media (width < 600px) and (height >= 400px) {
            legend {
                position: top;
            }
        }

        /* Wide and tall layouts: legend at bottom */
        @media (width >= 600px) and (height >= 400px) {
            legend {
                position: bottom;
            }
        }

        legend title {
            color: #374151;
            font-weight: 500;
            font-size: 1.0rem;
        }

        legend label {
            color: #6b7280;
            font-weight: 400;
            font-size: 0.9rem;
        }

        mark[type="symbol"] {
            size: 80px;
        }

        axis title {
            font-size: 13px;
            font-weight: 600;
            color: #1f2937;
        }

        axis label {
            font-size: 10px;
            color: #6b7280;
        }

        axis grid {
            stroke: #f3f4f6;
            stroke-width: 1px;
        }

        axis domain {
            stroke: #d1d5db;
            stroke-width: 1.5px;
        }
    "#;

    let ctx = SessionContext::new();
    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create sample data with categories
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    let mut categories = Vec::new();

    // Generate data for three categories
    for category in ["A", "B", "C"] {
        for i in 0..8 {
            x_values.push(i as f64);
            y_values.push(match category {
                "A" => 2.0 + i as f64 * 0.5 + (i as f64 * 0.3).sin(),
                "B" => 3.0 + i as f64 * 0.7 + (i as f64 * 0.4).cos(),
                "C" => 1.5 + i as f64 * 0.6 + (i as f64 * 0.5).sin(),
                _ => 0.0,
            });
            categories.push(category.to_string());
        }
    }

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(x_values)),
            Arc::new(Float64Array::from(y_values)),
            Arc::new(StringArray::from(categories)),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create width and height parameters
    let width_param = {
        let __avenger_param_name = "width";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Float32(Some(700.0))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };
    let height_param = {
        let __avenger_param_name = "height";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Float32(Some(500.0))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Create title that indicates the current layout
    let title_expr = when(
        width_param
            .expr()
            .gt_eq(lit(600))
            .and(height_param.expr().gt_eq(lit(400))),
        lit("Wide & Tall: Bottom Legend"),
    )
    .when(
        width_param
            .expr()
            .lt(lit(600))
            .and(height_param.expr().gt_eq(lit(400))),
        lit("Narrow & Tall: Top Legend"),
    )
    .otherwise(lit("Short: Right Legend"))
    .unwrap();

    // Create a SINGLE plot with combined responsive legend positioning
    let plot = Chart::<Cartesian>::new()
        .canvas_size(width_param.expr(), height_param.expr())
        .title(title_expr)
        .data(df)
        .param(width_param)
        .param(height_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                        .axis(|a| a.grid(true).title("X"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.grid(true).title("Y"))
                })
                .fill_with(col("category"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Category"))
                }),
        )
        .theme(theme);

    // Compile ONCE
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: Wide and tall (700x500) - legend should be at bottom
    let mut params_wide_tall = IndexMap::new();
    params_wide_tall.insert("width".to_string(), ScalarValue::Float32(Some(700.0)));
    params_wide_tall.insert("height".to_string(), ScalarValue::Float32(Some(500.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_wide_tall),
        "media_query",
        "legend_position_combined_wide_tall",
        0.9999,
    )
    .await;

    // Test 2: Narrow and tall (500x500) - legend should be at top
    let mut params_narrow_tall = IndexMap::new();
    params_narrow_tall.insert("width".to_string(), ScalarValue::Float32(Some(500.0)));
    params_narrow_tall.insert("height".to_string(), ScalarValue::Float32(Some(500.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_narrow_tall),
        "media_query",
        "legend_position_combined_narrow_tall",
        0.9999,
    )
    .await;

    // Test 3: Wide and short (700x350) - legend should be at right (default)
    let mut params_wide_short = IndexMap::new();
    params_wide_short.insert("width".to_string(), ScalarValue::Float32(Some(700.0)));
    params_wide_short.insert("height".to_string(), ScalarValue::Float32(Some(350.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_wide_short),
        "media_query",
        "legend_position_combined_wide_short",
        0.9999,
    )
    .await;

    // Test 4: Narrow and short (500x350) - legend should be at right (default)
    let mut params_narrow_short = IndexMap::new();
    params_narrow_short.insert("width".to_string(), ScalarValue::Float32(Some(500.0)));
    params_narrow_short.insert("height".to_string(), ScalarValue::Float32(Some(350.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_narrow_short),
        "media_query",
        "legend_position_combined_narrow_short",
        0.9999,
    )
    .await;
}
