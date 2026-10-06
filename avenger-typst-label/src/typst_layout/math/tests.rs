//! The parts of math layout that labels add: the label's style for equations and the layout
//! budget.

use super::{MATH_TOO_COMPLEX, MATH_WORK_LIMIT};
use crate::label::oracle::{
    default_settings, layout_source, layout_source_in, root_styles,
};
use crate::typst_library::foundations::{StyleChain, Styles};
use crate::typst_library::layout::{Abs, Em, Frame, FrameItem};
use crate::typst_library::math::{EquationElem, LabelMathStyle};
use crate::typst_library::text::{FontFamily, FontList, TextItem};
use crate::typst_library::visualize::{Color, ColorExt, Paint};

/// The text items of a frame, at any depth.
fn text_items(frame: &Frame) -> Vec<&TextItem> {
    let mut items = vec![];
    for (_, item) in frame.items() {
        match item {
            FrameItem::Text(text) => items.push(text),
            FrameItem::Group(group) => items.extend(text_items(&group.frame)),
            FrameItem::Shape(..) => {}
        }
    }
    items
}

/// Lays out a source with the label's style for equations replaced.
fn layout_with(source: &str, style: LabelMathStyle) -> Frame {
    let mut styles = Styles::new();
    styles.set(EquationElem::label_style, style);
    let root: &StyleChain = Box::leak(Box::new(root_styles(&default_settings())));
    layout_source_in(source, root.chain(Box::leak(Box::new(styles))))
        .0
        .unwrap()
}

/// The style a label sets for equations by default in these tests: the wrapper's math font.
fn math_font() -> LabelMathStyle {
    let font = FontList(vec![FontFamily::new(&default_settings().math_font)]);
    LabelMathStyle { font: Some(font), ..LabelMathStyle::default() }
}

#[test]
fn equations_take_the_labels_style() {
    // Math inherits the text's size and fill, as upstream's does.
    let frame = layout_with("#text(fill: rgb(\"#ff0000\"))[$x$]", math_font());
    let [x] = text_items(&frame)[..] else { panic!("one text item") };
    assert_eq!(x.size, Abs::pt(12.0));
    assert_eq!(x.fill, Paint::Solid(Color::from_u8(255, 0, 0, 255)));

    // The label's size is relative to the text, and its fill wins over the text's.
    let style = LabelMathStyle {
        size: Some(Em::new(1.5)),
        fill: Some(Paint::Solid(Color::from_u8(0, 0, 255, 255))),
        ..math_font()
    };
    let frame = layout_with("#text(fill: rgb(\"#ff0000\"))[$x$]", style);
    let [x] = text_items(&frame)[..] else { panic!("one text item") };
    assert_eq!(x.size, Abs::pt(18.0));
    assert_eq!(x.fill, Paint::Solid(Color::from_u8(0, 0, 255, 255)));
    assert_eq!(x.font.font().info().family, "Lete Sans Math");
}

#[test]
fn text_fonts_warn_in_math() {
    let mut settings = default_settings();
    settings.math_font = settings.text_font.clone();
    let (frame, warnings) = layout_source_in("$x$", root_styles(&settings));
    assert!(frame.is_ok());
    let messages: Vec<_> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages, ["current font is not designed for math"]);
}

#[test]
fn nested_mid_delimiters_exhaust_the_layout_budget() {
    // A fence lays its body out again when a mid delimiter in it stretches, so each fence
    // around another doubles the work.
    let nested = |depth| {
        let mut source = String::from("x");
        for _ in 0..depth {
            source = format!("lr(| {source} mid(|) y |)");
        }
        format!("${source}$")
    };
    assert!(layout_source(&nested(8), &default_settings()).is_ok());
    assert!(2_usize.pow(18) > MATH_WORK_LIMIT);
    let errors = layout_source(&nested(18), &default_settings()).unwrap_err();
    assert_eq!(errors[0].message, MATH_TOO_COMPLEX);
}
