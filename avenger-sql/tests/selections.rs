//! Selections: `DECLARE name SELECTION`, its options, its reads as tests of rows, and their
//! states, which a run binds.

mod pass;

use std::sync::Arc;

use avenger_selection::{Clause, Condition, Resolution, SelectionState};
use avenger_sql::analysis::{Analysis, Overrides};
use datafusion::arrow::array::StringArray;

/// The analysis of `text`, which has no errors.
fn analysis(text: &str) -> Analysis {
    let analysis = pass::analysis(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{text}: {:?}",
        analysis.diagnostics
    );
    analysis
}

#[test]
fn a_script_declares_selections_of_each_resolution() {
    let analysis = analysis(
        "DECLARE brush SELECTION;
DECLARE picks SELECTION OPTIONS (resolution = 'union');
DECLARE both SELECTION OPTIONS (resolution = 'INTERSECT');
FROM penguins",
    );
    let declared: Vec<(&str, Resolution)> = (analysis.selections.iter())
        .map(|selection| (selection.name.as_str(), selection.resolution))
        .collect();
    assert_eq!(
        declared,
        [
            ("brush", Resolution::Global),
            ("picks", Resolution::Union),
            ("both", Resolution::Intersect),
        ]
    );
    // A selection plans nothing: the script's one statement is its query.
    assert_eq!(analysis.statements.len(), 1);
}

#[test]
fn a_misuse_of_a_selection_is_an_error_at_its_place() {
    let cases = [
        (
            "DECLARE b SELECTION DEFAULT 1;\nFROM penguins",
            "a SELECTION takes no DEFAULT",
            "1",
        ),
        (
            "DECLARE b SELECTION OPTIONS (color = 'red');\nFROM penguins",
            "a SELECTION takes the option `resolution`, not `color`",
            "color",
        ),
        (
            "DECLARE b SELECTION OPTIONS (resolution = 'all');\nFROM penguins",
            "`resolution` is 'global', 'union', or 'intersect'",
            "'all'",
        ),
        (
            "DECLARE n INT64 OPTIONS (resolution = 'union');\nFROM penguins",
            "OPTIONS hold the properties of a SELECTION",
            "OPTIONS (resolution = 'union')",
        ),
        (
            "DECLARE b SELECTION;\nDECLARE b INT64 DEFAULT 1;\nFROM penguins",
            "Variable 'b' redeclaration",
            "b",
        ),
        (
            "DECLARE b SELECTION;\nSET b = 1;\nFROM penguins",
            "`b` is a selection, which its tools write, so SET can't assign it",
            "b",
        ),
        (
            "DECLARE b SELECTION;\nDECLARE n BOOL DEFAULT b;\nFROM penguins",
            "`b` is a selection, which tests rows, not a value",
            "b",
        ),
        (
            "DECLARE b SELECTION;\nFROM penguins |> WHERE b = TRUE",
            "a selection tests rows, so it combines with AND, OR, and NOT",
            "b = TRUE",
        ),
        (
            "FROM penguins |> WHERE selected(species)",
            "`species` is not a selection of the script",
            "species",
        ),
        (
            "DECLARE b SELECTION;\nFROM penguins |> WHERE selected(b, empty => 'some')",
            "`empty` is 'all' or 'none'",
            "empty => 'some'",
        ),
        (
            "DECLARE b SELECTION;\nFROM penguins |> WHERE selected(b, exclude => 3)",
            "`exclude` takes the id of a mark",
            "exclude => 3",
        ),
    ];
    for (text, message, at) in cases {
        let errors = pass::errors(text);
        assert!(
            (errors.iter()).any(|(found, range)| found.contains(message) && range == at),
            "{text}: {errors:?}"
        );
    }
}

/// A state of `resolution` whose clause of `owner` selects the penguins of `species`.
fn species(owner: &str, species: &str) -> SelectionState {
    let condition = Condition::values("species", None, Arc::new(StringArray::from(vec![species])));
    SelectionState::new(
        Resolution::Global,
        vec![Clause::new(owner).condition(condition)],
    )
}

/// The value of the one row of the last query of `text`, with the states `states` of its
/// selections, as a string.
async fn value(text: &str, states: &[(&str, SelectionState)]) -> String {
    let ctx = pass::session();
    let analysis = analysis(text);
    let mut query = analysis
        .planned_query(&ctx, &Overrides::new())
        .await
        .unwrap();
    for (name, state) in states {
        assert!(
            query.selections.contains_key(*name),
            "{text} declares {name}"
        );
        query.selections.insert(name.to_string(), state.clone());
    }
    let batches = query
        .dataframe(&ctx)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let column = batches[0].column(0);
    datafusion::arrow::util::display::array_value_to_string(column, 0).unwrap()
}

