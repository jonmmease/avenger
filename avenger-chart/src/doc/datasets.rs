use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        record_batch::RecordBatch,
    },
    prelude::{DataFrame, SessionContext},
};

/// Simple scatter data spanning four quadrants.
pub fn scatter_quadrants(ctx: &SessionContext) -> DataFrame {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "x",
            Arc::new(Float64Array::from(vec![-3.0, -1.0, 0.5, 1.2, 2.5, 3.0])) as ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from(vec![2.0, -2.5, 1.5, -1.0, 2.8, -3.5])) as ArrayRef,
        ),
    ])
    .expect("failed to build scatter_quadrants batch");

    ctx.read_batch(batch)
        .expect("failed to register scatter_quadrants batch")
}

/// Five categories with values – handy for bar-chart demos.
pub fn categorical_bars(ctx: &SessionContext) -> DataFrame {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "category",
            Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"])) as ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0])) as ArrayRef,
        ),
    ])
    .expect("failed to build categorical_bars batch");

    ctx.read_batch(batch)
        .expect("failed to register categorical_bars batch")
}
