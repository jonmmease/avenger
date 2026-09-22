use crate::{
    baseline::wait_for_targets,
    dataflow::{Engine, Evaluation, Warming},
    replay,
    selection::Selections,
};
use anyhow::Result;
use arrow::{
    array::{Int32Array, StringArray, UInt16Array},
    record_batch::RecordBatch,
};
use avenger_datafusion_dataflow::{TableSnapshot, TableStore};
use std::{sync::Arc, time::Duration};

pub(crate) fn batch(start: usize, count: usize) -> RecordBatch {
    RecordBatch::try_new(
        replay::schema(),
        vec![
            Arc::new(Int32Array::from_iter_values(
                (start..start + count).map(|i| [-100, -20, 5, 15, 45, 90, 180, 250][i % 8]),
            )),
            Arc::new(Int32Array::from_iter_values(
                (start..start + count).map(|i| [100, 500, 900, 1200, 2200][i % 5]),
            )),
            Arc::new(UInt16Array::from_iter_values(
                (start..start + count).map(|i| [60, 240, 600, 1100][i % 4]),
            )),
            Arc::new(StringArray::from_iter_values(
                (start..start + count).map(|i| if i % 3 == 0 { "AA" } else { "BB" }),
            )),
        ],
    )
    .unwrap()
}

pub(crate) async fn ready(warming: &Warming) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20), wait_for_targets(warming)).await??;
    Ok(())
}

fn same(actual: &Evaluation, expected: &Evaluation) {
    assert_eq!(actual.snapshot.id(), expected.snapshot.id());
    assert_eq!(actual.bins, expected.bins);
    assert_eq!(actual.carriers.len(), expected.carriers.len());
    for (a, e) in actual.carriers.iter().zip(&expected.carriers) {
        assert_eq!((&a.name, a.count), (&e.name, e.count));
        for (a, e) in [(a.mean, e.mean), (a.stddev, e.stddev)] {
            match (a, e) {
                (Some(a), Some(e)) => {
                    assert!((a - e).abs() <= 1e-10 * (1. + e.abs()), "{a} != {e}")
                }
                (None, None) => (),
                other => panic!("different null results: {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn appends_and_current_brush_match_direct_queries_for_each_focus() -> Result<()> {
    let mut selections = Selections::new(505.)?;
    let engine = Engine::new(replay::schema(), &selections, false).await?;
    let store = TableStore::new(TableSnapshot::empty(replay::schema()));
    let first = store.append_batch(batch(0, 80))?;
    assert!(
        engine
            .read(&selections, 0, first.clone(), &[])
            .await?
            .is_none()
    );
    ready(&engine.warm(&selections, 0, first.clone())?).await?;

    let second = store.append_batch(batch(80, 37))?;
    selections.set(0, Some([-30., 95.]))?;
    let fallback = engine
        .read(&selections, 0, second.clone(), std::slice::from_ref(&first))
        .await?
        .unwrap();
    assert_eq!(fallback.snapshot.id(), first.id());
    assert!(fallback.executed.iter().all(|n| n.ends_with("_rollup")));
    same(&fallback, &engine.direct(&selections, first.clone()).await?);

    // Preserve the captured intermediate candidate while ingestion advances.
    let warming = engine.warm(&selections, 0, second.clone())?;
    let third = store.append_batch(batch(117, 41))?;
    ready(&warming).await?;
    let intermediate = engine
        .read(&selections, 0, third.clone(), &[second.clone(), first])
        .await?
        .unwrap();
    assert_eq!(intermediate.snapshot.id(), second.id());
    assert!(intermediate.executed.iter().all(|n| n.ends_with("_rollup")));
    same(&intermediate, &engine.direct(&selections, second).await?);

    for focus in 0..3 {
        // A changed nonfocused brush changes the materialization context.
        selections.set(
            (focus + 1) % 3,
            Some(match (focus + 1) % 3 {
                0 => [-30., 100.],
                1 => [3., 15.],
                _ => [300., 1500.],
            }),
        )?;
        engine.clear();
        assert!(
            engine
                .read(&selections, focus, third.clone(), &[])
                .await?
                .is_none()
        );
        ready(&engine.warm(&selections, focus, third.clone())?).await?;
        let actual = engine
            .read(&selections, focus, third.clone(), &[])
            .await?
            .unwrap();
        assert!(
            actual.executed.iter().all(|n| n.ends_with("_rollup")),
            "{:?}",
            actual.executed
        );
        same(&actual, &engine.direct(&selections, third.clone()).await?);
    }

    engine.clear();
    assert!(engine.read(&selections, 2, third, &[]).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn empty_and_fully_filtered_snapshots_preserve_empty_charts() -> Result<()> {
    let mut selections = Selections::new(505.)?;
    let engine = Engine::new(replay::schema(), &selections, false).await?;
    for snapshot in [
        TableSnapshot::empty(replay::schema()),
        TableSnapshot::from_batches(replay::schema(), vec![batch(0, 10)])?,
    ] {
        selections.set(0, Some([100., 110.]))?;
        selections.set(1, Some([22., 23.]))?;
        ready(&engine.warm(&selections, 0, snapshot.clone())?).await?;
        let actual = engine
            .read(&selections, 0, snapshot.clone(), &[])
            .await?
            .unwrap();
        same(&actual, &engine.direct(&selections, snapshot).await?);
        assert!(actual.carriers.is_empty());
    }
    Ok(())
}
