// rustfmt::skip

use std::f32::consts::PI;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};

mod utils;
use utils::assert_runtime_image_equal;

use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn test_arcs() -> Result<(), AvengerChartError> {
    // runtime
    let runtime = AvengerRuntime::new(SessionContext::new());

    // params
    let stroke_color = Param::new("stroke_color", "cyan");
    let width = Param::new("width", 300.0);

    // Load dataframe
    let schema = Schema::new(vec![Field::new("a", DataType::Float32, true)]);
    let columns = vec![Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as ArrayRef];
    let batch = RecordBatch::try_new(Arc::new(schema), columns).unwrap();
    let data_0 = runtime.ctx().read_batch(batch).unwrap();

    // scales
    let x_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(data_0.clone()), "a")
        .range(ScaleRange::new_interval(lit(0.0), &width));

    let y_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));

    let color_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    let chart = Group::new()
        .x(10.0)
        .y(10.0)
        .mark(
            Mark::arc()
                .from(data_0)
                .x(scale_expr(&x_scale, col("a")).unwrap())
                .y(scale_expr(&y_scale, lit(5.0)).unwrap())
                .start_angle(lit(0.0))
                .end_angle(lit(PI / 2.0))
                .outer_radius(lit(50.0))
                .inner_radius(lit(20.0))
                .fill(scale_expr(&color_scale, col("a")).unwrap())
                .stroke(&stroke_color)
                .stroke_width(lit(3.0)),
        )
        .param(width)
        .param(stroke_color);

    assert_runtime_image_equal(&runtime, chart, "arcs").await?;

    Ok(())
}
