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
    ctx.read_batch(batch).expect("Failed to read batch into DataFrame")
}

/// Create a time series dataset
pub fn time_series() -> DataFrame {
    let dates = StringArray::from(vec![
        "2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04", "2024-01-05",
        "2024-01-06", "2024-01-07", "2024-01-08", "2024-01-09", "2024-01-10",
    ]);
    let values = Float64Array::from(vec![10.0, 15.0, 13.0, 17.0, 21.0, 18.0, 22.0, 25.0, 23.0, 28.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("date", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(dates), Arc::new(values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch).expect("Failed to read batch into DataFrame")
}

/// Create a grouped bar chart dataset
pub fn grouped_categories() -> DataFrame {
    let categories = StringArray::from(vec![
        "A", "A", "B", "B", "C", "C", "D", "D", "E", "E",
    ]);
    let groups = StringArray::from(vec![
        "Group1", "Group2", "Group1", "Group2", "Group1", 
        "Group2", "Group1", "Group2", "Group1", "Group2",
    ]);
    let values = Float64Array::from(vec![
        20.0, 35.0, 45.0, 30.0, 60.0, 40.0, 25.0, 50.0, 40.0, 55.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(groups), Arc::new(values)],
    ).expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch).expect("Failed to read batch into DataFrame")
}

/// Create a scatter plot dataset
pub fn scatter_data() -> DataFrame {
    let n = 50;
    let mut x_values = Vec::with_capacity(n);
    let mut y_values = Vec::with_capacity(n);
    let mut sizes = Vec::with_capacity(n);
    
    // Generate some interesting scatter data
    for i in 0..n {
        let x = i as f64 * 2.0;
        let y = 50.0 + x * 0.5 + ((i as f64) * 0.1).sin() * 20.0;
        let size = 100.0 + ((i as f64) * 0.2).cos() * 50.0;
        
        x_values.push(x);
        y_values.push(y);
        sizes.push(size);
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(x_values)),
            Arc::new(Float64Array::from(y_values)),
            Arc::new(Float64Array::from(sizes)),
        ],
    ).expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch).expect("Failed to read batch into DataFrame")
}