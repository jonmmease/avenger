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

/// The names of the columns of the layer of `chart`, a plot of one layer.
fn layer_columns(chart: &Chart) -> Vec<String> {
    let Some(avenger_chart::proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    let layer = avenger_chart::LayerRef::of(plot).remove(0);
    let table = chart.table(layer.table().unwrap());
    (table.schema().fields().iter())
        .map(|field| field.name().clone())
        .collect()
}

#[tokio::test]
async fn a_histogram_bins_the_whole_input() {
    same(
        &format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => COUNT(*)) AS chart"),
        &format!(
            "{PENGUINS} |> SET body_mass_g = bin(body_mass_g) OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY body_mass_g \
             |> AGGREGATE bar(x => body_mass_g, y => `COUNT(*)`) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn the_charts_of_the_groups_share_their_bins_or_take_their_own() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => COUNT(*)) AS chart \
             GROUP BY island"
        ),
        &format!(
            "{PENGUINS} |> SET body_mass_g = bin(body_mass_g) OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, body_mass_g \
             |> AGGREGATE bar(x => body_mass_g, y => `COUNT(*)`) AS chart GROUP BY island"
        ),
    )
    .await;
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER panel, y => COUNT(*)) AS chart \
             GROUP BY island"
        ),
        &format!(
            "{PENGUINS} |> SET body_mass_g = bin(body_mass_g) OVER (PARTITION BY island) \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, body_mass_g \
             |> AGGREGATE bar(x => body_mass_g, y => `COUNT(*)`) AS chart GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_lumped_legend_and_a_weighted_lump() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => COUNT(*), color => lump(species, n => 1)) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} |> SET species = lump(species, n => 1) OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species \
             |> AGGREGATE bar(x => island, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
    // The weight of a lump aggregates the rows of each category, so the legend is still a key.
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => COUNT(*), \
             color => lump(species, n => 1, weight => SUM(body_mass_g))) AS chart"
        ),
        &format!(
            "{PENGUINS} |> SET species = lump(species, n => 1, weight => SUM(body_mass_g)) OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species \
             |> AGGREGATE bar(x => island, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_window_that_reads_only_rows_is_a_key() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => NTILE(4) OVER (ORDER BY body_mass_g), y => COUNT(*)) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} |> EXTEND NTILE(4) OVER (ORDER BY body_mass_g) \
             AS `NTILE(4) OVER (ORDER BY body_mass_g)` \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY `NTILE(4) OVER (ORDER BY body_mass_g)` \
             |> AGGREGATE bar(x => `NTILE(4) OVER (ORDER BY body_mass_g)`, y => `COUNT(*)`) \
             AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_window_on_the_rows_inside_an_aggregate() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, \
             y => AVG(body_mass_g - AVG(body_mass_g) OVER ())) AS chart"
        ),
        &format!(
            "{PENGUINS} |> EXTEND body_mass_g - AVG(body_mass_g) OVER () AS d \
             |> AGGREGATE AVG(d) AS `AVG(body_mass_g - AVG(body_mass_g) OVER ())` GROUP BY species \
             |> AGGREGATE bar(x => species, y => `AVG(body_mass_g - AVG(body_mass_g) OVER ())`) \
             AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_call_without_an_aggregate_takes_windows_on_its_rows() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => body_mass_g, \
             y => RANK() OVER (ORDER BY body_mass_g)) AS chart"
        ),
        &format!(
            "{PENGUINS} |> EXTEND RANK() OVER (ORDER BY body_mass_g) \
             AS `RANK() OVER (ORDER BY body_mass_g)` \
             |> AGGREGATE symbol(x => body_mass_g, y => `RANK() OVER (ORDER BY body_mass_g)`) \
             AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn timeunits_and_a_window_of_a_window() {
    let dates = "FROM UNNEST([DATE '2024-01-05', DATE '2024-01-20', DATE '2024-02-03', \
                 DATE '2024-03-15', DATE '2024-03-16', DATE '2024-03-17']) AS d";
    same(
        &format!(
            "{dates} |> AGGREGATE bar(x => timeunit(d, unit => 'month'), y => COUNT(*)) AS chart"
        ),
        &format!(
            "{dates} |> SET d = timeunit(d, unit => 'month') OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY d \
             |> AGGREGATE bar(x => d, y => `COUNT(*)`) AS chart"
        ),
    )
    .await;
    same(
        &format!(
            "{dates} |> AGGREGATE bar(x => lump(timeunit(d, unit => 'month'), n => 1), \
             y => COUNT(*)) AS chart"
        ),
        &format!(
            "{dates} |> SET d = timeunit(d, unit => 'month') OVER () \
             |> SET d = lump(d, n => 1) OVER () \
             |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY d \
             |> AGGREGATE bar(x => d, y => `COUNT(*)`) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_lifted_value_leaves_a_column_its_name() {
    let charts = charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => COUNT(*), \
         color => body_mass_g) AS chart"
    ))
    .await;
    assert_eq!(
        layer_columns(&charts[0]),
        ["bin(body_mass_g)", "COUNT(*)", "body_mass_g"]
    );
}

