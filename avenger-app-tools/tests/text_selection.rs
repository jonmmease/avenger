//! Text selection across positions, words, lines, and labels in drawing order.
//! Tests cover highlights, cursor and copy outputs, and exclusive handling of presses on
//! selectable text.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Changes, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_app_tools::text_selection::TextSelection;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_common::types::{TextBaseline, TextSyntaxMode};
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{bundled_label_engine, Affinity, LabelWidth, TextBounds, TextPosition};
use lyon_path::PathEvent;

const TITLE: &str = "one two three";
const CHART: &str = "chart";
const UPPER: &str = "upper";

/// Horizontal position of labels other than "side".
const LEFT: f32 = 10.0;
const SIDE: [f32; 2] = [60.0, 80.0];

#[derive(Default)]
struct State {
    title: String,
    /// Replacement text marks for the chart scene.
    placed: Option<Vec<SceneTextMark>>,
    /// Whether the upper scene draws an interactive box over the title.
    cover: bool,
    /// Presses and clicks received by the chart stream beneath the selection tool.
    heard: Arc<Mutex<Vec<&'static str>>>,
}

fn text(name: &str, text: &str, y: f32) -> SceneTextMark {
    text_at(name, text, [LEFT, y])
}

fn text_at(name: &str, text: &str, [x, y]: [f32; 2]) -> SceneTextMark {
    SceneTextMark {
        name: name.into(),
        len: 1,
        text: text.to_string().into(),
        x: x.into(),
        y: y.into(),
        baseline: TextBaseline::Top.into(),
        font: "Lato".to_string().into(),
        font_size: 12.0.into(),
        width: LabelWidth::Max(40.0).into(),
        ..Default::default()
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> SceneRectMark {
    SceneRectMark {
        len: 1,
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        ..Default::default()
    }
}

fn graph(marks: Vec<SceneMark>) -> SceneGraph {
    SceneGraph {
        marks,
        width: 200.0,
        height: 200.0,
        origin: [0.0; 2],
    }
}

/// Chart fixture with a wrapped title, adjacent labels, a noninteractive caption, and a covered
/// label, all over an interactive background.
/// An upper scene adds a label and an optional interactive box over the title.
struct Texts;

#[async_trait]
impl AppBuild<State> for Texts {
    fn scenes(&self) -> Vec<Scene<State>> {
        let presses = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::Click,
                SceneGraphEventType::DoubleClick,
            ],
            ..Default::default()
        };
        vec![
            Scene::new(CHART, 0).stream(presses, Heard),
            Scene::new(UPPER, 5),
        ]
    }

    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(CHART) {
            let marks = match &state.placed {
                Some(placed) => placed.iter().cloned().map(SceneMark::from).collect(),
                None => vec![
                    rect(0.0, 0.0, 200.0, 200.0).into(),
                    text("title", &state.title, 10.0).into(),
                    text("note", "note", 80.0).into(),
                    text_at("side", "side", SIDE).into(),
                    SceneTextMark {
                        interactive: Some(false),
                        ..text("caption", "quiet", 120.0)
                    }
                    .into(),
                    text("hidden", "hidden", 140.0).into(),
                    rect(0.0, 138.0, 100.0, 20.0).into(),
                ],
            };
            scenes.set(CHART, graph(marks).into())?;
        }
        if scenes.requested(UPPER) {
            let mut marks: Vec<SceneMark> = vec![text("upper", "upper note", 170.0).into()];
            if state.cover {
                marks.push(rect(0.0, 0.0, 100.0, 60.0).into());
            }
            scenes.set(UPPER, graph(marks).into())?;
        }
        Ok(())
    }
}

/// Records presses and clicks received by the chart stream.
struct Heard;

#[async_trait]
impl EventStreamHandler<State> for Heard {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneTrees<'_>,
    ) -> Rebuild {
        let heard = match event {
            SceneGraphEvent::MouseDown(_) => "press",
            SceneGraphEvent::Click(_) => "click",
            SceneGraphEvent::DoubleClick(_) => "double click",
            _ => return Rebuild::none(),
        };
        state.heard.lock().unwrap().push(heard);
        Rebuild::none()
    }
}

async fn app_with(state: State) -> AvengerApp<State> {
    AvengerApp::builder(state, Texts, bundled_label_engine())
        .size([200.0, 200.0])
        .tool(TextSelection::new("selection", 10))
        .build()
        .await
        .unwrap()
}

