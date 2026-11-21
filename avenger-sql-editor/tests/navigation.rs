//! The definition and the references of the name at the cursor, in characters, on a script with
//! a variable, a temporary function, and a pipe alias.

use arrow::datatypes::DataType;
use avenger_sql_editor::assist::{definition, references};
use avenger_sql_editor::document::{catalog, Analyzed};
use avenger_sql_editor::samples::Sample;

/// A script whose names are each defined once and used after, with a multi-byte character in
/// a comment before them, so that character indices differ from byte offsets.
const SCRIPT: &str = "-- Pingüinos\n\
DECLARE threshold FLOAT64 DEFAULT 4;\n\
CREATE TEMP FUNCTION grams(mass FLOAT64) AS (mass / 1000);\n\
FROM samples.penguins\n\
|> AS p\n\
|> WHERE grams(p.body_mass_g) > threshold\n\
|> AGGREGATE mark_bar(x => p.species, y => count(*))\n";

fn analyzed() -> Analyzed {
    let penguins = Sample {
        name: "penguins".to_string(),
        columns: vec![
            ("species".to_string(), DataType::Utf8),
            ("body_mass_g".to_string(), DataType::Int64),
        ],
    };
    let mut analyzed = Analyzed::new();
    analyzed.set_catalog(catalog(&[], &[penguins]));
    analyzed
}

/// The character index of the `nth` occurrence of `name` in the script, from 0.
fn at(name: &str, nth: usize) -> usize {
    let (byte, _) = SCRIPT.match_indices(name).nth(nth).expect("an occurrence");
    SCRIPT[..byte].chars().count()
}

/// The character range of the `nth` occurrence of `name`.
fn range(name: &str, nth: usize) -> std::ops::Range<usize> {
    at(name, nth)..at(name, nth) + name.chars().count()
}

/// The range of the first character of the `nth` occurrence of `text`, such as the alias `p` of
/// `p.species`.
fn first(text: &str, nth: usize) -> std::ops::Range<usize> {
    at(text, nth)..at(text, nth) + 1
}

#[test]
fn the_definition_of_a_use_is_the_name_that_defines_it() {
    let mut analyzed = analyzed();
    let document = analyzed.document(SCRIPT);
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    assert_eq!(
        definition(document, at("threshold", 1) + 2),
        Some(range("threshold", 0))
    );
    assert_eq!(
        definition(document, at("grams", 1)),
        Some(range("grams", 0))
    );
    assert_eq!(
        definition(document, at("p.species", 0)),
        Some(first("p\n", 0))
    );
    assert_eq!(definition(document, at("FROM", 0)), None);
}

#[test]
fn the_references_of_a_name_are_its_definition_and_each_use() {
    let mut analyzed = analyzed();
    let document = analyzed.document(SCRIPT);
    assert_eq!(
        references(document, at("threshold", 0)),
        [range("threshold", 0), range("threshold", 1)]
    );
    assert_eq!(
        references(document, at("grams", 1)),
        [range("grams", 0), range("grams", 1)]
    );
    assert_eq!(
        references(document, at("p.species", 0)),
        [first("p\n", 0), first("p.body", 0), first("p.species", 0)]
    );
}
