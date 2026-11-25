//! The parts of the library that run without a browser: font files and the label engine, the
//! conversion of DOM events, and the styles of a project.

use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowEvent, WindowKeyboardInput,
    WindowMouseWheel,
};
use std::collections::BTreeMap;

use avenger_style::{ColorScheme, Element, HostInputs, Property, StyleTree};
use avenger_typst_label::{LabelOptions, RegisteredFont};

use crate::{
    fonts::{font_options, label_engine, read_fonts, NOT_A_FONT},
    input::{self, Touches, Wheel},
    styles::{cascade, load, ProjectStyles},
};

/// The faces that `avenger-fonts` bundles, in their order, as a page adds them.
fn bundled_faces() -> Vec<RegisteredFont> {
    (avenger_fonts::FACES.iter())
        .flat_map(|(_, face)| read_fonts(&avenger_fonts::decompress(face)).unwrap())
        .collect()
}

#[test]
fn a_font_file_reads_as_its_one_face() {
    let lato = avenger_fonts::decompress(avenger_fonts::LATO_REGULAR);
    let read = read_fonts(&lato).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(&*read[0].data, &*lato);
    assert_eq!(read[0].face_index, 0);
}

#[test]
fn a_file_that_is_not_a_font_is_an_error() {
    assert_eq!(read_fonts(b"not a font").unwrap_err(), NOT_A_FONT);
}

#[test]
fn the_engine_takes_the_bundled_families_and_googlesql_formats_from_the_page_fonts() {
    let faces = bundled_faces();
    let options = font_options(faces.clone());
    assert!(!options.load_system_fonts);
    assert_eq!(options.registered_fonts.len(), faces.len());
    let engine = label_engine(faces);
    for family in ["sans-serif", "monospace"] {
        let mut options = LabelOptions::default();
        options.text.font_family = family.into();
        let label = engine.compile_text("Text", &options).unwrap();
        assert!(!label.frame.text_items().is_empty(), "{family}");
    }
    // The charts of avenger-sql name GoogleSQL's syntaxes, which evaluation checks.
    let syntax = |provider: Option<&str>| provider.map(str::to_string);
    assert_eq!(
        syntax(engine.number_format().map(|provider| provider.syntax())),
        Some("googlesql".to_string())
    );
    assert_eq!(
        syntax(engine.datetime_format().map(|provider| provider.syntax())),
        Some("googlesql".to_string())
    );
}

#[test]
fn a_pointer_on_a_scaled_canvas_lands_in_the_canvas_s_own_pixels() {
    // Without a transform, the position is the offset from the box, less the border.
    let plain = input::canvas_position(
        [110.0, 70.0],
        [10.0, 20.0],
        [200.0, 100.0],
        [200.0, 100.0],
        [1.0, 1.0],
    );
    assert_eq!(plain, [99.0, 49.0]);
    // A slide that shows the canvas at half its size: 50 screen pixels are 100 of the canvas's.
    let half = input::canvas_position(
        [60.0, 45.0],
        [10.0, 20.0],
        [100.0, 50.0],
        [200.0, 100.0],
        [0.0, 0.0],
    );
    assert_eq!(half, [100.0, 50.0]);
    // A canvas that the page has not laid out keeps the screen's pixels.
    let unlaid = input::canvas_position(
        [15.0, 25.0],
        [10.0, 20.0],
        [0.0, 0.0],
        [0.0, 0.0],
        [0.0, 0.0],
    );
    assert_eq!(unlaid, [5.0, 5.0]);
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

/// A project of the stylesheets `files`, whose entry stylesheet is `chart.css`.
fn project(files: &[(&str, &str)]) -> ProjectStyles {
    ProjectStyles {
        entry: "chart.css".to_string(),
        files: (files.iter())
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// The colors of a bar chart's root and of a mark of its plot, under `project`, for a reader of
/// `scheme`, on a `transparent` view or not.
fn colors(
    project: Option<&ProjectStyles>,
    scheme: ColorScheme,
    transparent: bool,
) -> ([f32; 4], [f32; 4]) {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let mark = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(mark, "type", "bar", "bar");
    let sheet = project.map(|project| {
        let (sheet, diagnostics) = load(project);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        sheet
    });
    let styles = cascade(sheet.as_ref(), scheme, transparent);
    let computed = styles.compute(&tree, &HostInputs::default());
    let background = computed[tree.root()]
        .color(Property::Background)
        .unwrap_or([0.0; 4]);
    (
        background,
        computed[mark].color(Property::Color).expect("a color"),
    )
}

fn mark_color(project: Option<&ProjectStyles>) -> [f32; 4] {
    colors(project, ColorScheme::Light, false).1
}

#[test]
fn a_project_rule_wins_over_the_base_stylesheet() {
    let base = mark_color(None);
    let red = project(&[("chart.css", "mark { color: red; }")]);
    assert_eq!(mark_color(Some(&red)), [1.0, 0.0, 0.0, 1.0]);
    // The reset stylesheet removes the base stylesheet's color of marks.
    let reset = project(&[("chart.css", "@import 'std:reset.css';")]);
    assert_ne!(mark_color(Some(&reset)), base);
}

#[test]
fn a_project_stylesheet_imports_the_project_s_other_stylesheets() {
    let styles = project(&[
        ("chart.css", "@import 'styles/theme.css';"),
        ("styles/theme.css", "@import '../marks.css';"),
        ("marks.css", "mark { color: blue; }"),
    ]);
    assert_eq!(mark_color(Some(&styles)), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn the_scheme_sets_the_background_and_a_transparent_view_leaves_it_to_the_page() {
    let rgb = |color: [f32; 4]| color.map(|channel| (channel * 255.0).round() as u8);
    let (light, _) = colors(None, ColorScheme::Light, false);
    let (dark, _) = colors(None, ColorScheme::Dark, false);
    assert_eq!(rgb(light), [255, 255, 255, 255]);
    assert_eq!(rgb(dark), [0x1e, 0x1e, 0x1e, 255]);
    let (transparent, _) = colors(None, ColorScheme::Dark, true);
    assert_eq!(transparent[3], 0.0);
}

#[test]
fn a_project_stylesheet_reports_its_diagnostics_at_their_files_lines_and_columns() {
    let typo = project(&[(
        "chart.css",
        "mark { color: red; }\nplot {\n  colr: blue;\n}",
    )]);
    let (_, diagnostics) = load(&typo);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let found = &diagnostics[0];
    assert_eq!(
        (found.file.as_str(), found.line, found.column),
        ("chart.css", 3, 3)
    );
    let missing = project(&[("chart.css", "@import 'theme.css';")]);
    let (_, diagnostics) = load(&missing);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].severity, "error");
    assert!(
        diagnostics[0].message.contains("theme.css"),
        "{}",
        diagnostics[0].message
    );
}
