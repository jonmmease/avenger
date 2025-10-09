//! Visual tests for CSS Media Query support with guide background colors

use super::helpers::assert_visual_match;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use avenger_chart::theme::Theme;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

/// Shared CSS theme for media query tests
fn create_media_query_theme() -> Theme {
    let css = r#"
        /* Canvas background */
        canvas {
            background-color: #ffffff;
        }

        /* Default guide background (no media query) */
        guide {
            background-color: transparent;
        }

        /* Small screens (< 600px) - Light blue background */
        @media (width < 600px) {
            guide {
                background-color: rgba(33, 150, 243, 0.12);
            }
        }

        /* Medium screens (>= 600px and < 1200px) - Light green background */
        @media (width >= 600px) and (width < 1200px) {
            guide {
                background-color: rgba(76, 175, 80, 0.12);
            }
        }

        /* Large screens (>= 1200px) - Light red background */
        @media (width >= 1200px) {
            guide {
                background-color: rgba(244, 67, 54, 0.12);
            }
        }

        /* Mark styling */
        mark[type="symbol"] {
            size: 100px;
            fill: #2196f3;
            stroke: #1565c0;
            stroke-width: 2px;
        }

        /* Axis styling */
        axis title {
            font-size: 14px;
            font-weight: 600;
            color: #424242;
        }

        axis label {
            font-size: 11px;
            color: #616161;
        }

        axis grid {
            stroke: #e0e0e0;
            stroke-width: 1px;
            opacity: 0.5;
        }

        axis domain {
            stroke: #9e9e9e;
            stroke-width: 1.5px;
        }
    "#;

    Theme::from_css(css).expect("Failed to parse CSS theme")
}

/// Create test data for media query tests
fn create_test_data(ctx: &SessionContext) -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    ctx.read_batch(batch).expect("Failed to read batch")
}

// ============================================================================
// Basic Media Query Tests
// ============================================================================

/// Test CSS media query responsive guide backgrounds with width breakpoints
///
/// Creates a single compiled plot that uses CSS media queries to change the
/// guide (plot area) background color based on canvas width:
/// - Small screens (< 600px): Light blue background
/// - Medium screens (600px to < 1200px): Light green background
/// - Large screens (>= 1200px): Light red background
///
/// The plot is compiled once and rendered at three different widths using
/// runtime parameters, demonstrating true responsive behavior without recompilation.
#[tokio::test]
async fn test_media_query_guide_background_responsive() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);
    let theme = create_media_query_theme();

    // Create width and height parameters
    let width_param = Param::new("width", ScalarValue::Float32(Some(400.0)));
    let height_param = Param::new("height", ScalarValue::Float32(Some(300.0)));

    // Create CASE expressions for title and subtitle that match media query boundaries
    let title_expr = when(width_param.expr().lt(lit(600)), lit("Small Screen (400px)"))
        .when(
            width_param.expr().lt(lit(1200)),
            lit("Medium Screen (800px)"),
        )
        .otherwise(lit("Large Screen (1400px)"))
        .unwrap();

    let subtitle_expr = when(
        width_param.expr().lt(lit(600)),
        lit("Media Query: width < 600px → Light Blue Background"),
    )
    .when(
        width_param.expr().lt(lit(1200)),
        lit("Media Query: 600px ≤ width < 1200px → Light Green Background"),
    )
    .otherwise(lit("Media Query: width ≥ 1200px → Light Red Background"))
    .unwrap();

    // Create a SINGLE plot with responsive title/subtitle based on width parameter
    let plot = Plot::<Cartesian>::new()
        .canvas_size(width_param.expr(), height_param.expr())
        .title(title_expr)
        .subtitle(subtitle_expr)
        .data(df)
        .add_param(width_param)
        .add_param(height_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.grid(true).title("X Axis")))
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        )
        .theme(theme);

    // Compile ONCE
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: Small width (400px) - should trigger width < 600px media query (Light Blue)
    let mut params_small = IndexMap::new();
    params_small.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
    params_small.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_small),
        "media_query",
        "guide_background_small_400px",
        0.9999,
    )
    .await;

    // Test 2: Medium width (800px) - should trigger 600px <= width < 1200px media query (Light Green)
    let mut params_medium = IndexMap::new();
    params_medium.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
    params_medium.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_medium),
        "media_query",
        "guide_background_medium_800px",
        0.9999,
    )
    .await;

    // Test 3: Large width (1400px) - should trigger width >= 1200px media query (Light Red)
    let mut params_large = IndexMap::new();
    params_large.insert("width".to_string(), ScalarValue::Float32(Some(1400.0)));
    params_large.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_large),
        "media_query",
        "guide_background_large_1400px",
        0.9999,
    )
    .await;
}

