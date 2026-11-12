//! The charts of a pipe `AGGREGATE`: each mark call that lifts draws the chart of its printed lifting run as a query
//! (`r[lift.test.bytes]`), and `lift` prints the lifting that `tests/baselines/liftings/` holds,
//! which a change accepts by copying it from `tests/output/liftings/`. The validation rules have
//! one valid and one invalid call each, with the error and its span.

mod common;

use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, AsArray, Float64Array, RecordBatch, StringArray, TimestampMillisecondArray,
    },
    datatypes::DataType,
};
use avenger_chart::{charts_from_arrow, proto, Chart, Decode};
use datafusion::{
    common::DataFusionError,
    datasource::MemTable,
    prelude::{SessionConfig, SessionContext},
};

/// 182 days of sales from 2024-01-01: `t`, a timestamp, `units`, and `region`.
fn sales() -> RecordBatch {
    let day: i64 = 86_400_000;
    let start: i64 = 1_704_067_200_000; // 2024-01-01
    let t =
        TimestampMillisecondArray::from((0..182).map(|i| start + i * day).collect::<Vec<i64>>());
    let units: Float64Array = (0..182).map(|i| ((i * 7) % 11) as f64).collect();
    let region: StringArray = (0..182)
        .map(|i| Some(["east", "west", "north"][i % 3]))
        .collect();
    RecordBatch::try_from_iter([
        ("t", Arc::new(t) as ArrayRef),
        ("units", Arc::new(units)),
        ("region", Arc::new(region)),
    ])
    .unwrap()
}

/// A session of one partition, so a query and its lifting give their rows in one order, with the
/// penguins and the sales.
fn session() -> SessionContext {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
    avenger_sql::register(&ctx).unwrap();
    for (name, table) in [("penguins", common::penguins()), ("sales", sales())] {
        let table = MemTable::try_new(table.schema(), vec![vec![table]]).unwrap();
        ctx.register_table(name, Arc::new(table)).unwrap();
    }
    ctx
}

/// The charts of the column of charts of `query` ([`chart_column`]), in the order of its rows.
async fn charts(query: &str) -> Vec<Chart> {
    charts_in(query, &chart_column(query).await).await
}

/// The name of the column of charts of `query`, a column of the chart extension: `chart`, or the
/// one column of charts.
async fn chart_column(query: &str) -> String {
    let frame = avenger_sql::sql(&session(), query)
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"));
    let columns: Vec<String> = (frame.schema().fields().iter())
        .filter(|field| {
            field
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str)
                == Some(avenger_chart::EXTENSION_NAME)
        })
        .map(|field| field.name().clone())
        .collect();
    if columns.iter().any(|column| column == "chart") {
        return "chart".to_string();
    }
    assert_eq!(
        columns.len(),
        1,
        "one column of charts in {query}: {columns:?}"
    );
    columns[0].clone()
}

/// The charts of the column `name` of `query`, in the order of its rows, a column of the chart
/// extension.
async fn charts_in(query: &str, name: &str) -> Vec<Chart> {
    let batches = avenger_sql::sql(&session(), query)
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"))
        .collect()
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"));
    let mut charts = Vec::new();
    for batch in batches {
        let column = batch.column_by_name(name).expect("a chart column");
        let schema = batch.schema();
        let extension = (schema.field_with_name(name).unwrap().metadata())
            .get("ARROW:extension:name")
            .cloned();
        assert_eq!(
            extension.as_deref(),
            Some(avenger_chart::EXTENSION_NAME),
            "the column `{name}` of {query}"
        );
        for chart in charts_from_arrow(column, Decode::Auto).unwrap() {
            charts.push(chart.unwrap());
        }
    }
    charts
}

/// Checks that `query` draws the charts of its lifting, and that `lift` prints the lifting of the
/// baseline `name`. It returns the charts.
async fn lifted(name: &str, query: &str) -> Vec<Chart> {
    let column = chart_column(query).await;
    lifted_in(name, query, &[&column]).await.remove(0)
}

/// Checks that each chart column of `columns` of `query` holds the charts of its lifting, and that
/// `lift` prints the lifting of the baseline `name`. It returns the charts of each column.
async fn lifted_in(name: &str, query: &str, columns: &[&str]) -> Vec<Vec<Chart>> {
    let lifting = avenger_sql::lift(&session(), query).await.unwrap().text;
    assert!(!lifting.contains("__avenger_"), "no marker in {lifting}");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let output = format!("{dir}/output/liftings/{name}.sql");
    std::fs::create_dir_all(format!("{dir}/output/liftings")).unwrap();
    std::fs::write(&output, format!("{lifting}\n")).unwrap();
    let baseline = std::fs::read_to_string(format!("{dir}/baselines/liftings/{name}.sql"))
        .unwrap_or_else(|_| panic!("no baseline of {name}: accept {output}"));
    assert_eq!(baseline.trim_end(), lifting, "the lifting of {name}");
    let mut charts = Vec::new();
    for column in columns {
        let chart = charts_in(query, column).await;
        assert!(!chart.is_empty(), "{query}");
        assert_eq!(
            chart,
            charts_in(&lifting, column).await,
            "the charts of {name}"
        );
        charts.push(chart);
    }
    charts
}

/// The error of planning `query`.
async fn error(query: &str) -> DataFusionError {
    match avenger_sql::sql(&session(), query).await {
        Ok(_) => panic!("`{query}` plans"),
        Err(error) => error,
    }
}

/// Checks that `query` fails with a message that contains `message`, at the first `at` of
/// `query`.
async fn fails(query: &str, message: &str, at: &str) {
    let error = error(query).await;
    assert!(error.to_string().contains(message), "{query}: {error}");
    let span = (error.diagnostic())
        .and_then(|diagnostic| diagnostic.span)
        .unwrap_or_else(|| panic!("{query}: no span: {error}"));
    let column = query.find(at).expect("the text of the span") as u64 + 1;
    assert_eq!(
        (span.start.line, span.start.column),
        (1, column),
        "{query}: {error}"
    );
}

/// The layer table of the first layer of the plot at the root of `chart`.
fn table(chart: &Chart) -> RecordBatch {
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    let layer = avenger_chart::LayerRef::of(plot).remove(0);
    chart.table(layer.table().unwrap()).clone()
}

const PENGUINS: &str = "FROM penguins |> WHERE body_mass_g IS NOT NULL";

#[tokio::test]
async fn a_bar_of_counts_by_a_key() {
    let charts = lifted(
        "counts",
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*))"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 3);
    let charts = lifted(
        "counts-by-island",
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) GROUP BY island"),
    )
    .await;
    assert_eq!(charts.len(), 3, "one chart for each island");
}

#[tokio::test]
async fn a_histogram() {
    let charts = lifted(
        "histogram",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g, maxbins => 20), y => count(*))"
        ),
    )
    .await;
    let bins = table(&charts[0]);
    let field = bins
        .schema()
        .field_with_name("body_mass_g_bin")
        .unwrap()
        .clone();
    assert!(matches!(field.data_type(), DataType::Struct(_)), "{field}");
}

