mod common;

use std::sync::{Arc, Barrier};

use avenger_datafusion_dataflow::{
    arrow::{
        datatypes::Schema,
        error::ArrowError,
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    datafusion::{
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, Expr, LogicalPlanBuilder},
    },
    DataflowBuilder, Error, Result, Runtime, TableSnapshot, TableStore,
};

fn batch(values: &[i64]) -> RecordBatch {
    common::snapshot(values).batches()[0].clone()
}

#[test]
fn appends_publish_ordered_snapshots_and_share_buffers() -> Result<()> {
    let original = common::snapshot(&[1]);
    let store = TableStore::new(original.clone());
    let added = batch(&[2, 3]);
    let first = store.clone().append_batch(added.clone())?;
    let second = store.append_batches(vec![batch(&[4]), batch(&[]), batch(&[5, 6])])?;

    assert_ne!(original.id(), first.id());
    assert_ne!(first.id(), second.id());
    assert_eq!(store.snapshot().id(), second.id());
    assert_eq!(common::values(&original), [1]);
    assert_eq!(common::values(&first), [1, 2, 3]);
    assert_eq!(common::values(&second), [1, 2, 3, 4, 5, 6]);
    assert_eq!(second.num_rows(), 6);
    assert_eq!(
        second
            .batch_iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [1, 2, 1, 2]
    );
    assert!(Arc::ptr_eq(
        second.batches()[0].column(0),
        original.batches()[0].column(0)
    ));
    assert!(Arc::ptr_eq(second.batches()[1].column(0), added.column(0)));
    assert_eq!(
        second.batch_iter().collect::<Vec<_>>(),
        second.batches().iter().collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn empty_appends_preserve_identity_and_invalid_batches_publish_nothing() -> Result<()> {
    let original = common::snapshot(&[1]);
    let store = TableStore::new(original.clone());
    assert_eq!(store.append_batches(vec![])?.id(), original.id());
    assert_eq!(store.append_batch(batch(&[]))?.id(), original.id());
    assert_eq!(
        store.append_batches(vec![batch(&[]), batch(&[])])?.id(),
        original.id()
    );
    assert_eq!(store.snapshot().batches().len(), 1);

    let wrong_schema = Arc::new(
        common::schema()
            .as_ref()
            .clone()
            .with_metadata([("version".into(), "other".into())].into_iter().collect()),
    );
    let invalid_empty = RecordBatch::new_empty(wrong_schema);
    assert!(matches!(
        store.append_batch(invalid_empty.clone()),
        Err(Error::SchemaMismatch(_))
    ));
    assert!(matches!(
        store.append_batches(vec![batch(&[2]), invalid_empty, batch(&[3])]),
        Err(Error::SchemaMismatch(_))
    ));
    assert_eq!(store.snapshot().id(), original.id());
    assert_eq!(common::values(&store.snapshot()), [1]);
    assert_eq!(common::values(&store.append_batch(batch(&[4]))?), [1, 4]);
    Ok(())
}

#[test]
fn replacement_and_independent_stores_keep_their_snapshot_semantics() -> Result<()> {
    let original = common::snapshot(&[1]);
    let store = TableStore::new(original.clone());
    let independent = TableStore::new(original.clone());
    let previous = store.append_batch(batch(&[2]))?;
    assert_eq!(independent.snapshot().id(), original.id());

    store.replace(original.clone())?;
    assert_eq!(store.snapshot().id(), original.id());
    let added = batch(&[3]);
    let next = store.append_batch(added.clone())?;
    let repeated = store.append_batch(added)?;
    assert_eq!(common::values(&previous), [1, 2]);
    assert_eq!(common::values(&next), [1, 3]);
    assert_eq!(common::values(&repeated), [1, 3, 3]);
    assert_eq!(
        common::values(&independent.append_batch(batch(&[4]))?),
        [1, 4]
    );
    assert_eq!(store.snapshot().id(), repeated.id());
    Ok(())
}

#[test]
fn row_count_overflow_returns_an_error_without_publishing() -> Result<()> {
    let schema = Arc::new(Schema::empty());
    let batch = |rows| {
        RecordBatch::try_new_with_options(
            schema.clone(),
            vec![],
            &RecordBatchOptions::new().with_row_count(Some(rows)),
        )
        .unwrap()
    };
    assert!(matches!(
        TableSnapshot::from_batches(schema.clone(), vec![batch(usize::MAX), batch(1)]),
        Err(Error::Arrow(ArrowError::InvalidArgumentError(_)))
    ));
    let original = TableSnapshot::from_batches(schema.clone(), vec![batch(usize::MAX)])?;
    let store = TableStore::new(original.clone());
    assert!(matches!(
        store.append_batch(batch(1)),
        Err(Error::Arrow(ArrowError::InvalidArgumentError(_)))
    ));
    assert_eq!(store.snapshot().id(), original.id());
    assert_eq!(store.snapshot().num_rows(), usize::MAX);
    assert_eq!(store.append_batch(batch(0))?.id(), original.id());
    Ok(())
}

#[test]
fn concurrent_appends_return_their_own_complete_commit() {
    const WRITERS: usize = 16;
    let store = TableStore::new(TableSnapshot::empty(common::schema()));
    let barrier = Barrier::new(WRITERS);
    let mut committed = std::thread::scope(|scope| {
        let tasks = (0..WRITERS)
            .map(|writer| {
                let store = store.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    let value = (writer * 2) as i64;
                    let batches = vec![batch(&[value]), batch(&[value + 1])];
                    barrier.wait();
                    let snapshot = store.append_batches(batches).unwrap();
                    let rows = common::values(&snapshot);
                    assert_eq!(&rows[rows.len() - 2..], &[value, value + 1]);
                    snapshot
                })
            })
            .collect::<Vec<_>>();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    committed.sort_by_key(TableSnapshot::num_rows);
    let final_snapshot = store.snapshot();
    let final_rows = common::values(&final_snapshot);
    for (index, snapshot) in committed.iter().enumerate() {
        let rows = (index + 1) * 2;
        assert_eq!(snapshot.num_rows(), rows);
        assert_eq!(common::values(snapshot), final_rows[..rows]);
    }
    assert_eq!(committed.last().unwrap().id(), final_snapshot.id());
    let mut sorted = final_rows;
    sorted.sort_unstable();
    assert_eq!(sorted, (0..(WRITERS * 2) as i64).collect::<Vec<_>>());
}

#[tokio::test]
async fn existing_queries_return_complete_results_for_each_captured_snapshot() -> Result<()> {
    let mut builder = DataflowBuilder::new();
    let input = builder.table_input("source", common::schema())?;
    let total = builder.add_plan(
        "total",
        LogicalPlanBuilder::from(input.plan_ref())
            .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("total")])?
            .build()?,
    )?;
    let output = builder.table_output("total", &total)?;
    let runtime = Runtime::new(Default::default())?;
    let flow = runtime.prepare(&builder.finish()?).await?;
    let store = TableStore::new(TableSnapshot::empty(common::schema()));
    let first_snapshot = store.append_batch(batch(&[1, 2]))?;
    let first_inputs = flow.inputs().table(&input, first_snapshot)?.finish()?;
    let first = flow.query(&[output], &[], &first_inputs).await?;
    let second_snapshot = store.append_batch(batch(&[3, 4]))?;
    let second_inputs = first_inputs
        .edit()
        .table(&input, second_snapshot)?
        .finish()?;
    let second = flow.query(&[output], &[], &second_inputs).await?;
    assert_eq!(common::values(first.table(&output)?), [3]);
    assert_eq!(common::values(second.table(&output)?), [10]);
    flow.clear_results();
    let reread = flow.query(&[output], &[], &first_inputs).await?;
    assert_eq!(common::values(reread.table(&output)?), [3]);
    assert_eq!(common::values(&store.snapshot()), [1, 2, 3, 4]);
    Ok(())
}
