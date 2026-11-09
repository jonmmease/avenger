//! The mark functions of the `datafusion` feature: the binding of their calls at planning, and the
//! charts of their rows.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, Int64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field, Float64Type, Schema},
};
use avenger_chart::{
    charts_from_arrow,
    datafusion::{mark_names, register},
    prelude::*,
    proto::{encoding, node},
    Decode, LayerRef, EXTENSION_NAME,
};
use datafusion::{
    datasource::MemTable,
    prelude::{SessionConfig, SessionContext},
};

/// Eight rows in four batches: numbers `a` and `b`, a category `c`, a draw order `k`, strings `s`,
/// a number `m` whose field has metadata, and a struct `st`.
fn batches() -> (Arc<Schema>, Vec<RecordBatch>) {
    let metadata = HashMap::from([("label".to_string(), "Mass".to_string())]);
    let span = DataType::Struct(
        vec![
            Field::new("start", DataType::Float64, true),
            Field::new("end", DataType::Float64, true),
        ]
        .into(),
    );
    let schema = Arc::new(Schema::new(vec![
        Field::new("a", DataType::Float64, false),
        Field::new("b", DataType::Float64, false),
        Field::new("c", DataType::Utf8, false),
        Field::new("k", DataType::Int64, false),
        Field::new("s", DataType::Utf8, false),
        Field::new("m", DataType::Float64, false).with_metadata(metadata),
        Field::new("st", span.clone(), false),
    ]));
    let batch = |rows: &[(f64, f64, &str, i64)]| {
        let starts: ArrayRef = Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.0)));
        let ends: ArrayRef = Arc::new(Float64Array::from_iter_values(
            rows.iter().map(|r| r.0 + 1.0),
        ));
        let DataType::Struct(fields) = &span else {
            unreachable!()
        };
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.0))),
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.1))),
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.2))),
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.3))),
            Arc::new(StringArray::from_iter_values(
                rows.iter().map(|r| format!("s{}", r.3)),
            )),
            Arc::new(Float64Array::from_iter_values(
                rows.iter().map(|r| r.1 * 10.0),
            )),
            Arc::new(StructArray::new(fields.clone(), vec![starts, ends], None)),
        ];
        RecordBatch::try_new(schema.clone(), columns).unwrap()
    };
    let batches = vec![
        batch(&[(1.0, 4.0, "p", 7), (2.0, 6.0, "q", 2)]),
        batch(&[(3.0, 5.0, "p", 5), (4.0, 3.0, "q", 0)]),
        batch(&[(5.0, 2.0, "p", 6), (6.0, 1.0, "q", 3)]),
        batch(&[(7.0, 8.0, "p", 1), (8.0, 7.0, "q", 4)]),
    ];
    (schema, batches)
}

/// A session of four partitions with the marks and the table `t`, one batch in each partition.
fn session() -> SessionContext {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(4));
    register(&ctx);
    let (schema, batches) = batches();
    let partitions = batches.into_iter().map(|batch| vec![batch]).collect();
    let table = MemTable::try_new(schema, partitions).unwrap();
    ctx.register_table("t", Arc::new(table)).unwrap();
    ctx
}

/// The batches of `query`.
async fn run(query: &str) -> datafusion::error::Result<Vec<RecordBatch>> {
    session().sql(query).await?.collect().await
}

/// The charts of the column `chart` of `query`.
async fn charts(query: &str) -> Vec<Chart> {
    let batches = run(query).await.unwrap();
    let mut charts = Vec::new();
    for batch in batches {
        let column = batch.column_by_name("chart").expect("a chart column");
        for chart in charts_from_arrow(column, Decode::Auto).unwrap() {
            charts.push(chart.expect("a chart"));
        }
    }
    charts
}

/// The one chart of `query`.
async fn chart(query: &str) -> Chart {
    let mut charts = charts(query).await;
    assert_eq!(charts.len(), 1, "the query has one chart");
    charts.remove(0)
}

