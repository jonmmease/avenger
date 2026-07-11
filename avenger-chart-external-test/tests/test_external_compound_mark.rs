use std::sync::Arc;

use avenger_chart::plot::Chart;
use avenger_chart_cartesian::Cartesian;
use avenger_chart_external_test::external_compound_mark::ExternalMeanPoint;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{col, SessionContext},
};

fn grouped_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0, 3.0, 4.0, 6.0])) as ArrayRef,
        ],
    )
    .expect("batch");
    ctx.read_batch(batch).expect("dataframe")
}

#[tokio::test]
async fn external_compound_mark_compiles_and_supplies_scale_hint() {
    let ctx = SessionContext::new();
    let data = grouped_data(&ctx);
    let compiled = Chart::<Cartesian>::new()
        .data(data.clone())
        .mark(ExternalMeanPoint::new(col("category"), col("value")))
        .compile(&ctx)
        .await
        .expect("compile external compound mark");

    let scales = compiled
        .build_scales_for_dataframe(&data, 300.0, 200.0, &ctx, compiled.get_default_params())
        .await
        .expect("build scales");
    let x_scale = scales.get("x").expect("x scale");
    assert_eq!(x_scale.configured().scale_impl.scale_type(), "band");
}
