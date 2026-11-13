//! The log of events and the registry of queries over it: the log's numbering, the queries that
//! run as batch plans, a query registered after events, and the queries that a frame changes.

mod common;

use std::sync::Arc;

use avenger_streaming::{EventLog, EventQueries, Mode};
use common::{batch_rows, batches, context, events_schema, log, plan, rows, session};
use datafusion::{
    arrow::{
        array::{AsArray, RecordBatch, StringArray},
        datatypes::{DataType, Field, Int64Type, Schema},
    },
    common::ScalarValue,
};

/// The frames of a session that carry events.
fn frames(count: usize, seed: u64) -> Vec<RecordBatch> {
    batches(&session(count, seed, false))
        .into_iter()
        .filter(|f| f.num_rows() > 0)
        .collect()
}

#[tokio::test]
async fn the_log_numbers_events_in_the_order_of_arrival() {
    let log = log();
    let frames = frames(50, 1);
    let (a, b) = (frames[0].num_rows() as i64, frames[1].num_rows() as i64);
    assert_eq!(log.append(frames[0].clone()).await.unwrap(), 1..1 + a);
    assert_eq!(
        log.append(frames[1].clone()).await.unwrap(),
        1 + a..1 + a + b
    );
    let empty = frames[2].slice(0, 0);
    assert_eq!(log.append(empty).await.unwrap(), 1 + a + b..1 + a + b);
    assert_eq!(log.len(), (a + b) as usize);
    // The table has `seq` first, from 1, without gaps.
    let ctx = context();
    let plan = plan("FROM events |> SELECT seq", &log);
    let batches = ctx
        .execute_logical_plan(plan)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let seq: Vec<i64> = (batches.iter())
        .flat_map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
        .collect();
    assert_eq!(seq, (1..1 + a + b).collect::<Vec<_>>());
    assert_eq!(log.schema().field(0).name(), "seq");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_keeps_its_events_in_order_as_it_merges_batches() {
    // Single events, as frames append them, which the log merges into larger batches.
    let log = log();
    let frame = frames(10, 5)[0].slice(0, 1);
    for _ in 0..20_000 {
        log.append(frame.clone()).await.unwrap();
    }
    let ctx = context();
    let seq_plan = plan("FROM events |> SELECT seq", &log);
    let batches = ctx
        .execute_logical_plan(seq_plan)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let seq: Vec<i64> = (batches.iter())
        .flat_map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
        .collect();
    assert_eq!(seq, (1..=20_000).collect::<Vec<_>>());
    // A query registered over the merged log, and one that read it as it grew, agree.
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    let query = "FROM events |> AGGREGATE COUNT(*) AS n, MIN(seq) AS first, MAX(seq) AS last";
    queries.register("q", plan(query, &log)).await.unwrap();
    queries.push(frame.clone()).await.unwrap();
    assert_eq!(
        rows(queries.result("q").unwrap()),
        batch_rows(&ctx, &plan(query, &log), None).await
    );
}

#[tokio::test]
async fn the_log_takes_only_events_of_its_columns() {
    let with_seq = Arc::new(Schema::new(vec![
        Field::new("seq", DataType::Int64, false),
        Field::new("type", DataType::Utf8, false),
    ]));
    let error = EventLog::new(with_seq).err().unwrap().to_string();
    assert!(error.contains("numbers its events in `seq`"), "{error}");
    let other = Arc::new(Schema::new(vec![Field::new("type", DataType::Utf8, false)]));
    let batch =
        RecordBatch::try_new(other, vec![Arc::new(StringArray::from(vec!["click"]))]).unwrap();
    let error = log().append(batch).await.err().unwrap().to_string();
    assert!(error.contains("where the log takes"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_that_doesnt_split_runs_as_its_batch_plan() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    let query = "FROM events |> EXTEND ROW_NUMBER() OVER (PARTITION BY type ORDER BY seq) AS n |> WHERE n <= 2 |> AGGREGATE COUNT(*) AS first_two";
    let plan = plan(query, &log);
    let mode = queries.register("first two", plan.clone()).await.unwrap();
    assert_eq!(
        mode,
        Mode::Batch(avenger_streaming::Rejection {
            message: "a window over events, which reads every row, as a window function or HAVING MAX does".into()
        })
    );
    for frame in frames(300, 2) {
        queries.push(frame).await.unwrap();
        let want = batch_rows(&ctx, &plan, None).await;
        assert_eq!(rows(queries.result("first two").unwrap()), want);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_registered_after_events_catches_up() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    for frame in frames(5_000, 3) {
        queries.push(frame).await.unwrap();
    }
    let query = "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n GROUP BY target";
    let plan = plan(query, &log);
    assert_eq!(
        queries.register("late", plan.clone()).await.unwrap(),
        Mode::Streaming
    );
    assert_eq!(
        rows(queries.result("late").unwrap()),
        batch_rows(&ctx, &plan, None).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_frame_reports_the_queries_whose_result_changed() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    let last_click =
        "FROM events |> WHERE type = 'click' |> ORDER BY seq DESC |> LIMIT 1 |> SELECT seq";
    let moves = "FROM events |> WHERE type = 'pointermove' |> AGGREGATE COUNT(*) AS n";
    queries
        .register("last click", plan(last_click, &log))
        .await
        .unwrap();
    queries.register("moves", plan(moves, &log)).await.unwrap();
    let event = |kind: &str| {
        let frame = &frames(10, 4)[0];
        let mut columns = frame.slice(0, 1).columns().to_vec();
        columns[1] = Arc::new(StringArray::from(vec![kind]));
        RecordBatch::try_new(events_schema(), columns).unwrap()
    };
    assert_eq!(queries.push(event("pointermove")).await.unwrap(), ["moves"]);
    assert_eq!(queries.push(event("click")).await.unwrap(), ["last click"]);
    assert_eq!(
        queries.push(event("pointerup")).await.unwrap(),
        Vec::<String>::new()
    );
    // A removed query is no longer run or reported.
    assert!(queries.remove("moves"));
    assert_eq!(
        queries.push(event("pointermove")).await.unwrap(),
        Vec::<String>::new()
    );
    assert!(queries.result("moves").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_needs_a_name_of_its_own_and_its_parameters() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    let query = "FROM events |> WHERE x > @min_x |> AGGREGATE COUNT(*) AS n";
    let error = queries
        .register("q", plan(query, &log))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("reads the parameter `min_x`, which has no value"),
        "{error}"
    );
    let changed = queries
        .set_params([("min_x".to_string(), ScalarValue::Float64(Some(400.0)))])
        .await
        .unwrap();
    assert!(changed.is_empty());
    queries.register("q", plan(query, &log)).await.unwrap();
    let error = queries
        .register("q", plan(query, &log))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("a query is registered as `q`"), "{error}");
    // Setting a parameter to its value changes nothing.
    let same = [("min_x".to_string(), ScalarValue::Float64(Some(400.0)))];
    assert!(queries.set_params(same).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_query_reads_the_clock_when_it_runs() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    let query =
        "FROM events |> AGGREGATE MAX(ts) AS last |> SELECT CURRENT_TIMESTAMP() AS now, last";
    let mode = queries.register("idle", plan(query, &log)).await.unwrap();
    assert!(matches!(mode, Mode::Batch(_)), "{mode:?}");
    let now = |queries: &EventQueries| {
        let rows = queries.result("idle").unwrap();
        let column = datafusion::arrow::compute::cast(rows[0].column(0), &DataType::Int64).unwrap();
        column.as_primitive::<Int64Type>().value(0)
    };
    let frames = frames(50, 6);
    queries.push(frames[0].clone()).await.unwrap();
    let first = now(&queries);
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    queries.push(frames[1].clone()).await.unwrap();
    assert!(now(&queries) > first, "the clock moved");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_that_a_query_fails_on_fails_it_on_each_frame() {
    let ctx = context();
    let log = log();
    let mut queries = EventQueries::new(ctx.clone(), log.clone());
    // A click divides by zero, as its batch plan does over every event from then on.
    let query = "FROM events |> SELECT IF(type = 'click', DIV(1, 0 * CAST(inside AS INT64)), 1) AS one |> AGGREGATE SUM(one) AS total";
    assert_eq!(
        queries.register("q", plan(query, &log)).await.unwrap(),
        Mode::Streaming
    );
    let event = |kind: &str| {
        let frame = &frames(10, 4)[0];
        let mut columns = frame.slice(0, 1).columns().to_vec();
        columns[1] = Arc::new(StringArray::from(vec![kind]));
        RecordBatch::try_new(events_schema(), columns).unwrap()
    };
    queries.push(event("pointermove")).await.unwrap();
    let before = rows(queries.result("q").unwrap());
    assert!(queries.push(event("click")).await.is_err());
    assert!(
        queries.push(event("pointermove")).await.is_err(),
        "the click stays in the log"
    );
    assert_eq!(
        rows(queries.result("q").unwrap()),
        before,
        "the last result stays"
    );
}
