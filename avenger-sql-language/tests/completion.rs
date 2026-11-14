//! Completion of the names that the grammar, the analysis, and the catalog know at a cursor.

mod common;

use avenger_sql_language::{CompletionItem, CompletionKind, Completions};
use avenger_sql_syntax::FunctionRole;
use common::{at, at_cursor};

/// The completions of `text` at its cursor marker, and the text that they replace.
fn complete(text: &str) -> (Completions, String) {
    let (document, offset) = at_cursor(text);
    let completions = document.completions(offset);
    let replaced = at(&document, completions.range).to_string();
    (completions, replaced)
}

/// The labels of the items of `kind`.
fn labels(completions: &Completions, kind: CompletionKind) -> Vec<&str> {
    (completions.items.iter())
        .filter(|item| item.kind == kind)
        .map(|item| item.label.as_str())
        .collect()
}

fn item<'a>(completions: &'a Completions, label: &str) -> Option<&'a CompletionItem> {
    completions.items.iter().find(|item| item.label == label)
}

#[test]
fn a_table_completes_from_the_catalog_and_the_script() {
    let (completions, _) = complete("CREATE TEMP TABLE big AS FROM penguins;\nFROM $0");
    let tables = labels(&completions, CompletionKind::Table);
    assert!(
        tables.contains(&"penguins") && tables.contains(&"islands"),
        "{tables:?}"
    );
    assert!(labels(&completions, CompletionKind::Query).contains(&"big"));
    let (completions, _) = complete("FROM penguins |> CALL $0");
    let functions = labels(
        &completions,
        CompletionKind::Function(FunctionRole::Transform),
    );
    assert!(functions.contains(&"impute"), "{:?}", completions.items);
}

#[test]
fn an_expression_completes_the_columns_of_its_input() {
    let (completions, replaced) = complete("FROM penguins |> WHERE bod$0");
    assert_eq!(replaced, "bod");
    let mass = item(&completions, "body_mass_g").unwrap();
    assert_eq!(mass.kind, CompletionKind::Column);
    assert_eq!(mass.detail.as_deref(), Some("INT64"));
    let (completions, replaced) = complete("FROM penguins |> WHERE $0");
    assert_eq!(replaced, "");
    assert_eq!(
        labels(&completions, CompletionKind::Column),
        ["bill_length_mm", "body_mass_g", "island", "species"]
    );
    // Columns come first. A `WHERE` takes a window, as `COUNT(*) OVER ()`, and no mark.
    assert_eq!(completions.items[0].kind, CompletionKind::Column);
    assert!(item(&completions, "ABS").is_some());
    assert!(item(&completions, "COUNT").is_some());
    assert!(item(&completions, "bar").is_none());
    // A `GROUP BY` takes no aggregate.
    let (completions, _) = complete("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY $0");
    assert!(item(&completions, "COUNT").is_none());
}

#[test]
fn an_aggregate_completes_marks_and_a_mark_its_parameters() {
    let (completions, _) = complete("FROM penguins |> AGGREGATE $0");
    assert!(item(&completions, "COUNT").is_some());
    let bar = item(&completions, "bar").unwrap();
    assert_eq!(bar.kind, CompletionKind::Function(FunctionRole::Mark));
    assert!(bar.documentation.as_deref().unwrap().contains("## `bar`"));
    // After a named argument, an argument takes only a name.
    let (completions, _) = complete("FROM penguins |> AGGREGATE bar(x => species, $0) AS chart");
    let parameters = labels(&completions, CompletionKind::Parameter);
    assert!(parameters.contains(&"y"), "{parameters:?}");
    assert!(!parameters.contains(&"x"), "{parameters:?}");
    assert_eq!(item(&completions, "y").unwrap().insert, "y => ");
    assert!(completions
        .items
        .iter()
        .all(|item| item.kind == CompletionKind::Parameter));
    // The value of a mark's argument, where the constructors come first among the functions.
    let (completions, _) =
        complete("FROM penguins |> AGGREGATE bar(x => species, y => $0) AS chart");
    let constructors = labels(
        &completions,
        CompletionKind::Function(FunctionRole::Constructor),
    );
    assert!(
        constructors.contains(&"scaled") && constructors.contains(&"direct"),
        "{constructors:?}"
    );
    assert!(item(&completions, "COUNT").is_none());
}

#[test]
fn a_dot_completes_the_columns_of_a_range_variable_and_the_fields_of_a_struct() {
    let (completions, _) = complete("FROM penguins AS p |> WHERE p.$0");
    assert_eq!(
        labels(&completions, CompletionKind::Column),
        ["bill_length_mm", "body_mass_g", "island", "species"]
    );
    assert_eq!(completions.items.len(), 4);
    let (completions, _) = complete("FROM islands |> WHERE location.l$0");
    assert_eq!(labels(&completions, CompletionKind::Field), ["lat", "lon"]);
}

