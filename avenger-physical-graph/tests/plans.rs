//! A plan in the shape of a lifted chart, of DataFusion's operators and a mark aggregate, runs
//! natively and, under `wasm-pack test`, in a browser: a memory scan, a filter, the sums of a
//! group by, the mark aggregate of bars over them, and the cross join with a placeholder row that
//! a lifted chart holds.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{
    functions::{mark, ChannelArg},
    Chart,
};
use avenger_physical_graph::functions::MarkInstance;
use datafusion_common::ScalarValue;
use datafusion_datasource::memory::MemorySourceConfig;
use datafusion_execution::TaskContext;
use datafusion_expr::{AggregateUDF, Operator};
use datafusion_functions_aggregate::sum::sum_udaf;
use datafusion_physical_expr::{
    aggregate::AggregateExprBuilder,
    expressions::{binary, col, lit},
};
use datafusion_physical_plan::{
    aggregates::{AggregateExec, AggregateMode, PhysicalGroupBy},
    collect,
    filter::FilterExec,
    joins::CrossJoinExec,
    placeholder_row::PlaceholderRowExec,
    ExecutionPlan,
};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// The rows: a key of three categories and a value.
fn table() -> RecordBatch {
    let keys = StringArray::from(vec!["a", "b", "c", "a", "b", "c", "a"]);
    let values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    RecordBatch::try_from_iter([
        ("k", Arc::new(keys) as ArrayRef),
        ("v", Arc::new(values) as ArrayRef),
    ])
    .unwrap()
}

/// The chart of the bars of the sums of `v` by `k` over the rows whose `v` is above 2.
async fn bars() -> Chart {
    let batch = table();
    let schema = batch.schema();
    let scan = MemorySourceConfig::try_new_exec(&[vec![batch]], schema.clone(), None).unwrap();
    let v = col("v", &schema).unwrap();
    let above = binary(
        v.clone(),
        Operator::Gt,
        lit(ScalarValue::Float64(Some(2.0))),
        &schema,
    )
    .unwrap();
    let filter: Arc<dyn ExecutionPlan> = Arc::new(FilterExec::try_new(above, scan).unwrap());
    let sum = AggregateExprBuilder::new(sum_udaf(), vec![v])
        .schema(schema.clone())
        .alias("total")
        .build()
        .unwrap();
    let group = PhysicalGroupBy::new_single(vec![(col("k", &schema).unwrap(), "k".into())]);
    let sums: Arc<dyn ExecutionPlan> = Arc::new(
        AggregateExec::try_new(
            AggregateMode::Single,
            group,
            vec![Arc::new(sum)],
            vec![None],
            filter,
            schema,
        )
        .unwrap(),
    );

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
    let sums_schema = sums.schema();
    let args = vec![
        col("k", &sums_schema).unwrap(),
        col("total", &sums_schema).unwrap(),
    ];
    let mark_bar = AggregateExprBuilder::new(Arc::new(AggregateUDF::new_from_impl(instance)), args)
        .schema(sums_schema.clone())
        .alias("chart")
        .build()
        .unwrap();
    let marks: Arc<dyn ExecutionPlan> = Arc::new(
        AggregateExec::try_new(
            AggregateMode::Single,
            PhysicalGroupBy::new_single(vec![]),
            vec![Arc::new(mark_bar)],
            vec![None],
            sums,
            sums_schema,
        )
        .unwrap(),
    );
    let row = Arc::new(PlaceholderRowExec::new(Arc::new(Schema::empty())));
    let plan = Arc::new(CrossJoinExec::new(row, marks));

    let batches = collect(plan, Arc::new(TaskContext::default()))
        .await
        .unwrap();
    let column = batches[0].column_by_name("chart").unwrap();
    let bytes = column.as_binary_view().value(0).to_vec();
    Chart::from_bytes(bytes).unwrap()
}

/// The sums of the bars, by key.
fn sums(chart: &Chart) -> Vec<(String, f64)> {
    let table = &chart.tables()[0];
    let keys = table.column_by_name("k").unwrap();
    let keys = arrow::compute::cast(keys, &DataType::Utf8).unwrap();
    let keys = keys.as_string::<i32>();
    let totals = table.column_by_name("total").unwrap();
    let totals = totals.as_primitive::<arrow::datatypes::Float64Type>();
    let mut sums: Vec<(String, f64)> = (0..table.num_rows())
        .map(|row| (keys.value(row).to_string(), totals.value(row)))
        .collect();
    sums.sort_by(|a, b| a.0.cmp(&b.0));
    sums
}

fn expected() -> Vec<(String, f64)> {
    vec![("a".into(), 11.0), ("b".into(), 5.0), ("c".into(), 9.0)]
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_lifted_chart_runs_natively() {
    let chart = futures::executor::block_on(bars());
    assert_eq!(sums(&chart), expected());
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
async fn a_lifted_chart_runs_in_a_browser() {
    let chart = bars().await;
    assert_eq!(sums(&chart), expected());
}
