//! `|> CHART`: each chart call draws the chart of its printed lifting run as a query
//! (`r[lift.test.bytes]`), and `lift` prints the lifting that `tests/baselines/liftings/` holds,
//! which a change accepts by copying it from `tests/output/liftings/`. The validation rules have
//! one valid and one invalid call each, with the error and its span.

mod common;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray, TimestampMillisecondArray},
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

/// The charts of `query`, in the order of its rows.
async fn charts(query: &str) -> Vec<Chart> {
    let batches = avenger_sql::sql(&session(), query)
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"))
        .collect()
        .await
        .unwrap_or_else(|error| panic!("{query}: {error}"));
    let mut charts = Vec::new();
    for batch in batches {
        let column = batch.column_by_name("chart").expect("a chart column");
        for chart in charts_from_arrow(column, Decode::Auto).unwrap() {
            charts.push(chart.unwrap());
        }
    }
    charts
}

/// Checks that `query` draws the charts of its lifting, and that `lift` prints the lifting of the
/// baseline `name`. It returns the charts.
async fn lifted(name: &str, query: &str) -> Vec<Chart> {
    let lifting = avenger_sql::lift(&session(), query).await.unwrap();
    assert!(!lifting.contains("__avenger_"), "no marker in {lifting}");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let output = format!("{dir}/output/liftings/{name}.sql");
    std::fs::create_dir_all(format!("{dir}/output/liftings")).unwrap();
    std::fs::write(&output, format!("{lifting}\n")).unwrap();
    let baseline = std::fs::read_to_string(format!("{dir}/baselines/liftings/{name}.sql"))
        .unwrap_or_else(|_| panic!("no baseline of {name}: accept {output}"));
    assert_eq!(baseline.trim_end(), lifting, "the lifting of {name}");
    let chart = charts(query).await;
    assert!(!chart.is_empty(), "{query}");
    assert_eq!(chart, charts(&lifting).await, "the charts of {name}");
    chart
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
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*))"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 3);
    let charts = lifted(
        "counts-by-island",
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) GROUP BY island"),
    )
    .await;
    assert_eq!(charts.len(), 3, "one chart for each island");
}

#[tokio::test]
async fn a_histogram() {
    let charts = lifted(
        "histogram",
        &format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g, maxbins => 20), y => count(*))"),
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
            "{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER panel, y => count(*)) GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_lump_color() {
    lifted(
        "lump",
        &format!(
            "{PENGUINS} |> CHART bar(x => island, y => count(*), color => lump(species, n => 2, weight => sum(body_mass_g)))"
        ),
    )
    .await;
}

#[tokio::test]
async fn months_of_timestamps() {
    let charts = lifted(
        "months",
        "FROM sales |> CHART bar(x => timeunit(t, unit => 'month'), y => sum(units))",
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 6, "January to June");
}

#[tokio::test]
async fn nested_discretizers() {
    lifted(
        "nested",
        "FROM sales |> CHART bar(x => lump(timeunit(t, unit => 'month'), n => 3, weight => sum(units)), y => sum(units))",
    )
    .await;
}

#[tokio::test]
async fn a_key_expression() {
    lifted(
        "key-expression",
        &format!("{PENGUINS} |> CHART bar(x => upper(species), y => count(*))"),
    )
    .await;
}

#[tokio::test]
async fn a_post_aggregate_expression_and_two_uses_of_one_aggregate() {
    lifted(
        "post-aggregate",
        &format!("{PENGUINS} |> CHART symbol(x => species, y => count(*) / 2, size => count(*))"),
    )
    .await;
}

