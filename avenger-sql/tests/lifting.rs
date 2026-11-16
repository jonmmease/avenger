//! Statistics in the channels of mark calls: each lifted query gives the same charts, with the
//! same column names, as the staged query that a user could type for it.

mod common;

use avenger_chart::Chart;
use common::{charts_in, session_with};
use datafusion::prelude::{SessionConfig, SessionContext};

const PENGUINS: &str = "FROM penguins |> WHERE body_mass_g IS NOT NULL";

/// One target partition, so that an aggregate's rows come in one order.
fn session() -> SessionContext {
    session_with(SessionConfig::new().with_target_partitions(1))
}

async fn charts(query: &str) -> Vec<Chart> {
    charts_in(&session(), query).await
}

/// Asserts that `lifted` gives the charts of `staged`.
async fn same(lifted: &str, staged: &str) {
    let found = charts(lifted).await;
    assert!(!found.is_empty(), "{lifted}");
    assert_eq!(found, charts(staged).await, "{lifted}");
}

/// The message of the error of `query`.
async fn error(query: &str) -> String {
    match avenger_sql::sql(&session(), query).await {
        Ok(frame) => match frame.collect().await {
            Ok(_) => panic!("no error: {query}"),
            Err(error) => error.to_string(),
        },
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn a_count_by_category() {
    same(
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(*)) AS chart"),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY species \
             |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_mean_of_each_key_of_each_group() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => AVG(body_mass_g), color => sex) AS \
             chart GROUP BY island"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE AVG(body_mass_g) AS `AVG(body_mass_g)` \
             GROUP BY island, species, sex \
             |> AGGREGATE bar(x => species, y => `AVG(body_mass_g)`, color => sex) AS chart \
             GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_measure_computes_over_its_aggregates_under_its_text() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, y => AVG(body_mass_g)   /  1000) AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE AVG(body_mass_g) AS m GROUP BY species \
             |> SELECT species, m / 1000 AS `AVG(body_mass_g) / 1000` \
             |> AGGREGATE symbol(x => species, y => `AVG(body_mass_g) / 1000`) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn scaled_and_direct_keep_their_place() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => scaled(COUNT(*), title => 'Penguins')) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY species \
             |> AGGREGATE bar(x => species, y => scaled(`COUNT(*)`, title => 'Penguins')) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn the_order_by_of_a_lifted_call_draws_by_an_aggregate() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(*) ORDER BY AVG(body_mass_g) DESC) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)`, AVG(body_mass_g) AS m \
             GROUP BY species \
             |> AGGREGATE bar(x => species, y => `COUNT(*)` ORDER BY m DESC) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn items_beside_a_lifted_chart_join_on_the_groups() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE COUNT(*) AS n, bar(x => species, y => COUNT(*)) AS chart \
         GROUP BY island |> ORDER BY island"
    );
    let batches = avenger_sql::sql(&session(), &query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let batch = datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap();
    let names: Vec<&str> = (batch.schema_ref().fields().iter())
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, ["island", "n", "chart"]);
    assert_eq!(batch.num_rows(), 3);
    same(
        &query,
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species \
             |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart GROUP BY island \
             |> ORDER BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn two_lifted_marks_and_a_layer_of_two_grains() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE layer(bar(x => species, y => COUNT(*)), \
             rule(y => AVG(body_mass_g))) AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY species \
             |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart \
             |> CROSS JOIN ({PENGUINS} |> AGGREGATE AVG(body_mass_g) AS `AVG(body_mass_g)` \
             |> AGGREGATE rule(y => `AVG(body_mass_g)`) AS r) \
             |> SELECT layer(chart, r) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_facet_of_lifted_charts() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(*)) AS chart GROUP BY island \
             |> AGGREGATE facet_col(chart, island) AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species \
             |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart GROUP BY island \
             |> AGGREGATE facet_col(chart, island) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_flat_mark_beside_a_lifted_one_reads_the_rows() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE layer(symbol(x => bill_length_mm, y => bill_depth_mm), \
         rule(y => AVG(bill_depth_mm))) AS chart"
    );
    assert_eq!(charts(&query).await.len(), 1);
}

#[tokio::test]
async fn the_errors_of_a_lifted_call() {
    for (query, expected) in [
        (
            format!("{PENGUINS} |> AGGREGATE bar(x => species, y => body_mass_g + COUNT(*))"),
            "body_mass_g + COUNT(*) reads the column body_mass_g outside its aggregates, which \
             is neither a key of bar nor a group",
        ),
        (
            format!("{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(*) ORDER BY island)"),
            "The ORDER BY of bar orders by island, which is neither a channel of the call, a \
             group, nor an aggregate",
        ),
        (
            format!("{PENGUINS} |> AGGREGATE bar(x => species, y => SUM(COUNT(*)))"),
            "Aggregations of aggregations are not allowed",
        ),
        (
            format!(
                "{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(*)) AS chart \
                 GROUP BY ROLLUP (island)"
            ),
            "bar computes statistics in its channels, which an AGGREGATE with GROUPING SETS, \
             ROLLUP, or CUBE cannot hold",
        ),
        (
            // A mark outside the items of a pipe AGGREGATE keeps the errors of main.
            format!("{PENGUINS} |> SELECT bar(x => species, y => COUNT(*)) AS chart"),
            "Aggregations of aggregations are not allowed",
        ),
    ] {
        let message = error(&query).await;
        assert!(message.contains(expected), "{query}\n{message}");
    }
}