#[tokio::test]
async fn the_errors_of_windows_on_the_rows() {
    for (query, expected) in [
        (
            format!("{PENGUINS} |> AGGREGATE bar(x => species, y => ROW_NUMBER())"),
            "Analytic function ROW_NUMBER cannot be called without an OVER clause",
        ),
        (
            format!("{PENGUINS} |> EXTEND ROW_NUMBER() OVER panel AS n"),
            "panel",
        ),
    ] {
        let message = error(&query).await;
        assert!(message.contains(expected), "{query}\n{message}");
    }
}

/// The counts of the penguins by island and species, as `c`.
const COUNTS: &str = "|> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species";

#[tokio::test]
async fn a_default_stack_puts_the_first_series_on_top() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => stack(COUNT(*)), color => species) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> SET `COUNT(*)` = stack(`COUNT(*)`) OVER (PARTITION BY island \
             ORDER BY species DESC NULLS FIRST) \
             |> AGGREGATE bar(x => island, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn an_order_of_a_stack_comes_first_and_the_default_breaks_its_ties() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, \
             y => stack(COUNT(*)) OVER (position ORDER BY COUNT(*) DESC), color => species) AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> SET `COUNT(*)` = stack(`COUNT(*)`) OVER (PARTITION BY island \
             ORDER BY `COUNT(*)` DESC, species DESC NULLS FIRST) \
             |> AGGREGATE bar(x => island, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_horizontal_stack_and_a_normalized_stack() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(y => island, x => stack(COUNT(*)), color => species) \
             AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> SET `COUNT(*)` = stack(`COUNT(*)`) OVER (PARTITION BY island \
             ORDER BY species ASC NULLS LAST) \
             |> AGGREGATE bar(y => island, x => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, \
             y => stack(COUNT(*), offset => 'normalize'), color => species) AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> SET `COUNT(*)` = stack(`COUNT(*)`, offset => 'normalize') OVER \
             (PARTITION BY island ORDER BY species DESC NULLS FIRST) \
             |> AGGREGATE bar(x => island, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_stack_centered_with_windows_over_position() {
    let centered = "STRUCT(SUM(COUNT(*)) OVER (position ROWS UNBOUNDED PRECEDING) - COUNT(*) \
                    - SUM(COUNT(*)) OVER position / 2 AS start, \
                    SUM(COUNT(*)) OVER (position ROWS UNBOUNDED PRECEDING) \
                    - SUM(COUNT(*)) OVER position / 2 AS `end`)";
    let name = (centered.split_whitespace().collect::<Vec<_>>().join(" ")).replace('`', "\\`");
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => {centered}, color => species) AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> EXTEND STRUCT(SUM(`COUNT(*)`) OVER (PARTITION BY island \
             ORDER BY species DESC NULLS FIRST ROWS UNBOUNDED PRECEDING) - `COUNT(*)` \
             - SUM(`COUNT(*)`) OVER (PARTITION BY island) / 2 \
             AS start, SUM(`COUNT(*)`) OVER (PARTITION BY island ORDER BY species DESC NULLS FIRST \
             ROWS UNBOUNDED PRECEDING) - SUM(`COUNT(*)`) OVER (PARTITION BY island) / 2 \
             AS `end`) AS `{name}` \
             |> AGGREGATE bar(x => island, y => `{name}`, color => species) AS chart"
        ),
    )
    .await;
}

/// The penguins with the start of the bin of their mass, where some bins lack a species.
const BINNED: &str = "FROM penguins |> WHERE body_mass_g IS NOT NULL \
                      |> EXTEND bin(body_mass_g) OVER () AS b \
                      |> EXTEND b.start AS mass";