#[tokio::test]
async fn bins_in_each_panel() {
    lifted(
        "histogram-panel",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER panel, y => count(*)) GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_lump_color() {
    lifted(
        "lump",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => count(*), color => lump(species, n => 2, weight => sum(body_mass_g)))"
        ),
    )
    .await;
}

#[tokio::test]
async fn months_of_timestamps() {
    let charts = lifted(
        "months",
        "FROM sales |> AGGREGATE bar(x => timeunit(t, unit => 'month'), y => sum(units))",
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 6, "January to June");
}

#[tokio::test]
async fn nested_discretizers() {
    lifted(
        "nested",
        "FROM sales |> AGGREGATE bar(x => lump(timeunit(t, unit => 'month'), n => 3, weight => sum(units)), y => sum(units))",
    )
    .await;
}

#[tokio::test]
async fn a_key_expression() {
    lifted(
        "key-expression",
        &format!("{PENGUINS} |> AGGREGATE bar(x => upper(species), y => count(*))"),
    )
    .await;
}

#[tokio::test]
async fn a_post_aggregate_expression_and_two_uses_of_one_aggregate() {
    lifted(
        "post-aggregate",
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, y => count(*) / 2, size => count(*))"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_draw_order() {
    lifted(
        "order-aggregate",
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*) ORDER BY count(*) DESC)"),
    )
    .await;
    lifted(
        "order-key",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), color => island ORDER BY island DESC)"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_call_that_does_not_group() {
    let charts = lifted(
        "rows",
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm * 2, y => bill_depth_mm, color => lump(species, n => 2))"
        ),
    )
    .await;
    assert_eq!(
        table(&charts[0]).num_rows(),
        342,
        "one symbol for each penguin"
    );
}

#[tokio::test]
async fn a_name_that_an_input_column_has() {
    lifted(
        "collision",
        &format!("{PENGUINS} |> EXTEND 1 AS n |> AGGREGATE bar(x => species, y => count(*))"),
    )
    .await;
}

#[tokio::test]
async fn the_output_of_chart_composes() {
    // The partition is the key of a facet.
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => count(*)) AS chart GROUP BY island |> AGGREGATE facet_col(chart, island) AS chart"
    );
    let composed = charts(&query).await;
    let Some(proto::node::Node::Concat(concat)) = &composed[0].root().node else {
        panic!("a concat")
    };
    assert_eq!(concat.children.len(), 3);
    // A layer of the charts of a union draws them in the order of its branches.
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) AS chart |> UNION ALL ({PENGUINS} |> AGGREGATE rule(y => avg(body_mass_g)) AS chart) |> AGGREGATE layer_agg(chart) AS chart"
    );
    let layered = charts(&query).await;
    let Some(proto::node::Node::Plot(plot)) = &layered[0].root().node else {
        panic!("a plot")
    };
    let marks: Vec<&str> = avenger_chart::LayerRef::of(plot)
        .iter()
        .map(|layer| layer.mark().unwrap().name())
        .collect();
    assert_eq!(marks, ["bar", "rule"]);
}

#[tokio::test]
async fn the_mark_call_is_read_against_its_mark() {
    fails(
        "FROM penguins |> AGGREGATE bar(x => species, y => count(*), colour => island)",
        "bar() has no parameter `colour`",
        "colour",
    )
    .await;
    fails(
        "FROM penguins |> AGGREGATE bar(x => species, x => island, y => count(*))",
        "bar() takes `x` once",
        "x => island",
    )
    .await;
}

#[tokio::test]
async fn v1_arguments_read_only_earlier_stages() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => count(*))"
    ))
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => bin(count(*)), y => count(*))");
    fails(
        &query,
        "`count(*)` cannot be an argument of `bin`, because bin computes its bins before the aggregates",
        "count(*))",
    )
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => species, y => sum(count(*)))");
    fails(
        &query,
        "`count(*)` cannot be an argument of `sum`",
        "count(*)))",
    )
    .await;
}

#[tokio::test]
async fn v3_post_aggregate_expressions_read_keys() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => count(*) / bin(body_mass_g).end)"
    ))
    .await;
    let query =
        format!("{PENGUINS} |> AGGREGATE symbol(x => species, y => count(*) / body_mass_g)");
    fails(
        &query,
        "`body_mass_g` is neither a key of this chart nor inside an aggregate",
        "body_mass_g)",
    )
    .await;
}

#[tokio::test]
async fn v4_the_draw_order_adds_no_key() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*) ORDER BY species)"
    ))
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*) ORDER BY sex)");
    fails(
        &query,
        "the draw order `sex` is not a key of this chart",
        "sex)",
    )
    .await;
}

#[tokio::test]
async fn v5_scaled_and_direct_have_their_places() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), opacity => direct(0.5))"
    ))
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => species, y => direct(count(*)) + 1)");
    fails(
        &query,
        "scaled() and direct() are valid only as the complete value of a channel",
        "direct(",
    )
    .await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE text(x => species, y => count(*), text => scaled(species))"
    );
    fails(
        &query,
        "`text` has no scale, so its values are direct",
        "scaled",
    )
    .await;
}

#[tokio::test]
async fn v6_windows_have_over_and_the_names_of_the_mark() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER panel, y => count(*)) GROUP BY island"
    ))
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => species, y => rank())");
    fails(
        &query,
        "rank() is a window function, so it needs OVER",
        "rank",
    )
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER w, y => count(*))");
    fails(&query, "no window `w` in this chart", "w, y").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER (panel PARTITION BY sex), y => count(*))"
    );
    fails(&query, "the window `panel` has its partition", "sex)").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER (ORDER BY sex), y => count(*))"
    );
    fails(
        &query,
        "bin takes OVER (), OVER panel, or a window of only a PARTITION BY list",
        "sex)",
    )
    .await;
}

#[tokio::test]
async fn v8_options_are_constants() {
    charts(&format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), orient => 'vertical')"
    ))
    .await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g, maxbins => flipper_length_mm), y => count(*))"
    );
    fails(
        &query,
        "option `maxbins` of bin() needs a plan-time constant; `flipper_length_mm` is a column",
        "flipper_length_mm",
    )
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), orient => $1)");
    fails(
        &query,
        "option `orient` of bar() needs a plan-time constant; `$1` is a parameter",
        "$1",
    )
    .await;
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) OVER (PARTITION BY island)"
        ),
        "a mark call takes no OVER clause, so `bar` has none",
        "bar",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) GROUP BY island DESC"),
        "a group of an AGGREGATE with charts takes no ASC, DESC, or NULLS order",
        "island DESC",
    )
    .await;
}

#[tokio::test]
async fn v9_the_types_of_the_arguments_of_transforms() {
    charts("FROM sales |> AGGREGATE bar(x => timeunit(t, unit => 'month'), y => sum(units))").await;
    let query = format!("{PENGUINS} |> AGGREGATE bar(x => bin(species), y => count(*))");
    fails(&query, "bin bins numbers, not Utf8", "species)").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => timeunit(species, unit => 'month'), y => count(*))"
    );
    fails(
        &query,
        "timeunit takes a DATE or a TIMESTAMP, not Utf8",
        "species,",
    )
    .await;
    // The struct of a discretizer is the argument of the discretizer of the next depth.
    let query =
        "FROM sales |> AGGREGATE bar(x => bin(timeunit(t, unit => 'month')), y => sum(units))";
    fails(query, "bin bins numbers, not Struct", "timeunit(t").await;
}