#[test]
fn an_argument_completes_the_values_of_its_set() {
    let text =
        "FROM penguins |> EXTEND stack(body_mass_g, offset => $0) OVER (ORDER BY species) AS s";
    let (completions, _) = complete(text);
    let values = labels(&completions, CompletionKind::EnumValue);
    assert!(
        values.contains(&"'normalize'") && values.contains(&"'zero'"),
        "{values:?}"
    );
    // In the string literal, the item replaces the literal.
    let text = "FROM penguins |> EXTEND stack(body_mass_g, offset => 'nor$0') OVER (ORDER BY species) AS s";
    let (completions, replaced) = complete(text);
    assert_eq!(replaced, "'nor'");
    assert!(labels(&completions, CompletionKind::EnumValue).contains(&"'normalize'"));
}

#[test]
fn keywords_complete_operators_and_statements() {
    let (completions, _) = complete("FROM penguins |> $0");
    let keywords = labels(&completions, CompletionKind::Keyword);
    assert!(
        keywords.contains(&"AGGREGATE") && keywords.contains(&"ORDER BY"),
        "{keywords:?}"
    );
    let (completions, _) = complete("$0");
    let keywords = labels(&completions, CompletionKind::Keyword);
    assert!(
        keywords.contains(&"FROM") && keywords.contains(&"DECLARE"),
        "{keywords:?}"
    );
}

#[test]
fn a_variable_completes_after_its_declare() {
    let text = "DECLARE lo INT64 DEFAULT 1;\nFROM penguins |> WHERE body_mass_g > $0;\nDECLARE hi INT64 DEFAULT 2";
    let (completions, _) = complete(text);
    assert_eq!(labels(&completions, CompletionKind::Variable), ["lo"]);
    assert_eq!(
        item(&completions, "lo").unwrap().detail.as_deref(),
        Some("INT64")
    );
}

#[test]
fn a_subquery_completes_its_own_columns_before_those_around_it() {
    let text = "FROM penguins |> WHERE island IN (FROM islands |> WHERE name = $0 |> SELECT name)";
    let (completions, _) = complete(text);
    let columns = labels(&completions, CompletionKind::Column);
    let name = columns.iter().position(|c| *c == "name").unwrap();
    let species = columns.iter().position(|c| *c == "species").unwrap();
    assert!(name < species, "{columns:?}");
}

#[test]
fn a_reserved_name_inserts_in_backticks() {
    let (completions, _) = complete("FROM islands |> WHERE $0");
    assert_eq!(item(&completions, "order").unwrap().insert, "`order`");
}

#[test]
fn over_completes_after_a_window_or_aggregate_function_in_a_select_item() {
    let over = |text: &str| {
        let (completions, _) = complete(text);
        labels(&completions, CompletionKind::Keyword).contains(&"OVER")
    };
    assert!(over("FROM penguins |> EXTEND ROW_NUMBER() $0"));
    assert!(over("FROM penguins |> EXTEND SUM(body_mass_g) $0"));
    assert!(!over("FROM penguins |> EXTEND ABS(body_mass_g) $0"));
    // An `|> AGGREGATE` computes no window.
    assert!(!over("FROM penguins |> AGGREGATE COUNT(*) $0"));
}

