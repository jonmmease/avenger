//! Streaming against batch: each query of a corpus, over seeded sessions of pointer events split
//! into frames at random, has the batch plan's result over the same events after every frame.

mod common;

use std::collections::{HashMap, HashSet};

use avenger_streaming::{EventQueries, Mode};
use common::{Random, batch_rows, batches, context, log, plan, rows, session};
use datafusion::{
    arrow::{array::RecordBatch, compute::concat_batches},
    common::{ParamValues, ScalarValue},
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
    // MATCH_RECOGNIZE: a drag brush, the latest drag's extent.
    "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.x) AS x0, LAST(mv.x) AS x1, COUNT(mv.x) AS moves, MAX(seq) AS last_seq PATTERN (dn mv* up?) DEFINE dn AS type = 'pointerdown' AND inside, mv AS type = 'pointermove', up AS type = 'pointerup') |> WHERE moves > 0 |> ORDER BY last_seq DESC |> LIMIT 1 |> SELECT LEAST(x0, x1) AS lo, GREATEST(x0, x1) AS hi",
    // A brush that a double click clears.
    "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup', 'dblclick') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.x) AS x0, LAST(mv.x) AS x1, COUNT(mv.x) AS moves, COUNT(clr.seq) > 0 AS cleared, MAX(seq) AS last_seq PATTERN (dn mv* up? | clr) DEFINE dn AS type = 'pointerdown' AND inside, mv AS type = 'pointermove', up AS type = 'pointerup', clr AS type = 'dblclick' AND inside) |> WHERE moves > 0 OR cleared |> ORDER BY last_seq DESC |> LIMIT 1 |> WHERE NOT cleared |> SELECT LEAST(x0, x1) AS lo, GREATEST(x0, x1) AS hi",
    // The total time of the drags, the one in progress too.
    "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.ts) AS t0, LAST(mv.ts) AS t1 PATTERN (dn mv+) DEFINE dn AS type = 'pointerdown', mv AS type = 'pointermove') |> AGGREGATE SUM(TIMESTAMP_DIFF(t1, t0, MILLISECOND)) AS total_drag_ms, COUNT(*) AS drags",
    // Hot targets through MATCH_RECOGNIZE.
    "FROM events |> WHERE type = 'click' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES ANY_VALUE(c.target) AS target PATTERN (c) DEFINE c AS TRUE) |> AGGREGATE COUNT(*) AS n GROUP BY target |> WHERE n >= 3 |> AGGREGATE COUNT(*) AS hot_targets",
    // Pauses before a move, with NEXT, whose last row's condition changes as events arrive.
    "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES MIN(p.seq) AS pause_seq PATTERN (p) DEFINE p AS TIMESTAMP_DIFF(NEXT(ts), ts, MILLISECOND) > 400 OR NEXT(ts) IS NULL) |> AGGREGATE COUNT(*) AS pauses, MAX(pause_seq) AS last_pause",
    // Flicks, with PREV at two offsets, and in partitions.
    "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS moves PATTERN (f{3,}) DEFINE f AS ABS(x - PREV(x)) > 4 AND ABS(x - PREV(x, 2)) > 6) |> AGGREGATE COUNT(*) AS flicks, MAX(moves) AS longest",
    "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(PARTITION BY inside ORDER BY seq MEASURES COUNT(*) AS moves, MAX(MATCH_ROW_NUMBER()) AS last_row PATTERN (f{2,}) DEFINE f AS ABS(x - PREV(x)) > 3) |> AGGREGATE COUNT(*) AS flicks, MAX(moves) AS longest, SUM(last_row) AS row_total GROUP BY inside",
    // Double clicks in partitions of their target.
    "FROM events |> WHERE type IN ('click', 'dblclick') |> MATCH_RECOGNIZE(PARTITION BY target ORDER BY seq MEASURES COUNT(*) AS n, MATCH_NUMBER() AS m PATTERN (c c? d) DEFINE c AS type = 'click', d AS type = 'dblclick') |> AGGREGATE COUNT(*) AS doubles, SUM(n) AS clicks, MAX(m) AS last_match GROUP BY target",
    // The end of the partition, empty matches, and overlapping matches.
    "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS n, MATCH_NUMBER() AS m AFTER MATCH SKIP TO NEXT ROW PATTERN (f{2,} $ | g*) DEFINE f AS x > PREV(x), g AS x < PREV(x) - 3) |> AGGREGATE COUNT(*) AS matches, MAX(n) AS longest, SUM(n) AS matched_rows, MAX(m) AS last_match",
    // The longest match, over a reluctant quantifier.
    "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS n, STRING_AGG(CLASSIFIER(), '') AS path PATTERN (dn mv+? up?) DEFINE dn AS type = 'pointerdown', mv AS type = 'pointermove', up AS type = 'pointerup' OPTIONS (use_longest_match = TRUE)) |> AGGREGATE COUNT(*) AS drags, MAX(n) AS longest, MIN(LENGTH(path)) AS shortest_path",
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

#[tokio::test(flavor = "multi_thread")]
async fn streaming_queries_have_the_batch_result_after_every_frame() {
    let ctx = context();
    let mut checked = 0;
    for (q, query) in QUERIES.iter().enumerate() {
        for seed in [11u64, 23, 37] {
            let frames = batches(&session(300, seed * 1_000 + q as u64, true));
            let mut random = Random(seed + 7);
            let frames = regrouped(&frames, &mut random);
            let log = log();
            let plan = plan(query, &log);
            let mut queries = EventQueries::new(ctx.clone(), log);
            let mode = queries.register("q", plan.clone()).await.unwrap();
            assert_eq!(mode, Mode::Streaming, "{query}");
            let mut previous = batch_rows(&ctx, &plan, None).await;
            assert_eq!(
                rows(queries.result("q").unwrap()),
                previous,
                "{query} over no events"
            );
            for frame in frames {
                if frame.num_rows() == 0 {
                    continue;
                }
                let changed = queries.push(frame).await.unwrap();
                let want = batch_rows(&ctx, &plan, None).await;
                assert_eq!(
                    rows(queries.result("q").unwrap()),
                    want,
                    "{query}, seed {seed}, at {} events",
                    queries.log().len()
                );
                // A change of the result is reported.
                if want != previous {
                    assert_eq!(changed, ["q"], "{query}: the change is reported");
                }
                previous = want;
                checked += 1;
            }
        }
    }
    assert!(checked > 1_000, "{checked} frames");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_parameter_rebuilds_the_front_or_runs_the_downstream_again() {
    let ctx = context();
    let value = |name: &str, value: ScalarValue| (name.to_string(), value);
    let cases = [
        // The stages read the parameter, so its change replays every event.
        (
            "FROM events |> WHERE x > @min_x |> AGGREGATE COUNT(*) AS n, MAX(x) AS hi",
            value("min_x", ScalarValue::Float64(Some(380.0))),
            value("min_x", ScalarValue::Float64(Some(420.0))),
        ),
        // Only the downstream reads the parameter.
        (
            "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n GROUP BY target |> WHERE n >= @k |> AGGREGATE COUNT(*) AS hot_targets, SUM(n) AS hot_clicks",
            value("k", ScalarValue::Int64(Some(1))),
            value("k", ScalarValue::Int64(Some(2))),
        ),
    ];
    let params = |(name, value): &(String, ScalarValue)| {
        Some(ParamValues::Map(HashMap::from([(
            name.clone(),
            value.clone().into(),
        )])))
    };
    for (query, first, second) in cases {
        let frames: Vec<RecordBatch> = batches(&session(2_000, 5, false))
            .into_iter()
            .filter(|f| f.num_rows() > 0)
            .collect();
        // The parameter changes halfway, after the front has state.
        let change = frames.len() / 2;
        let log = log();
        let plan = plan(query, &log);
        let mut queries = EventQueries::new(ctx.clone(), log);
        queries.set_params([first.clone()]).await.unwrap();
        assert_eq!(
            queries.register("q", plan.clone()).await.unwrap(),
            Mode::Streaming
        );
        let mut results = HashSet::new();
        for (i, frame) in frames.into_iter().enumerate() {
            queries.push(frame).await.unwrap();
            if i == change {
                queries.set_params([second.clone()]).await.unwrap();
            }
            let current = if i >= change { &second } else { &first };
            let want = batch_rows(&ctx, &plan, params(current)).await;
            assert_eq!(
                rows(queries.result("q").unwrap()),
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