#[tokio::test]
async fn a_draw_order() {
    lifted(
        "order-aggregate",
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*) ORDER BY count(*) DESC)"),
    )
    .await;
    lifted(
        "order-key",
        &format!(
            "{PENGUINS} |> CHART bar(x => species, y => count(*), color => island ORDER BY island DESC)"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_call_that_does_not_group() {
    let charts = lifted(
        "rows",
        &format!(
            "{PENGUINS} |> CHART symbol(x => bill_length_mm * 2, y => bill_depth_mm, color => lump(species, n => 2))"
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
        &format!("{PENGUINS} |> EXTEND 1 AS n |> CHART bar(x => species, y => count(*))"),
    )
    .await;
}

#[tokio::test]
async fn the_output_of_chart_composes() {
    // The partition is the key of a facet.
    let query = format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g), y => count(*)) GROUP BY island |> AGGREGATE facet_col(chart, island) AS chart"
    );
    let composed = charts(&query).await;
    let Some(proto::node::Node::Concat(concat)) = &composed[0].root().node else {
        panic!("a concat")
    };
    assert_eq!(concat.children.len(), 3);
    // A layer of the charts of a union draws them in the order of its branches.
    let query = format!(
        "(SELECT * FROM ({PENGUINS} |> CHART bar(x => species, y => count(*)))) UNION ALL (SELECT * FROM ({PENGUINS} |> CHART rule(y => avg(body_mass_g)))) |> AGGREGATE layer(chart) AS chart"
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
        "FROM penguins |> CHART bars(x => species)",
        "`bars` is not a mark function",
        "bars",
    )
    .await;
    fails(
        "FROM penguins |> CHART bar(x => species, colour => island)",
        "bar() has no parameter `colour`",
        "colour",
    )
    .await;
    fails(
        "FROM penguins |> CHART bar(x => species, x => island)",
        "bar() takes `x` once",
        "x => island",
    )
    .await;
    fails("FROM penguins |> CHART", "CHART takes a mark call", "CHART").await;
}

#[tokio::test]
async fn v1_arguments_read_only_earlier_stages() {
    charts(&format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g), y => count(*))"
    ))
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => bin(count(*)), y => count(*))");
    fails(
        &query,
        "`count(*)` cannot be an argument of `bin`, because bin computes its bins before the aggregates",
        "count(*))",
    )
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => species, y => sum(count(*)))");
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
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g), y => count(*) / bin(body_mass_g).end)"
    ))
    .await;
    let query = format!("{PENGUINS} |> CHART symbol(x => species, y => count(*) / body_mass_g)");
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
        "{PENGUINS} |> CHART bar(x => species, y => count(*) ORDER BY species)"
    ))
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => species, y => count(*) ORDER BY sex)");
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
        "{PENGUINS} |> CHART bar(x => species, y => count(*), opacity => direct(0.5))"
    ))
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => species, y => direct(count(*)) + 1)");
    fails(
        &query,
        "scaled() and direct() are valid only as the complete value of a channel",
        "direct(",
    )
    .await;
    let query = format!(
        "{PENGUINS} |> CHART text(x => bill_length_mm, y => bill_depth_mm, text => scaled(species))"
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
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER panel, y => count(*)) GROUP BY island"
    ))
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => species, y => rank())");
    fails(
        &query,
        "rank() is a window function, so it needs OVER",
        "rank",
    )
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER w, y => count(*))");
    fails(&query, "no window `w` in this chart", "w, y").await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER (panel PARTITION BY sex), y => count(*))"
    );
    fails(&query, "the window `panel` has its partition", "sex)").await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER (ORDER BY sex), y => count(*))"
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
        "{PENGUINS} |> CHART bar(x => species, y => count(*), orient => 'vertical')"
    ))
    .await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g, maxbins => flipper_length_mm), y => count(*))"
    );
    fails(
        &query,
        "option `maxbins` of bin() needs a plan-time constant; `flipper_length_mm` is a column",
        "flipper_length_mm",
    )
    .await;
    let query = format!("{PENGUINS} |> CHART bar(x => species, y => count(*), orient => $1)");
    fails(
        &query,
        "option `orient` of bar() needs a plan-time constant; `$1` is a parameter",
        "$1",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) OVER (PARTITION BY island)"),
        "the mark call of CHART takes no OVER clause, so `bar` has none",
        "bar",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) GROUP BY island DESC"),
        "a group of CHART takes no ASC, DESC, or NULLS order",
        "island DESC",
    )
    .await;
}

