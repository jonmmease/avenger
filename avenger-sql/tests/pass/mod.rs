//! The pass over a test query: a catalog of the tests' tables, the analysis of a query, and its
//! rows.

#![allow(dead_code)]

use std::sync::Arc;

use arrow::{
    array::{Int64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_sql::{
    analysis::{analyze, Analysis, Options},
    catalog::MemoryCatalog,
    library::Library,
};
use datafusion::{arrow::util::pretty::pretty_format_batches, datasource::MemTable};

#[path = "../common/mod.rs"]
mod common;

pub use common::{penguins, session};

/// A table of `batch`.
fn table(batch: RecordBatch) -> Arc<MemTable> {
    Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap())
}

/// The table `one`, of one row and the column `one`, and the table `t`, of the rows
/// `(x, s)`: `(1, 'a')`, `(2, 'b')` and `(3, NULL)`.
pub fn small() -> (RecordBatch, RecordBatch) {
    let one = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("one", DataType::Int64, false)])),
        vec![Arc::new(Int64Array::from(vec![1]))],
    )
    .unwrap();
    let t = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, true),
            Field::new("s", DataType::Utf8, true),
        ])),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("a"), Some("b"), None])),
        ],
    )
    .unwrap();
    (one, t)
}

/// A batch of the columns `columns`, each a name and its values.
fn batch(columns: Vec<(&str, arrow::array::ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

fn ints(values: &[Option<i64>]) -> arrow::array::ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn strings(values: &[&str]) -> arrow::array::ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// The catalog of the tests: `penguins`, `one` and `t`, and the tables of the recorded queries,
/// whose GoogleSQL rows are in `scratch/projects/pipe-parser/queries/tables.json`: `u`, of
/// `(x, s)`; `sales`, of `(sp, isl, m)`; and `wide`, of `(k, a, b, c, d)`.
pub fn catalog() -> MemoryCatalog {
    let (one, t) = small();
    let u = batch(vec![
        ("x", ints(&[Some(2), Some(3), Some(4)])),
        ("s", strings(&["b", "c", "d"])),
    ]);
    let sales = batch(vec![
        ("sp", strings(&["A", "A", "B", "B"])),
        ("isl", strings(&["x", "y", "x", "y"])),
        ("m", ints(&[Some(1), Some(2), Some(5), Some(3)])),
    ]);
    let wide = batch(vec![
        ("k", ints(&[Some(1), Some(2)])),
        ("a", ints(&[Some(10), Some(20)])),
        ("b", ints(&[None, Some(2)])),
        ("c", ints(&[Some(3), None])),
        ("d", ints(&[Some(4), None])),
    ]);
    MemoryCatalog::new()
        .with_table(&["penguins"], table(penguins()))
        .with_table(&["one"], table(one))
        .with_table(&["t"], table(t))
        .with_table(&["u"], table(u))
        .with_table(&["sales"], table(sales))
        .with_table(&["wide"], table(wide))
}

/// The analysis of the script `text`.
pub fn analysis(text: &str) -> Analysis {
    analysis_in(&catalog(), text)
}

/// The analysis of the script `text` over the tables of `catalog`.
pub fn analysis_in(catalog: &MemoryCatalog, text: &str) -> Analysis {
    let parse = avenger_sql_syntax::parse(text);
    assert!(parse.ok(), "{text}: {:?}", parse.diagnostics());
    analyze(
        &parse.syntax(),
        catalog,
        &Library::new(),
        &Options::default(),
    )
}

/// The rows of the result of `text`, in their order, or the messages of its diagnostics.
pub async fn run(text: &str) -> Result<Vec<RecordBatch>, Vec<String>> {
    run_in(&session(), &catalog(), text).await
}

/// The rows of the result of `text` over the tables of `catalog`, in the session `ctx`.
pub async fn run_in(
    ctx: &datafusion::prelude::SessionContext,
    catalog: &MemoryCatalog,
    text: &str,
) -> Result<Vec<RecordBatch>, Vec<String>> {
    let analysis = analysis_in(catalog, text);
    if !analysis.diagnostics.is_empty() {
        return Err(analysis
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect());
    }
    let plan = analysis.plan.expect("a plan without diagnostics");
    let frame = ctx
        .execute_logical_plan(plan)
        .await
        .map_err(|error| vec![error.to_string()])?;
    frame
        .collect()
        .await
        .map_err(|error| vec![error.to_string()])
}

/// The rows of `text` as a table, in their order.
pub async fn table_text(text: &str) -> String {
    let batches = run(text)
        .await
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    pretty_format_batches(&batches).unwrap().to_string()
}

/// The lines of the rows of `text`, sorted, so a test compares sets of rows.
pub async fn rows(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = table_text(text).await.lines().map(String::from).collect();
    lines.sort();
    lines
}

/// The lines of the rows of the standard query `text`, which DataFusion plans over the session's
/// tables, sorted.
pub async fn standard_rows(text: &str) -> Vec<String> {
    let batches = session().sql(text).await.unwrap().collect().await.unwrap();
    let text = pretty_format_batches(&batches).unwrap().to_string();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.sort();
    lines
}

/// The messages of the diagnostics of `text`, and the text at each.
pub fn errors(text: &str) -> Vec<(String, String)> {
    analysis(text)
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let range = std::ops::Range::<usize>::from(diagnostic.range);
            (diagnostic.message.clone(), text[range].to_string())
        })
        .collect()
}
