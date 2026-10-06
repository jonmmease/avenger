//! Inline layout invariants, over hand-made evaluated content.

use std::ops::Range;

use crate::label::fixtures;
use crate::label::oracle::Case;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, NativeElement};
use crate::typst_library::layout::{Frame, FrameItem};
use crate::typst_library::model::StrongElem;
use crate::typst_library::text::{
    Case as TextCase, FontFamily, FontList, LinebreakElem, TextElem, check_font_list,
    families,
};
use crate::typst_syntax::{Span, Spanned};

/// The text of each text item in a frame, in order.
fn item_texts(frame: &Frame) -> Vec<String> {
    frame
        .items()
        .filter_map(|(_, item)| match item {
            FrameItem::Text(text) => Some(text.text.to_string()),
            _ => None,
        })
        .collect()
}

/// Each glyph's cluster text and source range, in visual order.
fn glyph_sources(frame: &Frame) -> Vec<(String, Range<usize>)> {
    frame
        .items()
        .filter_map(|(_, item)| match item {
            FrameItem::Text(text) => Some(text),
            _ => None,
        })
        .flat_map(|text| {
            text.glyphs
                .iter()
                .map(|glyph| (text.text[glyph.range()].to_string(), glyph.source.clone()))
        })
        .collect()
}

fn owned(expected: &[(&str, Range<usize>)]) -> Vec<(String, Range<usize>)> {
    expected
        .iter()
        .map(|(text, range)| (text.to_string(), range.clone()))
        .collect()
}

#[test]
fn style_chains_split_segments_by_identity() {
    // `*a**b*`: two strong elements style their text with equal but separate styles, so it
    // shapes as two segments.
    let mut apart = Case::new("strong-segments");
    let content = Content::sequence([
        StrongElem::new(apart.words("a")).pack(),
        StrongElem::new(apart.words("b")).pack(),
    ]);
    assert_eq!(item_texts(&apart.layout(&content).unwrap()), ["a", "b"]);

    // The text of one strong element shapes together.
    let mut together = Case::new("strong-segments");
    let body = Content::sequence([together.words("a"), together.words("b")]);
    let content = StrongElem::new(body).pack();
    assert_eq!(item_texts(&together.layout(&content).unwrap()), ["ab"]);
}

#[test]
fn glyphs_map_to_their_source() {
    let sources =
        |case: &Case, content: Content| glyph_sources(&case.layout(&content).unwrap());

    // Escapes and shorthands map to their whole node, verbatim text byte for byte.
    let mut escapes = Case::custom("\\#a -- \\u{41}");
    let content = Content::sequence([
        escapes.text("#", "\\#"),
        escapes.words("a "),
        escapes.text("–", "--"),
        escapes.words(" "),
        escapes.text("A", "\\u{41}"),
    ]);
    assert_eq!(
        sources(&escapes, content),
        owned(&[
            ("#", 0..2),
            ("a", 2..3),
            (" ", 3..4),
            ("–", 4..6),
            (" ", 6..7),
            ("A", 7..13)
        ])
    );

    // A ligature maps to all of its characters.
    let mut ligature = Case::custom("office");
    let content = ligature.words("office");
    assert_eq!(
        sources(&ligature, content),
        owned(&[("o", 0..1), ("ffi", 1..4), ("c", 4..5), ("e", 5..6)])
    );

    // A case change maps byte for byte while characters keep their lengths. `ß` becomes `SS`,
    // so its text maps to the whole node.
    let mut upper = Case::custom("#upper[ab] #upper[ß]");
    let content = Content::sequence([
        upper.words("ab").set(TextElem::case, Some(TextCase::Upper)),
        upper.words(" "),
        upper.words("ß").set(TextElem::case, Some(TextCase::Upper)),
    ]);
    assert_eq!(
        sources(&upper, content),
        owned(&[("A", 7..8), ("B", 8..9), (" ", 10..11), ("S", 18..20), ("S", 18..20)])
    );

    // A smart quote maps to its straight quote.
    let mut quotes = Case::custom("\"I'm\"");
    let content = quotes.markup("\"I'm\"");
    assert_eq!(
        sources(&quotes, content),
        owned(&[("“", 0..1), ("I", 1..2), ("’", 2..3), ("m", 3..4), ("”", 4..5)])
    );
}

#[test]
fn families_end_with_the_fallback_tail() {
    let case = Case::custom("");
    let names =
        |styles| families(styles).map(|family| family.as_str()).collect::<Vec<_>>();
    assert_eq!(
        names(case.root),
        [
            "lato",
            "libertinus serif",
            "twitter color emoji",
            "noto color emoji",
            "apple color emoji",
            "segoe ui emoji",
        ]
    );
    let mut styles = crate::typst_library::foundations::Styles::new();
    styles.set(TextElem::fallback, false);
    assert_eq!(names(case.root.chain(&styles)), ["lato"]);
}

#[test]
fn unknown_font_families_warn() {
    let world = fixtures::shared();
    let mut sink = Sink::new();
    let mut engine = Engine { world, sink: &mut sink };
    let families = ["Lato", "Nonexistent", "DejaVu Sans Mono"].map(FontFamily::new);
    check_font_list(
        &mut engine,
        &Spanned::new(FontList(families.into()), Span::detached()),
    );
    let warnings: Vec<_> = sink
        .warnings()
        .iter()
        .map(|warning| warning.message.to_string())
        .collect();
    assert_eq!(warnings, ["unknown font family: nonexistent"]);
}

#[test]
fn a_label_is_one_line() {
    let mut case = Case::custom("a b");
    let content = Content::sequence([
        case.words("a"),
        LinebreakElem::shared().clone(),
        case.words("b"),
    ]);
    let errors = case.layout(&content).unwrap_err();
    assert_eq!(errors[0].message, "a label must be a single line");

    // A break at the end leaves one line.
    let mut trailing = Case::custom("a");
    let content =
        Content::sequence([trailing.words("a"), LinebreakElem::shared().clone()]);
    assert_eq!(item_texts(&trailing.layout(&content).unwrap()), ["a"]);
}
