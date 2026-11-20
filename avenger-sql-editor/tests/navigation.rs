//! The definition and the references of the name at the cursor, in characters, on a script with
//! a variable, a temporary function, and a pipe alias.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema};
use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_editor::assist::{definition, references, Analysis, Target};
use avenger_sql_language::{FileRole, Project};

/// The path of the script in its project.
const CHART: &str = "charts/chart.sql";

/// A script whose names are each defined once and used after, with a multi-byte character in
/// a comment before them, so that character indices differ from byte offsets.
const SCRIPT: &str = "-- Pingüinos\n\
DECLARE threshold FLOAT64 DEFAULT 4;\n\
CREATE TEMP FUNCTION grams(mass FLOAT64) AS (mass / 1000);\n\
FROM samples.penguins\n\
|> AS p\n\
|> WHERE grams(p.body_mass_g) > threshold\n\
|> AGGREGATE mark_bar(x => p.species, y => count(*))\n";

/// A project of the script, over a table of penguins of the host, `samples.penguins`.
fn project() -> Project {
    let penguins = Schema::new(vec![
        Field::new("species", DataType::Utf8, true),
        Field::new("body_mass_g", DataType::Int64, true),
    ]);
    let catalog = MemoryCatalog::new().with_schema(&["samples", "penguins"], Arc::new(penguins));
    Project::new(
        Arc::new(catalog),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_files([(CHART, FileRole::Chart, SCRIPT)])
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

/// The definition of the name at the character `cursor` of the script, in the script.
fn defined(project: &Project, cursor: usize) -> Option<std::ops::Range<usize>> {
    let target = definition(Analysis::file(project, CHART)?, cursor)?;
    let Target { path: None, range } = target else {
        panic!("a definition in another file: {target:?}");
    };
    Some(range)
}

/// The range of the first character of the `nth` occurrence of `text`, such as the alias `p` of
/// `p.species`.
fn first(text: &str, nth: usize) -> std::ops::Range<usize> {
    at(text, nth)..at(text, nth) + 1
}

#[test]
fn the_definition_of_a_use_is_the_name_that_defines_it() {
    let project = project();
    let diagnostics = project.diagnostics(CHART);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        defined(&project, at("threshold", 1) + 2),
        Some(range("threshold", 0))
    );
    assert_eq!(defined(&project, at("grams", 1)), Some(range("grams", 0)));
    assert_eq!(defined(&project, at("p.species", 0)), Some(first("p\n", 0)));
    assert_eq!(defined(&project, at("FROM", 0)), None);
}

#[test]
fn the_references_of_a_name_are_its_definition_and_each_use() {
    let project = project();
    let analysis = Analysis::file(&project, CHART).unwrap();
    assert_eq!(
        references(analysis, at("threshold", 0)),
        [range("threshold", 0), range("threshold", 1)]
    );
    assert_eq!(
        references(analysis, at("grams", 1)),
        [range("grams", 0), range("grams", 1)]
    );
    assert_eq!(
        references(analysis, at("p.species", 0)),
        [first("p\n", 0), first("p.body", 0), first("p.species", 0)]
    );
}
