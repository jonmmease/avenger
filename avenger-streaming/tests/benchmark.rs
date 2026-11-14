//! Streaming against batch, timed: each frame of a session of 60 frames a second goes to the log
//! and the queries, as a chart pushes them, and at sessions of 1,000 to 100,000 events, frames with
//! events time each query as a streaming query and as its batch plan over the log. Each frame of
//! the batch's sample also checks the streaming result. The table goes to
//! `tests/output/benchmark.md`.
//!
//! ```bash
//! cargo test --release -p avenger-streaming --test benchmark -- --ignored --nocapture
//! ```

mod common;

use std::time::{Duration, Instant};

use avenger_streaming::{EventQueries, Mode};
use common::{batches, context, log, plan, rows, session};
use datafusion::arrow::array::RecordBatch;

const QUERIES: &[(&str, &str)] = &[
    (
        "counters",
        "FROM events |> AGGREGATE COUNT(*) AS n, COUNTIF(type = 'click') AS clicks, MAX(ts) AS last",
    ),
    (
        "hot targets",
        "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n GROUP BY target |> WHERE n >= 3 |> AGGREGATE COUNT(*) AS hot_targets",
    ),
    (
        "last click",
        "FROM events |> WHERE type = 'click' |> ORDER BY seq DESC |> LIMIT 1 |> SELECT target, x, y",
    ),
    (
        "last pointer position",
        "FROM events |> WHERE type = 'pointermove' |> ORDER BY seq DESC |> LIMIT 1 |> SELECT x, y",
    ),
    (
        "drag brush",
        "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.x) AS x0, LAST(mv.x) AS x1, COUNT(mv.x) AS moves, MAX(seq) AS last_seq PATTERN (dn mv* up?) DEFINE dn AS type = 'pointerdown' AND inside, mv AS type = 'pointermove', up AS type = 'pointerup') |> WHERE moves > 0 |> ORDER BY last_seq DESC |> LIMIT 1 |> SELECT LEAST(x0, x1) AS lo, GREATEST(x0, x1) AS hi",
    ),
    (
        "brush with double-click clear",
        "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup', 'dblclick') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.x) AS x0, LAST(mv.x) AS x1, COUNT(mv.x) AS moves, COUNT(clr.seq) > 0 AS cleared, MAX(seq) AS last_seq PATTERN (dn mv* up? | clr) DEFINE dn AS type = 'pointerdown' AND inside, mv AS type = 'pointermove', up AS type = 'pointerup', clr AS type = 'dblclick' AND inside) |> WHERE moves > 0 OR cleared |> ORDER BY last_seq DESC |> LIMIT 1 |> WHERE NOT cleared |> SELECT LEAST(x0, x1) AS lo, GREATEST(x0, x1) AS hi",
    ),
    (
        "total drag time",
        "FROM events |> WHERE type IN ('pointerdown', 'pointermove', 'pointerup') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.ts) AS t0, LAST(mv.ts) AS t1 PATTERN (dn mv+) DEFINE dn AS type = 'pointerdown', mv AS type = 'pointermove') |> AGGREGATE SUM(TIMESTAMP_DIFF(t1, t0, MILLISECOND)) AS total_drag_ms, COUNT(*) AS drags",
    ),
    (
        "pauses (NEXT)",
        "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES MIN(p.seq) AS pause_seq PATTERN (p) DEFINE p AS TIMESTAMP_DIFF(NEXT(ts), ts, MILLISECOND) > 400) |> AGGREGATE COUNT(*) AS pauses, MAX(pause_seq) AS last_pause",
    ),
    (
        "flicks (PREV)",
        "FROM events |> WHERE type = 'pointermove' |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS moves PATTERN (f{3,}) DEFINE f AS ABS(x - PREV(x)) > 4 AND ABS(x - PREV(x, 2)) > 6) |> AGGREGATE COUNT(*) AS flicks, MAX(moves) AS longest",
    ),
    (
        "three rightmost moves",
        "FROM events |> WHERE type = 'pointermove' |> ORDER BY x DESC, seq DESC |> LIMIT 3 |> SELECT seq, x",
    ),
];

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark; see the module's docs"]
async fn streaming_against_batch() {
    let checkpoints = [1_000usize, 10_000, 50_000, 100_000];
    let frames: Vec<RecordBatch> = batches(&session(checkpoints[3] + 20_000, 0x5eed, false));
    let (measured, compared) = (300, 30);
    let ctx = context();
    let mut report = String::from(
        "| Query | Events | Batch ms per frame | Streaming ms per frame | Streaming worst frame ms |\n| --- | ---: | ---: | ---: | ---: |\n",
    );
    for (name, query) in QUERIES {
        let log = log();
        let plan = plan(query, &log);
        let batch_plan = ctx.state().optimize(&plan).unwrap();
        let mut queries = EventQueries::new(ctx.clone(), log.clone());
        assert_eq!(
            queries.register(*name, plan).await.unwrap(),
            Mode::Streaming
        );
        let mut index = 0usize;
        for checkpoint in checkpoints {
            // The frames up to the checkpoint, one at a time, as a chart pushes them.
            while log.len() < checkpoint {
                let frame = frames[index].clone();
                index += 1;
                queries.push(frame).await.unwrap();
            }
            // Frames with events through the queries, and for the first of them, the batch plan
            // over the log too, timed apart.
            let (mut stream_time, mut worst, mut batch_time) =
                (Duration::ZERO, Duration::ZERO, Duration::ZERO);
            let mut count = 0;
            while count < measured {
                let frame = frames[index].clone();
                index += 1;
                if frame.num_rows() == 0 {
                    continue;
                }
                let start = Instant::now();
                queries.push(frame).await.unwrap();
                let elapsed = start.elapsed();
                stream_time += elapsed;
                worst = worst.max(elapsed);
                if count < compared {
                    let start = Instant::now();
                    let physical = ctx.state().create_physical_plan(&batch_plan).await.unwrap();
                    let batch = datafusion::physical_plan::collect(physical, ctx.task_ctx())
                        .await
                        .unwrap();
                    batch_time += start.elapsed();
                    assert_eq!(
                        rows(queries.result(name).unwrap()),
                        rows(&batch),
                        "{name} at {} events",
                        log.len()
                    );
                }
                count += 1;
            }
            let (b, s) = (
                ms(batch_time) / compared as f64,
                ms(stream_time) / measured as f64,
            );
            eprintln!(
                "{name} @ {}: batch {b:.2} ms, streaming {s:.3} ms",
                log.len()
            );
            report.push_str(&format!(
                "| {name} | {} | {b:.2} | {s:.3} | {:.3} |\n",
                log.len(),
                ms(worst)
            ));
        }
    }
    println!("{report}");
    std::fs::create_dir_all("tests/output").unwrap();
    std::fs::write("tests/output/benchmark.md", report).unwrap();
}
