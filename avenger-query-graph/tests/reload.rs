//! A reloaded document's plans reuse the results that the store keeps where their edits leave
//! the plans' subplans.

mod common;

use avenger_query_graph::{QueryGraph, ResultStore, SourceOptions};
use common::{context, inputs, outcomes, plan, rows, store, uncut};
use datafusion::{prelude::SessionContext, scalar::ScalarValue};

const COUNT: &str =
    "SELECT carrier, count(*) AS n FROM flights WHERE year = $year GROUP BY carrier";

/// A store whose flights' scans are materialized.
fn materialized() -> ResultStore {
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: true,
            ttl: None,
        },
    );
    store
}

async fn compile(ctx: &SessionContext, store: &ResultStore, sql: &str) -> QueryGraph {
    store
        .compile(&ctx.state(), vec![plan(ctx, sql).await])
        .unwrap()
}

/// The count under a new top, such as an edit of a chart's encoding and order.
fn new_top() -> String {
    format!("SELECT carrier, n * 2 AS twice FROM ({COUNT}) ORDER BY carrier")
}

#[tokio::test]
async fn a_new_top_reuses_the_count_that_the_store_keeps() {
    let ctx = context();
    let store = materialized();
    let values = inputs(&[("year", ScalarValue::from(2023i64))], &[]);
    let old = compile(&ctx, &store, COUNT).await;
    old.run(&values).await.unwrap();
    let new = compile(&ctx, &store, &new_top()).await;
    let run = new.run(&values).await.unwrap();
    // The new graph cuts below its new top at the kept count, so the count hits and the scan
    // below it is not read.
    assert_eq!(outcomes(&run), ["not read", "hit", "computed"]);
    let expected = uncut(&ctx, &plan(&ctx, &new_top()).await, &values).await;
    assert_eq!(rows(&run.batches[0]), expected);
}

#[tokio::test]
async fn a_store_without_results_adds_no_cut() {
    let ctx = context();
    let new = compile(&ctx, &materialized(), &new_top()).await;
    // The scan and the count with its new top.
    assert_eq!(new.nodes(None).unwrap().len(), 2);
}

#[tokio::test]
async fn a_kept_subplan_inside_a_recursive_query_is_not_cut() {
    let ctx = context();
    let store = store();
    let count = "SELECT count(*) AS c FROM flights WHERE delay > 0";
    compile(&ctx, &store, count)
        .await
        .run(&inputs(&[], &[]))
        .await
        .unwrap();
    let sql = format!(
        "WITH RECURSIVE t(n) AS (SELECT c FROM ({count}) AS s \
         UNION ALL SELECT n + 1 FROM t WHERE n < $max) SELECT * FROM t"
    );
    let graph = compile(&ctx, &store, &sql).await;
    assert_eq!(graph.nodes(None).unwrap().len(), 1);
    let values = inputs(&[("max", ScalarValue::from(1000i64))], &[]);
    let run = graph.run(&values).await.unwrap();
    assert_eq!(
        rows(&run.batches[0]),
        uncut(&ctx, &plan(&ctx, &sql).await, &values).await
    );
}
