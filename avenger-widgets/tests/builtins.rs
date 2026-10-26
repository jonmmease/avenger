//! The built-in widgets: their pointer and key handling, their measurements in a solved layout,
//! and their disabled paint.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::text_selection::TextSelection;
use avenger_color::ColorOrGradient;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_layout::{
    Column, Direction, Layout, Leaf, Measure, Rect, Row, Size, SolveOptions, Track,
};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{bundled_label_engine, LabelEngine};
use avenger_widgets::{Button, Checkbox, GroupBox, RadioGroup, Slider, WidgetStyle, Widgets};

const CHART: &str = "chart";
const CONTROLS: &str = "controls";
const SIZE: [f32; 2] = [300.0, 200.0];
const OUTSIDE: [f32; 2] = [295.0, 195.0];

#[derive(Default)]
struct State {
    rects: HashMap<String, Rect>,
    presses: usize,
    checked: bool,
    choice: u8,
    value: f64,
    settles: Vec<f64>,
    disabled: bool,
    /// Number of presses received by the chart.
    heard: usize,
}

/// Draws a chart that covers the window, with a stream that counts presses.
struct Chart;

#[async_trait]
impl AppBuild<State> for Chart {
    fn scenes(&self) -> Vec<Scene<State>> {
        let presses = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            ..Default::default()
        };
        vec![Scene::new(CHART, 0).stream(presses, Heard)]
    }

    async fn build(
        &self,
        _state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(CHART) {
            let background = SceneRectMark {
                width: Some(SIZE[0].into()),
                height: Some(SIZE[1].into()),
                ..Default::default()
            };
            let scene = SceneGraph {
                marks: vec![background.into()],
                width: SIZE[0],
                height: SIZE[1],
                origin: [0.0; 2],
            };
            scenes.set(CHART, scene.into())?;
        }
        Ok(())
    }
}

struct Heard;

#[async_trait]
impl EventStreamHandler<State> for Heard {
    async fn handle(&self, _: &SceneGraphEvent, state: &mut State, _: &SceneTrees<'_>) -> Rebuild {
        state.heard += 1;
        Rebuild::none()
    }
}