#[tokio::test]
async fn a_read_of_a_selection_tests_each_row_by_its_state() {
    let count = "DECLARE brush SELECTION;\nFROM penguins |> WHERE brush |> AGGREGATE COUNT(*) AS n";
    // An empty selection selects every row.
    assert_eq!(value(count, &[]).await, "344");
    let adelie = species("scatter", "Adelie");
    assert_eq!(value(count, &[("brush", adelie.clone())]).await, "152");
    // A selection reads as a boolean anywhere that rows are in scope.
    let countif = "DECLARE brush SELECTION;\nFROM penguins |> AGGREGATE COUNTIF(brush) AS n";
    assert_eq!(value(countif, &[("brush", adelie.clone())]).await, "152");
    let not = "DECLARE brush SELECTION;\nFROM penguins |> WHERE NOT brush AND island = 'Dream' |> AGGREGATE COUNT(*) AS n";
    assert_eq!(value(not, &[("brush", adelie.clone())]).await, "68");
    // `empty => 'none'` selects no row while the selection is empty.
    let none = "DECLARE brush SELECTION;\nFROM penguins |> WHERE selected(brush, empty => 'none') |> AGGREGATE COUNT(*) AS n";
    assert_eq!(value(none, &[]).await, "0");
    // `exclude` skips the clauses of the marks that it names.
    let exclude = "DECLARE brush SELECTION;\nFROM penguins |> WHERE selected(brush, exclude => 'scatter') |> AGGREGATE COUNT(*) AS n";
    assert_eq!(value(exclude, &[("brush", adelie.clone())]).await, "344");
    let others = "DECLARE brush SELECTION;\nFROM penguins |> WHERE selected(brush, exclude => ['bars', 'map']) |> AGGREGATE COUNT(*) AS n";
    assert_eq!(value(others, &[("brush", adelie)]).await, "152");
}

#[tokio::test]
async fn a_select_tool_writes_a_selection_that_every_chart_declares() {
    use avenger_chart::proto::{layer_tool::Kind, SelectionResolution};
    let script = |item: &str| {
        format!(
            "DECLARE brush SELECTION;
DECLARE picks SELECTION OPTIONS (resolution = 'union');
FROM penguins |> AGGREGATE {item} AS chart"
        )
    };
    let scatter = pass::charts(&script(
        "mark_symbol(x => bill_length_mm, y => body_mass_g, id => 'scatter', \
         tools => [select(brush, type => interval(channels => ['x']))])",
    ))
    .await;
    let bars = pass::charts(&script("mark_bar(x => species, y => COUNT(*))")).await;
    for chart in [&scatter[0], &bars[0]] {
        let declared: Vec<(&str, SelectionResolution)> = (chart.selections().iter())
            .map(|selection| (selection.name.as_str(), selection.resolution()))
            .collect();
        assert_eq!(
            declared,
            [
                ("brush", SelectionResolution::Global),
                ("picks", SelectionResolution::Union),
            ]
        );
    }
    let Some(avenger_chart::proto::node::Node::Plot(plot)) = &scatter[0].root().node else {
        panic!("a plot");
    };
    let Some(avenger_chart::proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot");
    };
    let layer = &cartesian.layers[0];
    assert_eq!(layer.id, "scatter");
    let Some(Kind::Select(select)) = &layer.tools[0].kind else {
        panic!("a select tool");
    };
    assert_eq!(select.selection, "brush");
}

/// The id of the first layer of the plot of `chart`.
fn layer_id(chart: &avenger_chart::Chart) -> String {
    let Some(avenger_chart::proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot");
    };
    let Some(avenger_chart::proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot");
    };
    cartesian.layers[0].id.clone()
}

#[tokio::test]
async fn a_mark_with_a_select_tool_and_no_id_takes_a_generated_id() {
    use avenger_chart::{charts_from_arrow, Decode};
    let text = "DECLARE brush SELECTION;
FROM penguins
|> AGGREGATE
     mark_symbol(x => bill_length_mm, y => body_mass_g, tools => [select(brush)]) AS first,
     mark_bar(x => species, y => COUNT(*), tools => [select(brush, type => point(keys => ['species']))]) AS second,
     mark_bar(x => species, y => COUNT(*)) AS plain";
    let batches = pass::run(text).await.unwrap();
    let ids: Vec<String> = ["first", "second", "plain"]
        .into_iter()
        .map(|name| {
            let column = batches[0].column_by_name(name).unwrap();
            let chart = charts_from_arrow(column, Decode::Auto)
                .unwrap()
                .remove(0)
                .unwrap();
            layer_id(&chart)
        })
        .collect();
    assert_eq!(ids, ["mark_symbol_1", "mark_bar_2", ""]);
}
