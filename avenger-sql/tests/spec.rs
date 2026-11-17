//! The examples of `SPEC.md`, the short guide to the language, which agents copy, and of the
//! README: each parses, and each runs in a session of the tests' tables, so a chart example with a
//! wrong name, such as a constructor that its argument does not take, fails.

mod common;

use std::sync::Arc;

use avenger_chart::{charts_from_arrow, Decode};
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
    prelude::SessionContext,
};

/// The `sql` examples of a Markdown text.
fn examples(text: &str) -> Vec<&str> {
    (text.split("```sql\n").skip(1))
        .map(|rest| rest.split("```").next().unwrap())
        .collect()
}

/// A session of the tests' tables, with the tables that only the examples read: the orders and
/// customers of the SPEC's joins, the prices of `MATCH_RECOGNIZE`, and the catches, sales, and
/// monthly sales of the README's transforms. These tables are empty, since those examples are not
/// charts, and their columns are those that the examples read.
fn session() -> SessionContext {
    let ctx = common::session();
    let column = |name: &str, data_type: DataType| Field::new(name, data_type, true);
    let tables = [
        (
            "orders",
            vec![
                column("customer_id", DataType::Int64),
                column("total", DataType::Float64),
            ],
        ),
        (
            "customers",
            vec![
                column("customer_id", DataType::Int64),
                column("region", DataType::Utf8),
            ],
        ),
        (
            "prices",
            vec![
                column("symbol", DataType::Utf8),
                column("day", DataType::Date32),
                column("price", DataType::Float64),
            ],
        ),
        (
            "catches",
            vec![
                column("port", DataType::Utf8),
                column("tons", DataType::Float64),
                column("region", DataType::Utf8),
            ],
        ),
        (
            "sales",
            vec![
                column("month", DataType::Date32),
                column("region", DataType::Utf8),
                column("units", DataType::Int64),
            ],
        ),
        (
            "monthly",
            vec![
                column("month", DataType::Date32),
                column("region", DataType::Utf8),
                column("units", DataType::Int64),
            ],
        ),
    ];
    for (name, columns) in tables {
        let table = MemTable::try_new(Arc::new(Schema::new(columns)), vec![vec![]]).unwrap();
        ctx.register_table(name, Arc::new(table)).unwrap();
    }
    ctx
}

/// The number of charts that `example` builds in its column `chart`, or the error of its parse,
/// its plan, its run, or the reading of a chart.
async fn run(ctx: &SessionContext, example: &str) -> Result<usize, String> {
    let parse = avenger_sql_syntax::parse(example);
    if !parse.diagnostics().is_empty() {
        return Err(format!("{:?}", parse.diagnostics()));
    }
    let frame = avenger_sql::sql(ctx, example)
        .await
        .map_err(|error| error.to_string())?;
    let batches = frame.collect().await.map_err(|error| error.to_string())?;
    let mut count = 0;
    for column in (batches.iter()).filter_map(|batch| batch.column_by_name("chart")) {
        let charts = charts_from_arrow(column, Decode::Auto).map_err(|error| error.to_string())?;
        if charts.iter().any(Option::is_none) {
            return Err("a row has no chart".into());
        }
        count += charts.len();
    }
    Ok(count)
}

/// Checks that each example of `text` fits in 100 columns, runs, and, when it names a column
/// `chart`, builds a chart or more.
async fn check(text: &str, count: usize) {
    let examples = examples(text);
    assert_eq!(examples.len(), count);
    let ctx = session();
    for example in examples {
        for line in example.lines() {
            assert!(
                line.chars().count() <= 100,
                "{example}has a line past 100 columns"
            );
        }
        let charts = run(&ctx, example)
            .await
            .unwrap_or_else(|error| panic!("{example}\n{error}"));
        if example.contains("AS chart") {
            assert!(charts > 0, "{example}builds no chart");
        } else {
            assert_eq!(charts, 0, "{example}");
        }
    }
}

#[tokio::test]
async fn the_examples_of_the_guide_run() {
    check(include_str!("../SPEC.md"), 9).await;
}

#[tokio::test]
async fn the_examples_of_the_readme_run() {
    check(include_str!("../README.md"), 8).await;
}

#[tokio::test]
async fn a_chart_example_with_a_wrong_name_fails() {
    let guide = include_str!("../SPEC.md");
    let example = (examples(guide).into_iter())
        .find(|example| example.contains("axis => axis(grid => FALSE)"))
        .unwrap();
    let wrong = example.replace("axis(grid => FALSE)", "axis(show_grid => FALSE)");
    let error = run(&session(), &wrong).await.unwrap_err();
    assert!(error.contains("show_grid"), "{error}");
}
