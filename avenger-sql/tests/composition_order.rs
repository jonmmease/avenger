//! The order of the children of `layer`, `hconcat`, and `vconcat`: the keys of their `ORDER BY`,
//! then the branch of a `UNION ALL`, then the other columns of their input.

mod common;

use std::sync::Arc;

use arrow::array::AsArray;
use avenger_chart::{charts_from_arrow, proto, Chart, Decode};
use datafusion::{datasource::MemTable, prelude::*};

/// A session with the penguins on 8 partitions, so that a lost order shows.
fn session() -> SessionContext {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(8));
    avenger_sql::register(&ctx).unwrap();
    let penguins = common::penguins();
    let size = penguins.num_rows().div_ceil(8);
    let partitions = (0..8)
        .map(|index| {
            let start = (index * size).min(penguins.num_rows());
            let len = size.min(penguins.num_rows() - start);
            vec![penguins.slice(start, len)]
        })
        .collect();
    let table = MemTable::try_new(penguins.schema(), partitions).unwrap();
    ctx.register_table("penguins", Arc::new(table)).unwrap();
    ctx
}

/// The one chart of `query`.
async fn chart(query: &str) -> Chart {
    let batches = avenger_sql::sql(&session(), query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut charts = Vec::new();
    for batch in batches {
        let column = batch.column_by_name("chart").expect("a chart column");
        charts.extend(charts_from_arrow(column, Decode::Auto).unwrap());
    }
    assert_eq!(charts.len(), 1, "{query}");
    charts.remove(0).unwrap()
}

/// A branch: the symbols of the penguins with the constant fill `fill`, and the extra columns
/// `columns`.
fn branch(fill: &str, columns: &str) -> String {
    format!(
        "SELECT *{columns} FROM (FROM penguins |> WHERE body_mass_g IS NOT NULL \
         |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, fill => '{fill}') AS chart)"
    )
}

/// The constant fill of each layer of a layered chart, in draw order.
fn fills(chart: &Chart) -> Vec<String> {
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot");
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot");
    };
    (cartesian.layers.iter())
        .map(|layer| {
            let Some(proto::cartesian_layer::Mark::Symbol(symbol)) = &layer.mark else {
                panic!("a symbol");
            };
            let fill = symbol.fill.as_ref().expect("a fill");
            let Some(proto::encoding::Kind::DirectValue(value)) = &fill.kind else {
                panic!("a direct value: {fill:?}");
            };
            chart.value(value).as_string::<i32>().value(0).to_string()
        })
        .collect()
}

#[tokio::test]
async fn the_branches_of_a_union_all_order_the_layers() {
    let union = [branch("red", ""), branch("green", ""), branch("blue", "")].join(" UNION ALL ");
    for _ in 0..4 {
        let chart = chart(&format!("{union} |> AGGREGATE layer(chart) AS chart")).await;
        assert_eq!(fills(&chart), ["red", "green", "blue"]);
    }
    let plan = avenger_sql::sql(
        &session(),
        &format!("{union} |> AGGREGATE layer(chart) AS chart"),
    )
    .await
    .unwrap()
    .into_optimized_plan()
    .unwrap();
    let plan = plan.display_indent().to_string();
    assert!(plan.contains("__avenger_branch"), "{plan}");
    // The ordinal survives a filter and an extension.
    let chart = chart(&format!(
        "{union} |> WHERE chart IS NOT NULL |> EXTEND 1 AS one |> AGGREGATE layer(chart) AS chart"
    ))
    .await;
    assert_eq!(fills(&chart), ["red", "green", "blue"]);
}

#[tokio::test]
async fn the_branches_of_a_union_by_name_order_the_layers() {
    // `UNION ALL BY NAME` writes a projection over the union before it, and the branches under it
    // still number among all the branches.
    let union =
        [branch("red", ""), branch("green", ""), branch("blue", "")].join(" UNION ALL BY NAME ");
    let pipe = format!(
        "{} |> UNION ALL BY NAME ({}) |> UNION ALL BY NAME ({})",
        branch("red", ""),
        branch("green", ""),
        branch("blue", "")
    );
    for union in [union, pipe] {
        let query = format!("{union} |> AGGREGATE layer(chart) AS chart");
        assert_eq!(fills(&chart(&query).await), ["red", "green", "blue"]);
        let plan = avenger_sql::sql(&session(), &query)
            .await
            .unwrap()
            .into_optimized_plan()
            .unwrap();
        let plan = plan.display_indent().to_string();
        for ordinal in 0..3 {
            let column = format!("Int64({ordinal}) AS __avenger_branch");
            assert_eq!(plan.matches(&column).count(), 1, "{column} in {plan}");
        }
    }
}

#[tokio::test]
async fn an_order_by_comes_first_and_the_branch_breaks_its_ties() {
    let union = [
        branch("red", ", 2 AS k"),
        branch("green", ", 1 AS k"),
        branch("blue", ", 2 AS k"),
    ]
    .join(" UNION ALL ");
    let chart = chart(&format!(
        "{union} |> AGGREGATE layer(chart ORDER BY k) AS chart"
    ))
    .await;
    assert_eq!(fills(&chart), ["green", "red", "blue"]);
}

#[tokio::test]
async fn without_a_union_the_other_columns_order_the_children() {
    // Without a union, the key columns of the input order the charts of a concat.
    let chart = chart(
        "FROM penguins |> WHERE body_mass_g IS NOT NULL \
         |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, fill => island) AS chart \
            GROUP BY island \
         |> AGGREGATE hconcat(chart) AS chart",
    )
    .await;
    let Some(proto::node::Node::Concat(concat)) = &chart.root().node else {
        panic!("a concat");
    };
    let islands: Vec<String> = (concat.children.iter())
        .map(|child| {
            let Some(proto::node::Node::Plot(plot)) = &child.node else {
                panic!("a plot");
            };
            let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
                panic!("a Cartesian plot");
            };
            let table = chart.table(cartesian.layers[0].table.as_ref().unwrap());
            let island = table.column_by_name("island").unwrap();
            island.as_string::<i32>().value(0).to_string()
        })
        .collect();
    assert_eq!(islands, ["Biscoe", "Dream", "Torgersen"]);
}

#[tokio::test]
async fn a_union_without_all_has_no_branch_order() {
    // A distinct union orders its rows by its other columns only.
    let union = [branch("red", ", 2 AS k"), branch("green", ", 1 AS k")].join(" UNION ");
    let chart = chart(&format!("{union} |> AGGREGATE layer(chart) AS chart")).await;
    assert_eq!(fills(&chart), ["green", "red"]);
}
