//! Hover on names, calls, and the values of sets.

mod common;

use avenger_sql_language::Document;
use common::{at, document, find};

const TEXT: &str = "DECLARE lo INT64 DEFAULT 1;
CREATE TEMP FUNCTION twice(v INT64) AS (v * 2);
FROM penguins AS p
|> WITH heavy AS (FROM penguins |> SELECT species, body_mass_g)
|> EXTEND twice(p.body_mass_g) AS doubled, SUBSTR(species, 1, 3) AS short
|> WHERE doubled > lo AND species IN (FROM heavy |> SELECT species)
|> EXTEND stack(body_mass_g, offset => 'normalize') OVER (PARTITION BY island ORDER BY species) AS share
|> AGGREGATE bar(x => species, y => share.end) AS chart";

/// The hover at the `n`th `needle`, with the text of its range.
fn hover(document: &Document, needle: &str, n: usize) -> Option<(String, String)> {
    let hover = document.hover(find(document, needle, n))?;
    Some((at(document, hover.range).to_string(), hover.markdown))
}

#[test]
fn hover_shows_names_with_their_types_and_definitions() {
    let document = document(TEXT);
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    let (range, markdown) = hover(&document, "doubled", 1).unwrap();
    assert_eq!(range, "doubled");
    assert_eq!(
        markdown,
        "```googlesql\ndoubled: INT64\n```\n\nDefined by `doubled`"
    );
    let (_, markdown) = hover(&document, "island", 0).unwrap();
    assert_eq!(
        markdown,
        "```googlesql\nisland: STRING\n```\n\nDefined by `penguins`"
    );
    let (range, markdown) = hover(&document, "p.body", 0).unwrap();
    assert_eq!(range, "p");
    assert!(
        markdown.starts_with(
            "```googlesql\np\n```\n\nA range variable of the columns:\n\n- `species`: STRING"
        ),
        "{markdown}"
    );
    let (_, markdown) = hover(&document, "lo", 1).unwrap();
    assert_eq!(markdown, "```googlesql\nDECLARE lo INT64\n```");
    let (_, markdown) = hover(&document, "penguins", 0).unwrap();
    assert_eq!(
        markdown,
        "```googlesql\nTABLE penguins\n```\n\n- `species`: STRING\n- `island`: STRING\n- `bill_length_mm`: FLOAT64\n- `body_mass_g`: INT64"
    );
    let (_, markdown) = hover(&document, "heavy", 1).unwrap();
    assert_eq!(
        markdown,
        "```googlesql\nheavy\n```\n\nA query of the columns:\n\n- `species`: STRING\n- `body_mass_g`: INT64"
    );
}

#[test]
fn hover_shows_functions_with_their_resolved_forms() {
    let document = document(TEXT);
    let (range, markdown) = hover(&document, "SUBSTR", 0).unwrap();
    assert_eq!(range, "SUBSTR");
    assert!(
        markdown.starts_with("Resolved as `SUBSTR(STRING, INT64, INT64) -> STRING`\n\n## `SUBSTR`"),
        "{markdown}"
    );
    let (_, markdown) = hover(&document, "bar", 0).unwrap();
    assert!(markdown.contains("## `bar`"), "{markdown}");
    let (_, markdown) = hover(&document, "twice", 1).unwrap();
    assert_eq!(
        markdown,
        "```googlesql\nCREATE TEMP FUNCTION twice(v INT64)\n```\n\nA temporary function."
    );
}

#[test]
fn hover_shows_the_parameter_of_a_value_of_a_set() {
    let document = document(TEXT);
    let (range, markdown) = hover(&document, "normalize", 0).unwrap();
    assert_eq!(range, "'normalize'");
    assert!(
        markdown.starts_with("```googlesql\n'normalize'\n```\n\nA value of `offset`: "),
        "{markdown}"
    );
    assert!(markdown.contains("\n\nValues: 'zero', "), "{markdown}");
}

#[test]
fn a_keyword_has_no_hover() {
    let document = document(TEXT);
    assert_eq!(hover(&document, "WHERE", 0), None);
}
