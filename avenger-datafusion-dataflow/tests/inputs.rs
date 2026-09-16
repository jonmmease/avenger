mod common;
use avenger_datafusion_dataflow::{
    arrow::{
        array::Int32Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datafusion::common::ScalarValue,
    Error, GraphBuilder, Runtime, RuntimeConfig, TableSnapshot, TableStore,
};
use std::sync::Arc;

#[test]
fn snapshots_have_allocation_identity_and_preserve_empty_schemas() {
    let first = common::snapshot(&[1, 2]);
    assert_eq!(first.id(), first.clone().id());
    assert_ne!(first.id(), common::snapshot(&[1, 2]).id());
    let empty = TableSnapshot::empty(common::schema());
    assert_eq!(empty.num_rows(), 0);
    assert_eq!(empty.schema(), &common::schema());
    assert!(empty.batches().is_empty());
    let wrong =
        RecordBatch::try_from_iter(vec![("value", Arc::new(Int32Array::from(vec![1])) as _)])
            .unwrap();
    assert!(matches!(
        TableSnapshot::from_batches(common::schema(), vec![wrong]),
        Err(Error::SchemaMismatch(_))
    ));
}

#[test]
fn stores_publish_data_and_identity_together_and_allow_republishing() {
    let original = common::snapshot(&[1]);
    let store = TableStore::new(original.clone());
    let captured = store.snapshot();
    let replacement = common::snapshot(&[2, 3]);
    store.clone().replace(replacement.clone()).unwrap();
    assert_eq!(store.snapshot().id(), replacement.id());
    assert_eq!(common::values(&captured), vec![1]);
    let wrong = TableSnapshot::empty(Arc::new(Schema::new(vec![Field::new(
        "wrong",
        DataType::Int64,
        false,
    )])));
    assert!(matches!(
        store.replace(wrong),
        Err(Error::SchemaMismatch(_))
    ));
    assert_eq!(store.snapshot().id(), replacement.id());
    store.replace(original.clone()).unwrap();
    assert_eq!(store.snapshot().id(), original.id());
}

#[tokio::test]
async fn bindings_validate_completeness_types_ownership_and_keep_old_values() {
    let mut graph = GraphBuilder::new();
    let table = graph.table_input("table", common::schema()).unwrap();
    let scalar = graph.scalar_input("scalar", DataType::Int64).unwrap();
    let plan = graph.add_plan("rows", table.plan_ref()).unwrap();
    let expression = graph.add_expr("parameter", scalar.expr_ref()).unwrap();
    let rows = graph.table_output("rows", &plan).unwrap();
    let value = graph.scalar_output("value", &expression).unwrap();
    let runtime = Runtime::new(RuntimeConfig::default()).unwrap();
    let prepared = runtime.prepare(&graph.finish().unwrap()).await.unwrap();
    assert!(matches!(
        prepared.inputs().finish(),
        Err(Error::MissingInput(_))
    ));
    assert!(matches!(
        prepared
            .inputs()
            .scalar(&scalar, ScalarValue::Float64(Some(1.0))),
        Err(Error::ScalarTypeMismatch { .. })
    ));
    let mut other = GraphBuilder::new();
    let other_scalar = other.scalar_input("scalar", DataType::Int64).unwrap();
    assert!(matches!(
        prepared
            .inputs()
            .scalar(&other_scalar, ScalarValue::Int64(None)),
        Err(Error::ForeignHandle)
    ));
    let store = TableStore::new(common::snapshot(&[1, 2]));
    let inputs = prepared
        .inputs()
        .table(&table, store.snapshot())
        .unwrap()
        .scalar(&scalar, ScalarValue::Int64(None))
        .unwrap()
        .finish()
        .unwrap();
    store.replace(common::snapshot(&[3])).unwrap();
    let changed = inputs
        .edit()
        .table(&table, store.snapshot())
        .unwrap()
        .scalar(&scalar, ScalarValue::Int64(Some(42)))
        .unwrap()
        .finish()
        .unwrap();
    let old = prepared.query(&[rows], &[value], &inputs).await.unwrap();
    let new = prepared.query(&[rows], &[value], &changed).await.unwrap();
    assert_eq!(common::values(old.table(&rows).unwrap()), vec![1, 2]);
    assert_eq!(old.scalar(&value).unwrap(), &ScalarValue::Int64(None));
    assert_eq!(common::values(new.table(&rows).unwrap()), vec![3]);
    assert_eq!(new.scalar(&value).unwrap(), &ScalarValue::Int64(Some(42)));
}