fn engine() -> LabelEngine {
    bundled_label_engine().with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

const ITEMS: [(u8, &str); 3] = [(1, "One"), (2, "Two"), (3, "Three")];

fn radio() -> RadioGroup<State, u8> {
    RadioGroup::new("radio", ITEMS)
        .value(|s: &State| s.choice)
        .on_change(|s, choice| {
            s.choice = choice;
            Rebuild::none()
        })
}

/// Each widget's rectangle in app state.
fn placed(state: &State, id: &str) -> Option<Rect> {
    state.rects.get(id).copied()
}

/// A button, a check box, a radio group and a stepped slider with a readout, placed by their
/// rectangles in app state.
fn widgets() -> Widgets<State> {
    Widgets::new(CONTROLS, 10, placed)
        .with(
            Button::new("button")
                .label("Reset")
                .on_press(|s: &mut State| {
                    s.presses += 1;
                    Rebuild::none()
                }),
        )
        .with(
            Checkbox::new("check")
                .label("Grid")
                .value(|s: &State| s.checked)
                .on_change(|s, on| {
                    s.checked = on;
                    Rebuild::none()
                })
                .enabled(|s| !s.disabled),
        )
        .with(radio())
        .with(
            Slider::new("slider", 0.0..=10.0)
                .label("Speed")
                .step(3.0)
                .value(|s: &State| s.value)
                .on_change(|s, value| {
                    s.value = value;
                    Rebuild::none()
                })
                .on_settle(|s, value| {
                    s.settles.push(value);
                    Rebuild::none()
                })
                .readout(".1f")
                .readout_label("#value m/s"),
        )
}

/// The widgets stacked down the window.
fn stacked() -> State {
    let mut state = State::default();
    for (id, rect) in [
        ("button", Rect::new(10.0, 10.0, 80.0, 24.0)),
        ("check", Rect::new(10.0, 40.0, 120.0, 20.0)),
        ("radio", Rect::new(10.0, 70.0, 280.0, 20.0)),
        ("slider", Rect::new(10.0, 100.0, 200.0, 40.0)),
    ] {
        state.rects.insert(id.to_string(), rect);
    }
    state
}

struct Fixture {
    app: AvengerApp<State>,
}

async fn fixture_of(state: State, widgets: Widgets<State>) -> Fixture {
    let app = AvengerApp::builder(state, Chart, engine())
        .size(SIZE)
        .tool(widgets)
        .build()
        .await
        .unwrap();
    Fixture { app }
}

async fn fixture(state: State) -> Fixture {
    fixture_of(state, widgets()).await
}

/// The widgets, under a text selection scene.
async fn under_selection(state: State) -> Fixture {
    let app = AvengerApp::builder(state, Chart, engine())
        .size(SIZE)
        .tool(widgets())
        .tool(TextSelection::new("selection", 20))
        .build()
        .await
        .unwrap();
    Fixture { app }
}

impl Fixture {
    async fn send(&mut self, events: impl IntoIterator<Item = WindowEvent>) {
        for event in events {
            self.app.update(&event, Instant::now()).await;
        }
    }

    async fn click(&mut self, at: [f32; 2]) {
        self.send([
            moved(at),
            left(ElementState::Pressed),
            left(ElementState::Released),
        ])
        .await;
    }

    async fn tap(&mut self, pressed: Key) {
        self.send([
            key(pressed, ElementState::Pressed),
            key(pressed, ElementState::Released),
        ])
        .await;
    }

    fn state(&mut self) -> &mut State {
        self.app.state_mut()
    }

    /// The marks that widget `id` drew, after its backing box.
    fn marks(&self, id: &str) -> Vec<SceneMark> {
        (self.app.scenes().iter())
            .flat_map(|scene| scene.marks.iter())
            .find_map(|mark| match mark {
                SceneMark::Group(group) if group.name == id => Some(group.marks[1..].to_vec()),
                _ => None,
            })
            .unwrap()
    }

    /// The boxes of the rectangles named `name` that widget `id` drew.
    fn boxes(&self, id: &str, name: &str) -> Vec<SceneRectMark> {
        (self.marks(id).into_iter())
            .filter_map(|mark| match mark {
                SceneMark::Rect(rect) if rect.name == name => Some(rect),
                _ => None,
            })
            .collect()
    }

    fn heard(&self) -> usize {
        self.app.state().heard
    }
}

/// A point in the first letter of the text mark named `name` that widget `id` drew.
fn in_text(f: &Fixture, id: &str, name: &str) -> [f32; 2] {
    (f.marks(id).into_iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == name => {
                Some([*text.x.first()? + 3.0, *text.y.first()? + 6.0])
            }
            _ => None,
        })
        .unwrap()
}

/// A rectangle mark's box as `[x, y, width, height]`.
fn bounds(rect: &SceneRectMark) -> [f32; 4] {
    let scalar = |value: &avenger_common::value::ScalarOrArray<f32>| *value.first().unwrap();
    [
        scalar(&rect.x),
        scalar(&rect.y),
        scalar(rect.width.as_ref().unwrap()),
        scalar(rect.height.as_ref().unwrap()),
    ]
}

fn center([x, y, width, height]: [f32; 4]) -> [f32; 2] {
    [x + width / 2.0, y + height / 2.0]
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn left(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn key(key: Key, state: ElementState) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput { key, state })
}

const TAB: Key = Key::Named(NamedKey::Tab);
const ENTER: Key = Key::Named(NamedKey::Enter);
const SPACE: Key = Key::Named(NamedKey::Space);
const ESCAPE: Key = Key::Named(NamedKey::Escape);
const RIGHT: Key = Key::Named(NamedKey::ArrowRight);
const LEFT: Key = Key::Named(NamedKey::ArrowLeft);
const PRESSED: ElementState = ElementState::Pressed;
const RELEASED: ElementState = ElementState::Released;

