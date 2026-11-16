//! The composition aggregates `layer_agg`, `hconcat_agg`, `vconcat_agg`, `facet_row`,
//! `facet_col`, and `facet_wrap` over a column of charts, and the scalar compositions `layer`,
//! `hconcat`, and `vconcat` over the charts of a row.

mod common;

use avenger_chart::{proto, Chart};
use common::{charts, charts_in, session, session_with};
use datafusion::prelude::SessionConfig;

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

/// Whether each layer of the plot at the root of `chart` sets its fill.
fn fills(chart: &Chart) -> Vec<bool> {
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot");
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot");
    };
    (cartesian.layers.iter())
        .map(|layer| match &layer.mark {
            Some(proto::cartesian_layer::Mark::Symbol(symbol)) => symbol.fill.is_some(),
            _ => panic!("a symbol"),
        })
        .collect()
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
async fn the_order_by_of_a_layer_orders_its_charts() {
    // On 8 partitions, the rows of the union reach the aggregate in no defined order.
    let ctx = session_with(SessionConfig::new().with_target_partitions(8));
    let union = format!(
        "FROM ({PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, \
         fill => 'black') AS chart |> EXTEND 2 AS k) \
         |> UNION ALL ({PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm) \
         AS chart |> EXTEND 1 AS k)"
    );
    for (order, expected) in [("k", [false, true]), ("k DESC", [true, false])] {
        let query = format!("{union} |> AGGREGATE layer_agg(chart ORDER BY {order}) AS chart");
        let charts = charts_in(&ctx, &query).await;
        assert_eq!(fills(&charts[0]), expected, "{query}");
    }
}

#[tokio::test]
async fn a_concat_composes_the_charts_of_each_group() {
    let charts = charts(&format!(
        "{} |> AGGREGATE hconcat_agg(chart ORDER BY sex) AS chart GROUP BY island",
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
        "{} |> AGGREGATE vconcat_agg(chart, id => 'islands' ORDER BY island DESC) AS chart",
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
            format!("{PENGUINS} |> AGGREGATE layer_agg(species) AS chart"),
            "layer_agg() takes a column of charts, and `species` is not one",
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
            "facet has no parameter named `column`",
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
                "{} |> AGGREGATE hconcat_agg(chart) AS chart |> AGGREGATE layer_agg(chart) AS chart",
                scatter("island")
            ),
            "has a concat at its root",
        ),
    ] {
        let message = error(&query).await;
        assert!(message.contains(expected), "{query}: {message}");
    }
}

/// Two scatter plots of the penguins, `a` and `b`, and a bar chart, `c`, as the columns of one row.
fn three() -> String {
    format!(
        "{PENGUINS} |> AGGREGATE \
         symbol(x => bill_length_mm, y => bill_depth_mm) AS a, \
         symbol(x => flipper_length_mm, y => body_mass_g) AS b, \
         bar(x => species, y => body_mass_g) AS c"
    )
}

/// The charts of the columns `names` of [`three`] as rows of a union, with their places in `k`.
fn rows(names: &[&str]) -> String {
    let branch = |(k, name): (usize, &&str)| {
        format!("FROM ({}) |> SELECT {name} AS chart, {k} AS k", three())
    };
    let mut branches = names.iter().enumerate().map(branch);
    let first = branches.next().unwrap();
    let rest: Vec<String> = branches
        .map(|branch| format!("|> UNION ALL ({branch})"))
        .collect();
    format!("{first} {}", rest.join(" "))
}