// ============================================================================
// Multi-Range Syntax Tests
// ============================================================================

/// Test CSS Media Queries Level 4 multi-range syntax
///
/// Verifies support for the compact multi-range syntax: `@media (600px <= width < 1200px)`
/// which is equivalent to `@media (width >= 600px) and (width < 1200px)`.
///
/// Tests three scenarios:
/// - 800px: Within range, should match (Light Purple background)
/// - 400px: Below range, should NOT match (Transparent)
/// - 1200px: At exclusive upper boundary, should NOT match (Transparent)
#[tokio::test]
async fn test_media_query_multi_range_syntax() {
    let css = r#"
        canvas {
            background-color: #ffffff;
        }

        guide {
            background-color: transparent;
        }

        /* Multi-range syntax for medium screens */
        @media (600px <= width < 1200px) {
            guide {
                background-color: rgba(156, 39, 176, 0.12);
            }
        }

        mark[type="symbol"] {
            size: 100px;
            fill: #9c27b0;
            stroke: #6a1b9a;
            stroke-width: 2px;
        }

        axis title {
            font-size: 14px;
            font-weight: 600;
            color: #424242;
        }

        axis label {
            font-size: 11px;
            color: #616161;
        }

        axis grid {
            stroke: #e0e0e0;
            stroke-width: 1px;
            opacity: 0.5;
        }

        axis domain {
            stroke: #9e9e9e;
            stroke-width: 1.5px;
        }
    "#;

    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);
    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create width and height parameters
    let width_param = Param::new("width", ScalarValue::Float32(Some(800.0)));
    let height_param = Param::new("height", ScalarValue::Float32(Some(300.0)));

    // Create CASE expressions for title and subtitle that match the multi-range boundaries
    // 600px <= width < 1200px
    let title_expr = when(
        width_param
            .expr()
            .gt_eq(lit(600))
            .and(width_param.expr().lt(lit(1200))),
        lit("Multi-Range Match (800px)"),
    )
    .when(
        width_param.expr().lt(lit(600)),
        lit("Multi-Range No Match (400px)"),
    )
    .otherwise(lit("Multi-Range Boundary (1200px)"))
    .unwrap();

    let subtitle_expr = when(
        width_param
            .expr()
            .gt_eq(lit(600))
            .and(width_param.expr().lt(lit(1200))),
        lit("Media Query: 600px ≤ width < 1200px → Light Purple Background"),
    )
    .when(
        width_param.expr().lt(lit(600)),
        lit("Media Query: 600px ≤ width < 1200px → No Match (Transparent)"),
    )
    .otherwise(lit(
        "Media Query: 600px ≤ width < 1200px → No Match (Exclusive)",
    ))
    .unwrap();

    // Create a SINGLE plot with responsive title/subtitle based on width parameter
    let plot = Plot::<Cartesian>::new()
        .canvas_size(width_param.expr(), height_param.expr())
        .title(title_expr)
        .subtitle(subtitle_expr)
        .data(df)
        .add_param(width_param)
        .add_param(height_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.grid(true).title("X Axis")))
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        )
        .theme(theme);

    // Compile ONCE
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: 800px width - should match the multi-range (in range) - Light Purple
    let mut params_match = IndexMap::new();
    params_match.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
    params_match.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_match),
        "media_query",
        "multi_range_match_800px",
        0.9999,
    )
    .await;

    // Test 2: 400px width - should NOT match (below range) - Transparent
    let mut params_no_match = IndexMap::new();
    params_no_match.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
    params_no_match.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_no_match),
        "media_query",
        "multi_range_no_match_400px",
        0.9999,
    )
    .await;

    // Test 3: 1200px width - should NOT match (at exclusive boundary) - Transparent
    let mut params_boundary = IndexMap::new();
    params_boundary.insert("width".to_string(), ScalarValue::Float32(Some(1200.0)));
    params_boundary.insert("height".to_string(), ScalarValue::Float32(Some(300.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_boundary),
        "media_query",
        "multi_range_boundary_1200px",
        0.9999,
    )
    .await;
}

