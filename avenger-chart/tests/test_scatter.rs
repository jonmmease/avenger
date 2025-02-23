// rustfmt::skip

use std::f32::consts::PI;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};

mod utils;
use datafusion::scalar::ScalarValue;
use utils::assert_runtime_image_equal;

use avenger_chart::prelude::*;
use datafusion::prelude::*;
use avenger_chart::guides::axis::Axis;
use avenger_chart::marks::encoding::css_color;
use avenger_guides::axis::opts::AxisOrientation;

#[tokio::test]
async fn test_scatter1() -> Result<(), AvengerChartError> {
    // Build Avenger runtime
    let runtime = Arc::new(AvengerRuntime::new(SessionContext::new()));

    // Load data
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_path = format!("{}/tests/data/Iris.csv", manifest_dir);
    let df = runtime
        .ctx()
        .read_csv(data_path, CsvReadOptions::default())
        .await?;

    // Build scales
    let x_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "SepalLengthCm")
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)))
        .option("nice", lit(true));
    let y_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "SepalWidthCm")
        .range(ScaleRange::new_interval(lit(400.0), lit(0.0)))
        .option("nice", lit(true));
    let color_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "PetalWidthCm")
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    // Build chart group
    let chart = Group::new()
        .x(0.0)
        .y(0.0)
        .size(400.0, 400.0)
        .mark(
            Mark::symbol()
                .from(df)
                .details(vec![
                    "SepalLengthCm",
                    "SepalWidthCm",
                    "PetalLengthCm",
                    "PetalWidthCm",
                    "Species",
                ])
                .x(scale_expr(&x_scale, ident("SepalLengthCm"))?)
                .y(scale_expr(&y_scale, ident("SepalWidthCm"))?)
                .fill(scale_expr(&color_scale, ident("PetalWidthCm"))?)
                .size(lit(60.0)),
        )
        .axis(Axis::new(&x_scale).orientation(AxisOrientation::Bottom))
        .axis(Axis::new(&y_scale).orientation(AxisOrientation::Left));

    // Default
    assert_runtime_image_equal(&runtime, chart.clone(), "scatter1", false, Vec::new()).await?;

    Ok(())
}