#[test]
fn a_window_completes_the_clauses_that_its_function_takes() {
    let keywords = |text: &str| {
        let (completions, _) = complete(text);
        (labels(&completions, CompletionKind::Keyword).into_iter())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    // `ROW_NUMBER` takes no frame. Nothing after the `)` that the text lacks.
    assert_eq!(
        keywords("FROM penguins |> EXTEND ROW_NUMBER() OVER ($0"),
        ["ORDER BY", "PARTITION BY"]
    );
    let after_key =
        keywords("FROM penguins |> EXTEND SUM(body_mass_g) OVER (PARTITION BY species $0)");
    for keyword in ["ORDER BY", "RANGE", "ROWS"] {
        assert!(
            after_key.iter().any(|k| k == keyword),
            "{keyword}: {after_key:?}"
        );
    }
    assert!(
        !after_key.iter().any(|k| k == "AS" || k == "WINDOW"),
        "{after_key:?}"
    );
    // Statements start after a `;`.
    let after_condition = keywords("FROM penguins |> WHERE species = 'Adelie' $0");
    assert!(
        after_condition.iter().any(|k| k == "AND"),
        "{after_condition:?}"
    );
    assert!(
        !after_condition
            .iter()
            .any(|k| k == "DECLARE" || k == "FROM"),
        "{after_condition:?}"
    );
}

#[test]
fn a_window_name_completes_from_the_window_clause() {
    let text = "FROM penguins |> EXTEND RANK() OVER $0 AS r WINDOW w AS (ORDER BY species)";
    let (completions, _) = complete(text);
    assert_eq!(labels(&completions, CompletionKind::Window), ["w"]);
}

#[test]
fn a_name_in_the_middle_of_a_word_replaces_the_whole_word() {
    let (document, offset) = at_cursor("FROM penguins |> WHERE bod$0y_mass_g > 1");
    let completions = document.completions(offset);
    assert_eq!(at(&document, completions.range), "bod");
    assert_eq!(at(&document, completions.replace), "body_mass_g");
    assert!(item(&completions, "body_mass_g").is_some());
}

#[test]
fn a_quoted_name_completes_inside_its_backticks() {
    // Without the closing backtick, an item inserts it.
    let (document, offset) = at_cursor("FROM islands |> WHERE `$0");
    let completions = document.completions(offset);
    assert_eq!(item(&completions, "order").unwrap().insert, "order`");
    assert!(labels(&completions, CompletionKind::Keyword).is_empty());
    // With it, an item replaces the name up to the backtick.
    let (document, offset) = at_cursor("FROM islands |> WHERE `or$0de` = 1");
    let completions = document.completions(offset);
    assert_eq!(at(&document, completions.range), "or");
    assert_eq!(at(&document, completions.replace), "orde");
    assert_eq!(item(&completions, "order").unwrap().insert, "order");
    // After a range variable's dot.
    let (completions, replaced) = complete("FROM penguins AS p |> WHERE p.`bo$0");
    assert_eq!(replaced, "bo");
    assert_eq!(
        item(&completions, "body_mass_g").unwrap().insert,
        "body_mass_g`"
    );
}

#[test]
fn a_place_takes_what_the_analysis_accepts_there() {
    // `LIMIT` takes a constant, and no column.
    let (completions, _) = complete("FROM penguins |> LIMIT $0");
    assert!(labels(&completions, CompletionKind::Column).is_empty());
    // A list of columns takes only columns.
    let (completions, _) = complete("FROM islands |> UNION ALL CORRESPONDING BY ($0");
    assert!(labels(&completions, CompletionKind::Column).contains(&"name"));
    assert!(!completions
        .items
        .iter()
        .any(|i| matches!(i.kind, CompletionKind::Function(_))));
    // After a pipe `CALL`, table functions, and no table.
    let (completions, _) = complete("FROM penguins |> CALL $0");
    assert!(labels(&completions, CompletionKind::Table).is_empty());
    // An item of `|> AGGREGATE` takes no column by itself.
    let (completions, _) = complete("FROM penguins |> AGGREGATE $0");
    assert!(labels(&completions, CompletionKind::Column).is_empty());
    // The functions of `MATCH_RECOGNIZE` only in their clauses.
    let (completions, _) = complete("FROM penguins |> EXTEND $0");
    assert!(item(&completions, "MATCH_NUMBER").is_none());
    // A filter takes a `BOOL`, so its `BOOL` columns come first.
    let (completions, _) = complete("FROM islands |> WHERE $0");
    assert_eq!(completions.items[0].label, "coastal");
}

#[test]
fn a_named_argument_at_the_cursor_completes_its_own_name() {
    let (completions, _) = complete("FROM penguins |> AGGREGATE bar($0x => species) AS chart");
    assert!(labels(&completions, CompletionKind::Parameter).contains(&"x"));
}

#[test]
fn a_statement_does_not_complete_its_own_definition() {
    let (completions, _) = complete("CREATE TEMP TABLE w AS FROM $0");
    assert!(!labels(&completions, CompletionKind::Query).contains(&"w"));
    let (completions, _) = complete("CREATE TEMP TABLE w AS FROM penguins;\nFROM $0");
    assert!(labels(&completions, CompletionKind::Query).contains(&"w"));
    // A `DECLARE` comes before the other statements.
    let (completions, _) = complete("DECLARE a INT64;\n$0");
    assert!(labels(&completions, CompletionKind::Keyword).contains(&"DECLARE"));
    let (completions, _) = complete("FROM penguins;\n$0");
    assert!(!labels(&completions, CompletionKind::Keyword).contains(&"DECLARE"));
}

#[test]
fn a_body_completes_the_parameters_of_its_function() {
    let text = "CREATE TEMP FUNCTION twice(v INT64) AS ($0v * 2);\nFROM penguins";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::Column).contains(&"v"));
    let text = "CREATE TEMP TABLE FUNCTION tf(src ANY TABLE) AS (FROM $0);\nFROM penguins";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::Table).contains(&"src"));
}

