use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::line::Line;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{ident, lit};
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data with two different scales
    let days = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let temperature = Float64Array::from(vec![20.0, 22.0, 21.0, 23.0, 24.0]);
    let humidity = Float64Array::from(vec![65.0, 70.0, 68.0, 72.0, 75.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("day", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
        Field::new("humidity", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(days), Arc::new(temperature), Arc::new(humidity)],
    )?;

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch)?;

    // Create plot with multiple y-axes
    let _plot = Plot::new(Cartesian)
        .preferred_size(800.0, 600.0)
        .data(df)
        // Configure primary x scale
        .scale_x(|scale| scale.domain((0.0, 6.0)))
        // Configure primary y scale for temperature
        .scale_y(|scale| scale.domain((15.0, 30.0)))
        // Add alternative y scale for humidity
        .scale_y_alt("y_humidity", |scale| scale.domain((60.0, 80.0)))
        // Configure axes
        .axis_x(|axis| axis.title("Day"))
        .axis_y(|axis| axis.title("Temperature (°C)"))
        .axis_y_alt("y_humidity", |axis| axis.title("Humidity (%)"))
        // Add temperature line using default y scale
        .mark(
            Line::new()
                .x(ident("day"))
                .y(ident("temperature"))
                .stroke(lit("#ff0000"))
                .stroke_width(lit(2.0)),
        )
        // Add humidity line using alternative y scale
        .mark(
            Line::new()
                .x(ident("day"))
                .y(ident("humidity").scaled().with_scale("y_humidity"))
                .stroke(lit("#0000ff"))
                .stroke_width(lit(2.0)),
        );

    println!("Created plot with multiple y-axes:");
    println!("- Primary y-axis (left): Temperature scale");
    println!("- Alternative y-axis (right): Humidity scale");
    println!("- Both lines share the same x-axis");

    Ok(())
}
