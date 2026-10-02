use crate::{
    config::Config,
    dataflow::{Engine, Warming},
    replay::{self, Cursor, Replay},
    selection::Selections,
};
use anyhow::{Context, Result, ensure};
use avenger_datafusion_dataflow::{TableSnapshot, TableStore};
use serde_json::json;
use std::time::Duration;

pub async fn wait_for_targets(warming: &Warming) -> Result<usize> {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(rows) = warming.probe().await? {
                return Ok(rows);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .context(
        "targets did not become cached within 120 seconds; check warming errors and cache capacity",
    )?
}

async fn brushes(
    engine: &Engine,
    snapshot: &TableSnapshot,
    fallback: Option<&TableSnapshot>,
    seed: usize,
) -> Result<Vec<f64>> {
    let mut selections = Selections::new(505.)?;
    let mut times = vec![];
    for i in 0..10 {
        let offset = ((seed * 11 + i) % 100) as f64;
        selections.set(0, Some([-50. + offset, 75. + offset]))?;
        let result = engine
            .read(
                &selections,
                0,
                snapshot.clone(),
                &fallback.into_iter().cloned().collect::<Vec<_>>(),
            )
            .await?
            .context("brush requires cached targets")?;
        ensure!(
            result.executed.iter().all(|name| name.ends_with("_rollup")),
            "raw foreground work: {:?}",
            result.executed
        );
        times.push(result.elapsed_ms);
    }
    Ok(times)
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(
        sorted[((sorted.len() as f64 * percentile).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1)],
    )
}

pub async fn run(config: Config) -> Result<()> {
    let replay = Replay::open(&config.data, config.batch_rows)?;
    let selections = Selections::new(505.)?;
    let engine = Engine::new(replay::schema(), &selections, config.diagnostics).await?;
    let store = TableStore::new(TableSnapshot::empty(replay::schema()));
    let mut cursor = Cursor::default();
    let mut fallback = None;
    loop {
        if config
            .max_batches
            .is_some_and(|limit| cursor.batches >= limit)
        {
            break;
        }
        let Some(appended) = replay.clone().append(cursor, store.clone()).await? else {
            break;
        };
        cursor = appended.next;
        let snapshot = appended.snapshot;
        let warming = engine.warm(&selections, 0, snapshot.clone())?;
        let observe = async {
            let rows = wait_for_targets(&warming).await?;
            Ok::<_, anyhow::Error>((rows, warming.elapsed_ms()))
        };
        let interact = async {
            if let Some(fallback) = &fallback {
                brushes(&engine, &snapshot, Some(fallback), cursor.batches).await
            } else {
                Ok(vec![])
            }
        };
        let ((target_rows, availability_ms), during) = tokio::try_join!(observe, interact)?;
        let after = brushes(&engine, &snapshot, None, cursor.batches + 50).await?;
        println!(
            "{}",
            json!({
                "batch": cursor.batches, "period": appended.period,
                "input_rows": snapshot.num_rows(), "input_batches": snapshot.batch_iter().count(),
                "target_partitions": 4,
                "append_ms": appended.append_ms, "targets_available_ms": availability_ms,
                "cached_target_rows": target_rows,
                "during_warming_brush_ms": during,
                "during_warming_p95_ms": percentile(&during, 0.95),
                "after_warming_brush_ms": after,
                "after_warming_p50_ms": percentile(&after, 0.5),
                "after_warming_p95_ms": percentile(&after, 0.95),
            })
        );
        fallback = Some(snapshot);
    }
    Ok(())
}
