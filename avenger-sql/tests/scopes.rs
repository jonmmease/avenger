//! The scope rules of names, each against GoogleSQL's `execute_query`, and the model's record of
//! what each name refers to.
//!
//! Each rule's GoogleSQL query reads `(SELECT 1 AS x, 'a' AS s)`, and its pass query reads the
//! table `one_row`, of the same row. The outcomes are those that `execute_query` 2026.11.1 gives,
//! with `--enabled_language_features=MAXIMUM`.

mod pass;

use std::sync::Arc;

use arrow::{
    array::{Int64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_sql::{analysis::Definition, catalog::MemoryCatalog};
use datafusion::{arrow::util::pretty::pretty_format_batches, datasource::MemTable};

/// The catalog of the tests, with the table `one_row` of the row `(1, 'a')`.
fn catalog() -> MemoryCatalog {
    let row = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, false),
            Field::new("s", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec!["a"])),
        ],
    )
    .unwrap();
    let table = Arc::new(MemTable::try_new(row.schema(), vec![vec![row]]).unwrap());
    pass::catalog().with_table(&["one_row"], table)
}

/// The outcome of a query: the names of its columns and its row, or its first error.
#[derive(Debug, PartialEq)]
enum Outcome {
    Row(&'static str, &'static str),
    Error(&'static str),
}

/// Each rule: GoogleSQL's query, the pass's query, and GoogleSQL's outcome.
const RULES: &[(&str, &str, Outcome)] = &[
    // Names match without regard to case, and an output column keeps its case.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) |> SELECT X, x",
        "FROM one_row |> SELECT X, x",
        Outcome::Row("| X | x |", "| 1 | 1 |"),
    ),
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> SELECT t.X",
        "FROM one_row AS t |> SELECT t.X",
        Outcome::Row("| X |", "| 1 |"),
    ),
    // `|> AS u` replaces the range variables of its input.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> AS u |> SELECT u.x",
        "FROM one_row AS t |> AS u |> SELECT u.x",
        Outcome::Row("| x |", "| 1 |"),
    ),
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> AS u |> SELECT t.x",
        "FROM one_row AS t |> AS u |> SELECT t.x",
        Outcome::Error("Unrecognized name: t"),
    ),
    // A column that EXTEND computes has no range variable.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> EXTEND x + 1 AS y |> SELECT t.y",
        "FROM one_row AS t |> EXTEND x + 1 AS y |> SELECT t.y",
        Outcome::Error("Name y not found inside t"),
    ),
    // An item of a SELECT doesn't see the alias of another.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> SELECT x AS y, y + 1 AS z",
        "FROM one_row AS t |> SELECT x AS y, y + 1 AS z",
        Outcome::Error("Unrecognized name: y"),
    ),
    // A name of two sources is ambiguous.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS a CROSS JOIN (SELECT 1 AS x, 'a' AS s) AS b |> SELECT x",
        "FROM one_row AS a CROSS JOIN one_row AS b |> SELECT x",
        Outcome::Error("Column name x is ambiguous"),
    ),
    // A SELECT can return two columns of one name.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> SELECT x, x",
        "FROM one_row AS t |> SELECT x, x",
        Outcome::Row("| x | x |", "| 1 | 1 |"),
    ),
    // A range variable reads its row as its source gives it.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> SET x = 2 |> SELECT t.x, x",
        "FROM one_row AS t |> SET x = 2 |> SELECT t.x, x",
        Outcome::Row("| x | x |", "| 1 | 2 |"),
    ),
    // A pipe WITH name is visible to the queries nested in the operators after it.
    (
        "FROM (SELECT 1 AS x) |> WITH w AS (SELECT 2 AS y) |> WHERE x < (FROM w |> AGGREGATE MAX(y))",
        "FROM one_row |> WITH w AS (FROM one_row |> SELECT 2 AS y) |> WHERE x < (FROM w |> AGGREGATE MAX(y)) |> SELECT x",
        Outcome::Row("| x |", "| 1 |"),
    ),
    // The query of a pipe WITH doesn't see the operator's input.
    (
        "FROM (SELECT 1 AS x) AS r |> WITH w AS (SELECT r.x AS y) |> CROSS JOIN w",
        "FROM one_row AS r |> WITH w AS (FROM one_row |> SELECT r.x AS y) |> CROSS JOIN w",
        Outcome::Error("Unrecognized name: r"),
    ),
    // A subquery reads the columns of the queries that enclose it.
    (
        "FROM (SELECT 1 AS x) AS t |> WHERE EXISTS (FROM (SELECT 1 AS k) |> WHERE k = t.x)",
        "FROM one_row AS t |> WHERE EXISTS (FROM one AS o |> WHERE o.one = t.x) |> SELECT x",
        Outcome::Row("| x |", "| 1 |"),
    ),
    // AGGREGATE ends the range variables.
    (
        "FROM (SELECT 1 AS x, 'a' AS s) AS t |> AGGREGATE COUNT(*) AS n GROUP BY s |> SELECT t.s",
        "FROM one_row AS t |> AGGREGATE COUNT(*) AS n GROUP BY s |> SELECT t.s",
        Outcome::Error("Unrecognized name: t"),
    ),
];

