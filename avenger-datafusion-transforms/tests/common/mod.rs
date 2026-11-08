//! Helpers that the tests share.

#![allow(dead_code)]

use std::sync::Arc;

use datafusion::arrow::array::{ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::catalog::MemTable;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;
use serde_json::Value;

/// Registers the columns as the table `name`, in place of any table of that name.
pub fn register(ctx: &SessionContext, name: &str, columns: Vec<(&str, ArrayRef)>) -> Result<()> {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(n, a)| Field::new(*n, a.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        columns.into_iter().map(|(_, a)| a).collect(),
    )?;
    ctx.deregister_table(name)?;
    ctx.register_table(
        name,
        Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    )?;
    Ok(())
}

/// A fixture of tests/fixtures, which tools/vega-transforms-reference writes.
pub fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap()
}

/// A number of a fixture, where JSON strings hold the infinities and NaN.
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Null => None,
        Value::Number(n) => n.as_f64(),
        Value::String(s) => Some(s.parse().expect("Infinity, -Infinity or NaN")),
        _ => panic!("not a number: {v}"),
    }
}

/// The same bits, or both NaN.
pub fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// The error of planning and running a query, or None.
pub async fn error(ctx: &SessionContext, sql: &str) -> Option<String> {
    match ctx.sql(sql).await {
        Ok(df) => df.collect().await.err().map(|e| e.to_string()),
        Err(e) => Some(e.to_string()),
    }
}
