//! The transforms of avenger-datafusion-transforms in a session of avenger-sql: `register` adds
//! them beside the functions of avenger-chart, whose names they do not take, and a chart reads
//! their results.

mod common;

use std::collections::BTreeSet;

use datafusion::prelude::SessionContext;

/// The names of the functions of `ctx`: its scalar, aggregate, window, and table functions.
fn names(ctx: &SessionContext) -> BTreeSet<String> {
    let state = ctx.state();
    (state.scalar_functions().keys())
        .chain(state.aggregate_functions().keys())
        .chain(state.window_functions().keys())
        .cloned()
        .chain(
            ["impute"]
                .into_iter()
                .filter(|name| ctx.table_function(name).is_ok())
                .map(String::from),
        )
        .collect()
}

#[test]
fn register_adds_the_transforms() {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    let names = names(&ctx);
    for name in ["bin", "stack", "timeunit", "lump", "bar", "scale"] {
        assert!(names.contains(name), "`{name}` is registered");
    }
    // A second registration adds nothing and does not fail.
    avenger_sql::register(&ctx).unwrap();
}

#[test]
fn the_transforms_take_no_name_of_avenger_chart() {
    let base = names(&SessionContext::new());
    use avenger_chart::functions::{constructors, marks, DIRECT, SCALED};
    let transforms = SessionContext::new();
    avenger_datafusion_transforms::register(&transforms).unwrap();
    let chart: BTreeSet<String> = (marks().iter().map(|mark| mark.name))
        .chain(constructors().iter().map(|constructor| constructor.name))
        .chain([SCALED, DIRECT])
        .map(String::from)
        .collect();
    let transforms: BTreeSet<String> = names(&transforms).difference(&base).cloned().collect();
    let shared: Vec<&String> = chart.intersection(&transforms).collect();
    assert!(shared.is_empty(), "both register {shared:?}");
}

#[tokio::test]
async fn a_chart_reads_the_bins_of_a_transform() {
    let query = "SELECT bar(x => b, y => n) AS chart FROM (
  SELECT b, count(*) AS n FROM (
    SELECT bin(body_mass_g) OVER () AS b FROM penguins WHERE body_mass_g IS NOT NULL
  ) GROUP BY b
)";
    let charts = common::charts(query).await;
    assert_eq!(charts.len(), 1);
    let chart = &charts[0];
    let Some(avenger_chart::proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    let layer = avenger_chart::LayerRef::of(plot).remove(0);
    let table = chart.table(layer.table().unwrap());
    assert!(table.num_rows() > 5, "the bins of the body masses");
    let bins = table
        .schema()
        .field_with_name("b")
        .unwrap()
        .data_type()
        .clone();
    assert!(
        matches!(bins, arrow::datatypes::DataType::Struct(_)),
        "{bins}"
    );
}

#[tokio::test]
async fn impute_fills_the_missing_keys() {
    // The penguins have 5 of the 9 pairs of a species and an island, and `impute` adds the rest.
    let query = "SELECT * FROM impute(
  (SELECT species, island, count(*) AS n FROM penguins GROUP BY species, island),
  island, [species], n, value => 0)";
    let ctx = common::session();
    let batches = avenger_sql::sql(&ctx, query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    assert_eq!(rows, 9);
}