#[tokio::test]
async fn v9_the_types_of_the_arguments_of_transforms() {
    charts("FROM sales |> CHART bar(x => timeunit(t, unit => 'month'), y => sum(units))").await;
    let query = format!("{PENGUINS} |> CHART bar(x => bin(species), y => count(*))");
    fails(&query, "bin bins numbers, not Utf8", "species)").await;
    let query =
        format!("{PENGUINS} |> CHART bar(x => timeunit(species, unit => 'month'), y => count(*))");
    fails(
        &query,
        "timeunit takes a DATE or a TIMESTAMP, not Utf8",
        "species,",
    )
    .await;
    // The struct of a discretizer is the argument of the discretizer of the next depth.
    let query = "FROM sales |> CHART bar(x => bin(timeunit(t, unit => 'month')), y => sum(units))";
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
    let query =
        format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) AS counts GROUP BY island");
    assert_eq!(columns(&query).await, ["island", "counts"]);
    let lifting = avenger_sql::lift(&session(), &query).await.unwrap();
    assert!(lifting.ends_with("AS counts GROUP BY island"), "{lifting}");
    // A group expression is a column of its alias, or of its text.
    let charts = lifted(
        "group-expression",
        &format!(
            "{PENGUINS} |> CHART bar(x => species, y => count(*)) GROUP BY upper(island) AS isle"
        ),
    )
    .await;
    assert_eq!(charts.len(), 3);
    let query =
        format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) GROUP BY upper(island)");
    assert_eq!(columns(&query).await, ["upper(island)", "chart"]);
    // CHART takes one item.
    fails(
        &format!(
            "{PENGUINS} |> CHART bar(x => species, y => count(*)), count(*) AS n GROUP BY island"
        ),
        "CHART takes one item, a mark call",
        "count(*) AS n",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*)) DESC"),
        "an item of CHART takes no ASC, DESC, or NULLS order",
        "bar",
    )
    .await;
}

#[tokio::test]
async fn a_stacked_histogram() {
    lifted(
        "stack-histogram",
        &format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g), y => stack(count(*)), color => species)"),
    )
    .await;
}

#[tokio::test]
async fn a_horizontal_stack() {
    lifted(
        "stack-horizontal",
        &format!("{PENGUINS} |> CHART bar(x => stack(count(*)), y => species, color => island)"),
    )
    .await;
}

#[tokio::test]
async fn a_stack_with_an_order_of_its_own() {
    lifted(
        "stack-order",
        &format!(
            "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)) OVER (stacks ORDER BY count(*) DESC), color => island)"
        ),
    )
    .await;
}

#[tokio::test]
async fn the_offsets_of_a_stack() {
    lifted(
        "stack-normalize",
        &format!(
            "{PENGUINS} |> CHART bar(x => species, y => stack(count(*), offset => 'normalize'), color => island)"
        ),
    )
    .await;
    lifted(
        "stack-center",
        &format!(
            "{PENGUINS} |> CHART area(x => bin(body_mass_g), y => stack(count(*), offset => 'center'), color => species)"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_stacked_lump_with_its_other_group_at_the_bottom() {
    let charts = lifted(
        "stack-lump",
        &format!("{PENGUINS} |> CHART bar(x => island, y => stack(count(*)), color => lump(species, n => 1))"),
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
        &format!("{PENGUINS} |> CHART polar_bar(theta => stack(count(*)), color => species)"),
    )
    .await;
}

#[tokio::test]
async fn windows_of_aggregates_in_a_partitioned_call() {
    lifted(
        "window-panel",
        &format!(
            "{PENGUINS} |> CHART symbol(x => species, y => count(*) / sum(count(*)) OVER panel) GROUP BY island"
        ),
    )
    .await;
    lifted(
        "window-all",
        &format!(
            "{PENGUINS} |> CHART symbol(x => species, y => sum(count(*)) OVER ()) GROUP BY island"
        ),
    )
    .await;
}

#[tokio::test]
async fn v6_the_stack_windows() {
    let query = format!(
        "{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER stacks, y => stack(count(*)), color => species)"
    );
    fails(&query, "the window `stacks` holds the stacks", "stacks,").await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)) OVER (stacked ORDER BY count(*)), color => island)"
    );
    fails(&query, "the window `stacked` has its order", "count(*)),").await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)) OVER (stacks PARTITION BY island), color => island)"
    );
    fails(&query, "the window `stacks` has its partition", "island),").await;
    let query = format!(
        "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)) OVER piles, color => island)"
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
        "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)) OVER stacks, color => island)"
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
    let query = format!("{PENGUINS} |> CHART bar(y => stack(count(*)), color => species)");
    fails(
        &query,
        "stack needs a key channel; this bar has no x",
        "stack(",
    )
    .await;
    let query = format!("{PENGUINS} |> CHART rule(x => species, y => stack(count(*)))");
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
         |> CHART bar(x => stack(count(*)), y => f, color => species)"
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
            "{PENGUINS} |> CHART bar(x => species, y => stack(count(*)), color => island, orient => 'horizontal')"
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
            "{PENGUINS} |> CHART bar(x => island, y => stack(count(*)), \
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
            "{PENGUINS} |> CHART bar(x => species, y => count(*), title => 'Penguins', id => 'bars', tools => [navigate(), tooltip()])"
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
        &format!("{SPARSE} |> CHART bar(x => date_trunc('month', t), y => sum(units), color => region, impute => 0)"),
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
        &format!("{SPARSE} |> CHART area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region, impute => 0)"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
}

#[tokio::test]
async fn impute_fills_the_empty_bins_of_a_histogram() {
    let query = |impute: &str| {
        format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g, maxbins => 40), y => stack(count(*)), color => species{impute})")
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
        &format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER panel, y => stack(count(*)), color => sex, impute => 0) GROUP BY island"),
    )
    .await;
    assert_eq!(charts.len(), 3);
}

