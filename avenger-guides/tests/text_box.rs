//! The box of a guide's text measures and draws alike.

use std::num::NonZeroUsize;

use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextSyntaxMode};
use avenger_guides::TextBox;
use avenger_scenegraph::marks::text::{text_label, text_style, SceneTextMark};
use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelOptions};

#[test]
fn a_text_box_measures_as_its_mark_draws() {
    let engine = avenger_typst_label::bundled_label_engine();
    let text = "A label of several words that wraps at its width";
    let style = text_style(
        "sans-serif",
        12.0,
        FontWeight::NORMAL,
        FontStyle::Normal,
        [0.0; 4],
    );
    for text_box in [
        TextBox::default(),
        TextBox {
            max_width: Some(80.0),
            wrap: true,
            max_lines: NonZeroUsize::new(2),
            ellipsis: true,
            line_height: LabelLineHeight::Fixed(18.0),
        },
        TextBox {
            max_width: Some(60.0),
            ellipsis: true,
            ..Default::default()
        },
    ] {
        let options = text_box.options(LabelOptions {
            text: style.clone(),
            ..Default::default()
        });
        let measured = engine
            .bounds(&text_label(text, TextSyntaxMode::Plain, options))
            .unwrap();
        let mark = text_box.mark(SceneTextMark {
            text: text.to_string().into(),
            font: "sans-serif".to_string().into(),
            font_size: 12.0.into(),
            align: TextAlign::Right.into(),
            ..Default::default()
        });
        let drawn = engine.bounds(&mark.labels().next().unwrap().label).unwrap();
        assert_eq!(
            (measured.width, measured.height),
            (drawn.width, drawn.height),
            "{text_box:?}"
        );
        // The lines align as the label does.
        assert_eq!(mark.line_align.as_vec(1, None)[0], LabelAlign::Right);
    }
}