#[tokio::test]
async fn a_scalar_composition_composes_the_charts_of_a_row() {
    for (scalar, aggregate) in [
        ("layer", "layer_agg"),
        ("hconcat", "hconcat_agg"),
        ("vconcat", "vconcat_agg"),
    ] {
        assert_eq!(
            chart(&format!(
                "FROM ({}) |> SELECT {scalar}(a, b) AS chart",
                three()
            ))
            .await,
            chart(&format!(
                "{} |> AGGREGATE {aggregate}(chart ORDER BY k) AS chart",
                rows(&["a", "b"])
            ))
            .await,
            "{scalar}"
        );
    }
    // The children take the order of the arguments.
    let ba = chart(&format!(
        "FROM ({}) |> SELECT hconcat(b, a) AS chart",
        three()
    ))
    .await;
    assert_ne!(
        ba,
        chart(&format!(
            "FROM ({}) |> SELECT hconcat(a, b) AS chart",
            three()
        ))
        .await
    );
    assert_eq!(
        ba,
        chart(&format!(
            "{} |> AGGREGATE hconcat_agg(chart ORDER BY k) AS chart",
            rows(&["b", "a"])
        ))
        .await
    );
    // Any number of charts, nested.
    assert_eq!(
        chart(&format!(
            "FROM ({}) |> SELECT vconcat(c, hconcat(a, b, a)) AS chart",
            three()
        ))
        .await,
        chart(&format!(
            "FROM ({0}) |> SELECT c AS chart, 0 AS k \
             |> UNION ALL (FROM ({0}) |> SELECT hconcat(a, b, a) AS chart, 1 AS k) \
             |> AGGREGATE vconcat_agg(chart ORDER BY k) AS chart",
            three()
        ))
        .await
    );
}

#[tokio::test]
async fn a_null_chart_is_no_child_of_a_scalar_composition() {
    assert_eq!(
        chart(&format!(
            "FROM ({}) |> SELECT hconcat(a, NULL, b) AS chart",
            three()
        ))
        .await,
        chart(&format!(
            "FROM ({}) |> SELECT hconcat(a, b) AS chart",
            three()
        ))
        .await
    );
    let empty = chart(&format!(
        "FROM ({}) |> SELECT layer(NULL, NULL) AS chart",
        three()
    ))
    .await;
    assert_eq!(children(empty.root()).0, "empty");
}

#[tokio::test]
async fn a_scalar_composition_takes_the_options_of_its_composition() {
    assert_eq!(
        chart(&format!(
            "FROM ({}) |> SELECT hconcat(a, b, layout => concat_layout(gap => 24)) AS chart",
            three()
        ))
        .await,
        chart(&format!(
            "{} |> AGGREGATE hconcat_agg(chart, layout => concat_layout(gap => 24) ORDER BY k) \
             AS chart",
            rows(&["a", "b"])
        ))
        .await
    );
}

#[tokio::test]
async fn a_scalar_composition_of_mark_aggregates_composes_each_group() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE layer(\
           symbol(x => bill_length_mm, y => bill_depth_mm), \
           symbol(x => flipper_length_mm, y => body_mass_g, size => 20)) AS chart \
         GROUP BY island"
    );
    assert_eq!(charts(&query).await.len(), 3, "one chart of each island");
    // The result is a column of charts, which an aggregate composes.
    let faceted = chart(&format!(
        "{query} |> AGGREGATE facet_col(chart, island) AS chart"
    ))
    .await;
    assert_eq!(children(faceted.root()).1.len(), 3);
}

#[tokio::test]
async fn the_errors_of_a_scalar_composition() {
    for (query, expected) in [
        (
            format!("FROM ({}) |> SELECT layer(a, 1) AS chart", three()),
            "layer() composes charts, and its argument 2 is not a chart",
        ),
        (
            format!(
                "FROM ({}) |> SELECT hconcat(a, b, layout => a) AS chart",
                three()
            ),
            "hconcat() needs a plan-time constant for `layout`",
        ),
        // A facet has no scalar form, and a layer's options go by name.
        (
            format!("FROM ({}) |> SELECT facet_col(a, b) AS chart", three()),
            "Aggregate function facet_col not allowed in pipe SELECT",
        ),
        (
            format!("FROM ({}) |> SELECT layer(id => 'l') AS chart", three()),
            "layer() takes at least one chart",
        ),
        (
            format!(
                "FROM ({}) |> SELECT hconcat(a, b, layout => concat_layout(gap => 24), c) \
                 AS chart",
                three()
            ),
            "positional argument",
        ),
    ] {
        let message = error(&query).await;
        assert!(message.contains(expected), "{query}: {message}");
    }
}
