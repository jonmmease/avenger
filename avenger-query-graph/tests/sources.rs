//! Sources with a time to live are checked at the start of a run and by `revalidate`.

mod common;

use std::{path::PathBuf, time::Duration};

use avenger_query_graph::{QueryGraph, ResultStore, SourceOptions};
use common::{inputs, outcomes, store};
use datafusion::{
    prelude::{CsvReadOptions, SessionContext},
    scalar::ScalarValue,
};

const SQL: &str = "SELECT carrier, count(*) AS n FROM flights WHERE year = $year GROUP BY carrier";

/// A CSV file of `rows` flights in a directory of the test `name`.
fn write(name: &str, rows: usize) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/output/sources")
        .join(name);
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("flights.csv");
    let mut text = String::from("carrier,year\n");
    for row in 0..rows {
        let carrier = ["AA", "DL", "UA"][row % 3];
        text.push_str(&format!("{carrier},{}\n", 2022 + (row % 2)));
    }
    std::fs::write(&path, text).unwrap();
    path
}

async fn csv_graph(name: &str, ttl: Duration) -> (PathBuf, ResultStore, QueryGraph) {
    let path = write(name, 20);
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx.register_csv("flights", path.to_str().unwrap(), CsvReadOptions::new())
        .await
        .unwrap();
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: false,
            ttl: Some(ttl),
        },
    );
    let graph = store
        .compile(&ctx.state(), vec![common::plan(&ctx, SQL).await])
        .unwrap();
    (path, store, graph)
}

fn year() -> avenger_query_graph::Inputs {
    inputs(&[("year", ScalarValue::from(2022i64))], &[])
}

#[tokio::test]
async fn an_unchanged_file_keeps_its_version_and_a_rewritten_one_does_not() {
    let (path, _store, graph) = csv_graph("zero", Duration::ZERO).await;
    let first = graph.run(&year()).await.unwrap();
    assert_eq!(outcomes(&first), ["computed"]);
    let unchanged = graph.run(&year()).await.unwrap();
    assert_eq!(outcomes(&unchanged), ["hit"]);
    assert!(path.ends_with("zero/flights.csv"));
    write("zero", 31);
    let rewritten = graph.run(&year()).await.unwrap();
    assert_eq!(outcomes(&rewritten), ["computed"]);
    assert_ne!(
        common::rows(&first.batches[0]),
        common::rows(&rewritten.batches[0])
    );
}

#[tokio::test]
async fn a_long_time_to_live_keeps_the_version_until_the_table_changes() {
    let (_, store, graph) = csv_graph("long", Duration::from_secs(3600)).await;
    graph.run(&year()).await.unwrap();
    write("long", 31);
    let kept = graph.run(&year()).await.unwrap();
    assert_eq!(outcomes(&kept), ["hit"]);
    store.table_changed("flights");
    let changed = graph.run(&year()).await.unwrap();
    assert_eq!(outcomes(&changed), ["computed"]);
}

#[tokio::test]
async fn a_table_of_another_kind_gets_a_new_version_at_each_check() {
    let ctx = common::context();
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: false,
            ttl: Some(Duration::ZERO),
        },
    );
    let graph = store
        .compile(&ctx.state(), vec![common::plan(&ctx, SQL).await])
        .unwrap();
    for _ in 0..2 {
        let run = graph.run(&year()).await.unwrap();
        assert_eq!(outcomes(&run), ["computed"]);
    }
}

#[tokio::test]
async fn revalidate_reports_a_change() {
    let (_, store, graph) = csv_graph("revalidate", Duration::ZERO).await;
    graph.run(&year()).await.unwrap();
    let before = graph.key(&year()).unwrap();
    assert!(!store.revalidate().await.unwrap());
    assert_eq!(graph.key(&year()).unwrap(), before);
    write("revalidate", 31);
    assert!(store.revalidate().await.unwrap());
    assert_ne!(graph.key(&year()).unwrap(), before);
}
