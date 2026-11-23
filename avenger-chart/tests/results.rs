//! The results of queries that hold charts: their shapes, their chart columns, the rows of their
//! batches, and the text of their values.

mod support;

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, BinaryViewArray, Int64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{
    chart_field, charts_to_arrow,
    results::{is_chart_field, QueryResult, Shape},
    Chart,
};
use support::*;

fn scatter() -> Chart {
    Chart::from_parts(chart(plot(vec![symbols(0)])), vec![points()], vec![]).unwrap()
}

/// A result of the columns `columns`, each a field and its values, in one batch.
fn result(columns: Vec<(Field, ArrayRef)>) -> QueryResult {
    let (fields, arrays): (Vec<Field>, Vec<ArrayRef>) = columns.into_iter().unzip();
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    QueryResult::new(schema, vec![batch]).unwrap()
}

fn charts(charts: &[Option<&Chart>]) -> (Field, ArrayRef) {
    let column = charts_to_arrow(charts.iter().copied()).unwrap();
    (chart_field("chart", true), Arc::new(column))
}

fn names(names: &[&str]) -> (Field, ArrayRef) {
    let column = StringArray::from(names.to_vec());
    (Field::new("name", DataType::Utf8, false), Arc::new(column))
}

#[test]
fn each_result_has_its_shape() {
    let chart = scatter();
    // One row of one column of charts is a chart.
    let single = result(vec![charts(&[Some(&chart)])]);
    assert_eq!(single.shape(), Shape::Chart);
    assert_eq!(single.chart(0, 0), Some(&chart));
    assert!(is_chart_field(&single.schema().fields()[0]));

    // A chart for each group, beside a column of names, is a table.
    let groups = result(vec![
        names(&["Adelie", "Chinstrap", "Gentoo"]),
        charts(&[Some(&chart), Some(&chart), Some(&chart)]),
    ]);
    assert_eq!(groups.shape(), Shape::Table);
    assert_eq!(groups.num_rows(), 3);
    assert_eq!(groups.chart_columns(), [1]);
    assert!((0..3).all(|row| groups.chart(row, 1).is_some()));

    // A null chart alone is a table, and so is a result without charts.
    let null = result(vec![charts(&[None])]);
    assert_eq!(null.shape(), Shape::Table);
    let rows = result(vec![names(&["a", "b"])]);
    assert_eq!(rows.shape(), Shape::Table);
    assert!(rows.chart_columns().is_empty());

    // A result without rows keeps its columns.
    let schema = Arc::new(Schema::new(vec![chart_field("chart", true)]));
    let empty = QueryResult::new(schema, Vec::new()).unwrap();
    assert_eq!((empty.num_rows(), empty.shape()), (0, Shape::Table));
    assert_eq!(empty.chart_columns(), [0]);
}

#[test]
fn bytes_that_are_not_a_chart_fail() {
    let column = BinaryViewArray::from(vec![Some(b"not a chart".as_slice())]);
    let schema = Arc::new(Schema::new(vec![chart_field("chart", true)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(column)]).unwrap();
    assert!(QueryResult::new(schema, vec![batch]).is_err());
}

/// A row is found in its batch across batches of different lengths.
#[test]
fn a_row_is_found_in_its_batch() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let batch = |values: Vec<i64>| {
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap()
    };
    let result = QueryResult::new(
        schema.clone(),
        vec![
            batch(vec![0, 1]),
            batch(vec![]),
            batch(vec![2, 3, 4]),
            batch(vec![5]),
        ],
    )
    .unwrap();
    assert_eq!(result.num_rows(), 6);
    for row in 0..6 {
        let (batch, offset) = result.locate(row).unwrap();
        let values = (batch.column(0).as_any())
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(values.value(offset), row as i64);
    }
    assert!(result.locate(6).is_none());
}

/// The text of a value, and `None` for a null or past the last row.
#[test]
fn a_value_has_its_text() {
    let column = StringArray::from(vec![Some("Adelie"), None]);
    let result = result(vec![(
        Field::new("sex", DataType::Utf8, true),
        Arc::new(column) as ArrayRef,
    )]);
    assert_eq!(result.value_text(0, 0).as_deref(), Some("Adelie"));
    assert_eq!(result.value_text(1, 0), None);
    assert_eq!(result.value_text(2, 0), None);
}
