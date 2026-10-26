//! A text input's typing, deletions, shortcuts, pastes and pointer selection, and which keys it
//! passes to the chart below it.

use std::collections::HashMap;
use std::ops::Range;

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_app_tools::text_selection::TextSelection;
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowComposing, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput,
};
use avenger_layout::Rect;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::bundled_label_engine;
use avenger_widgets::{TextInput, WidgetStyle, Widgets};

const CHART: &str = "chart";
const CONTROLS: &str = "controls";
const SIZE: [f32; 2] = [300.0, 200.0];
/// Where the chart's label starts, on its baseline.
const CHART_LABEL: [f32; 2] = [40.0, 140.0];
/// A point in the chart's label.
const ON_CHART_LABEL: [f32; 2] = [45.0, 135.0];

#[derive(Default)]
struct State {
    rects: HashMap<String, Rect>,
    query: String,
    submitted: Vec<String>,
    /// The key presses that the chart heard.
    heard: Vec<Key>,
}

/// Draws a chart that covers the window, with a label below the input, and a stream that records
/// key presses.
struct Chart;

#[async_trait]
impl AppBuild<State> for Chart {
    fn scenes(&self) -> Vec<Scene<State>> {
        let keys = EventStreamConfig {
            types: vec![SceneGraphEventType::KeyPress],
            ..Default::default()
        };
        vec![Scene::new(CHART, 0).stream(keys, Heard)]
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
            let label = SceneTextMark {
                text: "chart".to_string().into(),
                x: CHART_LABEL[0].into(),
                y: CHART_LABEL[1].into(),
                font_size: 24.0.into(),
                ..Default::default()
            };
            let scene = SceneGraph {
                marks: vec![background.into(), label.into()],
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
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneTrees<'_>,
    ) -> Rebuild {
        if let SceneGraphEvent::KeyPress(press) = event {
            state.heard.push(press.key);
        }
        Rebuild::none()
    }
}

/// A labeled text input that edits the query and records its submissions.
fn widgets() -> Widgets<State> {
    let input = TextInput::new("title")
        .label("Title")
        .placeholder("Filter by title")
        .value(|s: &State| s.query.clone())
        .on_change(|s, query| {
            s.query = query;
            Rebuild::none()
        })
        .on_submit(|s, query| {
            s.submitted.push(query);
            Rebuild::none()
        });
    Widgets::new(CONTROLS, 10, |s: &State, id| s.rects.get(id).copied()).with(input)
}

/// The input near the window's top, with the query `query`.
fn state(query: &str) -> State {
    let rects = HashMap::from([("title".to_string(), Rect::new(10.0, 10.0, 200.0, 50.0))]);
    State {
        rects,
        query: query.to_string(),
        ..Default::default()
    }
}

struct Fixture {
    app: AvengerApp<State>,
}

async fn fixture(query: &str) -> Fixture {
    let app = AvengerApp::builder(state(query), Chart, bundled_label_engine())
        .size(SIZE)
        .tool(widgets())
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
        self.send([moved(at), left(PRESSED), left(RELEASED)]).await;
    }

