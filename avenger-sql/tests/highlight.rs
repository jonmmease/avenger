//! The refinement of the syntax highlighter's ranges by the semantic model: the roles of the
//! names that the pass resolved, and the strings that a parameter takes as one value of a set.

mod pass;

use std::sync::Arc;

use avenger_sql::analysis::Analysis;
use avenger_sql_syntax::{highlight, parse, FunctionRole, Highlight, NameRole};
use datafusion::datasource::MemTable;

/// The ranges of `text`, refined by `analysis`: their text and class, without the classes of
/// tokens other than names and strings.
fn refined(text: &str, analysis: &Analysis) -> Vec<(String, Highlight)> {
    let mut ranges = highlight(&parse(text).syntax());
    analysis.model.refine_highlights(&mut ranges);
    ranges
        .into_iter()
        .filter(|range| {
            matches!(
                range.highlight,
                Highlight::Name(_) | Highlight::String | Highlight::EnumValue
            )
        })
        .map(|range| (text[range.range].to_string(), range.highlight))
        .collect()
}

/// The class of the `nth` range of `ranges` whose text is `word`, from 0.
fn class(ranges: &[(String, Highlight)], word: &str, nth: usize) -> Highlight {
    (ranges.iter())
        .filter(|(text, _)| text == word)
        .nth(nth)
        .map(|(_, highlight)| *highlight)
        .unwrap_or_else(|| panic!("no range {nth} of `{word}` in {ranges:?}"))
}

fn name(role: NameRole) -> Highlight {
    Highlight::Name(role)
}

fn function(role: FunctionRole) -> Highlight {
    name(NameRole::Function(role))
}

#[test]
fn names_take_the_roles_of_their_definitions() {
    let text = "DECLARE lo FLOAT64 DEFAULT 3000;\n\
                SET lo = lo + 1;\n\
                CREATE TEMP FUNCTION half(v FLOAT64) AS (v / 2);\n\
                CREATE TEMP TABLE FUNCTION evens(k INT64) AS (FROM t |> WHERE MOD(x, k) = 0);\n\
                CREATE TEMP FUNCTION heavy(v FLOAT64) AS (v > lo);\n\
                FROM penguins AS p\n\
                |> WHERE p.body_mass_g > lo\n\
                |> EXTEND half(p.body_mass_g) AS half_mass, UPPER(island) AS isle,\n\
                   ROW_NUMBER() OVER (ORDER BY body_mass_g) AS place, STRUCT(island AS home) AS info\n\
                |> EXTEND info.home AS home\n\
                |> AGGREGATE COUNT(*) AS n GROUP BY species;\n\
                FROM evens(2);\n\
                FROM t |> MATCH_RECOGNIZE(ORDER BY x MEASURES FIRST(a.s) AS first_s PATTERN (a+) DEFINE a AS x > 0)";
    let analysis = pass::analysis(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let ranges = refined(text, &analysis);
    // Variables, where the syntax gives a column.
    assert_eq!(class(&ranges, "lo", 1), name(NameRole::Variable));
    assert_eq!(class(&ranges, "lo", 2), name(NameRole::Variable));
    // In a temporary function's body, then in a WHERE.
    assert_eq!(class(&ranges, "lo", 3), name(NameRole::Variable));
    assert_eq!(class(&ranges, "lo", 4), name(NameRole::Variable));
    // A range variable and its column, where the syntax gives a column and a field.
    assert_eq!(class(&ranges, "p", 1), name(NameRole::RangeVariable));
    assert_eq!(class(&ranges, "body_mass_g", 0), name(NameRole::Column));
    // A field of a struct stays a field, and a definition keeps its class.
    assert_eq!(class(&ranges, "info", 1), name(NameRole::Column));
    assert_eq!(class(&ranges, "home", 1), name(NameRole::Field));
    assert_eq!(class(&ranges, "p", 0), name(NameRole::Alias));
    // A pattern variable that qualifies a column.
    assert_eq!(class(&ranges, "a", 0), name(NameRole::RangeVariable));
    // The roles of functions.
    assert_eq!(class(&ranges, "half", 1), function(FunctionRole::Scalar));
    assert_eq!(class(&ranges, "UPPER", 0), function(FunctionRole::Scalar));
    assert_eq!(
        class(&ranges, "ROW_NUMBER", 0),
        function(FunctionRole::Window)
    );
    assert_eq!(
        class(&ranges, "COUNT", 0),
        function(FunctionRole::Aggregate)
    );
    assert_eq!(class(&ranges, "evens", 1), function(FunctionRole::Table));
    assert_eq!(
        class(&ranges, "FIRST", 0),
        function(FunctionRole::Aggregate)
    );
}

#[test]
fn avengers_functions_and_their_enum_values() {
    let text = "FROM penguins\n\
                |> EXTEND stack(body_mass_g, offset => 'normalize') OVER (PARTITION BY island ORDER BY species) AS share,\n\
                   lump(island, n => 2, null_category => 'drop', other => 'Rest') OVER () AS isle\n\
                |> AGGREGATE bar(x => species, y => scaled(share.end, title => 'Share'), orient => 'vertical') AS chart;\n\
                FROM sales |> CALL impute(isl, [sp], m, method => 'avg')";
    let analysis = pass::analysis(text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let ranges = refined(text, &analysis);
    assert_eq!(
        class(&ranges, "stack", 0),
        function(FunctionRole::Transform)
    );
    assert_eq!(class(&ranges, "lump", 0), function(FunctionRole::Transform));
    assert_eq!(
        class(&ranges, "impute", 0),
        function(FunctionRole::Transform)
    );
    assert_eq!(class(&ranges, "bar", 0), function(FunctionRole::Mark));
    assert_eq!(
        class(&ranges, "scaled", 0),
        function(FunctionRole::Constructor)
    );
    // The strings of a set, and the strings of any value.
    for value in ["'normalize'", "'drop'", "'vertical'", "'avg'"] {
        assert_eq!(class(&ranges, value, 0), Highlight::EnumValue, "{value}");
    }
    assert_eq!(class(&ranges, "'Rest'", 0), Highlight::String);
    assert_eq!(class(&ranges, "'Share'", 0), Highlight::String);
}

#[test]
fn what_the_pass_does_not_resolve_keeps_the_syntax_classes() {
    // An unknown column, and a string that the call rejects.
    let text = "FROM t |> WHERE nope > 0;\n\
                FROM t |> EXTEND stack(x, offset => 'sideways') OVER () AS y";
    let analysis = pass::analysis(text);
    assert_eq!(analysis.diagnostics.len(), 2, "{:?}", analysis.diagnostics);
    let ranges = refined(text, &analysis);
    assert_eq!(class(&ranges, "nope", 0), name(NameRole::Column));
    assert_eq!(class(&ranges, "'sideways'", 0), Highlight::String);
}

#[test]
fn each_name_of_a_tables_path_is_a_table() {
    let (_, t) = pass::small();
    let table = Arc::new(MemTable::try_new(t.schema(), vec![vec![t]]).unwrap());
    let catalog = pass::catalog().with_table(&["samples", "t"], table);
    let text = "FROM samples.t AS samples |> SELECT samples.x";
    let analysis = pass::analysis_in(&catalog, text);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let ranges = refined(text, &analysis);
    assert_eq!(class(&ranges, "samples", 0), name(NameRole::Table));
    assert_eq!(class(&ranges, "t", 0), name(NameRole::Table));
    assert_eq!(class(&ranges, "samples", 2), name(NameRole::RangeVariable));
    assert_eq!(class(&ranges, "x", 0), name(NameRole::Column));
}