/// The planning error of `query`.
async fn error(query: &str) -> String {
    let result = session().sql(query).await;
    let result = match result {
        Ok(frame) => frame.into_optimized_plan().map(|_| ()),
        Err(error) => Err(error),
    };
    result.expect_err("the query fails to plan").to_string()
}

/// The first layer of the root plot of `chart`.
fn layer(chart: &Chart) -> LayerRef<'_> {
    let Some(node::Node::Plot(plot)) = &chart.root().node else {
        panic!("the root is a plot")
    };
    LayerRef::of(plot).remove(0)
}

/// The table of the first layer of `chart`.
fn table(chart: &Chart) -> &RecordBatch {
    chart.table(layer(chart).table().expect("the layer has a table"))
}

/// The encoding of `channel` of the first layer of `chart`.
fn encoding(chart: &Chart, channel: Channel) -> Option<&encoding::Kind> {
    layer(chart)
        .mark()
        .expect("a mark")
        .encoding(channel)?
        .kind
        .as_ref()
}

fn floats(table: &RecordBatch, column: &str) -> Vec<f64> {
    let column = table.column_by_name(column).expect("the column");
    column.as_primitive::<Float64Type>().values().to_vec()
}

#[tokio::test]
async fn a_call_binds_at_planning_and_keeps_its_name() {
    let ctx = session();
    let query = "SELECT symbol(x => a, y => b) FROM t";
    let frame = ctx.sql(query).await.unwrap();
    let name = frame.schema().field(0).name().clone();
    let plan = frame.clone().into_optimized_plan().unwrap();
    let display = plan.display_indent().to_string();
    assert!(
        !display.contains("NULL"),
        "the bound call takes only its columns: {display}"
    );
    let batches = frame.collect().await.unwrap();
    let field = batches[0].schema().field(0).clone();
    assert_eq!(field.name(), &name, "the column keeps the name of the call");
    assert_eq!(field.data_type(), &DataType::BinaryView);
    assert_eq!(
        field
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some(EXTENSION_NAME)
    );
}

#[tokio::test]
async fn a_chart_holds_the_columns_of_its_channels() {
    let chart = chart("SELECT symbol(x => a, y => b * 2, color => c) AS chart FROM t").await;
    let table = table(&chart);
    let names: Vec<&str> = table
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(names, ["a", "b * 2", "c"]);
    assert_eq!(table.num_rows(), 8);
    assert!(matches!(
        encoding(&chart, Channel::Y),
        Some(encoding::Kind::ScaledField(field)) if field == "b * 2"
    ));
}

#[tokio::test]
async fn channels_with_one_expression_share_a_column() {
    let chart = chart("SELECT symbol(x => a, y => b, color => a) AS chart FROM t").await;
    assert_eq!(table(&chart).num_columns(), 2);
    assert!(matches!(
        encoding(&chart, Channel::Color),
        Some(encoding::Kind::ScaledField(field)) if field == "a"
    ));
}

#[tokio::test]
async fn a_constant_argument_arrives_folded() {
    let chart = chart("SELECT symbol(x => a, y => 2 * 50) AS chart FROM t").await;
    let Some(encoding::Kind::ScaledValue(value)) = encoding(&chart, Channel::Y) else {
        panic!("a constant of a position is a scaled value")
    };
    let value = chart.value(value);
    assert_eq!(
        value.as_primitive::<arrow::datatypes::Int64Type>().value(0),
        100
    );
    assert_eq!(table(&chart).num_columns(), 1, "a constant is no column");
}

#[tokio::test]
async fn bare_values_take_the_kinds_of_the_builders() {
    let symbols =
        chart("SELECT symbol(x => a, y => 3, color => c, fill => 'red') AS chart FROM t").await;
    assert!(matches!(
        encoding(&symbols, Channel::X),
        Some(encoding::Kind::ScaledField(field)) if field == "a"
    ));
    assert!(matches!(
        encoding(&symbols, Channel::Y),
        Some(encoding::Kind::ScaledValue(_))
    ));
    assert!(matches!(
        encoding(&symbols, Channel::Color),
        Some(encoding::Kind::ScaledField(field)) if field == "c"
    ));
    assert!(
        matches!(
            encoding(&symbols, Channel::Fill),
            Some(encoding::Kind::DirectValue(_))
        ),
        "a constant of a channel other than a position is drawn as it is"
    );
    let text = chart("SELECT text(text => s, x => a, y => b) AS chart FROM t").await;
    assert!(matches!(
        encoding(&text, Channel::Text),
        Some(encoding::Kind::DirectField(field)) if field == "s"
    ));
}