// ============================================================================
// Responsive Layout Tests
// ============================================================================

/// Test responsive legend positioning via media queries
///
/// Demonstrates using media queries to change legend position based on canvas width:
/// - Wide layouts (>= 600px): Legend positioned on the right
/// - Narrow layouts (< 600px): Legend positioned on the top
///
/// This is useful for creating responsive visualizations that adapt to different
/// screen sizes while maintaining good layout and readability.
#[tokio::test]
async fn test_media_query_legend_position() {
    let css = r#"
        canvas {
            background-color: #ffffff;
        }

        /* Default: legend at right for wide layouts */
        legend {
            position: right;
        }

        /* Narrow screens (< 600px): legend at top */
        @media (width < 600px) {
            legend {
                position: top;
            }
        }

        mark[type="symbol"] {
            size: 100px;
        }

        axis title {
            font-size: 14px;
            font-weight: 600;
            color: #424242;
        }

        axis label {
            font-size: 11px;
            color: #616161;
        }

        axis grid {
            stroke: #e0e0e0;
            stroke-width: 1px;
            opacity: 0.5;
        }

        axis domain {
            stroke: #9e9e9e;
            stroke-width: 1.5px;
        }
    "#;

    let ctx = SessionContext::new();
    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create sample data with temperature values
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 7.5]);
    let temp_data = Float64Array::from(vec![10.0, 20.0, 30.0, 25.0, 15.0, 35.0, 28.0, 22.0]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(temp_data),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create width and height parameters
    let width_param = Param::new("width", ScalarValue::Float32(Some(800.0)));
    let height_param = Param::new("height", ScalarValue::Float32(Some(400.0)));

    // Create title that indicates the current layout
    let title_expr = when(width_param.expr().lt(lit(600)), lit("Narrow: Legend at Top"))
        .otherwise(lit("Wide: Legend at Right"))
        .unwrap();

    // Create a SINGLE plot with responsive legend positioning
    let plot = Plot::<Cartesian>::new()
        .canvas_size(width_param.expr(), height_param.expr())
        .title(title_expr)
        .data(df)
        .add_param(width_param)
        .add_param(height_param)
        .legend("fill", |legend| legend.title("Temperature °C"))
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|a| a.grid(true).title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|a| a.grid(true).title("Y Axis"))
                })
                .fill_with(col("temperature"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((5.0, 40.0)))
                })
                .size(100.0),
        )
        .theme(theme);

    // Compile ONCE
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: Wide layout (800px) - legend should be at right
    let mut params_wide = IndexMap::new();
    params_wide.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
    params_wide.insert("height".to_string(), ScalarValue::Float32(Some(400.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_wide),
        "media_query",
        "legend_position_wide_800px",
        0.9999,
    )
    .await;

    // Test 2: Narrow layout (400px) - legend should be at top
    let mut params_narrow = IndexMap::new();
    params_narrow.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
    params_narrow.insert("height".to_string(), ScalarValue::Float32(Some(400.0)));
    assert_visual_match(
        &compiled,
        &ctx,
        Some(params_narrow),
        "media_query",
        "legend_position_narrow_400px",
        0.9999,
    )
    .await;
}
