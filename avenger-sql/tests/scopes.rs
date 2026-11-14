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
                Definition::Function {
                    name,
                    role,
                    definition,
                } => match definition {
                    Some(r) => format!("function {name} ({role:?}) at {:?}", at(text, *r)),
                    None => format!("function {name} ({role:?})"),
                },
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
fn a_temporary_function_is_defined_by_the_name_of_its_create() {
    let text =
        "CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);\nFROM one_row |> SELECT twice(x) AS y";
    let analysis = pass::analysis_in(&catalog(), text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    // The call's name is defined by the name of the `CREATE`, at the start of the text.
    let definitions: Vec<(usize, Option<usize>)> = (analysis.model.names.iter())
        .filter_map(|name| match &name.definition {
            Definition::Function { definition, .. } => Some((
                usize::from(name.range.start()),
                definition.map(|r| usize::from(r.start())),
            )),
            _ => None,
        })
        .collect();
    let create = text.find("twice").unwrap();
    assert_eq!(
        definitions,
        [(text.find("twice(x)").unwrap(), Some(create))]
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

/// The analysis of `text`, which can have errors.
fn analysis_of(text: &str) -> avenger_sql::analysis::Analysis {
    let parse = avenger_sql_syntax::parse(text);
    avenger_sql::analyze(
        &parse.syntax(),
        &catalog(),
        &avenger_sql::library::Library::new(),
        &avenger_sql::analysis::Options::default(),
    )
}

#[test]
fn each_clause_records_the_place_of_its_expressions() {
    use avenger_sql::analysis::PlaceKind;
    let text = "FROM one_row |> WHERE x > 1 |> EXTEND x + 1 AS y \
                |> AGGREGATE SUM(y) AS t GROUP BY s |> LIMIT 1";
    let analysis = analysis_of(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let at = |needle: &str| {
        let offset = text.find(needle).unwrap() as u32;
        analysis.model.place_at(offset.into()).unwrap().clone()
    };
    let filter = at("x > 1");
    assert_eq!(filter.kind, PlaceKind::Filter);
    assert_eq!(filter.expected, Some(DataType::Boolean));
    assert!(filter.windows && !filter.aggregates);
    assert_eq!(at("x + 1").kind, PlaceKind::SelectItem);
    let item = at("SUM(y)");
    assert_eq!(item.kind, PlaceKind::AggregateItem);
    assert!(item.aggregates && !item.takes_columns());
    let argument = at("y) AS t");
    assert_eq!(argument.kind, PlaceKind::Argument);
    assert!(!argument.aggregates && argument.takes_columns());
    assert_eq!(at("s |> LIMIT").kind, PlaceKind::GroupKey);
    let count = text.rfind('1').unwrap() as u32;
    let constant = analysis.model.place_at(count.into()).unwrap();
    assert_eq!(constant.kind, PlaceKind::Constant);
    assert!(!constant.takes_columns());
}

#[test]
fn a_clause_without_expressions_at_the_end_of_a_text_has_a_place() {
    use avenger_sql::analysis::PlaceKind;
    for (text, kind) in [
        ("FROM one_row |> WHERE ", PlaceKind::Filter),
        ("FROM one_row |> AGGREGATE ", PlaceKind::AggregateItem),
        ("FROM one_row |> AGGREGATE SUM(", PlaceKind::Argument),
        ("FROM one_row |> LIMIT ", PlaceKind::Constant),
        (
            "FROM one_row |> UNION ALL CORRESPONDING BY (",
            PlaceKind::ColumnList,
        ),
    ] {
        // As completion does, the place of the last token before the cursor.
        let analysis = analysis_of(text);
        let place = analysis
            .model
            .place_at((text.trim_end().len() as u32).into());
        assert_eq!(place.map(|place| place.kind), Some(kind), "{text}");
    }
}

#[test]
fn a_place_takes_the_functions_that_the_pass_accepts_there() {
    let library = avenger_sql::library::Library::new();
    // Each place, with `{}` for a call.
    let places = [
        "FROM one_row |> WHERE {} IS NOT NULL",
        "FROM one_row |> EXTEND {} AS v",
        "FROM one_row |> AGGREGATE {} AS v",
        "FROM one_row |> AGGREGATE COUNT({}) AS v",
        "FROM one_row |> ORDER BY {}",
    ];
    // Each function, and its call without and with `OVER`.
    let calls = [
        ("SUM", "SUM(x)"),
        ("ROW_NUMBER", "ROW_NUMBER()"),
        ("UPPER", "UPPER(s)"),
        ("bar", "bar(x => s)"),
        ("MATCH_NUMBER", "MATCH_NUMBER()"),
    ];
    // Each text is valid but for the call, so the pass accepts the call when it reports no error.
    let mut failures = Vec::new();
    for place in places {
        for (name, call) in calls {
            // A scalar function stands in an item of `|> AGGREGATE` over an aggregate.
            let call = match (place, name) {
                ("FROM one_row |> AGGREGATE {} AS v", "UPPER") => "UPPER(MAX(s))",
                _ => call,
            };
            let accepts = |call: &str| {
                analysis_of(&place.replace("{}", call))
                    .diagnostics
                    .is_empty()
            };
            let accepted = accepts(call) || accepts(&format!("{call} OVER ()"));
            let text = place.replace("{}", call);
            let offset = text.find(call).unwrap() as u32;
            let analysis = analysis_of(&text);
            let takes = analysis
                .model
                .place_at(offset.into())
                .is_some_and(|p| p.takes(library.get(name).unwrap()));
            if takes != accepted {
                failures.push(format!(
                    "{text}: takes {takes}, the pass accepts {accepted}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_function_body_is_a_scope_of_its_parameters() {
    let text = "CREATE TEMP FUNCTION f(a INT64) AS (a + 1);\nFROM one_row |> SELECT f(x) AS v";
    let analysis = analysis_of(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let offset = text.find("a + 1").unwrap() as u32;
    let scope = analysis.model.operator_at(offset.into()).unwrap();
    let input: Vec<(&str, Option<&DataType>)> = (scope.input.iter())
        .map(|column| (column.name.as_str(), column.ty.as_ref()))
        .collect();
    assert_eq!(input, [("a", Some(&DataType::Int64))]);
}

#[test]
fn a_field_access_records_the_type_before_its_dot() {
    let text = "FROM one_row |> SELECT STRUCT(x AS a, s AS b).a AS v";
    let analysis = analysis_of(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let dot = text.find(").a").unwrap() as u32 + 1;
    let ty = analysis.model.field_base_at(dot.into()).unwrap();
    let DataType::Struct(fields) = ty else {
        panic!("{ty:?}")
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, ["a", "b"]);
}

#[test]
fn an_unpivot_without_value_columns_reports_an_error() {
    // The completion evaluation found a panic here: a group of columns without values.
    let analysis = analysis_of("FROM one_row UNPIVOT (FOR ( ))");
    let parse = avenger_sql_syntax::parse("FROM one_row UNPIVOT (FOR ( ))");
    assert!(!parse.ok() || !analysis.diagnostics.is_empty());
}

#[test]
fn a_pivot_and_an_unpivot_read_the_columns_of_their_input() {
    for (text, needle) in [
        ("FROM one_row |> PIVOT (SUM(x) FOR s IN ('a'))", "x) FOR"),
        ("FROM one_row |> UNPIVOT (v FOR k IN (x))", "x))"),
    ] {
        let analysis = analysis_of(text);
        let offset = text.find(needle).unwrap() as u32;
        let scope = analysis.model.operator_at(offset.into()).unwrap();
        let input: Vec<&str> = scope.input.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(input, ["x", "s"], "{text}");
        assert!(scope.range_variables.is_empty(), "{text}");
    }
}

#[test]
fn a_scope_holds_the_queries_in_scope() {
    let text = "CREATE TEMP TABLE big AS FROM one_row;\nFROM one_row |> WITH w AS (FROM one_row) |> WHERE x IN (FROM w)";
    let analysis = analysis_of(text);
    let offset = text.rfind("w)").unwrap() as u32;
    let scope = analysis.model.operator_at(offset.into()).unwrap();
    assert!(
        scope.queries.contains(&"w".to_string()),
        "{:?}",
        scope.queries
    );
    assert!(
        scope.queries.contains(&"big".to_string()),
        "{:?}",
        scope.queries
    );
}

#[test]
fn a_table_function_body_is_a_scope_of_its_value_parameters() {
    let text = "CREATE TEMP TABLE FUNCTION tf(n INT64, src ANY TABLE) AS (FROM src |> LIMIT n);\nFROM one_row";
    let analysis = analysis_of(text);
    let offset = text.find("LIMIT").unwrap() as u32;
    let scope = (analysis.model.operators.iter())
        .find(|scope| scope.range.contains(offset.into()) && !scope.input.is_empty())
        .unwrap();
    let input: Vec<&str> = scope.input.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(input, ["n"]);
}

#[test]
fn an_aggregate_item_takes_no_function_of_constants() {
    let library = avenger_sql::library::Library::new();
    let text = "FROM one_row |> AGGREGATE COUNT(*) AS n";
    let analysis = analysis_of(text);
    let place = analysis
        .model
        .place_at((text.find("COUNT").unwrap() as u32).into())
        .unwrap();
    assert!(!place.takes(library.get("CURRENT_DATE").unwrap()));
    assert!(place.takes(library.get("ABS").unwrap()));
}
