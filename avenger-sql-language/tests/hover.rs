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
|> AGGREGATE mark_bar(x => species, y => share.end) AS chart";

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
    let (_, markdown) = hover(&document, "mark_bar", 0).unwrap();
    assert!(
        markdown.starts_with("```googlesql\nmark_bar(...) -> CHART\n```\n\nA rectangle"),
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
        markdown.starts_with("```googlesql\nx => ANY\nx => x("),
        "{markdown}"
    );
    assert!(
        markdown.contains("```\n\nA parameter of `mark_bar`: "),
        "{markdown}"
    );
    // At the start of the name of an argument of a call in the value of another argument.
    let nested =
        common::document("FROM penguins |> AGGREGATE mark_bar(x => x(species, title => 'S'))");
    let start = nested.text().find("title").unwrap() as u32;
    let found = nested.hover(start.into()).unwrap();
    assert_eq!(at(&nested, found.range), "title");
    assert!(
        found.markdown.contains("A parameter of `x`"),
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
        "FROM penguins |> AGGREGATE mark_bar(x => species, y => y(COUNT(*), \
         scale => scale(pin => pin(0, position => 'low')))) AS chart",
    );
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    let (range, markdown) = hover(&document, "pin =>", 0).unwrap();
    assert_eq!(range, "pin");
    assert!(
        markdown.starts_with(
            "```googlesql\npin => pin(value: ANY, position => 'low' | 'mid' | 'high')\n```\n\n\
             A parameter of `scale`: "
        ),
        "{markdown}"
    );
    // A constructor's hover names the place that it fills.
    let (range, markdown) = hover(&document, "pin(0", 0).unwrap();
    assert_eq!(range, "pin");
    assert!(
        markdown.starts_with(
            "```googlesql\npin(value: ANY, position => 'low' | 'mid' | 'high') -> BYTES\n```\n\n\
             Builds the `pin` of the `scale` of the `y` of `mark_bar(...)`."
        ),
        "{markdown}"
    );
    // Three strings of the set, the parameter's own constructor, then the constructors one level
    // down.
    let (_, markdown) = hover(&document, "scale =>", 0).unwrap();
    assert!(
        markdown.starts_with(
            "```googlesql\nscale => 'linear' | 'log' | 'pow' | …\nscale => scale(...)\n```\n\n\
             A parameter of `y`: "
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains("\n- One level down: `type(...)`, `domain(...)`"),
        "{markdown}"
    );
    assert!(!markdown.contains("BYTES"), "{markdown}");
}

#[test]
fn hover_shows_a_channel_function_and_the_channel_that_takes_it() {
    let document = common::document(
        "FROM penguins |> AGGREGATE mark_bar(x => species, y => y(COUNT(*), title => 'N')) AS chart",
    );
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    // The channel function names the channel that it sets, with its options.
    let (range, markdown) = hover(&document, "y(COUNT", 0).unwrap();
    assert_eq!(range, "y");
    assert!(
        markdown.contains("Builds the `y` of `mark_bar(...)`."),
        "{markdown}"
    );
    assert!(markdown.contains("Sets the channel `y`"), "{markdown}");
    // The channel's parameter takes any value and its channel function.
    let (range, markdown) = hover(&document, "y =>", 0).unwrap();
    assert_eq!(range, "y");
    assert!(
        markdown.starts_with("```googlesql\ny => ANY\ny => y("),
        "{markdown}"
    );
}

#[test]
fn a_keyword_has_no_hover() {
    let document = document(TEXT);
    assert_eq!(hover(&document, "WHERE", 0), None);
}

#[test]
fn hover_shows_a_column_inside_an_aggregate_of_a_mark() {
    let document = document(
        "FROM penguins |> AGGREGATE mark_bar(x => species, y => AVG(body_mass_g)) AS chart",
    );
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

#[test]
fn hover_says_that_the_strings_of_a_parameter_are_typst_markup() {
    let document = common::document(
        "FROM penguins |> AGGREGATE mark_bar(x => species, title => title('Mass', subtitle => 'kg')) AS chart",
    );
    for (needle, markup) in [("title =>", true), ("subtitle =>", true), ("x =>", false)] {
        let (_, markdown) = hover(&document, needle, 0).unwrap();
        assert_eq!(
            markdown.contains("Its strings are Typst markup."),
            markup,
            "{needle}: {markdown}"
        );
    }
}

#[test]
fn hover_shows_a_selection_with_its_resolution() {
    let document = document(
        "DECLARE picks SELECTION OPTIONS (resolution = 'union');\nFROM penguins |> WHERE picks",
    );
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    let (range, markdown) = hover(&document, "picks", 1).unwrap();
    assert_eq!(range, "picks");
    assert!(
        markdown.starts_with(
            "```googlesql\nDECLARE picks SELECTION OPTIONS (resolution = 'union')\n```"
        ),
        "{markdown}"
    );
}
