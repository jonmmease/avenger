//! Scripts through the pass: the forms that GoogleSQL's `execute_query` doesn't run in its script
//! mode, such as temporary tables of columns and modules, and the plans of a script's statements.
//! `googlesql_queries.rs` checks the scripts that it runs.

mod pass;

use avenger_sql::{
    analysis::{Definition, StatementPlan},
    catalog::MemoryCatalog,
};

/// The module `util` of the tests.
const UTIL: &str = "MODULE util;
CREATE PUBLIC FUNCTION double(x INT64) AS (x * 2);
CREATE PRIVATE FUNCTION inc(x INT64) AS (x + 1);
CREATE PUBLIC FUNCTION double_inc(x INT64) AS (inc(x) * 2);
CREATE PUBLIC CONSTANT ten = 10;
CREATE PUBLIC TABLE FUNCTION upto(n INT64) AS (FROM UNNEST(GENERATE_ARRAY(1, n)) AS v);
";

fn catalog() -> MemoryCatalog {
    pass::catalog().with_module(&["util"], UTIL).with_module(
        &["broken"],
        "MODULE broken;\nCREATE PUBLIC FUNCTION f(x INT64) AS (nope + 1);\n",
    )
}

/// The rows of the script `text`, as a table.
async fn table(text: &str) -> String {
    let batches = pass::run_in(&pass::session(), &catalog(), text)
        .await
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    datafusion::arrow::util::pretty::pretty_format_batches(&batches)
        .unwrap()
        .to_string()
}

/// The first error of the script `text`.
fn error(text: &str) -> String {
    let analysis = pass::analysis_in(&catalog(), text);
    analysis
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| panic!("{text} has no error"))
}

/// The values of the column of a table of one column, in the order of its rows.
fn values(table: &str) -> Vec<String> {
    (table.lines().skip(3))
        .filter(|line| line.starts_with('|'))
        .map(|line| line.trim_matches(|c| c == '|' || c == ' ').to_string())
        .collect()
}

#[tokio::test]
async fn temporary_tables_are_views_of_their_queries() {
    let text =
        table("CREATE TEMP TABLE w (a INT64, b STRING); FROM w |> AGGREGATE COUNT(*) AS n").await;
    assert_eq!(values(&text), ["0"]);
    let text = table("CREATE TEMP TABLE w AS (FROM t); CREATE OR REPLACE TEMP TABLE w AS (FROM t |> SELECT 9 AS y); FROM w |> AGGREGATE SUM(y) AS s").await;
    assert_eq!(values(&text), ["27"]);
    let text = table("CREATE TEMP TABLE w AS (FROM t |> SELECT 1 AS y); CREATE TEMP TABLE IF NOT EXISTS w AS (FROM t |> SELECT 9 AS y); FROM w |> AGGREGATE SUM(y) AS s").await;
    assert_eq!(values(&text), ["3"]);
    let text = table("CREATE TEMP TABLE w (k INT64) AS (FROM t |> SELECT CAST(x AS FLOAT64) AS f); FROM w |> AGGREGATE SUM(k) AS s").await;
    assert_eq!(values(&text), ["6"]);
    // A temporary table hides a table of the catalog.
    let text =
        table("CREATE TEMP TABLE u AS (FROM t |> WHERE x = 1); FROM u |> AGGREGATE COUNT(*) AS n")
            .await;
    assert_eq!(values(&text), ["1"]);
}

#[tokio::test]
async fn temporary_functions_inline_their_bodies() {
    let text = table("CREATE TEMP FUNCTION f(x INT64) AS (x + 1); CREATE TEMP FUNCTION f2(x INT64) AS (f(x) * 2); FROM t |> SELECT f2(x) AS y |> ORDER BY y").await;
    assert_eq!(values(&text), ["4", "6", "8"]);
    let text = table("CREATE TEMP TABLE FUNCTION tf(n INT64) AS (FROM UNNEST(GENERATE_ARRAY(1, n)) AS v); FROM tf(3) |> AGGREGATE SUM(v) AS s").await;
    assert_eq!(values(&text), ["6"]);
    let text = table("CREATE TEMP TABLE FUNCTION tf2(src ANY TABLE, k INT64) AS (FROM src |> WHERE x >= k); FROM tf2(TABLE t, 2) |> AGGREGATE COUNT(*) AS n").await;
    assert_eq!(values(&text), ["2"]);
    // A function is visible to the statements after its CREATE.
    assert_eq!(
        error("FROM t |> SELECT f(1); CREATE TEMP FUNCTION f(x INT64) AS (x + 1); FROM t |> SELECT f(1)"),
        "Function not found: f"
    );
    // The body of an ANY TYPE parameter is planned at each call, and its errors go to the call.
    assert_eq!(
        error("CREATE TEMP FUNCTION g(v ANY TYPE) AS (v + 1); FROM t |> SELECT g(TRUE)"),
        "In g: No matching signature for operator + for argument types: BOOL, INT64"
    );
}

#[tokio::test]
async fn modules_give_their_public_functions_and_constants() {
    let text = table("IMPORT MODULE util; FROM t |> SELECT util.double(x) + util.double_inc(x) + util.ten AS v |> ORDER BY v").await;
    assert_eq!(values(&text), ["16", "20", "24"]);
    let text = table("IMPORT MODULE util AS m; FROM m.upto(4) |> AGGREGATE SUM(v) AS s").await;
    assert_eq!(values(&text), ["10"]);
    assert_eq!(
        error("IMPORT MODULE util; FROM t |> SELECT util.inc(1)"),
        "Function not found: util.inc"
    );
    assert_eq!(error("IMPORT MODULE nowhere"), "Module not found: nowhere");
    assert_eq!(
        error("IMPORT MODULE broken"),
        "In module broken: Unrecognized name: nope"
    );
}

#[test]
fn a_variable_is_a_plan_of_its_own_and_a_placeholder() {
    let text = "DECLARE k INT64 DEFAULT 2; SET k = k + 1; FROM t |> WHERE x >= k";
    let analysis = pass::analysis_in(&catalog(), text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let kinds: Vec<&str> = (analysis.statements.iter())
        .map(|statement| match &statement.plan {
            StatementPlan::Query(_) => "query",
            StatementPlan::Variable { .. } => "variable",
            StatementPlan::Definition => "definition",
        })
        .collect();
    assert_eq!(kinds, ["variable", "variable", "query"]);
    let ids: Vec<&str> = (analysis.statements.iter())
        .filter_map(|statement| match &statement.plan {
            StatementPlan::Variable { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids[0], ids[1], "a SET writes its variable's placeholder");
    let plan = analysis.plan.expect("a plan").display_indent().to_string();
    assert!(plan.contains(ids[0]), "{plan}");
    let variables = (analysis.model.names.iter())
        .filter(|name| matches!(name.definition, Definition::Variable { .. }))
        .count();
    assert_eq!(variables, 3);
}