/// The names of the columns of `query`.
async fn columns(query: &str) -> Vec<String> {
    let frame = avenger_sql::sql(&session(), query).await.unwrap();
    (frame.schema().fields().iter())
        .map(|field| field.name().clone())
        .collect()
}

#[tokio::test]
async fn chart_groups_its_rows_and_names_its_chart() {
    // One row for each group, with the group columns and the chart, named by its alias.
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) AS counts GROUP BY island"
    );
    assert_eq!(columns(&query).await, ["island", "counts"]);
    let lifting = avenger_sql::lift(&session(), &query).await.unwrap().text;
    let words: Vec<&str> = lifting.split_whitespace().collect();
    assert!(
        words.ends_with(&["AS", "counts", "GROUP", "BY", "island"]),
        "{lifting}"
    );
    // A group expression is a column of its alias, or of its text.
    let charts = lifted(
        "group-expression",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) GROUP BY upper(island) AS isle"
        ),
    )
    .await;
    assert_eq!(charts.len(), 3);
    let query =
        format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) GROUP BY upper(island)");
    assert_eq!(columns(&query).await, ["upper(island)", "bar"]);
    // A mark item without an alias takes the name of its mark, and two of one name are an error.
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)), symbol(x => species, y => count(*)) GROUP BY island"
    );
    assert_eq!(columns(&query).await, ["island", "bar", "symbol"]);
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)), bar(x => sex, y => count(*)) GROUP BY island"
        ),
        "`bar` names two columns of this AGGREGATE",
        "bar(x => sex",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) DESC"),
        "an item of an AGGREGATE with charts takes no ASC, DESC, or NULLS order",
        "bar",
    )
    .await;
}

#[tokio::test]
async fn a_stacked_histogram() {
    lifted(
        "stack-histogram",
        &format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => stack(count(*)), color => species)"),
    )
    .await;
}

#[tokio::test]
async fn a_horizontal_stack() {
    lifted(
        "stack-horizontal",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => stack(count(*)), y => species, color => island)"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_stack_with_an_order_of_its_own() {
    lifted(
        "stack-order",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)) OVER (stacks ORDER BY count(*) DESC), color => island)"
        ),
    )
    .await;
}

#[tokio::test]
async fn the_offsets_of_a_stack() {
    lifted(
        "stack-normalize",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*), offset => 'normalize'), color => island)"
        ),
    )
    .await;
    lifted(
        "stack-center",
        &format!(
            "{PENGUINS} |> AGGREGATE area(x => bin(body_mass_g), y => stack(count(*), offset => 'center'), color => species)"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_stacked_lump_with_its_other_group_at_the_bottom() {
    let charts = lifted(
        "stack-lump",
        &format!("{PENGUINS} |> AGGREGATE bar(x => island, y => stack(count(*)), color => lump(species, n => 1))"),
    )
    .await;
    // On Dream, the Other group, the chinstraps, is the bottom segment.
    let table = table(&charts[0]);
    let column = |name: &str| table.column_by_name(name).unwrap().clone();
    let (islands, lumps, stacks) = (column("island"), column("species_lump"), column("n_stack"));
    let islands = islands.as_any().downcast_ref::<StringArray>().unwrap();
    let lumps = lumps
        .as_any()
        .downcast_ref::<arrow::array::StructArray>()
        .unwrap();
    let others = lumps.column_by_name("other").unwrap();
    let stacks = stacks
        .as_any()
        .downcast_ref::<arrow::array::StructArray>()
        .unwrap();
    let starts = stacks.column_by_name("start").unwrap();
    let starts = starts.as_any().downcast_ref::<Float64Array>().unwrap();
    let other_start = (0..table.num_rows())
        .find(|row| islands.value(*row) == "Dream" && others.is_valid(*row))
        .map(|row| starts.value(row));
    assert_eq!(other_start, Some(0.0), "the Other group starts the stack");
}

#[tokio::test]
async fn a_stack_of_an_arc() {
    lifted(
        "stack-arc",
        &format!("{PENGUINS} |> AGGREGATE polar_bar(theta => stack(count(*)), color => species)"),
    )
    .await;
}

#[tokio::test]
async fn windows_of_aggregates_in_a_partitioned_call() {
    lifted(
        "window-panel",
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, y => count(*) / sum(count(*)) OVER panel) GROUP BY island"
        ),
    )
    .await;
    lifted(
        "window-all",
        &format!(
            "{PENGUINS} |> AGGREGATE symbol(x => species, y => sum(count(*)) OVER ()) GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn v6_the_stack_windows() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER stacks, y => stack(count(*)), color => species)"
    );
    fails(&query, "the window `stacks` holds the stacks", "stacks,").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)) OVER (stacked ORDER BY count(*)), color => island)"
    );
    fails(&query, "the window `stacked` has its order", "count(*)),").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)) OVER (stacks PARTITION BY island), color => island)"
    );
    fails(&query, "the window `stacks` has its partition", "island),").await;
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)) OVER piles, color => island)"
    );
    fails(
        &query,
        "no window `piles` in this chart; the windows are panel, stacks, and stacked",
        "piles",
    )
    .await;
}

#[tokio::test]
async fn v7_a_stack_has_an_order() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)) OVER stacks, color => island)"
    );
    fails(
        &query,
        "a stack needs an order: stack(n) OVER (stacks ORDER BY ...)",
        "stack(",
    )
    .await;
}

#[tokio::test]
async fn v11_a_stack_has_a_key_and_a_measure() {
    let query = format!("{PENGUINS} |> AGGREGATE bar(y => stack(count(*)), color => species)");
    fails(
        &query,
        "stack needs a key channel; this bar has no x",
        "stack(",
    )
    .await;
    let query = format!("{PENGUINS} |> AGGREGATE rule(x => species, y => stack(count(*)))");
    fails(
        &query,
        "stack needs a key channel and a measure channel",
        "stack(",
    )
    .await;
}

/// The start of the stack segment of `species` on `island` in the layer table of `chart`.
fn segment_start(chart: &Chart, island: &str, species: &str) -> f64 {
    let table = table(chart);
    let column = |name: &str| table.column_by_name(name).unwrap().clone();
    let (islands, kinds, stacks) = (column("island"), column("species"), column("n_stack"));
    let islands = islands.as_any().downcast_ref::<StringArray>().unwrap();
    let kinds = kinds.as_any().downcast_ref::<StringArray>().unwrap();
    let stacks = stacks
        .as_any()
        .downcast_ref::<arrow::array::StructArray>()
        .unwrap();
    let starts = stacks.column_by_name("start").unwrap();
    let starts = starts.as_any().downcast_ref::<Float64Array>().unwrap();
    let row = (0..table.num_rows())
        .find(|row| islands.value(*row) == island && kinds.value(*row) == species)
        .expect("a segment");
    starts.value(row)
}

