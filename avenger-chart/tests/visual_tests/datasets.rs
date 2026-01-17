// Test data generation utilities for visual tests

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a dataset for testing categorical scale sharing in nested facets
///
/// This dataset has:
/// - 2 groups (outer facet dimension)
/// - Different categories present in each group (inner facet tests sharing)
/// - Categories A, B, C in Group1; categories B, C, D in Group2
/// - This tests that shared categorical scales show all categories (A, B, C, D) in both facets
pub fn categorical_sharing_test_data() -> DataFrame {
    // Group1 has categories A, B, C
    // Group2 has categories B, C, D
    // With sharing, both should show A, B, C, D on the axis
    let groups = StringArray::from(vec![
        "Group1", "Group1", "Group1", // A, B, C
        "Group2", "Group2", "Group2", // B, C, D
    ]);
    let categories = StringArray::from(vec![
        "A", "B", "C", // Group1
        "B", "C", "D", // Group2
    ]);
    let values = Float64Array::from(vec![
        10.0, 20.0, 30.0, // Group1
        25.0, 35.0, 45.0, // Group2
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(groups), Arc::new(categories), Arc::new(values)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

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