#[tokio::test]
async fn names_resolve_as_googlesql_resolves_them() {
    let catalog = catalog();
    let mut failures = Vec::new();
    for (googlesql, query, outcome) in RULES {
        let found = match pass::run_in(&pass::session(), &catalog, query).await {
            Ok(batches) => {
                let text = pretty_format_batches(&batches).unwrap().to_string();
                let lines: Vec<&str> = text.lines().collect();
                (lines[1].to_string(), lines[3].to_string())
            }
            Err(errors) => (String::new(), errors[0].clone()),
        };
        let expected = match outcome {
            Outcome::Row(header, row) => (header.to_string(), row.to_string()),
            Outcome::Error(message) => (String::new(), message.to_string()),
        };
        if found != expected {
            failures.push(format!(
                "{googlesql}: GoogleSQL {expected:?}, the pass {found:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The text of `range` in `text`.
fn at(text: &str, range: avenger_sql_syntax::TextRange) -> &str {
    &text[std::ops::Range::<usize>::from(range)]
}

#[test]
fn the_model_resolves_each_name_to_its_definition() {
    let text = "FROM one_row AS a |> EXTEND x + 1 AS y |> SELECT a.x, y, UPPER(s) AS u";
    let analysis = pass::analysis_in(&catalog(), text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let names: Vec<(&str, String)> = (analysis.model.names.iter())
        .map(|name| {
            let definition = match &name.definition {
                Definition::Column(column) => {
                    format!(
                        "column {} at {:?}",
                        column.name,
                        column.definition.map(|r| at(text, r))
                    )
                }
                Definition::RangeVariable { name, definition } => {
                    format!(
                        "range variable {name} at {:?}",
                        definition.map(|r| at(text, r))
                    )
                }
                Definition::Table { names } => format!("table {}", names.join(".")),
                Definition::Function { name, role } => format!("function {name} ({role:?})"),
                Definition::Field { name, .. } => format!("field {name}"),
                Definition::Query { name, .. } => format!("query {name}"),
                Definition::Variable { name, .. } => format!("variable {name}"),
                Definition::Module { names } => format!("module {}", names.join(".")),
            };
            (at(text, name.range), definition)
        })
        .collect();
    assert_eq!(
        names,
        [
            ("one_row", "table one_row".to_string()),
            ("x", "column x at Some(\"one_row\")".to_string()),
            ("a", "range variable a at Some(\"a\")".to_string()),
            ("x", "column x at Some(\"one_row\")".to_string()),
            ("y", "column y at Some(\"y\")".to_string()),
            ("UPPER", "function UPPER (Scalar)".to_string()),
            ("s", "column s at Some(\"one_row\")".to_string()),
        ]
    );
    // Each operator records its input, its range variables and its output, with their types.
    let select = analysis.model.operators.last().unwrap();
    let output: Vec<(&str, Option<&DataType>)> = (select.output.iter())
        .map(|column| (column.name.as_str(), column.ty.as_ref()))
        .collect();
    assert_eq!(
        output,
        [
            ("x", Some(&DataType::Int64)),
            ("y", Some(&DataType::Int64)),
            ("u", Some(&DataType::Utf8)),
        ]
    );
    assert_eq!(select.range_variables.len(), 1);
    assert_eq!(select.range_variables[0].name, "a");
    let qualifiers: Vec<&[String]> = (select.input.iter())
        .map(|column| column.qualifiers.as_slice())
        .collect();
    assert_eq!(
        qualifiers,
        [&["a".to_string()][..], &["a".to_string()], &[]]
    );
}

#[test]
fn an_operator_with_an_error_keeps_the_names_after_it() {
    let text = "FROM one_row |> SELECT nope AS z, x |> SELECT z, x |> EXTEND z + 1 AS w";
    let analysis = pass::analysis_in(&catalog(), text);
    let messages: Vec<&str> = (analysis.diagnostics.iter())
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(messages, ["Unrecognized name: nope"]);
    assert!(analysis.plan.is_none());
    // The operators after the error see `z` and `x`, without types.
    let last = analysis.model.operators.last().unwrap();
    let output: Vec<(&str, Option<&DataType>)> = (last.output.iter())
        .map(|column| (column.name.as_str(), column.ty.as_ref()))
        .collect();
    assert_eq!(output, [("z", None), ("x", None), ("w", None)]);
    let resolved: Vec<&str> = (analysis.model.names.iter())
        .filter(|name| matches!(name.definition, Definition::Column(_)))
        .map(|name| at(text, name.range))
        .collect();
    assert_eq!(resolved, ["x", "z", "x", "z"]);
}