#[tokio::test]
async fn the_chart_takes_the_orientation_of_its_stack() {
    // The stack decides that `y` is the key, although the chart alone would draw vertical bars of
    // two continuous positions.
    let charts = charts(&format!(
        "{PENGUINS} |> EXTEND CAST(flipper_length_mm / 10 AS BIGINT) AS f \
         |> AGGREGATE bar(x => stack(count(*)), y => f, color => species)"
    ))
    .await;
    let Some(proto::node::Node::Plot(plot)) = &charts[0].root().node else {
        panic!("a plot")
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot")
    };
    let Some(proto::cartesian_layer::Mark::Bar(bar)) = &cartesian.layers[0].mark else {
        panic!("a bar")
    };
    assert_eq!(bar.orient, proto::Orient::Horizontal as i32);
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => stack(count(*)), color => island, orient => 'horizontal')"
        ),
        "this bar measures `y`, so its orient is 'vertical', not 'horizontal'",
        "'horizontal'",
    )
    .await;
}

#[tokio::test]
async fn a_stack_follows_the_order_of_a_category_fit() {
    // A vertical stack puts the first category of the legend at the top, so the last is at the
    // bottom.
    let order = |order: &str| {
        format!(
            "{PENGUINS} |> AGGREGATE bar(x => island, y => stack(count(*)), \
             color => scaled(species, scale => category_fit(order => {order})))"
        )
    };
    let descending = charts(&order("'descending'")).await;
    assert_eq!(segment_start(&descending[0], "Dream", "Adelie"), 0.0);
    // The categories that the list leaves out follow it in ascending order.
    let preferred = charts(&order("['Gentoo']")).await;
    assert_eq!(segment_start(&preferred[0], "Dream", "Chinstrap"), 0.0);
    let ascending = charts(&order("'ascending'")).await;
    assert_eq!(segment_start(&ascending[0], "Dream", "Chinstrap"), 0.0);
}

#[tokio::test]
async fn the_options_of_the_plot_and_the_layer_pass_as_written() {
    lifted(
        "tools",
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), title => 'Penguins', id => 'bars', tools => [navigate(), tooltip()])"
        ),
    )
    .await;
}

/// The sales without the west region in March and April.
const SPARSE: &str = "FROM sales |> WHERE NOT (region = 'west' AND t >= TIMESTAMP '2024-03-01' AND t < TIMESTAMP '2024-05-01')";

/// The values of the column `name` of the table of the first layer of `chart`, as text.
fn values(chart: &Chart, name: &str) -> Vec<String> {
    let table = table(chart);
    let column = table
        .column_by_name(name)
        .unwrap_or_else(|| panic!("a column `{name}`: {:?}", table.schema()));
    (0..column.len())
        .map(|row| arrow::util::display::array_value_to_string(column, row).unwrap())
        .collect()
}

#[tokio::test]
async fn impute_adds_the_keys_that_a_group_lacks() {
    let charts = lifted(
        "impute-bar",
        &format!("{SPARSE} |> AGGREGATE bar(x => date_trunc('month', t), y => sum(units), color => region, impute => 0)"),
    )
    .await;
    // 6 months of 3 regions, with the 2 months of the west region that impute adds.
    assert_eq!(table(&charts[0]).num_rows(), 18);
    let added = values(&charts[0], "sum_units");
    assert_eq!(
        added.iter().filter(|value| *value == "0.0").count(),
        2,
        "{added:?}"
    );
}

#[tokio::test]
async fn impute_fills_the_units_of_a_stacked_area() {
    let charts = lifted(
        "impute-stacked-area",
        &format!("{SPARSE} |> AGGREGATE area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region, impute => 0)"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
}

#[tokio::test]
async fn impute_fills_the_empty_bins_of_a_histogram() {
    let query = |impute: &str| {
        format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g, maxbins => 40), y => stack(count(*)), color => species{impute})")
    };
    let without = charts(&query("")).await;
    let charts = lifted("impute-bins", &query(", impute => 0")).await;
    let (bins, filled) = (table(&without[0]).num_rows(), table(&charts[0]).num_rows());
    assert!(filled > bins, "{filled} rows, {bins} without impute");
    assert_eq!(filled % 3, 0, "every bin of every species");
}

#[tokio::test]
async fn impute_fills_the_bins_of_each_panel() {
    let charts = lifted(
        "impute-bins-panel",
        &format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER panel, y => stack(count(*)), color => sex, impute => 0) GROUP BY island"),
    )
    .await;
    assert_eq!(charts.len(), 3);
}

#[tokio::test]
async fn impute_takes_the_keys_of_an_explicit_domain() {
    let charts = lifted(
        "impute-domain",
        &format!("{SPARSE} |> WHERE region <> 'north' |> AGGREGATE bar(x => scaled(region, scale => scale(domain => ['east', 'west', 'north'])), y => sum(units), impute => 0)"),
    )
    .await;
    assert_eq!(values(&charts[0], "region").len(), 3, "north, with 0 units");
    // `keys` replaces the keys of the domain, and `extra_keys` adds to them.
    let charts = lifted(
        "impute-extra-keys",
        &format!("{SPARSE} |> AGGREGATE bar(x => region, y => sum(units), impute => impute(value => 0, extra_keys => ['south']))"),
    )
    .await;
    assert_eq!(values(&charts[0], "region").len(), 4);
}

