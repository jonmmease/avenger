//! Streaming against batch: each query of a corpus, over seeded sessions of pointer events split
//! into frames at random, has the batch plan's result over the same events after every frame.

mod common;

use std::collections::HashMap;

use avenger_streaming::StreamingQuery;
use common::{Random, batches, context, events_table, plan, rows, session, split_of};
use datafusion::{
    arrow::{array::RecordBatch, compute::concat_batches},
    common::{ParamValues, ScalarValue},
    prelude::SessionContext,
};

/// The queries of the fronts: ungrouped and grouped aggregates, and `ORDER BY ... LIMIT`.
const QUERIES: &[&str] = &[
    // Ungrouped aggregates over every event, with NULL, NaN and the infinities.
    "FROM events |> AGGREGATE COUNT(*) AS n, COUNT(x) AS with_x, SUM(x) AS total, MIN(x) AS lo, MAX(x) AS hi, AVG(y) AS mean, MAX(ts) AS last, LOGICAL_OR(x IS NULL) AS any_null",
    // Ungrouped aggregates of a stage's rows, in their order, with DISTINCT and LIMIT.
    "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS clicks, ARRAY_AGG(target ORDER BY seq) AS targets, STRING_AGG(target, ',' ORDER BY seq DESC LIMIT 3) AS recent, COUNT(DISTINCT target) AS distinct_targets",
    // Grouped aggregates, with FILTER and a computed group. DataFusion's grouped MIN and MAX
    // over NaN differ from its ungrouped ones, which the streaming front keeps, so the grouped
    // extremes read finite values; the ungrouped query above reads NaN.
    "FROM events |> AGGREGATE COUNT(*) AS n, COUNTIF(x > 400) AS right_side, MIN(seq) AS first_seq, MAX(IF(y > -1e300 AND y < 1e300, y, NULL)) AS low_point GROUP BY type, inside",
    "FROM events |> WHERE x IS NOT NULL |> AGGREGATE COUNT(*) AS n, ARRAY_AGG(seq ORDER BY seq DESC LIMIT 2) AS last_two GROUP BY MOD(seq, 7) AS bucket",
    // Hot targets: a grouped front, then a filter and an ungrouped aggregate downstream.
    "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n GROUP BY target |> WHERE n >= 3 |> AGGREGATE COUNT(*) AS hot_targets",
    // A table function downstream of a grouped front.
    "FROM events |> AGGREGATE COUNT(*) AS n GROUP BY type, inside |> CALL impute(type, [inside], n, value => 0)",
    // The last click, and the last pointer position.
    "FROM events |> WHERE type = 'click' |> ORDER BY seq DESC |> LIMIT 1 |> SELECT target, x, y",
    "FROM events |> WHERE type = 'pointermove' |> ORDER BY seq DESC |> LIMIT 1 |> SELECT x, y, ts",
    // The three rightmost positions, ties by seq, with NaN, the infinities and zeros of either sign.
    "FROM events |> ORDER BY x DESC, seq DESC |> LIMIT 3 |> SELECT seq, x",
    // An offset, and a stage that computes the order's first key.
    "FROM events |> EXTEND ABS(y - 300) AS distance |> ORDER BY distance, seq |> LIMIT 2 OFFSET 1 |> SELECT seq, distance",
];

/// The events of `frames` in frames of 1 to 4 frames each, at random.
fn regrouped(frames: &[RecordBatch], random: &mut Random) -> Vec<RecordBatch> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < frames.len() {
        let take = (1 + random.below(4) as usize).min(frames.len() - at);
        out.push(concat_batches(&common::events_schema(), &frames[at..at + take]).unwrap());
        at += take;
    }
    out
}

async fn batch_rows(
    ctx: &SessionContext,
    query: &str,
    events: &[RecordBatch],
    params: Option<&ParamValues>,
) -> Vec<String> {
    let table = events_table();
    *table.batches[0].write().await = events.to_vec();
    let mut plan = plan(query, table);
    if let Some(params) = params {
        plan = plan.with_param_values(params.clone()).unwrap();
    }
    rows(
        &ctx.execute_logical_plan(plan)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_queries_have_the_batch_result_after_every_frame() {
    let ctx = context();
    let mut checked = 0;
    for (q, query) in QUERIES.iter().enumerate() {
        for seed in [11u64, 23, 37] {
            let frames = batches(&session(300, seed * 1_000 + q as u64, true));
            let mut random = Random(seed + 7);
            let frames = regrouped(&frames, &mut random);
            let split = split_of(query);
            let mut streaming = StreamingQuery::new(ctx.clone(), split, None).await.unwrap();
            assert_eq!(
                rows(streaming.result()),
                batch_rows(&ctx, query, &[], None).await,
                "{query} over no events"
            );
            let mut seen = Vec::new();
            for frame in frames {
                if frame.num_rows() == 0 {
                    continue;
                }
                seen.push(frame.clone());
                streaming.push(frame).await.unwrap();
                let want = batch_rows(&ctx, query, &seen, None).await;
                assert_eq!(
                    rows(streaming.result()),
                    want,
                    "{query}, seed {seed}, after {} frames",
                    seen.len()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1_000, "{checked} frames");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_parameter_rebuilds_the_front_or_runs_the_downstream_again() {
    let ctx = context();
    let params = |name: &str, value: ScalarValue| {
        ParamValues::Map(HashMap::from([(name.to_string(), value.into())]))
    };
    let cases = [
        // The stages read the parameter, so its change replays every event.
        (
            "FROM events |> WHERE x > @min_x |> AGGREGATE COUNT(*) AS n, MAX(x) AS hi",
            params("min_x", ScalarValue::Float64(Some(380.0))),
            params("min_x", ScalarValue::Float64(Some(420.0))),
        ),
        // Only the downstream reads the parameter.
        (
            "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n GROUP BY target |> WHERE n >= @k |> AGGREGATE COUNT(*) AS hot_targets, SUM(n) AS hot_clicks",
            params("k", ScalarValue::Int64(Some(1))),
            params("k", ScalarValue::Int64(Some(2))),
        ),
    ];
    for (query, first, second) in cases {
        let frames: Vec<RecordBatch> = batches(&session(2_000, 5, false))
            .into_iter()
            .filter(|f| f.num_rows() > 0)
            .collect();
        // The parameter changes halfway, after the front has state.
        let change = frames.len() / 2;
        let split = split_of(query);
        let mut streaming = StreamingQuery::new(ctx.clone(), split, Some(first.clone()))
            .await
            .unwrap();
        let mut seen = Vec::new();
        let mut results = std::collections::HashSet::new();
        for (i, frame) in frames.into_iter().enumerate() {
            seen.push(frame.clone());
            streaming.push(frame).await.unwrap();
            if i == change {
                streaming.set_params(second.clone()).await.unwrap();
            }
            let current = if i >= change { &second } else { &first };
            let want = batch_rows(&ctx, query, &seen, Some(current)).await;
            assert_eq!(
                rows(streaming.result()),
                want,
                "{query} after {} frames",
                i + 1
            );
            results.insert(want);
        }
        assert!(
            results.len() > 5,
            "{query}: the results vary, {}",
            results.len()
        );
    }
}
