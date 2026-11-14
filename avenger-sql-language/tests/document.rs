//! The document's diagnostics and highlights, and its outline, folding, and selection ranges.

mod common;

use avenger_sql_language::{Highlight, Source, SymbolKind};
use avenger_sql_syntax::{FunctionRole, NameRole};
use common::{at, document, find};

#[test]
fn diagnostics_leave_out_the_analysis_of_a_broken_statement() {
    let text = "FROM penguins |> WHERE nope = ;\nFROM penguins |> SELECT missing;\nFROM penguins |> SELECT 5abc";
    let document = document(text);
    let diagnostics: Vec<(Source, &str)> = (document.diagnostics().iter())
        .map(|diagnostic| (diagnostic.source, at(&document, diagnostic.range)))
        .collect();
    // The first statement's syntax error, with no error of the analysis for `nope`, the second
    // statement's analysis error, and the third's syntax error.
    assert_eq!(
        diagnostics
            .iter()
            .map(|(source, _)| *source)
            .collect::<Vec<_>>(),
        [Source::Syntax, Source::Analysis, Source::Syntax],
        "{:?}",
        document.diagnostics()
    );
    assert_eq!(diagnostics[1].1, "missing");
    // The lexer's fix of a name that starts with a digit comes through.
    let last = document.diagnostics().pop().unwrap();
    assert_eq!(last.fixes.len(), 1);
    assert_eq!(last.fixes[0].edits[0].replacement, "`5abc`");
}

#[test]
fn highlights_take_the_roles_that_the_analysis_resolved() {
    let text = "DECLARE lo INT64 DEFAULT 1;\nFROM penguins |> WHERE body_mass_g > lo |> AGGREGATE bar(x => species) AS chart";
    let document = document(text);
    let highlights = document.highlights();
    let class = |needle: &str, n: usize| {
        let start = find(&document, needle, n);
        (highlights.iter())
            .find(|range| range.range.start() == start)
            .map(|range| range.highlight)
    };
    assert_eq!(class("lo", 1), Some(Highlight::Name(NameRole::Variable)));
    assert_eq!(
        class("body_mass_g", 0),
        Some(Highlight::Name(NameRole::Column))
    );
    assert_eq!(
        class("bar", 0),
        Some(Highlight::Name(NameRole::Function(FunctionRole::Mark)))
    );
}

#[test]
fn the_outline_holds_the_statements_and_the_operators_of_each_query() {
    let text = "DECLARE lo INT64;
CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);
FROM penguins
|> WITH heavy AS (FROM penguins |> WHERE body_mass_g > 4000)
|> WHERE body_mass_g > lo
|> AGGREGATE COUNT(*) AS n GROUP BY species";
    let document = document(text);
    let outline: Vec<(SymbolKind, String, Option<String>, Vec<String>)> =
        (document.symbols().iter())
            .map(|symbol| {
                let children = (symbol.children.iter())
                    .map(|child| {
                        let grandchildren: Vec<&str> =
                            child.children.iter().map(|g| g.name.as_str()).collect();
                        match grandchildren.is_empty() {
                            true => format!(
                                "{} [{}]",
                                child.name,
                                child.detail.clone().unwrap_or_default()
                            ),
                            false => format!("{} {grandchildren:?}", child.name),
                        }
                    })
                    .collect();
                (
                    symbol.kind,
                    symbol.name.clone(),
                    symbol.detail.clone(),
                    children,
                )
            })
            .collect();
    assert_eq!(
        outline,
        [
            (
                SymbolKind::Variable,
                "lo".into(),
                Some("INT64".into()),
                vec![]
            ),
            (
                SymbolKind::Function,
                "twice".into(),
                Some("(v INT64)".into()),
                vec![]
            ),
            (
                SymbolKind::Query,
                "FROM penguins".into(),
                None,
                vec![
                    "|> WITH [\"heavy\"]".to_string(),
                    "|> WHERE [species, island, bill_length_mm, body_mass_g]".to_string(),
                    "|> AGGREGATE [species, n]".to_string(),
                ]
            ),
        ]
    );
    // Each name is inside its symbol's range.
    for symbol in document.symbols() {
        assert!(symbol.range.contains_range(symbol.name_range), "{symbol:?}");
        assert_eq!(
            at(&document, symbol.name_range).trim(),
            symbol.name.trim_end_matches('…')
        );
    }
}

#[test]
fn folding_ranges_hold_multi_line_statements_calls_and_comments() {
    let text = "-- penguins
-- by species
FROM penguins
|> AGGREGATE bar(
  x => species,
  y => COUNT(*)
) AS chart;
FROM islands";
    let document = document(text);
    let ranges: Vec<(bool, &str)> = (document.folding_ranges().iter())
        .map(|range| (range.comment, at(&document, range.range)))
        .collect();
    assert!(
        ranges.contains(&(true, "-- penguins\n-- by species")),
        "{ranges:?}"
    );
    assert!(
        ranges.contains(&(false, "(\n  x => species,\n  y => COUNT(*)\n)")),
        "{ranges:?}"
    );
    assert!(
        ranges
            .iter()
            .any(|(comment, text)| !comment && text.starts_with("FROM penguins\n")),
        "{ranges:?}"
    );
    // A statement on one line doesn't fold.
    assert!(
        !ranges.iter().any(|(_, text)| *text == "FROM islands"),
        "{ranges:?}"
    );
}

#[test]
fn selection_ranges_grow_from_the_name_at_the_cursor() {
    let text = "FROM penguins |> WHERE ROUND(body_mass_g) > 4000";
    let document = document(text);
    let ranges: Vec<&str> = (document.selection_ranges(find(&document, "mass", 0)).iter())
        .map(|range| at(&document, *range))
        .collect();
    assert_eq!(ranges[0], "body_mass_g");
    assert!(ranges.contains(&"ROUND(body_mass_g)"), "{ranges:?}");
    assert!(ranges.contains(&"ROUND(body_mass_g) > 4000"), "{ranges:?}");
    assert!(
        ranges.contains(&"|> WHERE ROUND(body_mass_g) > 4000"),
        "{ranges:?}"
    );
    assert_eq!(*ranges.last().unwrap(), text);
    // Each range holds the one before it.
    for pair in document
        .selection_ranges(find(&document, "mass", 0))
        .windows(2)
    {
        assert!(pair[1].contains_range(pair[0]) && pair[1] != pair[0]);
    }
}