#[test]
fn completion_offers_the_words_that_the_parser_expects() {
    let keywords = |text: &str| {
        let (completions, _) = complete(text);
        (labels(&completions, CompletionKind::Keyword).into_iter())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert!(keywords("FROM penguins |> SELECT INTERVAL 1 $0").contains(&"YEAR".to_string()));
    assert!(keywords("FROM penguins |> TABLESAMPLE $0").contains(&"BERNOULLI".to_string()));
    assert!(keywords("FROM penguins |> UNION ALL $0").contains(&"BY NAME".to_string()));
    assert!(keywords("FROM penguins |> WHERE $0").contains(&"EXISTS".to_string()));
    assert!(keywords("FROM penguins |> WHERE species $0").contains(&"IN".to_string()));
    assert!(keywords("FROM penguins |> $0").contains(&"INNER UNION ALL BY NAME".to_string()));
}

#[test]
fn a_dot_after_an_expression_completes_the_fields_of_its_struct() {
    let (completions, _) = complete("FROM penguins |> SELECT STRUCT(species AS a, island AS b).$0");
    assert_eq!(labels(&completions, CompletionKind::Field), ["a", "b"]);
}

#[test]
fn a_window_completes_its_base_window() {
    let text =
        "FROM penguins |> EXTEND RANK() OVER ($0 ORDER BY species) AS r WINDOW w AS (PARTITION BY island)";
    let (completions, _) = complete(text);
    assert_eq!(labels(&completions, CompletionKind::Window), ["w"]);
}

#[test]
fn a_clause_and_a_value_table_complete_the_columns_that_they_read() {
    // A value table's struct fields are its columns.
    let (completions, _) = complete("FROM UNNEST([STRUCT(1 AS a, 'p' AS b)]) |> SELECT $0");
    let columns = labels(&completions, CompletionKind::Column);
    assert!(
        columns.contains(&"a") && columns.contains(&"b"),
        "{columns:?}"
    );
    // A `PIVOT` takes an aggregate, and its arguments the input's columns, without a range
    // variable.
    let (completions, _) = complete("FROM penguins |> PIVOT ($0");
    assert!(item(&completions, "SUM").is_some());
    assert!(labels(&completions, CompletionKind::Keyword).is_empty());
    let (completions, _) = complete("FROM penguins PIVOT (MAX($0");
    assert!(labels(&completions, CompletionKind::Column).contains(&"body_mass_g"));
    assert!(labels(&completions, CompletionKind::RangeVariable).is_empty());
    // A list of columns takes no range variable.
    let (completions, _) = complete("FROM islands AS i |> UNION ALL CORRESPONDING BY ($0");
    assert!(labels(&completions, CompletionKind::RangeVariable).is_empty());
}

#[test]
fn a_table_function_body_and_a_subquery_complete_their_names() {
    let text =
        "CREATE TEMP TABLE FUNCTION tf(n INT64) AS (FROM UNNEST(GENERATE_ARRAY(1, $0)) AS v);";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::Column).contains(&"n"));
    // A pipe `WITH` name in a subquery that the text has not closed.
    let (completions, _) =
        complete("FROM penguins |> WITH w AS (FROM islands) |> WHERE island IN (FROM $0");
    assert!(labels(&completions, CompletionKind::Query).contains(&"w"));
}

#[test]
fn a_correlated_subquery_completes_the_range_variables_of_its_enclosing_query() {
    let text = "FROM islands AS i |> WHERE EXISTS (FROM penguins |> WHERE $0)";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::RangeVariable).contains(&"i"));
    let text = "FROM islands AS i |> WHERE EXISTS (FROM penguins |> WHERE i.$0)";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::Column).contains(&"coastal"));
    // An operand of a set operation reads no other query.
    let text = "FROM islands AS i |> UNION ALL (FROM islands AS j |> WHERE $0)";
    let (completions, _) = complete(text);
    let variables = labels(&completions, CompletionKind::RangeVariable);
    assert_eq!(variables, ["j"]);
}

#[test]
fn a_window_completes_no_aggregate_in_its_keys() {
    let text = "FROM penguins |> EXTEND RANK() OVER w AS r WINDOW w AS (PARTITION BY $0)";
    let (completions, _) = complete(text);
    assert!(labels(&completions, CompletionKind::Column).contains(&"island"));
    assert!(item(&completions, "SUM").is_none());
    assert!(item(&completions, "RANK").is_none());
    let (completions, _) = complete("FROM penguins |> EXTEND RANK() OVER (ORDER BY $0) AS r");
    assert!(item(&completions, "SUM").is_none());
}
