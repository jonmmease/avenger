//! Disabled paint, and the styles of the parts of widgets: their texts and the rail and the thumbs
//! of a slider.

use avenger_color::ColorOrGradient;
use avenger_layout::{Direction, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{bundled_label_engine, LabelEngine};
use avenger_widgets::{Look, Paint, Placed, Slider, Widget, WidgetContext, WidgetStyle};

/// A blend moves a color's red, green and blue toward the background and keeps its alpha, so an
/// opaque color stays opaque and a transparent one stays transparent.
#[test]
fn a_blend_keeps_alpha() {
    let paint = Paint {
        fill: [0.0, 0.0, 0.0, 1.0],
        border: [0.25, 0.5, 0.75, 0.0],
        foreground: [1.0, 1.0, 1.0, 1.0],
    };
    let blended = paint.blend([1.0, 1.0, 1.0, 1.0], 0.5);
    assert_eq!(blended.fill, [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(blended.border, [0.625, 0.75, 0.875, 0.0]);
    assert_eq!(blended.foreground, [1.0, 1.0, 1.0, 1.0]);
}

/// The presets draw every text in one font and in the neutral foreground, the rail in the neutral
/// border, and the thumbs in the neutral palette.
#[test]
fn the_presets_paint_the_parts_in_their_neutral_palette() {
    for style in [WidgetStyle::default(), WidgetStyle::dark()] {
        let neutral = &style.neutral;
        for text in [&style.title, &style.label, &style.readout] {
            assert_eq!(text, &style.title);
            assert_eq!(text.color, neutral.normal.foreground);
            // A disabled text dims as the disabled paint does.
            let disabled = Look {
                enabled: false,
                ..Look::default()
            };
            assert_eq!(
                text.color(disabled, style.background),
                neutral.disabled.foreground
            );
        }
        assert_eq!(style.rail.normal.fill, neutral.normal.border);
        assert_eq!(style.rail.disabled.fill, neutral.disabled.border);
        assert_eq!(&style.thumb, neutral);
    }
}

fn engine() -> LabelEngine {
    bundled_label_engine().with_number_formatting(std::sync::Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ))
}

/// A slider with a label and a readout, measured in `style`, and its marks in a 300 by 40 box.
fn slider(style: &WidgetStyle) -> ([f32; 2], Vec<SceneMark>) {
    let engine = engine();
    let cx = WidgetContext {
        engine: &engine,
        style,
    };
    let slider = Slider::<()>::new("speed", 0.0..=100.0)
        .label("Speed")
        .readout("");
    let size = [Direction::Horizontal, Direction::Vertical]
        .map(|direction| slider.max_content(direction, &cx));
    let placed = Placed {
        rect: Rect::new(0.0, 0.0, 300.0, 40.0),
        look: Look {
            enabled: true,
            ..Look::default()
        },
    };
    let view = slider.view(&(), &Default::default());
    (size, slider.draw(&view, &placed, &cx).unwrap())
}

/// The font size of the text mark named `name`.
fn font_size(marks: &[SceneMark], name: &str) -> f32 {
    (marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == name => text.font_size.first().copied(),
            _ => None,
        })
        .unwrap()
}

/// The fill of the rectangle mark named `name`.
fn fill(marks: &[SceneMark], name: &str) -> [f32; 4] {
    (marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) if rect.name == name => match rect.fill.first() {
                Some(ColorOrGradient::Color(color)) => Some(*color),
                _ => None,
            },
            _ => None,
        })
        .unwrap()
}

/// A slider measures and draws its label in the title's font and its readout in the readout's, and
/// paints its rail and its thumb with their palettes.
#[test]
fn a_slider_draws_its_parts_in_their_styles() {
    let plain = WidgetStyle::default();
    let (plain_size, marks) = slider(&plain);
    assert_eq!(font_size(&marks, "slider_label"), plain.title.font_size);
    assert_eq!(fill(&marks, "rail"), plain.rail.normal.fill);
    assert_eq!(fill(&marks, "thumb"), plain.thumb.normal.fill);

    // A larger title is taller, and a larger readout wider.
    let mut style = plain.clone();
    style.title.font_size = 26.0;
    let (size, marks) = slider(&style);
    assert_eq!(font_size(&marks, "slider_label"), 26.0);
    assert_eq!(font_size(&marks, "readout"), plain.readout.font_size);
    assert!(size[1] > plain_size[1], "{size:?} {plain_size:?}");
    let mut style = plain.clone();
    style.readout.font_size = 26.0;
    let (size, marks) = slider(&style);
    assert_eq!(font_size(&marks, "readout"), 26.0);
    assert!(size[0] > plain_size[0], "{size:?} {plain_size:?}");

    let mut style = plain.clone();
    style.rail.normal.fill = [0.0, 0.5, 0.0, 1.0];
    style.thumb.normal.fill = [1.0, 0.0, 0.0, 1.0];
    let (_, marks) = slider(&style);
    assert_eq!(fill(&marks, "rail"), [0.0, 0.5, 0.0, 1.0]);
    assert_eq!(fill(&marks, "thumb"), [1.0, 0.0, 0.0, 1.0]);
}
