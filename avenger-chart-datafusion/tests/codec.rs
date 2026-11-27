//! A logical plan that builds a chart travels through the codec: the plan of a mark's aggregate
//! over a table decodes to an equal plan in another session, natively and, under
//! `wasm-pack test`, in a browser, and builds the chart of the plan that was sent.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field},
};
use avenger_chart::{
    functions::{mark, ChannelArg},
    Chart,
};
use avenger_chart_datafusion::{
    codec::{Hosts, PlanCodec},
    functions::MarkInstance,
};
use datafusion::{
    catalog::TableProvider,
    dataframe::DataFrame,
    datasource::{provider_as_source, MemTable},
    functions_aggregate::sum::sum,
    logical_expr::{col, AggregateUDF, Expr, LogicalPlan, LogicalPlanBuilder},
    prelude::{SessionConfig, SessionContext},
};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// A table of a key of three categories and a value.
fn table() -> Arc<dyn TableProvider> {
    let keys = StringArray::from(vec!["a", "b", "c", "a", "b", "c", "a"]);
    let values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    let batch = RecordBatch::try_from_iter([
        ("k", Arc::new(keys) as ArrayRef),
        ("v", Arc::new(values) as ArrayRef),
    ])
    .unwrap();
    Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap())
}

/// The plan of the chart of the bars of the sums of `v` by `k` over `table`.
fn bars(table: &Arc<dyn TableProvider>) -> LogicalPlan {
    let mut call = mark("mark_bar").unwrap().call();
    call.channel("x", ChannelArg::Field("k".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("total".into()), None, None, &[], &[])
        .unwrap();
    let fields = vec![
        Arc::new(Field::new("k", DataType::Utf8, true)),
        Arc::new(Field::new("total", DataType::Float64, true)),
    ];
    let instance = MarkInstance::new(call.bind().unwrap(), fields, &[]);
    let mark_bar = AggregateUDF::new_from_impl(instance).call(vec![col("k"), col("total")]);
    LogicalPlanBuilder::scan("t", provider_as_source(table.clone()), None)
        .unwrap()
        .aggregate(vec![col("k")], vec![sum(col("v")).alias("total")])
        .unwrap()
        .aggregate(Vec::<Expr>::new(), vec![mark_bar.alias("chart")])
        .unwrap()
        .build()
        .unwrap()
}

/// The sums of the bars of the chart that `plan` builds in `ctx`, by key.
async fn sums(ctx: &SessionContext, plan: LogicalPlan) -> Vec<(String, f64)> {
    let batches = DataFrame::new(ctx.state(), plan).collect().await.unwrap();
    let bytes = batches[0].column(0).as_binary_view().value(0).to_vec();
    let chart = Chart::from_bytes(bytes).unwrap();
    let table = &chart.tables()[0];
    let keys = arrow::compute::cast(table.column_by_name("k").unwrap(), &DataType::Utf8).unwrap();
    let keys = keys.as_string::<i32>();
    let totals = table.column_by_name("total").unwrap();
    let totals = totals.as_primitive::<arrow::datatypes::Float64Type>();
    let mut sums: Vec<(String, f64)> = (0..table.num_rows())
        .map(|row| (keys.value(row).to_string(), totals.value(row)))
        .collect();
    sums.sort_by(|a, b| a.0.cmp(&b.0));
    sums
}

/// The plan of the bars decodes in another session to the plan that was sent, and builds its
/// chart there.
async fn travels() {
    let table = table();
    let plan = bars(&table);
    let codec = PlanCodec::new(Hosts::default(), vec![table]);
    let bytes = codec.encode(&plan).unwrap();
    let reader = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
    let decoded = codec.decode(&bytes, &reader.task_ctx()).unwrap();
    assert_eq!(decoded, plan);
    let expected = vec![("a".into(), 12.0), ("b".into(), 7.0), ("c".into(), 9.0)];
    assert_eq!(sums(&reader, decoded).await, expected);
}

/// A plan that scans a table outside the codec's tables does not encode.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_scan_of_another_table_does_not_encode() {
    let plan = bars(&table());
    let error = PlanCodec::new(Hosts::default(), Vec::new())
        .encode(&plan)
        .unwrap_err();
    assert!(
        error.to_string().contains("not one of its tables"),
        "{error}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn a_chart_plan_travels_natively() {
    travels().await;
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
async fn a_chart_plan_travels_in_a_browser() {
    travels().await;
}