#[tokio::test]
async fn a_stacked_area_fills_the_keys_that_a_series_lacks_with_zero() {
    same(
        &format!(
            "{BINNED} |> AGGREGATE area(x => mass, y => stack(COUNT(*)), color => species) AS chart"
        ),
        &format!(
            "{BINNED} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY mass, species \
             |> CALL impute(mass, [species], `COUNT(*)`, value => 0) \
             |> SET `COUNT(*)` = stack(`COUNT(*)`) OVER (PARTITION BY mass \
             ORDER BY species DESC NULLS FIRST) \
             |> AGGREGATE area(x => mass, y => `COUNT(*)`, color => species) AS chart"
        ),
    )
    .await;
    // A stacked line takes the end of each layer.
    same(
        &format!(
            "{BINNED} |> AGGREGATE line(x => mass, y => stack(COUNT(*)).`end`, color => species) \
             AS chart"
        ),
        &format!(
            "{BINNED} |> AGGREGATE COUNT(*) AS c GROUP BY mass, species \
             |> CALL impute(mass, [species], c, value => 0) \
             |> EXTEND stack(c) OVER (PARTITION BY mass ORDER BY species DESC NULLS FIRST).`end` \
             AS `stack(COUNT(*)).\\`end\\`` \
             |> AGGREGATE line(x => mass, y => `stack(COUNT(*)).\\`end\\``, color => species) \
             AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_bump_chart_ranks_without_a_fill_or_a_default_order() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE line(x => island, \
             y => RANK() OVER (position ORDER BY COUNT(*) DESC), color => species) AS chart"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> EXTEND RANK() OVER (PARTITION BY island ORDER BY `COUNT(*)` DESC) \
             AS `RANK() OVER (position ORDER BY COUNT(*) DESC)` \
             |> AGGREGATE line(x => island, y => `RANK() OVER (position ORDER BY COUNT(*) DESC)`, \
             color => species) AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn windows_over_a_panel_and_over_a_series() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, \
             y => COUNT(*) / SUM(COUNT(*)) OVER panel) AS chart GROUP BY island"
        ),
        &format!(
            "{PENGUINS} {COUNTS} \
             |> EXTEND `COUNT(*)` / SUM(`COUNT(*)`) OVER (PARTITION BY island) \
             AS `COUNT(*) / SUM(COUNT(*)) OVER panel` \
             |> AGGREGATE symbol(x => species, y => `COUNT(*) / SUM(COUNT(*)) OVER panel`) \
             AS chart GROUP BY island"
        ),
    )
    .await;
    for (window, staged) in [
        (
            "SUM(COUNT(*)) OVER (series ROWS UNBOUNDED PRECEDING)",
            "SUM(`COUNT(*)`) OVER (PARTITION BY island ORDER BY species ROWS UNBOUNDED PRECEDING)",
        ),
        (
            "LAG(COUNT(*)) OVER series",
            "LAG(`COUNT(*)`) OVER (PARTITION BY island ORDER BY species)",
        ),
        (
            "SUM(COUNT(*)) OVER series",
            "SUM(`COUNT(*)`) OVER (PARTITION BY island)",
        ),
    ] {
        same(
            &format!(
                "{PENGUINS} |> AGGREGATE line(x => species, y => {window}, color => island) \
                 AS chart"
            ),
            &format!(
                "{PENGUINS} {COUNTS} |> EXTEND {staged} AS `{window}` \
                 |> AGGREGATE line(x => species, y => `{window}`, color => island) AS chart"
            ),
        )
        .await;
    }
}

#[tokio::test]
async fn a_discretizer_of_an_aggregate_bins_the_aggregated_rows() {
    same(
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, y => COUNT(*), \
             color => bin(COUNT(*)).start) AS chart"
        ),
        &format!(
            "{PENGUINS} |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY species \
             |> EXTEND bin(`COUNT(*)`) OVER () AS b \
             |> EXTEND b.start AS `bin(COUNT(*)).start` \
             |> AGGREGATE symbol(x => species, y => `COUNT(*)`, color => `bin(COUNT(*)).start`) \
             AS chart"
        ),
    )
    .await;
}

#[tokio::test]
async fn the_errors_of_windows_on_the_aggregated_rows() {
    let message = error(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => COUNT(stack(COUNT(*))))"
    ))
    .await;
    assert!(
        message.contains("Aggregations of aggregations are not allowed"),
        "{message}"
    );
}
