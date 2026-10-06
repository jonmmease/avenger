//! Inline layout invariants. The frame oracle lays out every reference case, from its source
//! (`typst_layout::math::tests`).

use std::ops::Range;

use crate::label::fixtures;
use crate::label::oracle::{
    default_settings, layout_content, layout_source, root_styles,
};
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::Content;
use crate::typst_library::layout::{Frame, FrameItem};
use crate::typst_library::text::{
    FontFamily, FontList, LinebreakElem, TextElem, check_font_list, families,
};
use typst_syntax::{Span, Spanned};

/// Lays out a label's source under the default settings.
fn layout(source: &str) -> Frame {
    layout_source(source, &default_settings()).unwrap()
}

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
    assert_eq!(item_texts(&layout("*a**b*")), ["a", "b"]);
    // The text of one strong element shapes together.
    assert_eq!(item_texts(&layout("*ab*")), ["ab"]);
}

#[test]
fn glyphs_map_to_their_source() {
    let sources = |source| glyph_sources(&layout(source));

    // Escapes and shorthands map to their whole node, verbatim text byte for byte.
    assert_eq!(
        sources("\\#a -- \\u{41}"),
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
    assert_eq!(
        sources("office"),
        owned(&[("o", 0..1), ("ffi", 1..4), ("c", 4..5), ("e", 5..6)])
    );

    // A case change maps byte for byte while characters keep their lengths. `ß` becomes `SS`,
    // so its text maps to the whole node.
    assert_eq!(
        sources("#upper[ab] #upper[ß]"),
        owned(&[("A", 7..8), ("B", 8..9), (" ", 10..11), ("S", 18..20), ("S", 18..20)])
    );

    // A smart quote maps to its straight quote.
    assert_eq!(
        sources("\"I'm\""),
        owned(&[("“", 0..1), ("I", 1..2), ("’", 2..3), ("m", 3..4), ("”", 4..5)])
    );

    // Math glyphs map to their node.
    assert_eq!(sources("$x^2$"), owned(&[("𝑥", 1..2), ("2", 3..4)]));
}

#[test]
fn families_end_with_the_fallback_tail() {
    let root = root_styles(&default_settings());
    let names =
        |styles| families(styles).map(|family| family.as_str()).collect::<Vec<_>>();
    assert_eq!(
        names(root),
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
    assert_eq!(names(root.chain(&styles)), ["lato"]);
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
    // Evaluation turns line breaks in data into spaces and rejects explicit ones, so only
    // hand-made content has a line break.
    let a = || TextElem::packed("a");
    let content = Content::sequence([a(), LinebreakElem::shared().clone(), a()]);
    let errors = layout_content(&content).unwrap_err();
    assert_eq!(errors[0].message, "a label must be a single line");

    // A break at the end leaves one line.
    let content = Content::sequence([a(), LinebreakElem::shared().clone()]);
    assert_eq!(item_texts(&layout_content(&content).unwrap()), ["a"]);
}

#[test]
fn named_spacings_are_spacing_in_text() {
    // `math.quad` outside of an equation is horizontal spacing of 1em.
    let frame = layout("a#math.quad;b");
    let texts: Vec<_> = frame
        .items()
        .filter_map(|(pos, item)| match item {
            FrameItem::Text(text) => Some((pos.x, text.width())),
            _ => None,
        })
        .collect();
    let [(a_x, a_width), (b_x, _)] = texts[..] else { panic!("two text items") };
    let gap = (b_x - a_x - a_width).to_pt();
    assert!((gap - 12.0).abs() < 1e-9, "{gap}");
}