#[test]
fn a_button_activates_from_the_pointer_enter_and_space() {
    pollster::block_on(async {
        let mut f = fixture(stacked()).await;
        let button = center(bounds(&f.boxes("button", "button")[0]));
        f.click(button).await;
        assert_eq!(f.state().presses, 1);

        // A release outside cancels.
        f.send([moved(button), left(PRESSED), moved(OUTSIDE), left(RELEASED)])
            .await;
        assert_eq!(f.state().presses, 1);

        // Enter activates on its press, once for each hold.
        f.send([
            key(ENTER, PRESSED),
            key(ENTER, PRESSED),
            key(ENTER, RELEASED),
        ])
        .await;
        assert_eq!(f.state().presses, 2);
        f.send([key(SPACE, PRESSED)]).await;
        assert_eq!(f.state().presses, 2);
        f.send([key(SPACE, RELEASED)]).await;
        assert_eq!(f.state().presses, 3);

        // Escape cancels a pending Space or pointer press.
        f.send([
            key(SPACE, PRESSED),
            key(ESCAPE, PRESSED),
            key(SPACE, RELEASED),
        ])
        .await;
        f.send([
            moved(button),
            left(PRESSED),
            key(ESCAPE, PRESSED),
            left(RELEASED),
        ])
        .await;
        assert_eq!(f.state().presses, 3);
    });
}

#[test]
fn a_check_box_toggles_from_the_pointer_and_space() {
    pollster::block_on(async {
        let mut f = fixture(stacked()).await;
        let check = center(bounds(&f.boxes("check", "checkbox")[0]));
        f.click(check).await;
        assert!(f.state().checked);
        f.tap(SPACE).await;
        assert!(!f.state().checked);
    });
}

#[test]
fn arrow_keys_move_the_radio_selection_and_wrap() {
    pollster::block_on(async {
        let mut f = fixture(stacked()).await;
        let first = center(bounds(&f.boxes("radio", "radio")[0]));
        f.click(first).await;
        assert_eq!(f.state().choice, 1);
        f.tap(LEFT).await;
        assert_eq!(f.state().choice, 3);
        f.tap(RIGHT).await;
        f.tap(RIGHT).await;
        assert_eq!(f.state().choice, 2);
    });
}

/// Place a radio group beside a 40-pixel leaf in a window of the given `width`.
/// Return the fixture, solved rectangle and measured height at the solved width.
async fn radio_in(width: f32) -> (Fixture, Rect, f32) {
    let widgets = Widgets::new(CONTROLS, 10, placed).with(radio());
    let layout: Layout<String> = Row::new([
        Leaf::new().id("radio".to_string()),
        Leaf::new().min_size(Size::new(40.0, 10.0)),
    ])
    .track(0, Track::auto())
    .into();
    let engine = engine();
    let mut measure = widgets.measure(&engine);
    let solution = (layout.measure(&SolveOptions::sized(width, 200.0), &mut measure))
        .unwrap()
        .into_solution();
    let rect = solution.region(&"radio".to_string()).unwrap().content;
    let height = measure
        .height(&"radio".to_string(), rect.width)
        .unwrap()
        .size;
    let mut state = State::default();
    state.rects.insert("radio".to_string(), rect);
    (fixture_of(state, widgets).await, rect, height)
}

#[test]
fn a_radio_group_is_a_row_when_it_fits_and_a_column_otherwise() {
    pollster::block_on(async {
        let (mut wide, wide_rect, wide_height) = radio_in(400.0).await;
        let (mut narrow, narrow_rect, narrow_height) = radio_in(120.0).await;
        assert!(narrow_rect.width < wide_rect.width);
        assert!(narrow_height > wide_height);

        let circles = |f: &Fixture| {
            f.boxes("radio", "radio")
                .iter()
                .map(bounds)
                .collect::<Vec<_>>()
        };
        let (row, column) = (circles(&wide), circles(&narrow));
        assert!(row.iter().all(|circle| circle[1] == row[0][1]));
        assert!(column.iter().all(|circle| circle[0] == column[0][0]));

        // The drawing and the hit test agree in both arrangements.
        for (f, circles) in [(&mut wide, row), (&mut narrow, column)] {
            f.click(center(circles[1])).await;
            assert_eq!(f.state().choice, 2);
        }
    });
}

