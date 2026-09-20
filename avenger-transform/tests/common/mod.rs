#![allow(dead_code)]
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StructArray},
        compute::concat_batches,
        record_batch::RecordBatch,
    },
    common::{Result, ScalarValue},
    logical_expr::{lit, Expr, LogicalPlan},
    prelude::SessionContext,
};
use std::sync::Arc;

pub fn batch(values: Vec<Option<f64>>) -> RecordBatch {
    RecordBatch::try_from_iter([("x", Arc::new(Float64Array::from(values)) as ArrayRef)]).unwrap()
}
pub fn source(ctx: &SessionContext, values: Vec<Option<f64>>) -> LogicalPlan {
    ctx.read_batch(batch(values))
        .unwrap()
        .into_unoptimized_plan()
}
pub async fn collect(ctx: &SessionContext, plan: LogicalPlan) -> Result<RecordBatch> {
    let schema = Arc::new(plan.schema().as_arrow().clone());
    concat_batches(
        &schema,
        ctx.execute_logical_plan(plan)
            .await?
            .collect()
            .await?
            .iter(),
    )
    .map_err(Into::into)
}
pub fn extent(min: Option<f64>, max: Option<f64>) -> Expr {
    datafusion::functions::core::named_struct().call(vec![
        lit("min"),
        lit(ScalarValue::Float64(min)),
        lit("max"),
        lit(ScalarValue::Float64(max)),
    ])
}
pub async fn evaluate(ctx: &SessionContext, expr: Expr) -> Result<ScalarValue> {
    let plan = datafusion::logical_expr::LogicalPlanBuilder::empty(true)
        .project(vec![expr.alias("v")])?
        .build()?;
    let batch = collect(ctx, plan).await?;
    ScalarValue::try_from_array(batch.column(0), 0)
}
pub fn struct_values(value: &ScalarValue) -> Vec<Option<f64>> {
    let ScalarValue::Struct(s) = value else {
        panic!("expected struct: {value}")
    };
    fields(s)
}
pub fn fields(s: &StructArray) -> Vec<Option<f64>> {
    s.columns()
        .iter()
        .map(|a| match ScalarValue::try_from_array(a, 0).unwrap() {
            ScalarValue::Float64(v) => v,
            v => panic!("{v}"),
        })
        .collect()
}
pub fn number(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Number(n) => n.as_f64(),
        _ => match v["$number"].as_str().unwrap() {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            "undefined" => None,
            _ => panic!("unknown {v}"),
        },
    }
}
pub fn assert_number(actual: Option<f64>, expected: Option<f64>) {
    match (actual, expected) {
        (None, None) => (),
        (Some(a), Some(e))
            if a == e
                || a.is_nan() && e.is_nan()
                || a.is_finite() && e.is_finite() && (a - e).abs() <= 1e-10 * e.abs().max(1.0) => {}
        _ => panic!("actual {actual:?}, expected {expected:?}"),
    }
}
pub fn measures() -> Vec<Expr> {
    use avenger_transform::expr_fn::*;
    use datafusion::logical_expr::col;
    vec![
        count().alias("count"),
        valid(col("x")).alias("valid"),
        missing(col("x")).alias("missing"),
        sum(col("x")).alias("sum"),
        min(col("x")).alias("min"),
        max(col("x")).alias("max"),
        mean(col("x")).alias("mean"),
        variance(col("x")).alias("variance"),
        variancep(col("x")).alias("variancep"),
        stdev(col("x")).alias("stdev"),
        stdevp(col("x")).alias("stdevp"),
    ]
}
