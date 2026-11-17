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
    // The forms of a function and its summary, without the parameters and the examples of the
    // reference.
    assert!(
        markdown.starts_with("```googlesql\nSUBSTR(value: STRING, position: INT64"),
        "{markdown}"
    );
    assert!(!markdown.contains("Resolved as"), "{markdown}");
    assert!(!markdown.contains("**Parameters**"), "{markdown}");
    // A function with several forms names the form of the call.
    let several = common::document("FROM penguins |> AGGREGATE AVG(body_mass_g) AS m");
    let (_, markdown) = hover(&several, "AVG", 0).unwrap();
    assert!(
        markdown.contains("```\n\nResolved as `AVG(INT64) -> FLOAT64`\n\nReturns the mean"),
        "{markdown}"
    );
    // A mark's form, with its every channel, is longer than a hover writes.
    let (_, markdown) = hover(&document, "bar", 0).unwrap();
    assert!(
        markdown.starts_with("```googlesql\nbar(...) -> CHART\n```\n\nA rectangle"),
        "{markdown}"
    );
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
fn hover_shows_the_parameter_of_an_argument_name() {
    let document = document(TEXT);
    let (range, markdown) = hover(&document, "offset =>", 0).unwrap();
    assert_eq!(range, "offset");
    assert!(
        markdown.starts_with("```googlesql\noffset => "),
        "{markdown}"
    );
    assert!(
        markdown.contains("```\n\nA parameter of `stack`: "),
        "{markdown}"
    );
    let (range, markdown) = hover(&document, "x =>", 0).unwrap();
    assert_eq!(range, "x");
    assert!(
        markdown.starts_with("```googlesql\nx: ANY\n```\n\nA parameter of `bar`: "),
        "{markdown}"
    );
    // At the start of the name of an argument of a call in the value of another argument.
    let nested =
        common::document("FROM penguins |> AGGREGATE bar(x => scaled(species, title => 'S'))");
    let start = nested.text().find("title").unwrap() as u32;
    let found = nested.hover(start.into()).unwrap();
    assert_eq!(at(&nested, found.range), "title");
    assert!(
        found.markdown.contains("A parameter of `scaled`"),
        "{}",
        found.markdown
    );
    // The value of an argument has the hover of its expression.
    let (range, _) = hover(&document, "species, y => share", 0).unwrap();
    assert_eq!(range, "species");
}

#[test]
fn hover_shows_the_forms_of_a_parameter_that_takes_constructors() {
    let document = common::document(
        "FROM penguins |> AGGREGATE bar(x => species, y => scaled(COUNT(*), \
         scale => scale(pin => domain_pin(0, 'low')))) AS chart",
    );
    let (range, markdown) = hover(&document, "pin =>", 0).unwrap();
    assert_eq!(range, "pin");
    assert!(
        markdown.starts_with(
            "```googlesql\npin => domain_pin(value: ANY, position: 'low' | 'mid' | 'high')\n```\n\n\
             A parameter of `scale`: "
        ),
        "{markdown}"
    );
    // Three strings of the set, the target's own constructor, then the groups of its parts.
    let (_, markdown) = hover(&document, "scale =>", 0).unwrap();
    assert!(
        markdown.starts_with(
            "```googlesql\nscale => 'linear' | 'log' | 'pow' | …\nscale => scale(...)\n```\n\n\
             A parameter of `scaled`: "
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains("\n- Its type: `band_scale(...)`"),
        "{markdown}"
    );
    assert!(!markdown.contains("BYTES"), "{markdown}");
}

#[test]
fn a_keyword_has_no_hover() {
    let document = document(TEXT);
    assert_eq!(hover(&document, "WHERE", 0), None);
}

#[test]
fn hover_shows_a_column_inside_an_aggregate_of_a_mark() {
    let document =
        document("FROM penguins |> AGGREGATE bar(x => species, y => AVG(body_mass_g)) AS chart");
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    let (range, markdown) = hover(&document, "body_mass_g", 0).unwrap();
    assert_eq!(range, "body_mass_g");
    assert!(
        markdown.starts_with("```googlesql\nbody_mass_g: INT64"),
        "{markdown}"
    );
}