#[test]
fn a_stepped_slider_follows_its_keys() {
    pollster::block_on(async {
        let mut state = stacked();
        state.value = 4.5;
        let mut f = fixture(state).await;
        // A press on the label focuses the slider without moving it.
        let slider = f.state().rects["slider"];
        f.click([slider.x + 2.0, slider.y + 2.0]).await;
        assert_eq!(f.state().value, 4.5);

        // 4.5 snaps up to 6, a tie, and one step up is 9.
        f.tap(RIGHT).await;
        assert_eq!(f.state().value, 9.0);
        f.tap(RIGHT).await;
        assert_eq!(f.state().value, 10.0);
        f.tap(RIGHT).await;
        // One step down from the end is the last step.
        f.tap(LEFT).await;
        assert_eq!(f.state().value, 9.0);
        assert_eq!(f.state().settles, [9.0, 10.0, 9.0]);

        f.tap(Key::Named(NamedKey::Home)).await;
        assert_eq!(f.state().value, 0.0);
        f.tap(Key::Named(NamedKey::PageUp)).await;
        assert_eq!(f.state().value, 10.0);
        f.tap(Key::Named(NamedKey::PageDown)).await;
        assert_eq!(f.state().value, 0.0);
    });
}

#[test]
fn a_slider_gesture_settles_once_and_escape_or_a_cancel_restores_it() {
    pollster::block_on(async {
        let mut state = stacked();
        state.value = 4.5;
        let mut f = fixture(state).await;
        let [x, y, width, height] = bounds(&f.boxes("slider", "track")[0]);
        let at = |along: f32| [x + along * width, y + height / 2.0];

        // Escape restores exactly the value at the press, which no step admits, and the release
        // settles it.
        f.send([moved(at(0.97)), left(PRESSED)]).await;
        assert_eq!(f.state().value, 10.0);
        f.send([key(ESCAPE, PRESSED), moved(at(0.6)), left(RELEASED)])
            .await;
        assert_eq!(f.state().value, 4.5);
        assert_eq!(f.state().settles, [4.5]);

        // A drag settles once, at its release.
        f.send([moved(at(0.97)), left(PRESSED), moved(at(0.3))])
            .await;
        assert_eq!(f.state().value, 3.0);
        f.send([left(RELEASED)]).await;
        assert_eq!(f.state().settles, [4.5, 3.0]);

        // A drag that the window's focus loss cancels restores its value and settles.
        let unfocused = WindowEvent::WindowFocused(false);
        f.send([moved(at(0.97)), left(PRESSED), unfocused]).await;
        assert_eq!(f.state().value, 3.0);
        assert_eq!(f.state().settles, [4.5, 3.0, 3.0]);
    });
}

/// Focus loss clears held-key state: Enter released elsewhere does not block the next press,
/// and Space released after focus returns does not activate the button.
#[test]
fn a_held_key_ends_when_the_window_loses_focus() {
    pollster::block_on(async {
        let mut f = fixture(stacked()).await;
        f.tap(TAB).await;
        let (unfocused, focused) = (
            WindowEvent::WindowFocused(false),
            WindowEvent::WindowFocused(true),
        );
        f.send([key(ENTER, PRESSED), unfocused.clone(), focused.clone()])
            .await;
        f.tap(ENTER).await;
        assert_eq!(f.state().presses, 2);

        f.send([
            key(SPACE, PRESSED),
            unfocused,
            focused,
            key(SPACE, RELEASED),
        ])
        .await;
        assert_eq!(f.state().presses, 2);
    });
}

#[test]
fn a_slider_readout_formats_its_value_into_its_label() {
    pollster::block_on(async {
        let mut state = stacked();
        state.value = 3.0;
        let f = fixture(state).await;
        let readout = (f.marks("slider").into_iter())
            .find_map(|mark| match mark {
                SceneMark::Text(text) if text.name == "readout" => text.text.first().cloned(),
                _ => None,
            })
            .unwrap();
        // The bound label is markup that shows the formatted value.
        assert_eq!(readout, "#(\"3.0\") m/s");
    });
}