async fn app() -> AvengerApp<State> {
    app_with(State {
        title: TITLE.to_string(),
        ..Default::default()
    })
    .await
}

/// Return the scene point at source `offset` in a label at `(10, y)`.
fn caret(source: &str, y: f32, offset: usize) -> [f32; 2] {
    caret_at(source, [LEFT, y], offset)
}

/// Return the scene point at source `offset` in a label at `[x, y]`.
fn caret_at(source: &str, [x, y]: [f32; 2], offset: usize) -> [f32; 2] {
    let label = text_at("", source, [x, y]);
    let label = label.labels().next().unwrap();
    let layout = bundled_label_engine().text_layout(&label.label).unwrap();
    let caret = layout.caret(TextPosition::new(offset, Affinity::Downstream));
    [x + caret.x + 0.5, y + caret.y + caret.height / 2.0]
}

fn moved(point: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: point })
}

fn button(state: ElementState, button: MouseButton) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput { state, button })
}

fn left(state: ElementState) -> WindowEvent {
    button(state, MouseButton::Left)
}

/// Create a drag from `from` through each point in `to`, followed by a release.
fn drag(from: [f32; 2], to: &[[f32; 2]]) -> Vec<WindowEvent> {
    let mut events = vec![moved(from), left(ElementState::Pressed)];
    events.extend(to.iter().map(|point| moved(*point)));
    events.push(left(ElementState::Released));
    events
}

/// Create a press and release at `point`.
fn click(point: [f32; 2]) -> Vec<WindowEvent> {
    vec![
        moved(point),
        left(ElementState::Pressed),
        left(ElementState::Released),
    ]
}

/// Create two consecutive clicks at `point` to select a word.
fn double_click(point: [f32; 2]) -> Vec<WindowEvent> {
    [click(point), click(point)].concat()
}

/// Send `events` and collect the last reported cursor and copy-text changes.
async fn send(app: &mut AvengerApp<State>, events: &[WindowEvent]) -> Changes {
    let mut gathered = Changes::default();
    for event in events {
        let changes = app.update(event, Instant::now()).await;
        gathered.cursor = changes.cursor.or(gathered.cursor);
        gathered.copy_text = changes.copy_text.or(gathered.copy_text);
    }
    gathered
}

/// Count highlight shapes in the selection scene.
fn shapes(app: &AvengerApp<State>) -> usize {
    fn count(marks: &[SceneMark]) -> usize {
        (marks.iter())
            .map(|mark| match mark {
                SceneMark::Group(group) => count(&group.marks),
                SceneMark::Path(path) => (path.path_iter().next().unwrap().iter())
                    .filter(|event| matches!(event, PathEvent::Begin { .. }))
                    .count(),
                _ => 0,
            })
            .sum()
    }
    count(&app.scenes().last().unwrap().marks)
}

fn heard(app: &AvengerApp<State>) -> Vec<&'static str> {
    std::mem::take(&mut *app.state().heard.lock().unwrap())
}

/// Dragging across a wrapped label selects multiple lines, draws highlights, and reports the
/// text cursor and selected text.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_selects_across_lines() {
    let mut app = app().await;
    let start = caret(TITLE, 10.0, 0);
    let hovered = send(&mut app, &[moved(start)]).await;
    assert_eq!(hovered.cursor, Some(CursorStyle::Text));
    let changes = send(&mut app, &drag(start, &[caret(TITLE, 10.0, TITLE.len())])).await;
    assert_eq!(changes.copy_text.as_deref(), Some(TITLE));
    assert_eq!(app.copy_text(), TITLE);
    assert_eq!(shapes(&app), 3);
}

/// Two consecutive presses select a word, and three select a line.
#[tokio::test(flavor = "current_thread")]
async fn presses_in_a_row_select_words_and_lines() {
    let mut app = app().await;
    let two = caret(TITLE, 10.0, "one t".len());
    send(&mut app, &double_click(two)).await;
    assert_eq!(app.copy_text(), "two");
    send(&mut app, &click(two)).await;
    assert_eq!(app.copy_text(), "two ");
}

