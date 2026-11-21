//! Syntax highlighting: the classes of the tokens of a tree.

mod common;

use avenger_sql_syntax::{
    highlight, parse, parse_data, parse_module, FunctionRole, Highlight, NameRole,
};
use common::*;

/// The highlighted ranges of `text`: their text, class and whether they define a name.
fn classes(text: &str) -> Vec<(String, Highlight, bool)> {
    highlight(&parse(text).syntax())
        .into_iter()
        .map(|range| {
            (
                text[range.range].to_string(),
                range.highlight,
                range.definition,
            )
        })
        .collect()
}

/// The class of the first range of `text` whose text is `word`.
fn class_of(text: &str, word: &str) -> (Highlight, bool) {
    classes(text)
        .into_iter()
        .find(|(found, _, _)| found == word)
        .map(|(_, highlight, definition)| (highlight, definition))
        .unwrap_or_else(|| panic!("no range `{word}` in {text}: {:?}", classes(text)))
}

fn name(role: NameRole) -> Highlight {
    Highlight::Name(role)
}

#[test]
fn a_word_is_a_keyword_where_the_parser_reads_one() {
    let text = "FROM t |> SELECT name, offset |> UNION ALL BY NAME (FROM u)";
    let ranges = classes(text);
    assert!(ranges.contains(&("name".to_string(), name(NameRole::Column), false)));
    assert!(ranges.contains(&("offset".to_string(), name(NameRole::Column), false)));
    assert!(ranges.contains(&("NAME".to_string(), Highlight::Keyword, false)));
}

#[test]
fn a_name_takes_the_role_of_its_node() {
    let text = "FROM my-project-2.ds.t AS o JOIN UNNEST(o.items) AS i \
                |> WHERE EXTRACT(MONTH FROM o.day) = @m AND f(x => 1) > 0 \
                |> EXTEND CAST(o.total AS NUMERIC) AS total, SUM(i) OVER w AS s \
                WINDOW w AS (ORDER BY o.day)";
    assert_eq!(
        class_of(text, "my-project-2."),
        (name(NameRole::Table), false)
    );
    assert_eq!(class_of(text, "ds"), (name(NameRole::Table), false));
    assert_eq!(class_of(text, "o"), (name(NameRole::Alias), true));
    assert_eq!(class_of(text, "items"), (name(NameRole::Field), false));
    assert_eq!(class_of(text, "MONTH"), (name(NameRole::DatePart), false));
    assert_eq!(class_of(text, "@m"), (Highlight::QueryParameter, false));
    let call = name(NameRole::Function(FunctionRole::Unresolved));
    assert_eq!(class_of(text, "f"), (call, false));
    assert_eq!(class_of(text, "x"), (name(NameRole::Parameter), false));
    assert_eq!(class_of(text, "NUMERIC"), (name(NameRole::Type), false));
    assert_eq!(class_of(text, "SUM"), (call, false));
    assert_eq!(class_of(text, "total"), (name(NameRole::Field), false));
    assert_eq!(class_of(text, "w"), (name(NameRole::Window), false));
}

#[test]
fn definitions_are_marked() {
    let text = "DECLARE n INT64 DEFAULT 1; \
                CREATE TEMP FUNCTION add(a INT64) AS (a + n); \
                CREATE TEMP TABLE t (k STRING) ; \
                FROM t |> WITH u AS (FROM t) |> AS v";
    assert_eq!(class_of(text, "n"), (name(NameRole::Variable), true));
    let scalar = name(NameRole::Function(FunctionRole::Scalar));
    assert_eq!(class_of(text, "add"), (scalar, true));
    assert_eq!(class_of(text, "a"), (name(NameRole::Parameter), true));
    assert_eq!(class_of(text, "k"), (name(NameRole::Column), true));
    assert_eq!(class_of(text, "u"), (name(NameRole::Query), true));
    assert_eq!(class_of(text, "v"), (name(NameRole::Alias), true));
    let module = "MODULE charts.scales; IMPORT MODULE base AS b;";
    let ranges = highlight(&parse_module(module).syntax());
    let first = ranges
        .iter()
        .find(|range| &module[range.range] == "charts")
        .unwrap();
    assert_eq!(first.highlight, name(NameRole::Module));
}

#[test]
fn the_syntax_tells_table_functions_and_the_kinds_of_created_functions() {
    let role = |role| name(NameRole::Function(role));
    let text = "CREATE TEMP AGGREGATE FUNCTION total(x INT64) AS (SUM(x)); \
                CREATE TEMP TABLE FUNCTION rows_of(n INT64) AS (FROM t); \
                FROM rows_of(3) |> CALL impute(k, v, value => 0)";
    assert_eq!(
        class_of(text, "total"),
        (role(FunctionRole::Aggregate), true)
    );
    assert_eq!(class_of(text, "rows_of"), (role(FunctionRole::Table), true));
    assert_eq!(
        classes(text)
            .iter()
            .filter(|(word, ..)| word == "rows_of")
            .nth(1)
            .map(|(_, highlight, definition)| (*highlight, *definition)),
        Some((role(FunctionRole::Table), false))
    );
    assert_eq!(class_of(text, "impute"), (role(FunctionRole::Table), false));
    assert_eq!(
        class_of(text, "SUM"),
        (role(FunctionRole::Unresolved), false)
    );
}

