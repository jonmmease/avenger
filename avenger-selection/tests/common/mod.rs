#![allow(dead_code)]
use avenger_selection::*;
use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    logical_expr::{col, Expr},
    prelude::SessionContext,
};
use std::{ops::Bound, sync::Arc};

pub fn id() -> SelectionId {
    SelectionId::new("filters").unwrap()
}
pub fn view(name: &str) -> ViewAddress {
    ViewAddress::root(ViewId::new(name).unwrap())
}
pub fn address(name: &str, origin: ViewAddress) -> ProducerAddress {
    ProducerAddress {
        selection: id(),
        producer: ProducerId::new(name).unwrap(),
        origin,
    }
}
pub fn producer(
    name: &str,
    origin: ViewAddress,
    kind: SelectionKind,
    fields: &[&str],
) -> ProducerDefinition {
    ProducerDefinition::new(
        address(name, origin),
        kind,
        fields
            .iter()
            .map(|f| Projection::new(ProjectionId::new(*f).unwrap(), col(*f)).unwrap())
            .collect(),
    )
    .unwrap()
}
pub fn point(name: &str, field: &str) -> ProducerDefinition {
    producer(name, view(name), SelectionKind::Point, &[field])
}
pub fn interval(name: &str, field: &str) -> ProducerDefinition {
    producer(name, view(name), SelectionKind::Interval, &[field])
}
pub fn term(field: &str, test: ValueTest) -> SelectionTerm {
    SelectionTerm {
        projection: ProjectionId::new(field).unwrap(),
        test,
    }
}
pub fn tuple(field: &str, value: impl Into<ScalarValue>) -> SelectionTuple {
    SelectionTuple {
        terms: vec![term(field, ValueTest::Equal(value.into()))],
    }
}
pub fn values(field: &str, values: impl IntoIterator<Item = ScalarValue>) -> SelectionValue {
    SelectionValue::Tuples(values.into_iter().map(|v| tuple(field, v)).collect())
}
pub fn range(field: &str, lower: Bound<ScalarValue>, upper: Bound<ScalarValue>) -> SelectionValue {
    SelectionValue::Tuples(vec![SelectionTuple {
        terms: vec![term(field, ValueTest::Range { lower, upper })],
    }])
}
pub fn between(field: &str, lower: i64, upper: i64) -> SelectionValue {
    range(
        field,
        Bound::Included(lower.into()),
        Bound::Excluded(upper.into()),
    )
}
pub fn state(resolution: Resolution) -> SelectionSet {
    SelectionSet::new([SelectionSnapshot::new(SelectionDefinition::new(id(), resolution)).unwrap()])
        .unwrap()
}
pub fn filter(consumer: SelectionConsumer, filter: SelectionFilter) -> ConsumerFilter {
    SelectionCompiler::new().filter(&consumer, filter).unwrap()
}
pub fn membership() -> ConsumerFilter {
    filter(
        SelectionConsumer::new(view("summary")),
        SelectionFilter::membership(&id(), EmptySelection::MatchAll),
    )
}
pub fn cross(origin: ViewAddress) -> ConsumerFilter {
    filter(
        SelectionConsumer::new(origin),
        SelectionFilter::cross_filter([&id()]),
    )
}
pub fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, col)| Field::new(*name, col.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(schema, columns.into_iter().map(|(_, col)| col).collect()).unwrap()
}
pub fn flights() -> RecordBatch {
    batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4, 5, 6]))),
        (
            "delay",
            Arc::new(Int64Array::from(vec![0, 10, 20, 30, 20, 20, 15])),
        ),
        (
            "distance",
            Arc::new(Int64Array::from(vec![600, 600, 1000, 600, 2000, 600, 800])),
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec![
                Some("AA"),
                Some("AA"),
                Some("DL"),
                Some("AA"),
                Some("AA"),
                Some("UA"),
                None,
            ])),
        ),
        (
            "region",
            Arc::new(StringArray::from(vec![
                "East", "East", "West", "East", "West", "West", "East",
            ])),
        ),
    ])
}
pub async fn selected(batch: RecordBatch, expr: Expr) -> Vec<i64> {
    let rows = SessionContext::new()
        .read_batch(batch)
        .unwrap()
        .filter(expr)
        .unwrap()
        .collect()
        .await
        .unwrap();
    ids(&rows)
}
pub fn ids(batches: &[RecordBatch]) -> Vec<i64> {
    let mut ids: Vec<_> = batches
        .iter()
        .flat_map(|b| {
            b.column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect();
    ids.sort();
    ids
}
