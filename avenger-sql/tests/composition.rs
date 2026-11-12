//! The composition aggregates: `layer`, `hconcat`, `vconcat`, `facet_row`, `facet_col`, and
//! `facet_wrap` over a column of charts.

mod common;

use avenger_chart::{proto, Chart};
use common::{charts, session};

/// The penguins that have measurements.
const PENGUINS: &str = "FROM penguins |> WHERE body_mass_g IS NOT NULL";

/// The scatter plot of the penguins of each group of `keys`.
fn scatter(keys: &str) -> String {
    format!(
        "{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm) AS chart \
         GROUP BY {keys}"
    )
}

/// The one chart of `query`.
async fn chart(query: &str) -> Chart {
    let mut charts = charts(query).await;
    assert_eq!(charts.len(), 1, "{query}");
    charts.remove(0)
}

/// The error of running `query`, at planning or at execution.
async fn error(query: &str) -> String {
    let ctx = session();
    let result = async {
        let frame = avenger_sql::sql(&ctx, query).await?;
        frame.collect().await
    }
    .await;
    match result {
        Ok(_) => panic!("`{query}` runs"),
        Err(error) => error.to_string(),
    }
}

/// The member and the children of a node.
fn children(node: &proto::Node) -> (&'static str, &[proto::Node]) {
    match &node.node {
        Some(proto::node::Node::Concat(concat)) => ("concat", &concat.children),
        Some(proto::node::Node::Wrap(wrap)) => ("wrap", &wrap.children),
        Some(proto::node::Node::Plot(_)) => ("plot", &[]),
        Some(proto::node::Node::Empty(_)) => ("empty", &[]),
        None => ("none", &[]),
    }
}

/// The facet of a concat or a wrap.
fn facet(node: &proto::Node) -> &proto::Facet {
    match &node.node {
        Some(proto::node::Node::Concat(concat)) => concat.facet.as_ref().unwrap(),
        Some(proto::node::Node::Wrap(wrap)) => wrap.facet.as_ref().unwrap(),
        _ => panic!("a facet"),
    }
}

#[tokio::test]
async fn facet_col_lays_out_the_charts_of_a_key() {
    let chart = chart(&format!(
        "{} |> AGGREGATE facet_col(chart, island) AS chart",
        scatter("island")
    ))
    .await;
    let (member, children) = children(chart.root());
    assert_eq!(member, "concat");
    assert_eq!(children.len(), 3);
    assert_eq!(facet(chart.root()).column, "island");
}

#[tokio::test]
async fn a_facet_row_of_facet_col_charts_is_a_grid() {
    let chart = chart(&format!(
        "{} |> AGGREGATE facet_col(chart, island) AS chart GROUP BY sex \
         |> AGGREGATE facet_row(chart, sex) AS chart",
        scatter("sex, island")
    ))
    .await;
    let (_, rows) = children(chart.root());
    // FEMALE, MALE, and NULL, each with a cell for every island.
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(children(row).1.len(), 3, "{row:?}");
    }
    assert_eq!(facet(chart.root()).column, "sex");
}

#[tokio::test]
async fn facet_wrap_takes_the_options_of_a_wrap() {
    let chart = chart(&format!(
        "{} |> AGGREGATE facet_wrap(chart, species, layout => wrap_layout(max_columns => 2)) \
         AS chart",
        scatter("species")
    ))
    .await;
    let Some(proto::node::Node::Wrap(wrap)) = &chart.root().node else {
        panic!("a wrap");
    };
    assert_eq!(wrap.layout.as_ref().unwrap().max_columns, Some(2));
    assert_eq!(wrap.children.len(), 3);
}

#[tokio::test]
async fn layer_overlays_the_charts_of_a_union_in_its_order() {
    let chart = chart(&format!(
        "SELECT *, 2 AS k FROM ({PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => \
         bill_depth_mm, fill => 'black') AS chart) \
         UNION ALL BY NAME \
         SELECT *, 1 AS k FROM ({PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => \
         bill_depth_mm) AS chart) \
         |> AGGREGATE layer(chart ORDER BY k) AS chart"
    ))
    .await;
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot");
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot");
    };
    let fills: Vec<bool> = (cartesian.layers.iter())
        .map(|layer| match &layer.mark {
            Some(proto::cartesian_layer::Mark::Symbol(symbol)) => symbol.fill.is_some(),
            _ => panic!("a symbol"),
        })
        .collect();
    assert_eq!(fills, [false, true], "the layer of k = 1 draws first");
}

#[tokio::test]
async fn a_concat_composes_the_charts_of_each_group() {
    let charts = charts(&format!(
        "{} |> AGGREGATE hconcat(chart ORDER BY sex) AS chart GROUP BY island",
        scatter("island, sex")
    ))
    .await;
    assert_eq!(charts.len(), 3);
    for chart in &charts {
        let Some(proto::node::Node::Concat(concat)) = &chart.root().node else {
            panic!("a concat");
        };
        assert_eq!(concat.direction, proto::Direction::Row as i32);
    }
    let chart = chart(&format!(
        "{} |> AGGREGATE vconcat(chart, id => 'islands' ORDER BY island DESC) AS chart",
        scatter("island")
    ))
    .await;
    let Some(proto::node::Node::Concat(concat)) = &chart.root().node else {
        panic!("a concat");
    };
    assert_eq!(concat.direction, proto::Direction::Column as i32);
    assert_eq!(concat.id, "islands");
}

#[tokio::test]
async fn a_composition_of_no_rows_is_an_empty_chart() {
    let chart = chart(&format!(
        "{} |> WHERE island = 'none' |> AGGREGATE facet_col(chart, island) AS chart",
        scatter("island")
    ))
    .await;
    assert_eq!(children(chart.root()).0, "empty");
}

#[tokio::test]
async fn the_errors_of_a_composition() {
    for (query, expected) in [
        (
            format!("{PENGUINS} |> AGGREGATE layer(species) AS chart"),
            "layer() takes a column of charts, and `species` is not one",
        ),
        (
            format!(
                "{} |> AGGREGATE facet_col(chart, island, id => island) AS chart",
                scatter("island")
            ),
            "facet_col() needs a plan-time constant for `id`",
        ),
        (
            format!(
                "{} |> AGGREGATE facet_col(chart, island, facet => facet(column => 'sex')) AS chart",
                scatter("island")
            ),
            "Unknown parameter name 'column'",
        ),
        (
            format!(
                "{} |> AGGREGATE facet_col(chart, 'one') AS chart",
                scatter("island")
            ),
            "two rows have the key value one",
        ),
        (
            format!(
                "{} |> AGGREGATE hconcat(chart) AS chart |> AGGREGATE layer(chart) AS chart",
                scatter("island")
            ),
            "has a concat at its root",
        ),
    ] {
        let message = error(&query).await;
        assert!(message.contains(expected), "{query}: {message}");
    }
}
