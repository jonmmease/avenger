//! Visual tests for CSS guide subtype selectors (guide[type=...])

use super::helpers::assert_visual_match;
use avenger_chart::prelude::*;
use avenger_chart::theme::Theme;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_css_guide_subtypes() {
    // Define a CSS theme that uses guide subtype selectors for different coordinate systems
    let css = r#"
        /* Canvas background */
        canvas {
            background-color: #f8f9fa;
        }

        /* Guide subtype selectors - different backgrounds for cartesian vs polar */
        guide[type="cartesian"] {
            background-color: #e3f2fd;  /* Light blue for cartesian */
        }

        guide[type="polar"] {
            background-color: #fff3e0;  /* Light orange for polar */
        }

        /* Cartesian axis styling - nested under guide[type="cartesian"] */
        guide[type="cartesian"] axis title {
            font-size: 14px;
            font-weight: 700;
            color: #1976d2;  /* Blue for cartesian */
        }

        guide[type="cartesian"] axis label {
            font-size: 11px;
            color: #424242;
        }

        guide[type="cartesian"] axis grid {
            stroke: #90caf9;
            stroke-width: 1.5px;
            opacity: 0.4;
        }

        guide[type="cartesian"] axis domain {
            stroke: #1976d2;
            stroke-width: 2px;
        }

        /* Polar axis styling - different from cartesian */
        guide[type="polar"] axis title {
            font-size: 13px;
            font-weight: 600;
            color: #e65100;  /* Orange for polar */
        }

        guide[type="polar"] axis label {
            font-size: 10px;
            color: #5d4037;
        }

        guide[type="polar"] axis grid {
            stroke: #ffb74d;
            stroke-width: 1px;
            opacity: 0.5;
        }

        /* Mark styling */
        mark[type="symbol"] {
            size: 100px;
            fill: #ff6b6b;
            stroke: #1976d2;
            stroke-width: 2px;
        }

        mark[type="line"] {
            stroke: #4ecdc4;
            stroke-width: 3px;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create test data
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).expect("Failed to read batch");

    // Create a cartesian plot (should get light blue background)
    let plot = Plot::<Cartesian>::new()
        .title("Cartesian Guide")
        .subtitle("Should have light blue background from guide[type=\"cartesian\"]")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.grid(true).title("X")))
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y"))),
        )
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_guide_subtype_cartesian", 0.9999).await;
}

#[tokio::test]
async fn test_css_guide_subtypes_polar() {
    let css = r#"
        canvas {
            background-color: #f8f9fa;
        }

        guide[type="polar"] {
            background-color: #fff3e0;  /* Light orange for polar */
        }

        /* Polar-specific axis styling */
        guide[type="polar"] axis title {
            font-size: 13px;
            font-weight: 600;
            color: #e65100;  /* Orange */
        }

        guide[type="polar"] axis label {
            font-size: 10px;
            color: #5d4037;  /* Brown */
        }

        guide[type="polar"] axis grid {
            stroke: #ffb74d;  /* Light orange */
            stroke-width: 1px;
            opacity: 0.5;
        }

        mark[type="symbol"] {
            size: 120px;
            fill: #9c27b0;
            stroke: #e65100;
            stroke-width: 2px;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS theme");

    // Create polar data
    let radius_values = Float64Array::from(vec![30.0, 50.0, 70.0, 90.0, 60.0, 40.0]);
    let theta_values = Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("r", DataType::Float64, false),
        Field::new("theta", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(radius_values), Arc::new(theta_values)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).expect("Failed to read batch");

    // Create a polar plot (should get light orange background)
    let plot = Plot::<Polar>::new()
        .title("Polar Guide")
        .subtitle("Should have light orange background from guide[type=\"polar\"]")
        .data(df)
        .mark(Symbol::<Polar>::new().r(col("r")).theta(col("theta")))
        .theme(theme);

    assert_visual_match(plot, "css_theme", "css_guide_subtype_polar", 0.9999).await;
}