#[tokio::test]
async fn impute_computes_a_statistic_or_a_gap() {
    let charts = lifted(
        "impute-mean",
        &format!("{SPARSE} |> AGGREGATE line(x => date_trunc('month', t), y => sum(units), color => region, impute => impute(method => 'mean', frame => [-1, 1]))"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
    let charts = lifted(
        "impute-gap",
        &format!("{SPARSE} |> AGGREGATE line(x => date_trunc('month', t), y => sum(units), color => region, impute => impute(method => 'gap'))"),
    )
    .await;
    let added = values(&charts[0], "sum_units");
    assert_eq!(
        added.iter().filter(|value| value.is_empty()).count(),
        2,
        "{added:?}"
    );
}

#[tokio::test]
async fn a_key_of_the_imputed_key_computes_after_the_imputation() {
    let charts = lifted(
        "impute-dependent",
        &format!("{SPARSE} |> AGGREGATE text(x => date_trunc('month', t), y => sum(units), color => region, text => to_char(date_trunc('month', t), '%b'), impute => 0)"),
    )
    .await;
    let months = values(&charts[0], "text_expr");
    assert_eq!(months.len(), 18);
    assert!(months.iter().all(|month| !month.is_empty()), "{months:?}");
}

#[tokio::test]
async fn impute_in_a_call_that_does_not_group() {
    let charts = lifted(
        "impute-rows",
        &format!("{SPARSE} |> AGGREGATE sum(units) AS units GROUP BY date_trunc('month', t) AS month, region |> AGGREGATE line(x => month, y => units, color => region, impute => 0)"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
}

#[tokio::test]
async fn v10_the_impute_option() {
    let bar =
        |impute: &str| format!("{SPARSE} |> AGGREGATE bar(x => region, y => sum(units), {impute})");
    fails(
        &format!("{SPARSE} |> AGGREGATE polar_bar(theta => region, radius => sum(units), impute => 0)"),
        "`impute` is valid only on bar(), area(), line(), symbol(), and text(); found on polar_bar()",
        "impute =>",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), impute => island)"),
        "option `impute` of bar() needs a plan-time constant; `island` is a column",
        "island",
    )
    .await;
    fails(
        &bar("impute => impute(frame => [-1, 1])"),
        "an impute() option needs exactly one of `value` and `method`",
        "impute(frame",
    )
    .await;
    fails(
        &bar("impute => impute(value => 0, method => 'mean')"),
        "exactly one of `value` and `method`",
        "impute(value",
    )
    .await;
    fails(
        &bar("impute => impute(method => 'linear')"),
        "`method` must be 'mean', 'median', 'min', 'max', or 'gap'; found 'linear'",
        "'linear'",
    )
    .await;
    fails(
        &bar("impute => impute(value => 0, frame => [-1, 1])"),
        "`frame` needs a method of 'mean', 'median', 'min', or 'max'; found `value`",
        "frame",
    )
    .await;
    fails(
        &bar("impute => impute(method => 'gap', frame => [-1, 1])"),
        "found 'gap'",
        "frame",
    )
    .await;
    fails(
        &bar("impute => impute(method => 'mean', frame => [1, -1])"),
        "the start of `frame`, 1, is after its end, -1",
        "frame",
    )
    .await;
    fails(
        &bar("impute => impute(method => 'mean', frame => [1.5, 2])"),
        "`frame` is a list of two bounds, each an integer or NULL",
        "1.5",
    )
    .await;
    fails(
        &bar("impute => impute(value => 0, limit => 3)"),
        "impute() has no argument `limit`",
        "limit",
    )
    .await;
    fails(
        &format!("{SPARSE} |> AGGREGATE bar(y => sum(units), impute => 0)"),
        "`impute` needs a key channel; this bar has no x",
        "impute =>",
    )
    .await;
    fails(
        &format!("{SPARSE} |> AGGREGATE bar(x => region, y => sum(units) / 2, impute => 0)"),
        "`impute` fills the measure of bar(), which is one aggregate, such as sum(units), in a chart that aggregates; found `sum(units) / 2`",
        "sum(units) / 2",
    )
    .await;
    fails(
        &format!("{SPARSE} |> AGGREGATE line(x => t, y => units * 2, impute => 0)"),
        "a column in a chart that does not aggregate; found `units * 2`",
        "units * 2",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g) OVER (PARTITION BY sex), y => count(*), impute => 0)"),
        "`impute` fills the bins of each `sex`, which is not a key of this chart",
        "sex)",
    )
    .await;
    // The option lifts a mark call that does not aggregate, which imputes its rows.
    assert!(
        !charts("FROM sales |> AGGREGATE bar(x => region, y => units, impute => 0) AS chart")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn order_is_the_path_order_of_a_line() {
    let charts = lifted(
        "path-order",
        &format!("{PENGUINS} |> AGGREGATE line(x => avg(bill_length_mm), y => avg(bill_depth_mm), order => body_mass_g)"),
    )
    .await;
    assert!(table(&charts[0]).num_rows() > 1);
    // `order` is a channel of the paths only.
    fails(
        &format!("{PENGUINS} |> AGGREGATE bar(x => species, y => count(*), order => island)"),
        "bar() has no parameter `order`",
        "order =>",
    )
    .await;
}

/// The lifting of `query`.
async fn lifting(query: &str) -> String {
    avenger_sql::lift(&session(), query).await.unwrap().text
}

#[tokio::test]
async fn a_stacked_path_imputes_zero_by_default() {
    let area = |impute: &str| {
        format!("{SPARSE} |> AGGREGATE area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region{impute})")
    };
    let charts = lifted("default-stacked-area", &area("")).await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
    assert_eq!(charts, self::charts(&area(", impute => 0")).await);
    // `impute => NULL` is an omitted option.
    assert_eq!(
        lifting(&area(", impute => NULL")).await,
        lifting(&area("")).await
    );
    // A stacked line imputes too, although the format draws no line of spans yet.
    let line = lifting(&format!("{SPARSE} |> AGGREGATE line(x => date_trunc('month', t), y => stack(sum(units)), color => region)")).await;
    assert!(
        line.contains("|> CALL impute(x_expr, [region], sum_units, value => 0, flag => imputed)"),
        "{line}"
    );
    // A stacked histogram area fills its empty bins.
    let charts = lifted(
        "default-stacked-bins",
        &format!("{PENGUINS} |> AGGREGATE area(x => bin(body_mass_g, maxbins => 40), y => stack(count(*)), color => species)"),
    )
    .await;
    assert_eq!(
        table(&charts[0]).num_rows() % 3,
        0,
        "every bin of every species"
    );
}

#[tokio::test]
async fn the_default_imputes_only_where_it_applies() {
    let unimputed = [
        // Turned off.
        format!("{SPARSE} |> AGGREGATE area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region, impute => FALSE)"),
        // A path that does not stack, and a stacked bar.
        format!("{SPARSE} |> AGGREGATE area(x => timeunit(t, unit => 'month'), y => sum(units), color => region)"),
        format!("{SPARSE} |> AGGREGATE bar(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region)"),
        // One group, whose keys are the distinct keys of the input.
        format!("{PENGUINS} |> AGGREGATE area(x => species, y => stack(count(*)))"),
        // Where the written option would be an error: a measure of two aggregates, and the bins of
        // each sex, which is not a key.
        format!("{PENGUINS} |> AGGREGATE area(x => species, y => stack(count(*) / 2), color => island)"),
        format!("{PENGUINS} |> AGGREGATE area(x => bin(body_mass_g) OVER (PARTITION BY sex), y => stack(count(*)), color => species)"),
    ];
    for query in unimputed {
        let lifting = lifting(&query).await;
        assert!(!lifting.contains("CALL impute"), "{query}: {lifting}");
        assert!(!charts(&query).await.is_empty(), "{query}");
    }
    // The mark aggregates have no default.
    let query = "FROM sales |> AGGREGATE sum(units) AS units GROUP BY region, t |> AGGREGATE area(x => t, y => units, color => region) AS chart";
    assert_eq!(charts(query).await.len(), 1);
    // FALSE on a mark that does not impute is the error of the option.
    fails(
        &format!("{PENGUINS} |> AGGREGATE polar_bar(theta => species, radius => count(*), impute => FALSE)"),
        "`impute` is valid only on bar(), area(), line(), symbol(), and text(); found on polar_bar()",
        "impute =>",
    )
    .await;
}

/// The rows of `query`, as lines of text in their order.
async fn text_rows(query: &str) -> Vec<String> {
    let batches = avenger_sql::sql(&session(), query)
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"))
        .collect()
        .await
        .unwrap();
    let text = arrow::util::pretty::pretty_format_batches(&batches)
        .unwrap()
        .to_string();
    text.lines().map(String::from).collect()
}

/// A histogram of the body masses of each island, with `items` after it.
fn histograms(items: &str, groups: &str) -> String {
    format!(
        "{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => count(*)) AS chart{items}{groups}"
    )
}

#[tokio::test]
async fn aggregate_items_join_the_charts_of_their_groups() {
    let query = histograms(
        ", count(*) AS penguins, avg(body_mass_g) AS mean_mass",
        " GROUP BY island",
    );
    let charts = lifted("items", &format!("{query} |> ORDER BY island")).await;
    assert_eq!(
        columns(&query).await,
        ["island", "chart", "penguins", "mean_mass"]
    );
    // The charts are those of the call without the items, and the items those of an AGGREGATE.
    assert_eq!(
        charts,
        self::charts(&format!(
            "{} |> ORDER BY island",
            histograms("", " GROUP BY island")
        ))
        .await
    );
    assert_eq!(
        text_rows(&format!("{query} |> SELECT island, penguins, mean_mass |> ORDER BY island")).await,
        text_rows(&format!("{PENGUINS} |> AGGREGATE count(*) AS penguins, avg(body_mass_g) AS mean_mass GROUP BY island |> ORDER BY island")).await
    );
    // The mark call takes its place among the items.
    let query = histograms(", count(*) AS penguins", " GROUP BY island").replace(
        "|> AGGREGATE bar(",
        "|> AGGREGATE count(DISTINCT species) AS species, bar(",
    );
    assert_eq!(
        columns(&query).await,
        ["island", "species", "chart", "penguins"]
    );
}

#[tokio::test]
async fn every_aggregate_of_aggregate_is_an_item() {
    let items = "count(DISTINCT species) AS species, median(body_mass_g) AS median_mass, \
        count(*) FILTER (WHERE sex = 'male') AS males, \
        array_agg(species ORDER BY body_mass_g, species) AS lightest_first, \
        percentile_cont(0.5) WITHIN GROUP (ORDER BY body_mass_g) AS percentile, \
        island || ' (' || CAST(count(*) AS VARCHAR) || ')' AS label, count(*)";
    let query = histograms(&format!(", {items}"), " GROUP BY island");
    assert_eq!(
        text_rows(&format!(
            "{query} |> SELECT * EXCLUDE (chart) |> ORDER BY island"
        ))
        .await,
        text_rows(&format!(
            "{PENGUINS} |> AGGREGATE {items} GROUP BY island |> ORDER BY island"
        ))
        .await
    );
    // An unaliased item is named by its text.
    assert_eq!(columns(&query).await.last().unwrap(), "count(*)");
}

#[tokio::test]
async fn a_null_group_has_its_chart_and_its_items() {
    let query = histograms(", count(*) AS penguins", " GROUP BY sex");
    let rows = text_rows(&format!(
        "{query} |> SELECT sex, chart IS NOT NULL AS charted, penguins |> ORDER BY sex"
    ))
    .await;
    assert!(
        rows.iter()
            .any(|row| row.starts_with("|        | true    | 9 ")),
        "{rows:#?}"
    );
    assert_eq!(charts(&query).await.len(), 3);
    // Without groups, the call has one row.
    let query = histograms(", count(*) AS penguins", "");
    assert_eq!(columns(&query).await, ["chart", "penguins"]);
    let charts = lifted("items-without-groups", &query).await;
    assert_eq!(charts.len(), 1);
    // A group expression, with an item.
    lifted(
        "items-group-expression",
        &histograms(
            ", count(*) AS penguins",
            " GROUP BY upper(island) AS isle |> ORDER BY isle",
        ),
    )
    .await;
}

#[tokio::test]
async fn the_lifting_of_items_takes_free_names() {
    // The columns `aggregates` and `charts` take the names of the relations of the join.
    let query = "FROM penguins |> WHERE body_mass_g IS NOT NULL \
        |> EXTEND 1 AS aggregates, 2 AS charts \
        |> AGGREGATE bar(x => species, y => count(*)), count(*) AS penguins GROUP BY island";
    let lifting = avenger_sql::lift(&session(), query).await.unwrap().text;
    assert!(lifting.contains("|> AS aggregates_2"), "{lifting}");
    assert!(lifting.contains(") AS charts_2 ON"), "{lifting}");
    assert_eq!(charts(query).await, charts(&lifting).await);
}

#[tokio::test]
async fn the_errors_of_items() {
    // An item reads columns inside its aggregates or groups, and calls no window function.
    fails(
        &histograms(", species AS name", " GROUP BY island"),
        "`species` is neither a group of this AGGREGATE nor inside an aggregate",
        "species AS",
    )
    .await;
    fails(
        &histograms(", row_number() OVER () AS r", " GROUP BY island"),
        "an item of AGGREGATE is an aggregate, not the window function `row_number`",
        "row_number",
    )
    .await;
}

/// The bills and the counts of the species of each island, as two mark items of one AGGREGATE, with
/// `items` between them.
fn bills_and_counts(items: &str, groups: &str) -> String {
    format!("{PENGUINS} |> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS bills{items}, bar(x => species, y => count(*)) AS counts{groups}")
}

#[tokio::test]
async fn several_mark_calls_draw_a_chart_each() {
    let query = format!(
        "{} |> ORDER BY island",
        bills_and_counts("", " GROUP BY island")
    );
    let charts = lifted_in("several-marks", &query, &["bills", "counts"]).await;
    assert_eq!(columns(&query).await, ["island", "bills", "counts"]);
    // The charts are those of two AGGREGATE operators with the same groups.
    let alone =
        |mark: &str| format!("{PENGUINS} |> AGGREGATE {mark} GROUP BY island |> ORDER BY island");
    assert_eq!(
        charts[0],
        self::charts(&alone(
            "symbol(x => bill_length_mm, y => bill_depth_mm, color => species)"
        ))
        .await
    );
    assert_eq!(
        charts[1],
        self::charts(&alone("bar(x => species, y => count(*))")).await
    );
    // An aggregate item between the charts takes its place among the items.
    let query = bills_and_counts(", count(*) AS penguins", " GROUP BY island");
    assert_eq!(
        columns(&query).await,
        ["island", "bills", "penguins", "counts"]
    );
    lifted_in(
        "several-marks-items",
        &format!("{query} |> ORDER BY island"),
        &["bills", "counts"],
    )
    .await;
}

#[tokio::test]
async fn several_mark_calls_join_on_every_group() {
    // Without groups, one row.
    let query = bills_and_counts("", "");
    assert_eq!(columns(&query).await, ["bills", "counts"]);
    assert_eq!(
        text_rows(&format!("{query} |> AGGREGATE count(*) AS n")).await[3],
        "| 1 |"
    );
    // A NULL group has both of its charts.
    let query = bills_and_counts("", " GROUP BY sex");
    let rows = text_rows(&format!(
        "{query} |> SELECT sex, bills IS NOT NULL AS b, counts IS NOT NULL AS c |> ORDER BY sex"
    ))
    .await;
    assert!(
        rows.iter()
            .any(|row| row.starts_with("|        | true | true |")),
        "{rows:#?}"
    );
}

/// The `FILTER` of a mark call selects the rows of every stage, so the chart is the chart of the
/// rows that a `WHERE` keeps: on a column that is not a key, and in a stack, whose offsets count
/// only the selected rows.
#[tokio::test]
async fn a_filter_selects_the_rows_of_every_stage() {
    for (mark, condition) in [
        ("bar(x => species, y => count(*))", "sex = 'male'"),
        (
            "bar(x => island, y => stack(count(*)), color => species)",
            "species <> 'Gentoo'",
        ),
        (
            "bar(x => island, y => CAST(count(*) AS DOUBLE) / sum(count(*)) OVER ())",
            "species <> 'Adelie'",
        ),
    ] {
        let filtered = charts(&format!(
            "{PENGUINS} |> AGGREGATE {mark} FILTER (WHERE {condition})"
        ))
        .await;
        let selected = charts(&format!(
            "{PENGUINS} |> WHERE {condition} |> AGGREGATE {mark}"
        ))
        .await;
        assert_eq!(filtered, selected, "{mark} FILTER (WHERE {condition})");
    }
}

/// A group of whose rows the `FILTER` of a mark call selects none keeps its row, with no chart of
/// that mark, also when the mark call is the first.
#[tokio::test]
async fn a_filter_keeps_every_group() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) FILTER (WHERE species = 'Gentoo') \
         AS gentoo GROUP BY island |> SELECT island, gentoo IS NOT NULL AS drawn |> ORDER BY island"
    );
    let frame = avenger_sql::sql(&session(), &query).await.unwrap();
    let batches = frame.collect().await.unwrap();
    let rows: Vec<(String, bool)> = (batches.iter())
        .flat_map(|batch| {
            let islands = batch.column(0).as_string::<i32>().clone();
            let drawn = batch.column(1).as_boolean().clone();
            (0..batch.num_rows()).map(move |row| (islands.value(row).to_string(), drawn.value(row)))
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Biscoe".to_string(), true),
            ("Dream".to_string(), false),
            ("Torgersen".to_string(), false)
        ]
    );
}