/// Escape and a press outside all labels clear the selection. Moving off text resets the
/// cursor.
#[tokio::test(flavor = "current_thread")]
async fn escape_and_a_press_elsewhere_clear() {
    let mut app = app().await;
    let word = caret(TITLE, 10.0, 1);
    send(&mut app, &double_click(word)).await;
    assert_eq!(app.copy_text(), "one");
    let escape = WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Named(NamedKey::Escape),
        state: ElementState::Pressed,
    });
    let changes = send(&mut app, &[escape]).await;
    assert_eq!(changes.copy_text.as_deref(), Some(""));
    assert_eq!(shapes(&app), 0);

    send(&mut app, &double_click(word)).await;
    let away = send(&mut app, &click([150.0, 150.0])).await;
    assert_eq!(app.copy_text(), "");
    assert_eq!(away.cursor, Some(CursorStyle::Default));
}

/// The tool consumes presses on selectable text, including noninteractive text marks.
/// Right-button presses and presses outside text still reach the chart.
#[tokio::test(flavor = "current_thread")]
async fn presses_on_text_are_the_selections_alone() {
    let mut app = app().await;
    send(&mut app, &double_click(caret(TITLE, 10.0, 1))).await;
    assert_eq!(app.copy_text(), "one");
    let quiet = caret("quiet", 120.0, 1);
    let hovered = send(&mut app, &[moved(quiet)]).await;
    assert_eq!(hovered.cursor, None, "the cursor stays on text");
    send(&mut app, &double_click(quiet)).await;
    assert_eq!(app.copy_text(), "quiet");
    assert!(heard(&app).is_empty());

    let right = [
        button(ElementState::Pressed, MouseButton::Right),
        button(ElementState::Released, MouseButton::Right),
    ];
    send(&mut app, &right).await;
    assert_eq!(heard(&app), ["press", "click"]);
    send(&mut app, &click([150.0, 100.0])).await;
    assert_eq!(heard(&app), ["press", "click"]);
}

/// An interactive mark in the same scene or a higher scene blocks text selection beneath it,
/// and its scene receives the press.
#[tokio::test(flavor = "current_thread")]
async fn a_label_under_a_mark_that_takes_hits_doesnt_select() {
    let mut app = app_with(State {
        title: TITLE.to_string(),
        cover: true,
        ..Default::default()
    })
    .await;
    for point in [caret("hidden", 140.0, 1), caret(TITLE, 10.0, 1)] {
        let hovered = send(&mut app, &[moved(point)]).await;
        assert_ne!(app.cursor(), CursorStyle::Text, "{hovered:?}");
        send(&mut app, &double_click(point)).await;
        assert_eq!(app.copy_text(), "");
    }
    // The chart hears the presses on its hidden label, and the cover's scene stops those on the
    // title.
    assert_eq!(heard(&app), ["press", "click", "press", "double click"]);
}

/// Selection survives rebuilds with unchanged text and clears when the anchor source changes.
#[tokio::test(flavor = "current_thread")]
async fn a_selection_ends_when_its_text_changes() {
    let mut app = app().await;
    send(&mut app, &double_click(caret(TITLE, 10.0, 1))).await;
    app.waker().request(Refresh::Rebuild);
    app.refresh().await;
    assert_eq!(app.copy_text(), "one");

    app.state_mut().title = "uno dos".to_string();
    app.waker().request(Refresh::Rebuild);
    let changes = app.refresh().await;
    assert_eq!(app.copy_text(), "");
    assert_eq!(changes.copy_text.as_deref(), Some(""));
}

/// Dragging selects labels in drawing order, including intermediate labels in full.
/// Tests cover tab and newline separators, reverse drags, word selection, and endpoints outside
/// text.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_reads_across_labels_in_the_order_they_draw() {
    let mut app = app().await;
    let start = caret(TITLE, 10.0, 0);
    send(&mut app, &drag(start, &[caret("note", 80.0, 2)])).await;
    assert_eq!(app.copy_text(), "one two three\nno");

    let end = caret_at("side", SIDE, "side".len());
    send(&mut app, &drag(start, &[end])).await;
    assert_eq!(app.copy_text(), "one two three\nnote\tside");
    assert_eq!(shapes(&app), 5);
    // The caret at the end lies just past the label, so the press goes down just inside it.
    send(&mut app, &drag([end[0] - 1.0, end[1]], &[start])).await;
    assert_eq!(app.copy_text(), "one two three\nnote\tside");

    let two = caret(TITLE, 10.0, "one t".len());
    send(&mut app, &click(two)).await;
    send(&mut app, &drag(two, &[caret("note", 80.0, 1)])).await;
    assert_eq!(app.copy_text(), "two three\nnote");

    // Below the note's end, nearer it than "side" or the caption.
    send(&mut app, &drag(start, &[[40.0, 100.0]])).await;
    assert_eq!(app.copy_text(), "one two three\nnote");
}

