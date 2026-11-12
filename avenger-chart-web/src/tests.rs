//! The parts of the library that run without a browser: font files, the conversion of DOM events,
//! and page stylesheets.

use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowEvent, WindowKeyboardInput,
    WindowMouseWheel,
};
use avenger_style::{Element, HostInputs, Property, StyleTree};
use avenger_typst_label::{bundled_font_options, EngineOptions, LabelEngine, LabelOptions};

use crate::{
    fonts::{font_options, read_fonts, FontsError},
    input::{self, Touches, Wheel},
    styles::cascade,
};

#[test]
fn a_font_file_reads_as_its_one_face() {
    let lato = bundled_font_options().registered_fonts[2].data.clone();
    let read = read_fonts(&lato).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(&*read[0].data, &*lato);
    assert_eq!(read[0].face_index, 0);
}

#[test]
fn a_file_that_is_not_a_font_is_an_error() {
    assert_eq!(read_fonts(b"not a font").unwrap_err(), FontsError::NotAFont);
}

#[test]
fn the_engine_has_the_bundled_families_and_then_the_page_fonts() {
    let bundled = bundled_font_options().registered_fonts.len();
    let page = read_fonts(&bundled_font_options().registered_fonts[0].data).unwrap();
    let options = font_options(page);
    assert!(!options.load_system_fonts);
    assert_eq!(options.registered_fonts.len(), bundled + 1);
    let engine = LabelEngine::new(EngineOptions {
        fonts: font_options(Vec::new()),
    });
    for family in ["sans-serif", "monospace"] {
        let mut options = LabelOptions::default();
        options.text.font_family = family.into();
        let label = engine.compile_text("Text", &options).unwrap();
        assert!(!label.frame.text_items().is_empty(), "{family}");
    }
}

#[test]
fn wheel_deltas_point_as_winit_deltas_do() {
    let delta = |event: WindowEvent| match event {
        WindowEvent::MouseWheel(WindowMouseWheel { delta }) => delta,
        other => panic!("a wheel event, got {other:?}"),
    };
    // A notch of a mouse wheel scrolls the page down by 100 pixels, which zooms out one step.
    assert_eq!(
        delta(input::wheel(0.0, 100.0, 0)),
        MouseScrollDelta::PixelDelta(-0.0, -100.0)
    );
    assert_eq!(
        delta(input::wheel(0.0, -3.0, 1)),
        MouseScrollDelta::LineDelta(-0.0, 3.0)
    );
    assert_eq!(
        delta(input::wheel(0.0, 1.0, 2)),
        MouseScrollDelta::LineDelta(-0.0, -10.0)
    );
}

#[test]
fn the_wheel_option_chooses_when_the_wheel_reaches_the_chart() {
    assert_eq!(Wheel::default(), Wheel::Focus);
    assert_eq!(Wheel::parse("always"), Ok(Wheel::Always));
    assert!(Wheel::parse("sometimes").is_err());
    assert!(!Wheel::Focus.reaches(false));
    assert!(Wheel::Focus.reaches(true));
    assert!(Wheel::Always.reaches(false));
}

#[test]
fn dom_keys_and_buttons_name_window_keys_and_buttons() {
    assert_eq!(
        input::key("a", ElementState::Pressed),
        Some(WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Character('a'),
            state: ElementState::Pressed,
            text: Some("a".to_string()),
        }))
    );
    assert_eq!(
        input::key("Escape", ElementState::Released),
        Some(WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Named(NamedKey::Escape),
            state: ElementState::Released,
            text: None,
        }))
    );
    assert_eq!(input::key("Unidentified", ElementState::Pressed), None);
    assert_eq!(input::button(0), MouseButton::Left);
    assert_eq!(input::button(2), MouseButton::Right);
}

fn left(state: ElementState) -> WindowEvent {
    input::press(MouseButton::Left, state)
}