#[tokio::test]
async fn the_names_of_the_items_are_unique() {
    fails(
        &bills_and_counts(", count(*) AS bills", " GROUP BY island"),
        "`bills` names two columns of this AGGREGATE",
        "bills, bar",
    )
    .await;
    // The second of two columns of one name, in the order of the text, is the error, and the
    // columns have no qualifiers of the input.
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) AS island GROUP BY penguins.island"
        ),
        "`island` names two columns of this AGGREGATE",
        "penguins.island",
    )
    .await;
    fails(
        "FROM penguins AS a |> JOIN penguins AS b USING (species) \
         |> AGGREGATE bar(x => species, y => count(*)), count(*) AS n GROUP BY a.island, b.island",
        "`island` names two columns of this AGGREGATE",
        "b.island",
    )
    .await;
    fails(
        &histograms(", count(*) AS Island", " GROUP BY island"),
        "`island` names two columns of this AGGREGATE",
        "island",
    )
    .await;
}

/// An unquoted alias of an item or of the chart names its column in lowercase, as DataFusion
/// normalizes an identifier, so a reference of any case finds it, and a quoted alias keeps its
/// case. The lifting keeps the qualifiers of the input's tables.
#[tokio::test]
async fn items_take_normalized_names_and_keep_qualifiers() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE bar(x => species, y => count(*)) AS Counts, count(*) AS Penguins, \
         count(*) AS \"Total\" GROUP BY Island |> ORDER BY PENGUINS"
    );
    assert_eq!(
        columns(&query).await,
        ["island", "counts", "penguins", "Total"]
    );
    let query = "FROM penguins AS p |> WHERE p.body_mass_g IS NOT NULL \
        |> AGGREGATE bar(x => p.species, y => count(*)), count(*) AS n GROUP BY p.island";
    let lifting = avenger_sql::lift(&session(), query).await.unwrap().text;
    assert_eq!(charts(query).await, charts(&lifting).await, "{lifting}");
}

