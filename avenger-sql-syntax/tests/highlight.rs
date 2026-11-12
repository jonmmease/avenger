//! Syntax highlighting: the classes of the tokens of a tree.

mod common;

use avenger_sql_syntax::{highlight, parse, parse_module, Highlight, NameRole};
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
    assert_eq!(class_of(text, "f"), (name(NameRole::Function), false));
    assert_eq!(class_of(text, "x"), (name(NameRole::Parameter), false));
    assert_eq!(class_of(text, "NUMERIC"), (name(NameRole::Type), false));
    assert_eq!(class_of(text, "SUM"), (name(NameRole::Function), false));
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
    assert_eq!(class_of(text, "add"), (name(NameRole::Function), true));
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
fn broken_text_keeps_its_classes() {
    let text = "FROM t |> WHERE a = |> SELECT 'x' -- note";
    assert!(!parse(text).ok());
    let ranges = classes(text);
    assert!(ranges.contains(&("SELECT".to_string(), Highlight::Keyword, false)));
    assert!(ranges.contains(&("'x'".to_string(), Highlight::String, false)));
    assert!(ranges.contains(&("-- note".to_string(), Highlight::Comment, false)));
}

#[test]
fn the_classes_of_every_operator() {
    let ranges = highlight(&parse(OPERATORS).syntax());
    let dump: String = ranges
        .iter()
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