    /// Type `text` one character at a time.
    async fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = match c {
                ' ' => Key::Named(NamedKey::Space),
                c => Key::Character(c),
            };
            self.send([typing(key, Some(c)), typing(key, None)]).await;
        }
    }

    async fn tap(&mut self, key: Key) {
        self.send([press(key, PRESSED), press(key, RELEASED)]).await;
    }

    /// Tap `key` while holding `modifier`.
    async fn chord(&mut self, modifier: NamedKey, key: Key) {
        let modifier = Key::Named(modifier);
        self.send([press(modifier, PRESSED)]).await;
        self.tap(key).await;
        self.send([press(modifier, RELEASED)]).await;
    }

    fn state(&self) -> &State {
        self.app.state()
    }

    fn query(&self) -> &str {
        &self.state().query
    }

    /// The marks that the input drew, after its backing box.
    fn marks(&self) -> Vec<SceneMark> {
        (self.app.scenes().iter())
            .flat_map(|scene| scene.marks.iter())
            .find_map(|mark| match mark {
                SceneMark::Group(group) if group.name == "title" => Some(group.marks[1..].to_vec()),
                _ => None,
            })
            .unwrap()
    }

    /// The marks in the input's text line, in drawing order.
    fn line_marks(&self) -> Vec<SceneMark> {
        (self.marks().into_iter())
            .find_map(|mark| match mark {
                SceneMark::Group(group) if group.name == "line" => Some(group.marks.clone()),
                _ => None,
            })
            .unwrap()
    }

    /// The names of the marks in the input's text line, in drawing order.
    fn line(&self) -> Vec<String> {
        (self.line_marks().iter())
            .map(|mark| match mark {
                SceneMark::Rect(rect) => rect.name.clone(),
                SceneMark::Text(text) => text.name.clone(),
                _ => String::new(),
            })
            .collect()
    }

    /// The box of the rectangle named `name` in `marks`.
    fn rect(marks: Vec<SceneMark>, name: &str) -> [f32; 4] {
        let rect = (marks.into_iter())
            .find_map(|mark| match mark {
                SceneMark::Rect(rect) if rect.name == name => Some(rect),
                _ => None,
            })
            .unwrap();
        let scalar = |value: &avenger_common::value::ScalarOrArray<f32>| *value.first().unwrap();
        [
            scalar(&rect.x),
            scalar(&rect.y),
            scalar(rect.width.as_ref().unwrap()),
            scalar(rect.height.as_ref().unwrap()),
        ]
    }

    /// The input's box as `[x, y, width, height]`.
    fn field(&self) -> [f32; 4] {
        Self::rect(self.marks(), "field")
    }

    /// The caret's box.
    fn caret(&self) -> [f32; 4] {
        Self::rect(self.line_marks(), "caret")
    }

    /// The text that the input's line shows.
    fn text(&self) -> String {
        (self.line_marks().into_iter())
            .find_map(|mark| match mark {
                SceneMark::Text(text) if text.name == "text" => text.text.first().cloned(),
                _ => None,
            })
            .unwrap()
    }

    /// A point in the box before the text's start.
    fn start(&self) -> [f32; 2] {
        let [x, y, _, height] = self.field();
        [x + 2.0, y + height / 2.0]
    }

    /// A point in the box after the text's end.
    fn end(&self) -> [f32; 2] {
        let [x, y, width, height] = self.field();
        [x + width - 2.0, y + height / 2.0]
    }

    /// Focus the input with a press after its text.
    async fn focus(&mut self) {
        self.click(self.end()).await;
    }
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

fn press(key: Key, state: ElementState) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key,
        state,
        text: None,
    })
}

/// A press of `key` that types `text`, or the key's release without text.
fn typing(key: Key, text: Option<char>) -> WindowEvent {
    let state = match text {
        Some(_) => PRESSED,
        None => RELEASED,
    };
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key,
        state,
        text: text.map(String::from),
    })
}

const PRESSED: ElementState = ElementState::Pressed;
const RELEASED: ElementState = ElementState::Released;
const LEFT: Key = Key::Named(NamedKey::ArrowLeft);
const RIGHT: Key = Key::Named(NamedKey::ArrowRight);
const BACKSPACE: Key = Key::Named(NamedKey::Backspace);
const DELETE: Key = Key::Named(NamedKey::Delete);