/// The bars of the mean body mass of each species and a rule at the mean of all, as named mark
/// calls of one AGGREGATE.
const BARS: &str = "bar(x => species, y => avg(body_mass_g))";
const MEAN: &str = "rule(y => avg(body_mass_g))";

#[tokio::test]
async fn an_expression_item_composes_its_mark_calls() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE layer({BARS}, {MEAN}) AS chart GROUP BY island |> ORDER BY island"
    );
    let charts = lifted("expression-item", &query).await;
    assert_eq!(columns(&query).await, ["island", "chart"]);
    // The item is the composition of the columns of its mark calls.
    let columns_first = format!(
        "{PENGUINS} |> AGGREGATE {BARS} AS bars, {MEAN} AS mean GROUP BY island \
         |> SELECT island, layer(bars, mean) AS chart |> ORDER BY island"
    );
    assert_eq!(charts, self::charts(&columns_first).await);
    // An item of charts without an alias takes the name of its function, and an aggregate item
    // takes its place.
    let query =
        format!("{PENGUINS} |> AGGREGATE count(*) AS n, layer({BARS}, {MEAN}) GROUP BY island");
    assert_eq!(columns(&query).await, ["island", "n", "layer"]);
    lifted(
        "expression-item-aggregate",
        &format!("{query} |> ORDER BY island"),
    )
    .await;
}

#[tokio::test]
async fn expression_items_nest_and_take_options() {
    let query = format!(
        "{PENGUINS} |> AGGREGATE hconcat(layer({BARS}, {MEAN}), symbol(x => bill_length_mm, y => bill_depth_mm), \
         layout => concat_layout(gap => 24)) AS chart GROUP BY island |> ORDER BY island"
    );
    let charts = lifted("expression-item-nested", &query).await;
    // A call in an option stays in the expression, as without groups.
    let tools =
        format!("{PENGUINS} |> AGGREGATE layer({BARS}, {MEAN}, tools => [navigate(), tooltip()])");
    assert_eq!(
        self::charts(&tools).await,
        self::charts(&format!(
            "{PENGUINS} |> AGGREGATE {BARS} AS bars, {MEAN} AS mean \
             |> SELECT layer(bars, mean, tools => [navigate(), tooltip()]) AS chart"
        ))
        .await
    );
    let columns_first = format!(
        "{PENGUINS} |> AGGREGATE {BARS} AS bars, {MEAN} AS mean, \
         symbol(x => bill_length_mm, y => bill_depth_mm) AS bills GROUP BY island \
         |> SELECT island, hconcat(layer(bars, mean), bills, layout => concat_layout(gap => 24)) AS chart \
         |> ORDER BY island"
    );
    assert_eq!(charts, self::charts(&columns_first).await);
    // Two items of charts, each with a name.
    let query = format!(
        "{PENGUINS} |> AGGREGATE layer({BARS}, {MEAN}) AS means, hconcat({BARS}, {MEAN}) AS side \
         GROUP BY island |> ORDER BY island"
    );
    let charts = lifted_in("expression-items", &query, &["means", "side"]).await;
    assert_ne!(charts[0], charts[1]);
}

