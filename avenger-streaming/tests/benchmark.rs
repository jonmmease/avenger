//! Streaming against batch, timed: the frames with events of a session of 60 frames a second,
//! through each query as a streaming query and as the batch plan over every event so far, at
//! sessions of 1,000 to 100,000 events. Each frame of the batch's sample also checks the streaming
//! result. The table goes to `tests/output/benchmark.md`.
//!
//! ```bash
//! cargo test --release -p avenger-streaming --test benchmark -- --ignored --nocapture
//! ```

mod common;

use std::time::{Duration, Instant};

use avenger_streaming::StreamingQuery;
use common::{batches, context, events_table, plan, rows, session, split_of};
use datafusion::arrow::{array::RecordBatch, compute::concat_batches};

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
    let frames = batches(&session(checkpoints[3] + 20_000, 0x5eed, false));
    let (measured, compared) = (300, 30);
    let ctx = context();
    let mut report = String::from(
        "| Query | Events | Batch ms per frame | Streaming ms per frame | Streaming worst frame ms |\n| --- | ---: | ---: | ---: | ---: |\n",
    );
    for (name, query) in QUERIES {
        let all = events_table();
        let batch_plan = ctx.state().optimize(&plan(query, all.clone())).unwrap();
        let mut streaming = StreamingQuery::new(ctx.clone(), split_of(query), None)
            .await
            .unwrap();
        let mut stored: Vec<RecordBatch> = Vec::new();
        let (mut count, mut index) = (0usize, 0usize);
        for checkpoint in checkpoints {
            // The events up to the checkpoint, 50 frames at a time until near it, as the result
            // doesn't depend on how events split into frames.
            let mut chunk = Vec::new();
            while count < checkpoint {
                let frame = frames[index].clone();
                index += 1;
                count += frame.num_rows();
                stored.push(frame.clone());
                if frame.num_rows() > 0 {
                    chunk.push(frame);
                }
                if !chunk.is_empty() && (chunk.len() >= 50 || count + 500 >= checkpoint) {
                    let events = concat_batches(&common::events_schema(), &chunk).unwrap();
                    streaming.push(events).await.unwrap();
                    chunk.clear();
                }
            }
            if !chunk.is_empty() {
                let events = concat_batches(&common::events_schema(), &chunk).unwrap();
                streaming.push(events).await.unwrap();
            }
            // Frames with events through the streaming query, then the first of them through the
            // batch plan, timed apart.
            let (mut stream_time, mut worst) = (Duration::ZERO, Duration::ZERO);
            let mut results = Vec::new();
            let mut ends = Vec::new();
            while ends.len() < measured {
                let frame = frames[index].clone();
                index += 1;
                count += frame.num_rows();
                stored.push(frame.clone());
                if frame.num_rows() == 0 {
                    continue;
                }
                ends.push(stored.len());
                let start = Instant::now();
                streaming.push(frame).await.unwrap();
                let elapsed = start.elapsed();
                stream_time += elapsed;
                worst = worst.max(elapsed);
                results.push(rows(streaming.result()));
            }
            let mut batch_time = Duration::ZERO;
            for (result, end) in results.iter().zip(&ends).take(compared) {
                let start = Instant::now();
                *all.batches[0].write().await = stored[..*end].to_vec();
                let physical = ctx.state().create_physical_plan(&batch_plan).await.unwrap();
                let batch = datafusion::physical_plan::collect(physical, ctx.task_ctx())
                    .await
                    .unwrap();
                batch_time += start.elapsed();
                assert_eq!(result, &rows(&batch), "{name} at frame {end}");
            }
            let (b, s) = (
                ms(batch_time) / compared as f64,
                ms(stream_time) / measured as f64,
            );
            eprintln!("{name} @ {count}: batch {b:.2} ms, streaming {s:.3} ms");
            report.push_str(&format!(
                "| {name} | {count} | {b:.2} | {s:.3} | {:.3} |\n",
                ms(worst)
            ));
        }
    }
    println!("{report}");
    std::fs::create_dir_all("tests/output").unwrap();
    std::fs::write("tests/output/benchmark.md", report).unwrap();
}