#[test]
fn typing_inserts_at_the_caret_and_replaces_the_selection() {
    pollster::block_on(async {
        let mut f = fixture("").await;
        assert_eq!(f.line(), ["text"], "the placeholder");
        f.focus().await;
        assert_eq!(f.app.cursor(), CursorIcon::Text);
        assert_eq!(f.line(), ["text", "caret"]);

        f.type_text("star wars").await;
        assert_eq!(f.query(), "star wars");
        assert!(f.state().heard.is_empty(), "{:?}", f.state().heard);

        for _ in 0..4 {
            f.chord(NamedKey::Shift, LEFT).await;
        }
        assert_eq!(f.line(), ["selection", "text"]);
        f.type_text("trek").await;
        assert_eq!(f.query(), "star trek");
    });
}

#[test]
fn deletions_and_movements_reach_a_grapheme_a_word_or_the_line() {
    pollster::block_on(async {
        let mut f = fixture("the dark knight").await;
        f.focus().await;
        f.chord(NamedKey::Alt, BACKSPACE).await;
        assert_eq!(f.query(), "the dark ");
        f.chord(NamedKey::Control, LEFT).await;
        f.tap(BACKSPACE).await;
        assert_eq!(f.query(), "thedark ");
        f.chord(NamedKey::Super, LEFT).await;
        f.tap(DELETE).await;
        assert_eq!(f.query(), "hedark ");
        f.tap(RIGHT).await;
        f.chord(NamedKey::Super, DELETE).await;
        assert_eq!(f.query(), "h");
        f.chord(NamedKey::Super, BACKSPACE).await;
        assert_eq!(f.query(), "");
    });
}

#[test]
fn select_all_copies_and_cut_deletes_while_undo_reaches_the_chart() {
    pollster::block_on(async {
        let mut f = fixture("alien").await;
        f.focus().await;
        assert_eq!(f.app.copy_text(), "");
        f.chord(NamedKey::Control, Key::Character('a')).await;
        assert_eq!(f.app.copy_text(), "alien");
        f.chord(NamedKey::Super, Key::Character('x')).await;
        assert_eq!((f.query(), f.app.copy_text()), ("", ""));

        f.chord(NamedKey::Super, Key::Character('z')).await;
        let heard = &f.state().heard;
        assert!(heard.contains(&Key::Character('z')), "{heard:?}");
        assert!(!heard.contains(&Key::Character('x')), "{heard:?}");
    });
}

#[test]
fn keyboard_focus_selects_all() {
    pollster::block_on(async {
        let mut f = fixture("alien").await;
        f.tap(Key::Named(NamedKey::Tab)).await;
        assert_eq!(f.app.copy_text(), "alien");
    });
}

#[test]
fn a_paste_inserts_its_text_on_one_line() {
    pollster::block_on(async {
        let mut f = fixture("").await;
        f.focus().await;
        f.send([WindowEvent::Paste("one\r\ntwo\tthree\n".to_string())])
            .await;
        assert_eq!(f.query(), "one two three ");
    });
}

fn composing(text: &str, cursor: Option<Range<usize>>) -> WindowEvent {
    WindowEvent::Composing(WindowComposing {
        text: text.to_string(),
        cursor,
    })
}

/// While the input has focus, the app reports its caret, beside which input methods place their
/// candidates. Text that an input method composes shows underlined in place of the selection,
/// without changing the value, until the input method commits or cancels it.
#[test]
fn input_methods_compose_in_place_of_the_selection() {
    pollster::block_on(async {
        let mut f = fixture("caf").await;
        assert_eq!(f.app.text_entry(), None);
        f.focus().await;
        let caret = f.caret();
        assert_eq!(f.app.text_entry(), Some(caret));

        f.send([composing("´", Some(2..2))]).await;
        assert_eq!((f.query(), f.text().as_str()), ("caf", "caf´"));
        assert_eq!(f.line(), ["text", "composing", "caret"]);
        assert!(f.caret()[0] > caret[0]);
        assert_eq!(f.app.text_entry(), Some(f.caret()));
        f.send([WindowEvent::Composed("é".to_string())]).await;
        assert_eq!((f.query(), f.text().as_str()), ("café", "café"));
        assert_eq!(f.line(), ["text", "caret"]);

        // An empty composition cancels it, and a composition replaces the selection.
        f.send([composing("¨", Some(2..2)), composing("", None)])
            .await;
        assert_eq!((f.query(), f.text().as_str()), ("café", "café"));
        f.chord(NamedKey::Control, Key::Character('a')).await;
        f.send([composing("´", Some(2..2))]).await;
        assert_eq!((f.query(), f.text().as_str()), ("café", "´"));
        f.send([WindowEvent::Composed("é".to_string())]).await;
        assert_eq!(f.query(), "é");

        f.click([290.0, 190.0]).await;
        assert_eq!(f.app.text_entry(), None);
    });
}