#[test]
fn broken_text_keeps_its_classes() {
    let text = "FROM t |> WHERE a = |> SELECT 'x' -- note";
    assert!(!parse(text).ok());
    let ranges = classes(text);
    assert!(ranges.contains(&("SELECT".to_string(), Highlight::Keyword, false)));
    assert!(ranges.contains(&("'x'".to_string(), Highlight::String, false)));
    assert!(ranges.contains(&("-- note".to_string(), Highlight::Comment, false)));
}

#[test]
fn the_names_of_every_operator() {
    // The roles of the names, which the parser's nodes give them. The classes of the other
    // tokens follow from their kinds.
    let ranges = highlight(&parse(OPERATORS).syntax());
    let dump: String = ranges
        .iter()
        .filter(|range| matches!(range.highlight, Highlight::Name(_)))
        .map(|range| {
            let mark = if range.definition { " definition" } else { "" };
            format!(
                "{:?} {:?} {:?}{mark}\n",
                range.range, &OPERATORS[range.range], range.highlight
            )
        })
        .collect();
    assert_text_baseline("highlight", &dump);
}

#[test]
fn a_reserved_word_that_names_an_argument_or_a_call_is_a_name() {
    let text = "FROM t |> SELECT f(range => 1, select => 2) AS a, select(selection => 'b') AS b";
    assert_eq!(class_of(text, "range"), (name(NameRole::Parameter), false));
    let ranges = classes(text);
    let selects: Vec<&Highlight> = (ranges.iter())
        .filter(|(word, _, _)| word == "select")
        .map(|(_, highlight, _)| highlight)
        .collect();
    assert_eq!(selects.len(), 2, "{ranges:?}");
    assert_eq!(*selects[0], name(NameRole::Parameter));
    assert!(
        matches!(selects[1], Highlight::Name(NameRole::Function(_))),
        "{ranges:?}"
    );
    assert_eq!(class_of(text, "SELECT"), (Highlight::Keyword, false));
}

#[test]
fn a_data_script_defines_tables_and_schemas() {
    let text = "CREATE SCHEMA finance; \
                CREATE EXTERNAL TABLE finance.orders (id INT64) OPTIONS (format = 'CSV'); \
                LOAD DATA INTO TEMP TABLE rates FROM FILES (uris = ['r.csv']); \
                CREATE VIEW recent AS FROM finance.orders";
    let classes: Vec<(String, Highlight, bool)> = highlight(&parse_data(text).syntax())
        .into_iter()
        .map(|range| {
            (
                text[range.range].to_string(),
                range.highlight,
                range.definition,
            )
        })
        .collect();
    let class = |word: &str| {
        (classes.iter())
            .find(|(found, _, _)| found == word)
            .map(|(_, highlight, definition)| (*highlight, *definition))
            .unwrap_or_else(|| panic!("no range `{word}`: {classes:?}"))
    };
    assert_eq!(class("finance"), (name(NameRole::Schema), true));
    assert_eq!(class("orders"), (name(NameRole::Table), true));
    assert_eq!(class("id"), (name(NameRole::Column), true));
    assert_eq!(class("format"), (name(NameRole::Option), true));
    assert_eq!(class("rates"), (name(NameRole::Table), true));
    assert_eq!(class("uris"), (name(NameRole::Option), true));
    assert_eq!(class("recent"), (name(NameRole::Table), true));
    // The format of files is a constant, and a URI of files a path.
    assert_eq!(class("'CSV'"), (Highlight::Format, false));
    assert_eq!(class("'r.csv'"), (Highlight::Uri, false));
    for keyword in [
        "EXTERNAL", "LOAD", "DATA", "INTO", "TEMP", "FILES", "VIEW", "SCHEMA",
    ] {
        assert_eq!(class(keyword), (Highlight::Keyword, false), "{keyword}");
    }
}

#[test]
fn only_the_options_of_files_hold_formats_and_uris() {
    let text = "CREATE VIEW v OPTIONS (description = 'CSV') AS FROM t; \
                CREATE EXTERNAL TABLE e OPTIONS (format = 'JSON', uris = ['a.json', 'b.json'], \
                description = 'x');";
    let classes: Vec<(String, Highlight)> = highlight(&parse_data(text).syntax())
        .into_iter()
        .map(|range| (text[range.range].to_string(), range.highlight))
        .collect();
    let class = |word: &str| {
        (classes.iter())
            .find(|(found, _)| found == word)
            .map(|(_, highlight)| *highlight)
            .unwrap_or_else(|| panic!("no range `{word}`: {classes:?}"))
    };
    assert_eq!(class("'CSV'"), Highlight::String);
    assert_eq!(class("'JSON'"), Highlight::Format);
    assert_eq!(class("'a.json'"), Highlight::Uri);
    assert_eq!(class("'b.json'"), Highlight::Uri);
    assert_eq!(class("'x'"), Highlight::String);
}