#[tokio::test]
async fn scaled_and_direct_give_their_kinds() {
    let query = "SELECT symbol(x => direct(0.5), y => b, color => scaled('p'), fill => direct(c), \
                 stroke => direct(CAST(NULL AS VARCHAR))) AS chart FROM t";
    let chart = chart(query).await;
    assert!(
        matches!(
            encoding(&chart, Channel::X),
            Some(encoding::Kind::DirectValue(_))
        ),
        "a direct position is a fraction of the plot"
    );
    assert!(matches!(
        encoding(&chart, Channel::Color),
        Some(encoding::Kind::ScaledValue(_))
    ));
    assert!(matches!(
        encoding(&chart, Channel::Fill),
        Some(encoding::Kind::DirectField(field)) if field == "c"
    ));
    assert!(
        matches!(
            encoding(&chart, Channel::Stroke),
            Some(encoding::Kind::DirectValue(_))
        ),
        "only a bare NULL leaves its channel unset, and a typed NULL is a value"
    );
}

#[tokio::test]
async fn a_null_argument_leaves_its_channel_unset() {
    let chart = chart("SELECT symbol(x => a, y => b, color => NULL) AS chart FROM t").await;
    assert!(encoding(&chart, Channel::Color).is_none());
}

#[tokio::test]
async fn a_named_argument_after_an_omitted_one_needs_the_omitted_one() {
    // DataFusion binds named arguments only as a prefix of the parameters; avenger-sql fills the
    // omitted ones with NULL.
    let message = error("SELECT symbol(x => a, y => b, opacity => a) AS chart FROM t").await;
    assert!(message.contains("Missing required parameter"), "{message}");
}

#[tokio::test]
async fn the_draw_order_survives_a_partial_aggregation() {
    let query = "SELECT * FROM t |> AGGREGATE symbol(x => a, y => b ORDER BY k DESC) AS chart";
    let chart = chart(query).await;
    let table = table(&chart);
    assert_eq!(table.num_columns(), 2, "the order keys are no columns");
    assert_eq!(floats(table, "a"), [1.0, 5.0, 3.0, 8.0, 6.0, 2.0, 7.0, 4.0]);
}

#[tokio::test]
async fn group_by_gives_a_chart_for_each_group() {
    let batches = run("SELECT c, bar(x => c, y => a) AS chart FROM t GROUP BY c ORDER BY c")
        .await
        .unwrap();
    let batch = arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap();
    let charts = charts_from_arrow(batch.column_by_name("chart").unwrap(), Decode::Auto).unwrap();
    assert_eq!(charts.len(), 2);
    for (chart, expected) in charts
        .iter()
        .zip([[1.0, 3.0, 5.0, 7.0], [2.0, 4.0, 6.0, 8.0]])
    {
        let mut values = floats(table(chart.as_ref().unwrap()), "a");
        values.sort_by(f64::total_cmp);
        assert_eq!(values, expected);
    }
}

#[tokio::test]
async fn a_filter_selects_the_rows() {
    let chart = chart("SELECT symbol(x => a, y => b) FILTER (WHERE c = 'p') AS chart FROM t").await;
    assert_eq!(table(&chart).num_rows(), 4);
}

#[tokio::test]
async fn a_call_builds_the_chart_of_its_builder() {
    let chart = chart("SELECT bar(x => c, y => a ORDER BY k) AS chart FROM t").await;
    let table = table(&chart).clone();
    let layer = CartesianLayer::bar(table, BarMark::builder().x("c").y("a"));
    let expected = Chart::builder(Plot::cartesian([layer])).build().unwrap();
    assert_eq!(chart, expected);
}

