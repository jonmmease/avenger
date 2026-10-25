//! Text positions, carets, selection geometry, and copied text across lines, text directions,
//! grapheme clusters, markup, and equations.

mod common;

use std::sync::Arc;

use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::{
    Affinity, Label, LabelAlign, LabelEngine, LabelOptions, LabelSource, LabelWidth,
    TextLayout, TextPosition, TextStyle,
};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

fn style() -> TextStyle {
    TextStyle {
        font_family: "Lato".into(),
        font_size: 12.0,
        ..Default::default()
    }
}

/// Build a selection layout for literal text or markup.
fn layout(source: LabelSource, options: LabelOptions) -> Arc<TextLayout> {
    engine().text_layout(&Label { source, options }).unwrap()
}

fn markup(source: &str, width: LabelWidth) -> Arc<TextLayout> {
    layout(
        LabelSource::Markup(source),
        LabelOptions { text: style(), width, ..Default::default() },
    )
}

fn text(text: &str) -> Arc<TextLayout> {
    layout(LabelSource::Text(text), LabelOptions { text: style(), ..Default::default() })
}

/// Lay out text with explicit newline breaks at the given width and alignment.
fn lines(text: &str, width: LabelWidth, align: LabelAlign) -> Arc<TextLayout> {
    let options = LabelOptions {
        text: style(),
        width,
        align,
        newline_breaks: true,
        ..Default::default()
    };
    layout(LabelSource::Text(text), options)
}

fn at(offset: usize, affinity: Affinity) -> TextPosition {
    TextPosition::new(offset, affinity)
}

/// Return the vertical midpoint of the caret at `offset`.
fn caret_point(layout: &TextLayout, offset: usize, affinity: Affinity) -> [f32; 2] {
    let caret = layout.caret(at(offset, affinity));
    [caret.x, caret.y + caret.height / 2.0]
}

/// Affinity chooses between visual positions at a text-direction boundary. Selection rectangles
/// follow contiguous visual spans.
#[test]
fn affinity_picks_a_side_where_the_direction_changes() {
    let mixed = text("abc אבג xyz");
    let hebrew = "abc ".len();
    let upstream = mixed.caret(at(hebrew, Affinity::Upstream));
    let downstream = mixed.caret(at(hebrew, Affinity::Downstream));
    assert!(downstream.x > upstream.x + 1.0, "{upstream:?} {downstream:?}");
    // "c " and "א" lie apart: the Hebrew run starts at its right.
    let rects = mixed.selection_rects(2..hebrew + "א".len());
    assert_eq!(rects.len(), 2, "{rects:?}");
    assert_eq!(mixed.selection_rects(0..mixed.source().len()).len(), 1);
}

/// Positions fall on grapheme boundaries, including within ligatures.
#[test]
fn positions_fall_between_graphemes() {
    let office = text("office");
    let rect = office.selection_rects(1..4)[0];
    let quarter = office.position_at([rect.x + rect.width * 0.25, rect.y + 1.0]);
    let three_quarters = office.position_at([rect.x + rect.width * 0.75, rect.y + 1.0]);
    assert_ne!(quarter.offset, three_quarters.offset);
    assert!(
        (1..=4).contains(&quarter.offset) && (1..=4).contains(&three_quarters.offset)
    );

    let accent = text("e\u{301}x");
    let rect = accent.selection_rects(0..accent.source().len())[0];
    let offsets: Vec<usize> = (0..=20)
        .map(|step| rect.x + rect.width * step as f32 / 20.0)
        .map(|x| accent.position_at([x, rect.y + 1.0]).offset)
        .collect();
    assert!(!offsets.contains(&1), "{offsets:?}");
}

/// An empty label has a caret, and every point maps to its start.
#[test]
fn an_empty_label_has_a_caret() {
    let empty = text("");
    assert!(empty.caret(TextPosition::default()).height > 0.0);
    assert_eq!(empty.position_at([10.0, 10.0]).offset, 0);
}

/// Points map to the nearest line. At a wrap, upstream affinity selects the preceding line's
/// end and downstream affinity selects the following line's start.
#[test]
fn wrapped_lines_take_points_and_carets_by_line() {
    let wrapped = lines("one two three", LabelWidth::Max(40.0), LabelAlign::Start);
    let two = "one ".len();
    let lower = caret_point(&wrapped, two, Affinity::Downstream);
    let upper = caret_point(&wrapped, two, Affinity::Upstream);
    assert!(lower[1] > upper[1] + 5.0 && lower[0] < upper[0], "{upper:?} {lower:?}");
    assert_eq!(wrapped.position_at([0.0, lower[1]]), at(two, Affinity::Downstream));
    assert_eq!(wrapped.line_at(at(two, Affinity::Downstream)), two..two + "two ".len());
    // A wrap runs on into the next line, so the selected text keeps its space.
    assert_eq!(wrapped.text_in(0.."one two".len()), "one two");
}