#[tokio::test]
async fn an_expression_item_reads_aggregates_outside_its_mark_calls() {
    // The islands of more than 100 penguins have bars, and the others have the bills.
    let bills = "symbol(x => bill_length_mm, y => bill_depth_mm)";
    let query = format!(
        "{PENGUINS} |> AGGREGATE CASE WHEN count(*) > 100 THEN {BARS} ELSE {bills} END AS chart \
         GROUP BY island |> ORDER BY island"
    );
    let charts = lifted("expression-item-case", &query).await;
    let alone =
        |mark: &str| format!("{PENGUINS} |> AGGREGATE {mark} GROUP BY island |> ORDER BY island");
    let bars = self::charts(&alone(BARS)).await;
    let bills = self::charts(&alone(bills)).await;
    assert_eq!(charts, [bars[0].clone(), bars[1].clone(), bills[2].clone()]);
    // A composition reads the charts of a `CASE`, and an island without bars has the mean only.
    let query = format!(
        "{PENGUINS} |> AGGREGATE layer(CASE WHEN count(*) > 100 THEN {BARS} END, {MEAN}) AS chart \
         GROUP BY island |> ORDER BY island"
    );
    let charts = lifted("expression-item-case-layer", &query).await;
    let columns_first = format!(
        "{PENGUINS} |> AGGREGATE count(*) AS n, {BARS} AS bars, {MEAN} AS mean GROUP BY island \
         |> SELECT island, layer(CASE WHEN n > 100 THEN bars END, mean) AS chart |> ORDER BY island"
    );
    assert_eq!(charts, self::charts(&columns_first).await);
    assert_eq!(charts[2], self::charts(&alone(MEAN)).await[2]);
}

#[tokio::test]
async fn the_errors_of_expression_items() {
    // An item of charts without an alias takes the name of its function, and an item that is not a
    // call needs one.
    fails(
        &format!("{PENGUINS} |> AGGREGATE layer({BARS}, {MEAN}), hconcat({BARS}) AS layer GROUP BY island"),
        "`layer` names two columns of this AGGREGATE",
        "layer GROUP",
    )
    .await;
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE CASE WHEN count(*) > 100 THEN {BARS} END GROUP BY island"
        ),
        "this item holds a mark call, so it needs a name",
        "CASE",
    )
    .await;
    // A column outside a mark call is a group or inside an aggregate.
    fails(
        &format!("{PENGUINS} |> AGGREGATE coalesce({BARS}, species) AS chart GROUP BY island"),
        "`species` is neither a group of this AGGREGATE nor inside an aggregate",
        "species) AS",
    )
    .await;
    // An item composes the charts of its group, so no aggregate takes a mark call.
    fails(
        &format!("{PENGUINS} |> AGGREGATE layer_agg({BARS}) AS chart GROUP BY island"),
        "`layer_agg` reads the values of the rows, so it takes no mark call `bar`",
        "bar(",
    )
    .await;
    // A mark call in an expression takes no OVER clause.
    fails(
        &format!(
            "{PENGUINS} |> AGGREGATE layer({BARS}, rule(y => avg(body_mass_g)) OVER ()) AS chart"
        ),
        "a mark call takes no OVER clause, so `rule` has none",
        "rule",
    )
    .await;
}

#[tokio::test]
async fn the_lifting_of_a_lifting_is_itself() {
    for query in [
        format!("{PENGUINS} |> AGGREGATE bar(x => bin(body_mass_g), y => stack(count(*)), color => species)"),
        format!("{PENGUINS} |> AGGREGATE count(*) AS n, symbol(x => bill_length_mm, y => bill_depth_mm) AS bills, bar(x => species, y => count(*)) AS counts GROUP BY island"),
        format!("{PENGUINS} |> AGGREGATE layer(bar(x => species, y => avg(body_mass_g)), rule(y => avg(body_mass_g))) GROUP BY island"),
        "FROM sales |> AGGREGATE area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region)".to_string(),
    ] {
        let lifting = avenger_sql::lift(&session(), &query).await.unwrap().text;
        assert_eq!(
            avenger_sql::lift(&session(), &lifting).await.unwrap().text,
            lifting,
            "{query}"
        );
    }
}

/// The text of `lifted` outside the ranges that `lift` wrote.
fn kept(lifted: &avenger_sql::Lifted) -> String {
    let mut kept = String::new();
    let mut at = 0;
    for range in &lifted.generated {
        kept.push_str(&lifted.text[at..range.start]);
        at = range.end;
    }
    kept.push_str(&lifted.text[at..]);
    kept
}

#[tokio::test]
async fn a_query_with_nothing_to_lift_comes_back_as_written() {
    let query = "FROM penguins -- the penguins\n|> WHERE body_mass_g IS NOT NULL,\n   |> AGGREGATE count(*) AS n, symbol(x => bill_length_mm, y => bill_depth_mm) GROUP BY island";
    let lifted = avenger_sql::lift(&session(), query).await.unwrap();
    assert_eq!(lifted.text, query);
    assert!(lifted.generated.is_empty());
}

#[tokio::test]
async fn lift_keeps_the_text_before_and_after_a_lifted_operator() {
    let before = "FROM penguins  -- the penguins\n|> WHERE body_mass_g IS NOT NULL,\n";
    let operator = "|> AGGREGATE bar(x => species, y => count(*),) AS bars GROUP BY island";
    let after = "\n|>   ORDER BY island";
    let query = format!("{before}{operator}{after}");
    let lifted = avenger_sql::lift(&session(), &query).await.unwrap();
    assert!(lifted.text.starts_with(before), "{}", lifted.text);
    assert!(lifted.text.ends_with(after), "{}", lifted.text);
    // The stages are one range in place of the operator.
    assert_eq!(
        lifted.generated,
        vec![before.len()..lifted.text.len() - after.len()]
    );
    assert_eq!(kept(&lifted), format!("{before}{after}"));
    assert_eq!(charts(&query).await, charts(&lifted.text).await);
}

#[tokio::test]
async fn the_ranges_of_lift_cover_the_stages_and_the_copies_of_the_input() {
    // With an aggregate item, the target of the join holds a copy of the input, which `lift`
    // wrote, and that copy ends in a comment of the text.
    let before = "FROM penguins -- the penguins\n";
    let query = format!(
        "{before}|> AGGREGATE count(*) AS n, bar(x => species, y => count(*)) AS bars GROUP BY island"
    );
    let lifted = avenger_sql::lift(&session(), &query).await.unwrap();
    assert_eq!(lifted.generated, vec![before.len()..lifted.text.len()]);
    let stages = &lifted.text[before.len()..];
    assert!(
        stages.contains("LEFT OUTER JOIN (\n     FROM penguins -- the penguins\n"),
        "{stages}"
    );
    assert_eq!(charts(&query).await, charts(&lifted.text).await);
    // A lifted AGGREGATE in the target of a join of the text lifts in place.
    let target =
        "(FROM penguins |> AGGREGATE bar(x => species, y => count(*)) AS chart GROUP BY island)";
    let query =
        format!("FROM (VALUES ('Dream')) AS i(island) |> JOIN {target} AS c USING (island)");
    let lifted = avenger_sql::lift(&session(), &query).await.unwrap();
    assert_eq!(lifted.generated.len(), 1);
    assert_eq!(
        kept(&lifted),
        "FROM (VALUES ('Dream')) AS i(island) |> JOIN (FROM penguins ) AS c USING (island)"
    );
    assert_eq!(charts(&query).await, charts(&lifted.text).await);
    let again = avenger_sql::lift(&session(), &lifted.text).await.unwrap();
    assert_eq!(again.text, lifted.text);
    assert!(again.generated.is_empty());
}
