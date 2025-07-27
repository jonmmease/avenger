//! Test data generation utilities for visual tests

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a simple categorical bar chart dataset
pub fn simple_categories() -> DataFrame {
    let categories = StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    let values = Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(categories), Arc::new(values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}