#[test]
fn a_press_places_the_caret_a_drag_selects_and_a_double_press_selects_a_word() {
    pollster::block_on(async {
        let mut f = fixture("star wars").await;
        f.focus().await;
        f.type_text("!").await;
        f.click(f.start()).await;
        f.type_text("¡").await;
        assert_eq!(f.query(), "¡star wars!");

        // From the end, so that the press doesn't make a double press with the last one.
        let (start, end) = (f.start(), f.end());
        f.send([moved(end), left(PRESSED), moved(start), left(RELEASED)])
            .await;
        assert_eq!(f.app.copy_text(), "¡star wars!");

        f.click(f.start()).await;
        f.click(f.start()).await;
        assert_eq!(f.app.copy_text(), "¡");
        f.click(f.end()).await;
        f.click(f.end()).await;
        f.click(f.end()).await;
        assert_eq!(f.app.copy_text(), "¡star wars!");
    });
}

/// The point in the box after `prefix` of the input's text, while the text does not scroll.
fn after(f: &Fixture, prefix: &str) -> [f32; 2] {
    let style = WidgetStyle::default();
    let text = SceneTextMark {
        text: prefix.to_string().into(),
        font: style.font_family.into(),
        font_size: style.font_size.into(),
        ..Default::default()
    };
    let label = text.labels().next().unwrap().label;
    let width = f.app.text_engine().bounds(&label).unwrap().width;
    let [x, y, _, height] = f.field();
    [x + style.button_padding[0] + width, y + height / 2.0]
}

#[test]
fn a_drag_from_a_double_press_selects_whole_words() {
    pollster::block_on(async {
        let mut f = fixture("alpha bravo charlie").await;
        let bravo = after(&f, "alpha br");
        f.click(bravo).await;
        f.send([left(PRESSED)]).await;
        assert_eq!(f.app.copy_text(), "bravo");
        f.send([moved(f.end())]).await;
        assert_eq!(f.app.copy_text(), "bravo charlie");
        f.send([moved(f.start()), left(RELEASED)]).await;
        assert_eq!(f.app.copy_text(), "alpha bravo");
    });
}

#[test]
fn enter_submits_and_escape_reaches_the_chart() {
    pollster::block_on(async {
        let mut f = fixture("alien").await;
        f.focus().await;
        f.tap(Key::Named(NamedKey::Enter)).await;
        f.tap(Key::Named(NamedKey::Escape)).await;
        assert_eq!(f.state().submitted, ["alien"]);
        assert_eq!(f.state().heard, [Key::Named(NamedKey::Escape)]);
    });
}

#[test]
fn a_long_value_scrolls_to_keep_the_caret_in_the_box() {
    pollster::block_on(async {
        let mut f = fixture("").await;
        f.focus().await;
        let [x, _, width, _] = f.field();
        let inside =
            |[caret, _, caret_width, _]: [f32; 4]| x < caret && caret + caret_width < x + width;
        let first = f.caret();
        f.type_text(&"the good the bad and the ugly ".repeat(3))
            .await;
        assert!(inside(f.caret()), "{:?} in {:?}", f.caret(), f.field());
        f.chord(NamedKey::Super, LEFT).await;
        assert_eq!(f.caret(), first);
        f.chord(NamedKey::Super, RIGHT).await;
        assert!(inside(f.caret()), "{:?} in {:?}", f.caret(), f.field());
    });
}