/// An empty line has a caret and a narrow selection rectangle, and contributes a newline to
/// copied text. Its caret follows the line alignment.
#[test]
fn empty_lines_take_carets_and_selections() {
    let source = "a\n\nb";
    let broken = lines(source, LabelWidth::Auto, LabelAlign::Start);
    let empty = caret_point(&broken, 2, Affinity::Downstream);
    assert_eq!(broken.position_at(empty).offset, 2);
    assert_eq!(broken.selection_rects(0..source.len()).len(), 3);
    assert_eq!(broken.text_in(0..source.len()), source);

    let centered = lines(source, LabelWidth::Fixed(40.0), LabelAlign::Center);
    let x = centered.caret(at(2, Affinity::Downstream)).x;
    assert!((x - 20.0).abs() < 0.5, "{x}");
}

/// Each explicit break that a range selects copies as a newline, whether a newline or
/// `#linebreak()` makes it, and also when the range ends just past the break.
#[test]
fn explicit_breaks_copy_as_newlines() {
    let source = "one#linebreak()two";
    let markup_break = markup(source, LabelWidth::Auto);
    assert_eq!(markup_break.selection_rects(0..source.len()).len(), 2);
    assert_eq!(markup_break.text_in(0..source.len()), "one\ntwo");

    let newline = lines("one\ntwo", LabelWidth::Auto, LabelAlign::Start);
    assert_eq!(newline.text_in(0.."one\n".len()), "one\n");
    assert_eq!(newline.text_in(0.."one".len()), "one");
}

/// Markup positions and word ranges use source offsets. Copied text contains rendered text.
#[test]
fn markup_positions_are_source_offsets() {
    let source = "*bold* text";
    let options = LabelOptions { text: style(), ..Default::default() };
    let markup = layout(LabelSource::Markup(source), options);
    let b = markup.caret(at(1, Affinity::Downstream));
    assert_eq!(markup.position_at([b.x + 0.5, b.y + 1.0]).offset, 1);
    assert_eq!(markup.word_at(at(2, Affinity::Downstream)), 1..5);
    assert_eq!(markup.text_in(0..source.len()), "bold text");
}

/// Formatted values select as whole units, with positions only at their ends, and copy as
/// rendered text.
#[test]
fn formatted_values_select_whole() {
    let source = "#numfmt(1234.5, \",.0f\") units";
    let options = LabelOptions { text: style(), ..Default::default() };
    let formatted = layout(LabelSource::Markup(source), options);
    // The number comes from the call, after its `#`.
    let call = source.find("numfmt").unwrap()..source.find(" units").unwrap();
    assert_eq!(formatted.word_at(at(3, Affinity::Downstream)), call);
    let rect = formatted.selection_rects(call.clone())[0];
    let middle = formatted.position_at([rect.x + rect.width * 0.4, rect.y + 1.0]);
    assert!([call.start, call.end].contains(&middle.offset), "{middle:?}");
    assert_eq!(formatted.text_in(call), "1,235");
}

/// Equations select as whole units with one highlight rectangle and copy as markup.
#[test]
fn equations_select_whole_and_copy_as_markup() {
    let source = "$x^2 + y$ and $a/b$";
    let math = markup(source, LabelWidth::Auto);
    let equation = 0.."$x^2 + y$".len();
    let rect = math.selection_rects(equation.clone())[0];
    let offsets: Vec<usize> = (0..=20)
        .map(|step| rect.x + rect.width * step as f32 / 20.0)
        .map(|x| math.position_at([x, rect.y + 1.0]).offset)
        .collect();
    assert!(
        offsets.iter().all(|offset| [0, equation.end].contains(offset)),
        "{offsets:?}"
    );
    assert_eq!(math.word_at(at(3, Affinity::Downstream)), equation);
    assert_eq!(math.selection_rects(0..source.len()).len(), 1);
    assert_eq!(math.text_in(0..source.len()), source);
    // A range inside an equation copies and highlights the whole equation.
    assert_eq!(math.text_in(1..2), "$x^2 + y$");
    assert_eq!(math.selection_rects(1..2), [rect]);
}

/// Generated scientific notation copies as equivalent Typst markup.
#[test]
fn scientific_notation_copies_as_markup() {
    let source = "#numfmt(1234.5, \".1e\") m";
    let formatted = markup(source, LabelWidth::Auto);
    assert_eq!(formatted.text_in(0..source.len()), "$#\"1.2\" times 10^(3)$ m");
}

/// An equation spanning multiple lines has highlights on each line and copies once, and a
/// position at its end stays on its last line.
#[test]
fn an_equation_across_lines_copies_once() {
    let source = "a $x + y + z + w$";
    let wrapped = markup(source, LabelWidth::Max(30.0));
    let rects = wrapped.selection_rects(0..source.len());
    assert!(rects.len() > 1);
    assert_eq!(wrapped.text_in(0..source.len()), source);
    // The equation's end on its last line keeps the caret and the line there.
    let last = rects.last().unwrap();
    let end = wrapped.position_at([last.x + last.width, last.y + last.height / 2.0]);
    assert!((wrapped.caret(end).y - last.y).abs() < 1.0);
    assert_eq!(wrapped.line_at(end), "a ".len()..source.len());
}
