//! The editor's assistance outside egui: the completion popup, the fixes of errors, and the blocks
//! of a hover's Markdown.

use arrow::datatypes::DataType;
use avenger_sql_editor::assist::{apply, blocks, fixes, Analysis, Block, Popup, Span};
use avenger_sql_editor::document::{catalog, Analyzed};
use avenger_sql_editor::samples::Sample;

/// The analysis of texts over a sample table of penguins.
fn analyzed() -> Analyzed {
    let penguins = Sample {
        name: "penguins".to_string(),
        columns: vec![
            ("species".to_string(), DataType::Utf8),
            ("island".to_string(), DataType::Utf8),
            ("body_mass_g".to_string(), DataType::Int64),
        ],
    };
    let mut analyzed = Analyzed::new();
    analyzed.set_catalog(catalog(&[], &[penguins]));
    analyzed
}

/// The text of `text` with `$0` for the cursor, and the cursor's character index.
fn at_cursor(text: &str) -> (String, usize) {
    let byte = text.find("$0").expect("a cursor");
    (text.replacen("$0", "", 1), text[..byte].chars().count())
}

#[test]
fn the_popup_completes_the_name_at_the_cursor_and_filters_as_one_types() {
    let mut analyzed = analyzed();
    let (text, cursor) = at_cursor("FROM samples.penguins |> WHERE sp$0");
    let popup = Popup::open(analyzed.document(&text), cursor).unwrap();
    let labels: Vec<&str> = (popup.items(&text, cursor).iter())
        .map(|item| item.label.as_str())
        .collect();
    assert!(labels.contains(&"species"), "{labels:?}");
    assert!(!labels.contains(&"island"), "{labels:?}");
    // Typing narrows the items, and the selected item inserts in place of the typed name.
    let typed = format!("{text}eci");
    let cursor = cursor + 3;
    assert!(popup
        .items(&typed, cursor)
        .iter()
        .all(|item| item.label.starts_with("speci")));
    let (accepted, after) = popup.accept(&typed, cursor).unwrap();
    assert_eq!(accepted, "FROM samples.penguins |> WHERE species");
    assert_eq!(after, accepted.chars().count());
}

#[test]
fn the_popup_closes_when_the_cursor_leaves_its_name() {
    let mut analyzed = analyzed();
    let (text, cursor) = at_cursor("FROM samples.penguins |> WHERE sp$0");
    let popup = Popup::open(analyzed.document(&text), cursor).unwrap();
    assert!(!popup.closes(&text, cursor));
    let spaced = format!("{text} ");
    assert!(popup.closes(&spaced, cursor + 1));
    assert!(popup.closes(&text, cursor - 3));
}

#[test]
fn the_selection_moves_among_the_kept_items_and_stops_at_their_ends() {
    let mut analyzed = analyzed();
    let (text, cursor) = at_cursor("FROM samples.penguins |> WHERE $0");
    let mut popup = Popup::open(analyzed.document(&text), cursor).unwrap();
    let count = popup.items(&text, cursor).len();
    assert!(count > 2);
    popup.select(&text, cursor, -1);
    assert_eq!(popup.selected(&text, cursor), 0);
    popup.select(&text, cursor, 2);
    assert_eq!(popup.selected(&text, cursor), 2);
    popup.select(&text, cursor, count as isize);
    assert_eq!(popup.selected(&text, cursor), count - 1);
}

#[test]
fn a_fix_of_the_error_at_the_cursor_rewrites_the_text() {
    let mut analyzed = analyzed();
    let text = "SELECT species FROM samples.penguins";
    let found = fixes(Analysis::query(analyzed.document(text)), 2);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].label, "Rewrite as a pipe query");
    assert_eq!(
        apply(text, &found[0]),
        "FROM samples.penguins |> SELECT species"
    );
    // No error holds a cursor in a query without one.
    let document = analyzed.document("FROM samples.penguins");
    assert!(fixes(Analysis::query(document), 3).is_empty());
}

#[test]
fn a_hover_reads_as_blocks_of_text_with_code_and_strong_spans() {
    let markdown = "## `mark_bar`\n\n```googlesql\nmark_bar(x: ANY) -> CHART\n```\n\nA rectangle\nfrom a baseline.\n\n**Parameters**\n\n+ `x`: a position, in this grammar:\n\n  ```text\n  x = NUMBER\n    | NAME\n  ```\n\n  Lengths are\n  in pixels.\n- `y`: the other\n";
    use Block::{Heading, Item, Paragraph};
    let text = |text: &str| Span::Plain(text.to_string());
    let code = |text: &str| Span::Code(text.to_string());
    assert_eq!(
        blocks(markdown),
        [
            Heading(vec![code("mark_bar")]),
            Block::Code("mark_bar(x: ANY) -> CHART".to_string()),
            Paragraph(vec![text("A rectangle from a baseline.")]),
            Paragraph(vec![Span::Strong("Parameters".to_string())]),
            Item(vec![code("x"), text(": a position, in this grammar:")]),
            Block::Code("x = NUMBER\n  | NAME".to_string()),
            Paragraph(vec![text("Lengths are in pixels.")]),
            Item(vec![code("y"), text(": the other")]),
        ]
    );
}

#[test]
fn an_item_replaces_the_whole_name_or_string_at_the_cursor() {
    let mut analyzed = analyzed();
    for (case, selected, expected) in [
        (
            "FROM samples.penguins |> WHERE isl$0and",
            "island",
            "FROM samples.penguins |> WHERE island",
        ),
        (
            "FROM samples.penguins |> WHERE `spe$0",
            "species",
            "FROM samples.penguins |> WHERE `species`",
        ),
        (
            "FROM samples.penguins |> WHERE `spe$0`",
            "species",
            "FROM samples.penguins |> WHERE `species`",
        ),
        (
            "FROM samples.penguins |> AGGREGATE mark_bar(x => species, orient => 'ver$0')",
            "'vertical'",
            "FROM samples.penguins |> AGGREGATE mark_bar(x => species, orient => 'vertical')",
        ),
    ] {
        let (text, cursor) = at_cursor(case);
        let mut popup = Popup::open(analyzed.document(&text), cursor).expect(case);
        let labels: Vec<&str> = (popup.items(&text, cursor).iter())
            .map(|item| item.label.as_str())
            .collect();
        let index = labels
            .iter()
            .position(|label| *label == selected)
            .expect(case);
        popup.select_at(index);
        let (accepted, _) = popup.accept(&text, cursor).expect(case);
        assert_eq!(accepted, expected);
    }
}
