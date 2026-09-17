#![allow(dead_code)]
pub mod source;
use std::sync::Arc;

use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    TableSnapshot,
};

pub fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]))
}
pub fn snapshot(values: &[i64]) -> TableSnapshot {
    TableSnapshot::from_batches(
        schema(),
        vec![
            RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(values.to_vec()))])
                .unwrap(),
        ],
    )
    .unwrap()
}
pub fn values(table: &TableSnapshot) -> Vec<i64> {
    table
        .batches()
        .iter()
        .flat_map(|batch| {
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}