#[tokio::test]
async fn impute_takes_the_keys_of_an_explicit_domain() {
    let charts = lifted(
        "impute-domain",
        &format!("{SPARSE} |> WHERE region <> 'north' |> CHART bar(x => scaled(region, scale => scale(domain => ['east', 'west', 'north'])), y => sum(units), impute => 0)"),
    )
    .await;
    assert_eq!(values(&charts[0], "region").len(), 3, "north, with 0 units");
    // `keys` replaces the keys of the domain, and `extra_keys` adds to them.
    let charts = lifted(
        "impute-extra-keys",
        &format!("{SPARSE} |> CHART bar(x => region, y => sum(units), impute => impute(value => 0, extra_keys => ['south']))"),
    )
    .await;
    assert_eq!(values(&charts[0], "region").len(), 4);
}

#[tokio::test]
async fn impute_computes_a_statistic_or_a_gap() {
    let charts = lifted(
        "impute-mean",
        &format!("{SPARSE} |> CHART line(x => date_trunc('month', t), y => sum(units), color => region, impute => impute(method => 'mean', frame => [-1, 1]))"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
    let charts = lifted(
        "impute-gap",
        &format!("{SPARSE} |> CHART line(x => date_trunc('month', t), y => sum(units), color => region, impute => impute(method => 'gap'))"),
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
        &format!("{SPARSE} |> CHART text(x => date_trunc('month', t), y => sum(units), color => region, text => to_char(date_trunc('month', t), '%b'), impute => 0)"),
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
        &format!("{SPARSE} |> AGGREGATE sum(units) AS units GROUP BY date_trunc('month', t) AS month, region |> CHART line(x => month, y => units, color => region, impute => 0)"),
    )
    .await;
    assert_eq!(table(&charts[0]).num_rows(), 18);
}

#[tokio::test]
async fn v10_the_impute_option() {
    let bar =
        |impute: &str| format!("{SPARSE} |> CHART bar(x => region, y => sum(units), {impute})");
    fails(
        &format!("{SPARSE} |> CHART polar_bar(theta => region, radius => sum(units), impute => 0)"),
        "`impute` is valid only on bar(), area(), line(), symbol(), and text(); found on polar_bar()",
        "impute =>",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> CHART bar(x => species, y => count(*), impute => island)"),
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
        &format!("{SPARSE} |> CHART bar(y => sum(units), impute => 0)"),
        "`impute` needs a key channel; this bar has no x",
        "impute =>",
    )
    .await;
    fails(
        &format!("{SPARSE} |> CHART bar(x => region, y => sum(units) / 2, impute => 0)"),
        "`impute` fills the measure of bar(), which is one aggregate, such as sum(units), in a chart that aggregates; found `sum(units) / 2`",
        "sum(units) / 2",
    )
    .await;
    fails(
        &format!("{SPARSE} |> CHART line(x => t, y => units * 2, impute => 0)"),
        "a column in a chart that does not aggregate; found `units * 2`",
        "units * 2",
    )
    .await;
    fails(
        &format!("{PENGUINS} |> CHART bar(x => bin(body_mass_g) OVER (PARTITION BY sex), y => count(*), impute => 0)"),
        "`impute` fills the bins of each `sex`, which is not a key of this chart",
        "sex)",
    )
    .await;
    // The option is a parameter of the mark calls of CHART, not of the mark aggregates.
    let error =
        error("FROM sales |> AGGREGATE bar(x => region, y => units, impute => 0) AS chart").await;
    assert!(error.to_string().contains("impute"), "{error}");
}