#[tokio::test]
async fn field_metadata_and_struct_types_reach_the_layer_table() {
    let chart = chart("SELECT bar(x => st, y => m) AS chart FROM t").await;
    let schema = table(&chart).schema();
    assert!(matches!(
        schema.field_with_name("st").unwrap().data_type(),
        DataType::Struct(_)
    ));
    let metadata = schema.field_with_name("m").unwrap().metadata();
    assert_eq!(metadata.get("label").map(String::as_str), Some("Mass"));
}

#[tokio::test]
async fn every_mark_builds_a_chart_of_its_channels() {
    let names: Vec<&str> = mark_names().collect();
    assert!(names.contains(&"bar") && names.contains(&"polar_bar"));
    assert!(names.contains(&"geo_shape") && names.contains(&"geo_symbol"));
    // A geo mark needs `coord`, which the tests of avenger-sql give, because a call in plain
    // DataFusion names its parameters in order.
    for name in names.into_iter().filter(|name| !name.starts_with("geo_")) {
        // The required channels take strings, which every one of them takes, and the positions
        // are numbers, or the categories that a rect spans the bands of.
        let required = match name {
            "text" | "polar_text" => "text => s, ",
            "image" | "polar_image" => "image => s, ",
            "path" | "polar_path" => "path => s, ",
            _ => "",
        };
        let positions = match name {
            "rect" => "x => c, y => s",
            "polar_rect" => "theta => c, radius => s",
            "polar_bar" => "theta => c, radius => a",
            _ if name.starts_with("polar_") => "theta => a, radius => b",
            _ => "x => a, y => b",
        };
        let query = format!("SELECT {name}({required}{positions}) AS chart FROM t");
        let charts = charts(&query).await;
        let chart = &charts[0];
        let layer = layer(chart);
        let expected = match name.strip_prefix("polar_") {
            Some(base) => base,
            None => name,
        };
        assert_eq!(layer.mark().unwrap().name(), expected, "{query}");
        assert_eq!(table(chart).num_rows(), 8, "{query}");
    }
}

#[tokio::test]
async fn invalid_calls_fail_at_planning() {
    for (query, expected) in [
        (
            "SELECT text(text => NULL, x => a) FROM t",
            "text() needs a value for `text`",
        ),
        ("SELECT symbol(DISTINCT a, b) FROM t", "takes no DISTINCT"),
        (
            "SELECT symbol(x => direct(a) + 1, y => b) FROM t",
            "direct() is the whole value",
        ),
        (
            "SELECT symbol(x => scaled(direct(a)), y => b) FROM t",
            "direct() is the whole value",
        ),
        (
            "SELECT symbol(x => a, y => b, color => direct(NULL)) FROM t",
            "direct(NULL) of `color` has no type",
        ),
        (
            "SELECT text(text => scaled(s), x => a) FROM t",
            "scaled() needs a channel with a scale, and `text` of text() has none",
        ),
        (
            "SELECT symbol(x => 1, y => 2) FROM t",
            "needs a channel that reads a column",
        ),
    ] {
        let message = error(query).await;
        assert!(message.contains(expected), "{query}: {message}");
    }
}

#[tokio::test]
async fn an_empty_group_gives_a_chart_of_no_rows() {
    let chart = chart("SELECT symbol(x => a, y => b) AS chart FROM t WHERE a > 100").await;
    assert_eq!(table(&chart).num_rows(), 0);
}

#[test]
fn every_mark_has_documentation() {
    let ctx = SessionContext::new();
    register(&ctx);
    let state = ctx.state();
    for name in mark_names() {
        let function = state.aggregate_functions().get(name).unwrap();
        let documentation = function.documentation().expect("a mark has documentation");
        assert!(!documentation.description.is_empty(), "{name}");
        assert!(documentation
            .syntax_example
            .starts_with(&format!("{name}(")));
        let arguments = documentation.arguments.as_deref().unwrap_or_default();
        assert!(!arguments.is_empty(), "{name} documents its channels");
        for (argument, description) in arguments {
            assert!(
                !description.is_empty(),
                "{name}({argument} => ...) has documentation"
            );
        }
    }
}
