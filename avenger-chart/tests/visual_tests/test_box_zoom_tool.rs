use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

fn scatter_data() -> DataFrame {
    let x_values = Float64Array::from(vec![0.8, 1.8, 3.2, 4.8, 6.1, 7.6, 8.8]);
    let y_values = Float64Array::from(vec![1.2, 3.1, 2.4, 6.8, 4.9, 7.7, 5.8]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("scatter batch");
    let ctx = SessionContext::new();
    ctx.read_batch(batch).expect("scatter dataframe")
}

#[tokio::test]
async fn test_box_zoom_active_overlay() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .canvas_size(460.0, 340.0)
        .data(scatter_data())
        .tool(BoxZoom::cartesian())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("y"))
                })
                .size(120.0)
                .fill("#3b7ddd")
                .stroke("#1e3f73")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let mut params = IndexMap::new();
    params.insert(
        "__tool_box_zoom__active".to_string(),
        ScalarValue::Boolean(Some(true)),
    );
    params.insert(
        "__tool_box_zoom__x0".to_string(),
        ScalarValue::Float64(Some(2.0)),
    );
    params.insert(
        "__tool_box_zoom__y0".to_string(),
        ScalarValue::Float64(Some(2.0)),
    );
    params.insert(
        "__tool_box_zoom__x1".to_string(),
        ScalarValue::Float64(Some(7.5)),
    );
    params.insert(
        "__tool_box_zoom__y1".to_string(),
        ScalarValue::Float64(Some(8.0)),
    );

    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params),
        "tool",
        "box_zoom_active_overlay",
    )
    .await;
}