#[test]
fn a_value_that_app_state_shortens_moves_the_caret_into_it() {
    pollster::block_on(async {
        let mut f = fixture("abcdef").await;
        f.focus().await;
        f.app.state_mut().query = "ab".to_string();
        f.tap(BACKSPACE).await;
        assert_eq!(f.query(), "a");
    });
}

/// The input under a text selection scene, as the movie search example has it.
async fn under_selection(query: &str) -> Fixture {
    let app = AvengerApp::builder(state(query), Chart, bundled_label_engine())
        .size(SIZE)
        .tool(widgets())
        .tool(TextSelection::new("selection", 20))
        .build()
        .await
        .unwrap();
    Fixture { app }
}

#[test]
fn a_text_selection_scene_above_leaves_drags_in_the_box_to_the_input() {
    pollster::block_on(async {
        let mut f = under_selection("star wars").await;
        // A press on the text's first letter.
        let [x, y, _, height] = f.field();
        let on_text = [x + 14.0, y + height / 2.0];
        f.send([
            moved(on_text),
            left(PRESSED),
            moved(f.end()),
            left(RELEASED),
        ])
        .await;
        assert_eq!(f.line(), ["selection", "text"]);
        assert!(f.app.scenes()[2].marks.is_empty(), "no text selection");
        assert_eq!(f.app.copy_text(), "tar wars");
    });
}

/// The focused input owns copy and cut over a chart label's selection, and a press that the text
/// selection scene takes on the label ends the input's focus.
#[test]
fn the_focused_input_owns_copy_until_a_press_on_chart_text() {
    pollster::block_on(async {
        let mut f = under_selection("input").await;
        f.click(ON_CHART_LABEL).await;
        f.click(ON_CHART_LABEL).await;
        assert_eq!(f.app.copy_text(), "chart");

        // Keyboard focus selects the input's text, which the cut copies.
        f.tap(Key::Named(NamedKey::Tab)).await;
        assert_eq!(f.app.copy_text(), "input");
        f.chord(NamedKey::Super, Key::Character('x')).await;
        assert_eq!((f.query(), f.app.copy_text()), ("", ""));

        f.click(ON_CHART_LABEL).await;
        assert_eq!(f.app.text_entry(), None);
        f.type_text("z").await;
        assert_eq!(f.query(), "");
        assert!(f.state().heard.contains(&Key::Character('z')));
    });
}

/// After app state replaces the value, the copy text is the selection that the input shows.
#[test]
fn copy_text_follows_a_value_that_app_state_replaces() {
    pollster::block_on(async {
        let mut f = fixture("old").await;
        f.tap(Key::Named(NamedKey::Tab)).await;
        assert_eq!(f.app.copy_text(), "old");
        f.app.state_mut().query = "new".to_string();
        f.app.waker().request(Refresh::Rebuild);
        f.app.refresh().await;
        assert_eq!((f.text().as_str(), f.app.copy_text()), ("new", "new"));
    });
}

/// The caret stays in the box across long right-to-left text, whose start is at its right.
#[test]
fn the_caret_stays_in_the_box_in_right_to_left_text() {
    pollster::block_on(async {
        let mut f = fixture(&"שלום".repeat(20)).await;
        f.tap(Key::Named(NamedKey::Tab)).await;
        f.tap(Key::Named(NamedKey::Home)).await;
        let [x, _, width, _] = f.field();
        for step in 0..=80 {
            let [caret, _, caret_width, _] = f.caret();
            assert!(
                x <= caret && caret + caret_width <= x + width,
                "step {step}: caret {:?} outside {:?}",
                f.caret(),
                f.field()
            );
            f.tap(RIGHT).await;
        }
    });
}