#[test]
fn a_tap_moves_the_cursor_and_clicks() {
    let mut touches = Touches::default();
    assert!(touches.down(1, [10.0, 20.0]).is_empty());
    assert!(touches.moved(1, [12.0, 21.0]).is_empty());
    assert_eq!(
        touches.up(1, [12.0, 21.0]),
        [
            WindowEvent::CursorEntered,
            input::moved([12.0, 21.0]),
            left(ElementState::Pressed),
            left(ElementState::Released),
        ]
    );
}

#[test]
fn one_finger_that_moves_far_scrolls_the_page_without_events() {
    let mut touches = Touches::default();
    touches.down(1, [10.0, 20.0]);
    assert!(touches.moved(1, [10.0, 60.0]).is_empty());
    assert!(touches.up(1, [10.0, 60.0]).is_empty());
    touches.down(2, [10.0, 20.0]);
    assert!(touches.cancel(2).is_empty());
}

#[test]
fn two_fingers_drag_their_midpoint_and_zoom_by_their_distance() {
    let mut touches = Touches::default();
    touches.down(1, [0.0, 0.0]);
    assert_eq!(
        touches.down(2, [100.0, 0.0]),
        [
            WindowEvent::CursorEntered,
            input::moved([50.0, 0.0]),
            left(ElementState::Pressed)
        ]
    );
    assert!(touches.pinching());
    // Moving both fingers apart by 1.2 zooms in one notch about the new midpoint.
    touches.moved(1, [-10.0, 10.0]);
    let events = touches.moved(2, [110.0, 10.0]);
    assert_eq!(events[0], input::moved([50.0, 10.0]));
    let WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::PixelDelta(x, y),
    }) = events[1]
    else {
        panic!("a wheel event, got {:?}", events[1]);
    };
    assert_eq!(x, 0.0);
    // The first move spread the fingers to √(110² + 10²), and the second to 120.
    let notches = (120.0_f64 / 110.0_f64.hypot(10.0)).ln() / 1.2_f64.ln();
    assert!((y - notches * 100.0).abs() < 1e-3, "{y}");
    // A moved finger that is not one of the two moves nothing.
    touches.down(3, [0.0, 50.0]);
    assert!(touches.moved(3, [0.0, 70.0]).is_empty());
    // Lifting one of the two ends the drag, and the finger that stays does not tap.
    assert_eq!(touches.up(1, [-10.0, 10.0]), [left(ElementState::Released)]);
    assert!(!touches.pinching());
    assert!(touches.up(2, [110.0, 10.0]).is_empty());
}

#[test]
fn a_cancel_of_a_dragging_finger_ends_the_drag() {
    let mut touches = Touches::default();
    touches.down(1, [0.0, 0.0]);
    touches.down(2, [100.0, 0.0]);
    assert_eq!(touches.cancel(2), [left(ElementState::Released)]);
}

/// The color of a mark of a bar chart's plot, under the styles of `page`.
fn mark_color(page: Option<&str>) -> [f32; 4] {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let mark = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(mark, "type", "bar", "bar");
    let (styles, diagnostics) = cascade(page);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let computed = styles.compute(&tree, &HostInputs::default());
    computed[mark].color(Property::Color).expect("a color")
}

#[test]
fn a_page_rule_wins_over_the_base_stylesheet() {
    let base = mark_color(None);
    assert_eq!(
        mark_color(Some("mark { color: red; }")),
        [1.0, 0.0, 0.0, 1.0]
    );
    // The reset stylesheet removes the base stylesheet's color of marks.
    assert_ne!(mark_color(Some("@import 'std:reset.css';")), base);
}

#[test]
fn a_page_stylesheet_reports_its_diagnostics_at_their_lines_and_columns() {
    let (_, diagnostics) = cascade(Some("mark { color: red; }\nplot {\n  colr: blue;\n}"));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!((diagnostics[0].line, diagnostics[0].column), (3, 3));
    let (_, diagnostics) = cascade(Some("@import 'theme.css';"));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].severity, "error");
    assert!(
        diagnostics[0].message.contains("theme.css"),
        "{}",
        diagnostics[0].message
    );
}