/// A drag selects only labels in its starting scene.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_stays_in_its_scene() {
    let mut app = app().await;
    let upper = caret("upper note", 170.0, 1);
    send(&mut app, &drag(upper, &[caret(TITLE, 10.0, 0)])).await;
    assert_eq!(app.copy_text(), "u");
}

/// Create a single-line label containing `text` at `at`.
fn plain(text: &str, at: [f32; 2]) -> SceneTextMark {
    SceneTextMark {
        width: LabelWidth::Auto.into(),
        ..text_at("", text, at)
    }
}

/// Map a point in the label box to scene coordinates.
fn in_label(mark: &SceneTextMark, point: impl Fn(TextBounds) -> [f32; 2]) -> [f32; 2] {
    let label = mark.labels().next().unwrap();
    let bounds = bundled_label_engine().bounds(&label.label).unwrap();
    label.placement(&bounds, [0.0; 2]).to_scene(point(bounds))
}

/// Return text selected by a drag spanning all `marks` in a chart.
async fn copied(marks: Vec<SceneTextMark>) -> String {
    let start = in_label(&marks[0], |_| [1.0, 2.0]);
    let end = in_label(marks.last().unwrap(), |b| [b.width + 2.0, b.height - 2.0]);
    let mut app = app_with(State {
        placed: Some(marks),
        ..Default::default()
    })
    .await;
    send(&mut app, &drag(start, &[end])).await;
    app.copy_text().to_string()
}

/// A label turned near vertical requests a vertical text cursor, and a drag from it keeps that
/// cursor over upright text until the release.
#[tokio::test(flavor = "current_thread")]
async fn a_turned_label_requests_a_vertical_cursor() {
    let turned = SceneTextMark {
        angle: 90.0.into(),
        ..plain("turned", [30.0, 10.0])
    };
    let upright = plain("upright", [60.0, 100.0]);
    let middle = |b: TextBounds| [b.width / 2.0, b.height / 2.0];
    let (on_turned, on_upright) = (in_label(&turned, middle), in_label(&upright, middle));
    let mut app = app_with(State {
        placed: Some(vec![turned, upright]),
        ..Default::default()
    })
    .await;
    send(&mut app, &[moved(on_turned)]).await;
    assert_eq!(app.cursor(), CursorStyle::VerticalText);
    send(&mut app, &[moved(on_upright)]).await;
    assert_eq!(app.cursor(), CursorStyle::Text);

    let press = [
        moved(on_turned),
        left(ElementState::Pressed),
        moved(on_upright),
    ];
    send(&mut app, &press).await;
    assert_eq!(app.cursor(), CursorStyle::VerticalText);
    send(&mut app, &[left(ElementState::Released)]).await;
    assert_eq!(app.cursor(), CursorStyle::Text);
}

/// Tabs require alignment with the previous label's last selected line, measured in that
/// label's coordinates.
#[tokio::test(flavor = "current_thread")]
async fn a_tab_joins_a_label_that_goes_on_from_the_last_line() {
    let right_then_left = vec![plain("right", [100.0, 10.0]), plain("left", [10.0, 10.0])];
    assert_eq!(copied(right_then_left).await, "right\nleft");

    let lines = SceneTextMark {
        text_syntax: TextSyntaxMode::PlainLines,
        ..plain("one\ntwo", [10.0, 10.0])
    };
    let beside_first = vec![lines.clone(), plain("beside", [60.0, 10.0])];
    assert_eq!(copied(beside_first).await, "one\ntwo\nbeside");
    let label = lines.labels().next().unwrap();
    let layout = bundled_label_engine().text_layout(&label.label).unwrap();
    let second = layout.caret(TextPosition::new("one\n".len(), Affinity::Downstream));
    let second_top = in_label(&lines, |_| [0.0, second.y])[1];
    let beside_last = vec![lines.clone(), plain("after", [60.0, second_top])];
    assert_eq!(copied(beside_last).await, "one\ntwo\tafter");

    let turned = SceneTextMark {
        angle: 90.0.into(),
        ..plain("turned", [10.0, 10.0])
    };
    assert_eq!(
        copied(vec![turned, plain("upright", [30.0, 20.0])]).await,
        "turned\nupright"
    );
}