#[test]
fn a_disabled_check_box_ignores_input_and_dims() {
    pollster::block_on(async {
        let mut state = stacked();
        state.disabled = true;
        let mut f = fixture(state).await;
        let check = f.boxes("check", "checkbox")[0].clone();
        f.click(center(bounds(&check))).await;
        assert!(!f.state().checked);
        assert_eq!(f.heard(), 0);
        let disabled = WidgetStyle::default().neutral.disabled.fill;
        assert_eq!(check.fill.first(), Some(&ColorOrGradient::Color(disabled)));
    });
}

/// Under a text selection scene, a press on a control's label acts on the control, and a
/// title's text is selectable.
#[test]
fn a_selection_above_takes_titles_but_not_control_labels() {
    pollster::block_on(async {
        let mut f = under_selection(stacked()).await;
        f.click(in_text(&f, "button", "button_label")).await;
        assert_eq!(f.state().presses, 1);
        f.click(in_text(&f, "check", "checkbox_label")).await;
        assert!(f.state().checked);
        f.click(in_text(&f, "radio", "radio_label")).await;
        assert_eq!(f.state().choice, 1);
        assert_eq!(f.app.copy_text(), "");

        // Two presses on the slider's label select its word.
        let label = in_text(&f, "slider", "slider_label");
        f.click(label).await;
        f.click(label).await;
        assert_eq!(f.app.copy_text(), "Speed");
    });
}

/// Omitted labels and readouts reserve no space.
#[test]
fn widgets_without_text_take_no_room_for_it() {
    let widgets = Widgets::new(CONTROLS, 10, placed)
        .with(Slider::new("slider", 0.0..=1.0))
        .with(Checkbox::new("check"));
    let engine = engine();
    let mut measure = widgets.measure(&engine);
    let mut size = |id: &str, direction| measure.max_content(&id.to_string(), direction).unwrap();
    let style = WidgetStyle::default();
    assert_eq!(size("slider", Direction::Vertical), style.thumb_size);
    assert_eq!(size("check", Direction::Horizontal), style.control_size);
}

const TITLE: &str = "A group box title wider than its radio group";

#[test]
fn a_group_box_frames_the_widgets_inside_it() {
    pollster::block_on(async {
        let widgets = Widgets::new(CONTROLS, 10, placed)
            .with(GroupBox::new("box", TITLE))
            .with(radio());
        // The box's container, nested in a column, holds the radio group inside the box's
        // padding, below the title.
        let engine = engine();
        let radio_leaf = Layout::from(Leaf::new().id("radio".to_string()));
        let boxed = widgets.group_box_layout("box", "frame".to_string(), &engine, [radio_leaf]);
        let below = Leaf::new().min_size(Size::new(10.0, 10.0));
        let layout: Layout<String> = Column::new([Layout::from(boxed), below.into()]).into();
        let mut measure = widgets.measure(&engine);
        let options = SolveOptions {
            width: None,
            height: None,
            ..SolveOptions::sized(0.0, 0.0)
        };
        let solution = layout
            .measure(&options, &mut measure)
            .unwrap()
            .into_solution();
        let region = |id: &str| solution.region(&id.to_string()).unwrap().clone();
        let (frame, radio) = (region("frame").slot, region("radio").content);

        // The title, wider than the radio group, widens the box.
        let radio_width = measure.max_content(&"radio".to_string(), Direction::Horizontal);
        assert!(radio.width > radio_width.unwrap());
        let style = WidgetStyle::default();
        assert_eq!(radio.x, frame.x + style.border_width + style.group_padding);
        assert!(radio.y > frame.y);

        let mut state = State::default();
        state.rects.insert("box".to_string(), frame);
        state.rects.insert("radio".to_string(), radio);
        let mut f = fixture_of(state, widgets).await;

        // A press in the box outside the radio group reaches neither the group nor the chart.
        f.click([frame.x + 2.0, frame.bottom() - 2.0]).await;
        assert_eq!(f.state().choice, 0);
        assert_eq!(f.heard(), 0);

        let second = center(bounds(&f.boxes("radio", "radio")[1]));
        f.click(second).await;
        assert_eq!(f.state().choice, 2);
    });
}
