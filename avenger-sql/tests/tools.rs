//! The options of a mark's plot and layer in SQL: a title, a navigation of the plot, a tooltip of
//! the layer, and their merge when `layer` composes charts.

mod common;

use avenger_chart::{proto, Chart};
use common::{charts, session};

const PENGUINS: &str = "FROM penguins |> WHERE body_mass_g IS NOT NULL";

/// The one chart of `query`.
async fn chart(query: &str) -> Chart {
    let mut charts = charts(query).await;
    assert_eq!(charts.len(), 1, "{query}");
    charts.remove(0)
}

/// The plot at the root of `chart`, and its Cartesian layers.
fn plot(chart: &Chart) -> (&proto::Plot, &[proto::CartesianLayer]) {
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot")
    };
    (plot, &cartesian.layers)
}

/// Whether `tools` are exactly one navigation.
fn navigates(tools: &[proto::Tool]) -> bool {
    matches!(
        tools,
        [proto::Tool {
            kind: Some(proto::tool::Kind::Navigate(_)),
            ..
        }]
    )
}

/// Whether `tools` are exactly one tooltip.
fn shows_tooltips(tools: &[proto::LayerTool]) -> bool {
    matches!(
        tools,
        [proto::LayerTool {
            kind: Some(proto::layer_tool::Kind::Tooltip(_)),
            ..
        }]
    )
}

/// The error of running `query`.
async fn error(query: &str) -> String {
    let ctx = session();
    let result = async { avenger_sql::sql(&ctx, query).await?.collect().await }.await;
    match result {
        Ok(_) => panic!("`{query}` runs"),
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn a_mark_declares_a_navigation_of_its_plot_and_a_tooltip_of_its_layer() {
    let chart = chart(&format!(
        "{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, title => 'Bills', \
         tools => [navigate(), tooltip(find => nearest_find(max_radius => 20))]) AS chart"
    ))
    .await;
    let (root, layers) = plot(&chart);
    assert_eq!(root.title.as_ref().unwrap().text, "Bills");
    assert!(navigates(&root.tools));
    assert!(shows_tooltips(&layers[0].tools));
    // Without an id, the plot's and the layer's paths identify them.
    assert_eq!((root.id.as_str(), layers[0].id.as_str()), ("", ""));
    // An id names the layer.
    let named = self::chart(&format!(
        "{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, id => 'points', \
         tools => [tooltip()]) AS chart"
    ))
    .await;
    assert_eq!(plot(&named).1[0].id, "points");
}

#[tokio::test]
async fn layer_merges_the_options_of_its_charts() {
    let chart = chart(&format!(
        "(SELECT * FROM ({PENGUINS} |> CHART bar(x => species, y => avg(body_mass_g), \
         title => 'Mass by species', tools => [navigate(), tooltip()]))) \
         UNION ALL (SELECT * FROM ({PENGUINS} |> CHART rule(y => avg(body_mass_g), \
         tools => [navigate()]))) \
         |> AGGREGATE layer(chart) AS chart"
    ))
    .await;
    let (root, layers) = plot(&chart);
    assert_eq!(root.title.as_ref().unwrap().text, "Mass by species");
    assert!(navigates(&root.tools), "two equal navigations are one");
    assert!(shows_tooltips(&layers[0].tools));
    assert!(layers[1].tools.is_empty());
}

#[tokio::test]
async fn the_panels_of_a_facet_take_tools_without_ids() {
    let chart = chart(&format!(
        "{PENGUINS} |> CHART symbol(x => bill_length_mm, y => bill_depth_mm, \
         tools => [navigate(), tooltip()]) GROUP BY island \
         |> AGGREGATE facet_col(chart, island) AS chart"
    ))
    .await;
    let Some(proto::node::Node::Concat(concat)) = &chart.root().node else {
        panic!("a concat")
    };
    assert_eq!(concat.children.len(), 3);
    let resolved = chart.resolve();
    let names: Vec<String> = (resolved.tree.plots.iter())
        .map(|plot| plot.identity.name())
        .collect();
    assert_eq!(
        names,
        [
            "root.concat.children[*].plot[island=Biscoe]",
            "root.concat.children[*].plot[island=Dream]",
            "root.concat.children[*].plot[island=Torgersen]",
        ]
    );
}

#[tokio::test]
async fn an_option_is_a_plan_time_constant() {
    let message = error(&format!(
        "{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, title => species) \
         AS chart"
    ))
    .await;
    assert!(message.contains("needs a plan-time constant"), "{message}");
    let message = error(&format!(
        "{PENGUINS} |> CHART symbol(x => bill_length_mm, y => bill_depth_mm, title => species)"
    ))
    .await;
    assert!(
        message.contains("option `title` of symbol() needs a plan-time constant"),
        "{message}"
    );
}
