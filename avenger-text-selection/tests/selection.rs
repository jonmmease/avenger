//! Text selection selects a label's text under a drag, by position, word, or line, across the
//! label's lines, in the scene or in an overlay beneath, draws it in the overlay, and gives the
//! app a text cursor and the selected text to copy. A drag reads across labels in the order they
//! draw. Presses on an overlay's text are its alone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, Changes, OverlayBuilder, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_common::types::{TextBaseline, TextSyntaxMode};
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text_selection::TextSelection;
use avenger_typst_label::{
    bundled_label_engine, Affinity, LabelEngine, LabelWidth, TextBounds, TextPosition,
};
use lyon_path::PathEvent;

const TITLE: &str = "one two three";

#[derive(Clone)]
struct State {
    title: String,
}

/// A wrapped title at (10, 10), a note at (10, 80) with "side" beside it at (60, 80), and a
/// non-interactive caption at (10, 120).
struct Texts;

/// The x of the labels other than "side".
const LEFT: f32 = 10.0;
const SIDE: [f32; 2] = [60.0, 80.0];

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

#[async_trait]
impl SceneGraphBuilder<State> for Texts {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError> {
        let quiet = SceneTextMark {
            interactive: Some(false),
            ..text("caption", "quiet", 120.0)
        };
        let marks: Vec<SceneMark> = vec![
            text("title", &state.title, 10.0).into(),
            text("note", "note", 80.0).into(),
            text_at("side", "side", SIDE).into(),
            quiet.into(),
        ];
        Ok(SceneGraph {
            marks,
            width: 200.0,
            height: 200.0,
            origin: [0.0; 2],
        }
        .into())
    }
}

async fn app(selection: &TextSelection) -> AvengerApp<State> {
    let state = State {
        title: TITLE.to_string(),
    };
    AvengerApp::try_new(state, Arc::new(Texts), Vec::new(), bundled_label_engine())
        .await
        .unwrap()
        .with_overlay(Arc::new(selection.clone()))
        .unwrap()
}

/// The scene point at the caret of `offset` in a label with `source` at (10, `y`).
fn caret(source: &str, y: f32, offset: usize) -> [f32; 2] {
    caret_at(source, [LEFT, y], offset)
}

/// The scene point at the caret of `offset` in a label with `source` at `[x, y]`.
fn caret_at(source: &str, [x, y]: [f32; 2], offset: usize) -> [f32; 2] {
    let label = text_at("", source, [x, y]);
    let label = label.labels().next().unwrap();
    let layout = bundled_label_engine().text_layout(&label.label).unwrap();
    let caret = layout.caret(TextPosition::new(offset, Affinity::Downstream));
    [x + caret.x + 0.5, y + caret.y + caret.height / 2.0]
}

/// A press at `from`, a drag to each of `to`, and the release.
fn drag(from: [f32; 2], to: &[[f32; 2]]) -> Vec<WindowEvent> {
    let mut events = vec![moved(from), button(ElementState::Pressed)];
    events.extend(to.iter().map(|point| moved(*point)));
    events.push(button(ElementState::Released));
    events
}

fn moved(point: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: point })
}

fn button(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

/// Send `events`, and gather their changes: the last cursor, copy text, and overlay of each.
async fn send(app: &mut AvengerApp<State>, events: &[WindowEvent]) -> Changes {
    let mut gathered = Changes::default();
    for event in events {
        let changes = app.update(event, Instant::now()).await.unwrap();
        gathered.cursor = changes.cursor.or(gathered.cursor);
        gathered.copy_text = changes.copy_text.or(gathered.copy_text);
        gathered.overlay = changes.overlay.or(gathered.overlay);
    }
    gathered
}

/// A press at `point`, released there.
fn click(point: [f32; 2]) -> [WindowEvent; 3] {
    [
        moved(point),
        button(ElementState::Pressed),
        button(ElementState::Released),
    ]
}

/// The number of shapes that the overlay's selection draws.
fn shapes(marks: &[SceneMark]) -> usize {
    marks
        .iter()
        .map(|mark| match mark {
            SceneMark::Group(group) => shapes(&group.marks),
            SceneMark::Path(path) => (path.path_iter().next().unwrap().iter())
                .filter(|event| matches!(event, PathEvent::Begin { .. }))
                .count(),
            _ => 0,
        })
        .sum()
}

/// A drag across a wrapped label selects across its lines, with a shape on each, and the
/// selection gives the app a text cursor and its text to copy.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_selects_across_lines() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let start = caret(TITLE, 10.0, 0);
    let hovered = send(&mut app, &[moved(start)]).await;
    assert_eq!(hovered.cursor, Some(CursorStyle::Text));
    let end = caret(TITLE, 10.0, TITLE.len());
    let events = [
        button(ElementState::Pressed),
        moved(end),
        button(ElementState::Released),
    ];
    let changes = send(&mut app, &events).await;
    assert_eq!(changes.copy_text.as_deref(), Some(TITLE));
    assert_eq!(app.copy_text(), TITLE);
    assert_eq!(shapes(&app.overlay().marks), 3);
}

/// A second press in a row selects a word, and a third the line.
#[tokio::test(flavor = "current_thread")]
async fn presses_in_a_row_select_words_and_lines() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let two = caret(TITLE, 10.0, "one t".len());
    send(&mut app, &[click(two), click(two)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("two"));
    send(&mut app, &click(two)).await;
    assert_eq!(selection.text().as_deref(), Some("two "));
}

/// Escape and a press off every label clear the selection, and the cursor leaves text.
#[tokio::test(flavor = "current_thread")]
async fn escape_and_a_press_elsewhere_clear() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let word = caret(TITLE, 10.0, 1);
    send(&mut app, &[click(word), click(word)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("one"));
    let escape = WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Named(NamedKey::Escape),
        state: ElementState::Pressed,
    });
    let changes = send(&mut app, &[escape]).await;
    assert_eq!(changes.copy_text.as_deref(), Some(""));
    assert_eq!(shapes(&app.overlay().marks), 0);

    send(&mut app, &[click(word), click(word)].concat()).await;
    let away = send(&mut app, &click([150.0, 150.0])).await;
    assert_eq!(selection.text(), None);
    assert_eq!(away.cursor, Some(CursorStyle::Default));
}

/// A label selects whether its text mark takes hits or not, so selection needs nothing of the
/// chart.
#[tokio::test(flavor = "current_thread")]
async fn a_label_selects_whether_it_takes_hits_or_not() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let quiet = caret("quiet", 120.0, 1);
    let hovered = send(&mut app, &[moved(quiet)]).await;
    assert_eq!(hovered.cursor, Some(CursorStyle::Text));
    send(&mut app, &[click(quiet), click(quiet)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("quiet"));
}

/// A selection lasts while its label shows the same text, through rebuilds, and ends when the
/// label's text changes.
#[tokio::test(flavor = "current_thread")]
async fn a_selection_ends_when_its_text_changes() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let word = caret(TITLE, 10.0, 1);
    send(&mut app, &[click(word), click(word)].concat()).await;
    app.waker().request(Refresh::Rebuild);
    app.refresh().await.unwrap();
    assert_eq!(selection.text().as_deref(), Some("one"));

    app.app_state_mut().title = "uno dos".to_string();
    app.waker().request(Refresh::Rebuild);
    let changes = app.refresh().await.unwrap();
    assert_eq!(selection.text(), None);
    assert_eq!(changes.copy_text.as_deref(), Some(""));
}

/// An overlay that draws a note at (10, 160) while it's shown, as a pinned tooltip draws its
/// text.
struct Note(Arc<AtomicBool>);

impl OverlayBuilder<State> for Note {
    fn build(
        &self,
        _: &State,
        _: &Arc<SceneGraph>,
        _: &[SceneMark],
        _: &SceneData,
        _: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let shown = self.0.load(Ordering::Relaxed);
        Ok(match shown {
            true => vec![text("note", "pinned note", 160.0).into()],
            false => Vec::new(),
        })
    }
}

/// The text of an overlay added before the selection, such as a pinned tooltip's, selects, and
/// the selection ends when the overlay no longer draws it.
#[tokio::test(flavor = "current_thread")]
async fn an_overlays_text_beneath_selects() {
    let shown = Arc::new(AtomicBool::new(true));
    let selection = TextSelection::new();
    let state = State {
        title: TITLE.to_string(),
    };
    let mut app = AvengerApp::try_new(state, Arc::new(Texts), Vec::new(), bundled_label_engine())
        .await
        .unwrap()
        .with_overlay(Arc::new(Note(shown.clone())))
        .unwrap()
        .with_overlay(Arc::new(selection.clone()))
        .unwrap();
    let pinned = caret("pinned note", 160.0, 1);
    let hovered = send(&mut app, &[moved(pinned)]).await;
    assert_eq!(hovered.cursor, Some(CursorStyle::Text));
    send(&mut app, &[click(pinned), click(pinned)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("pinned"));

    shown.store(false, Ordering::Relaxed);
    app.waker().request(Refresh::Rebuild);
    app.refresh().await.unwrap();
    assert_eq!(selection.text(), None);
}

/// An overlay that draws a box over the scene's title, as a pinned tooltip's box can.
struct Cover;

impl OverlayBuilder<State> for Cover {
    fn build(
        &self,
        _: &State,
        _: &Arc<SceneGraph>,
        _: &[SceneMark],
        _: &SceneData,
        _: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let cover = SceneRectMark {
            len: 1,
            x: 0.0.into(),
            y: 0.0.into(),
            width: Some(100.0.into()),
            height: Some(60.0.into()),
            ..Default::default()
        };
        Ok(vec![cover.into()])
    }
}

/// The scene's text beneath an overlay's box doesn't select, and calls for no text cursor, while
/// the scene's text elsewhere still does.
#[tokio::test(flavor = "current_thread")]
async fn scene_text_beneath_an_overlay_doesnt_select() {
    let selection = TextSelection::new();
    let state = State {
        title: TITLE.to_string(),
    };
    let mut app = AvengerApp::try_new(state, Arc::new(Texts), Vec::new(), bundled_label_engine())
        .await
        .unwrap()
        .with_overlay(Arc::new(Cover))
        .unwrap()
        .with_overlay(Arc::new(selection.clone()))
        .unwrap();
    let title = caret(TITLE, 10.0, 1);
    let hovered = send(&mut app, &[moved(title)]).await;
    assert_ne!(hovered.cursor, Some(CursorStyle::Text));
    send(&mut app, &[click(title), click(title)].concat()).await;
    assert_eq!(selection.text(), None);

    let note = caret("note", 80.0, 1);
    send(&mut app, &[click(note), click(note)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("note"));
}

/// A stream of the scene's that records each press and click that it hears, as a pan's would.
struct Heard(Arc<Mutex<Vec<&'static str>>>);

#[async_trait]
impl EventStreamHandler<State> for Heard {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let heard = match event {
            SceneGraphEvent::MouseDown(_) => "press",
            SceneGraphEvent::Click(_) => "click",
            SceneGraphEvent::DoubleClick(_) => "double click",
            _ => return UpdateStatus::UNCHANGED,
        };
        self.0.lock().unwrap().push(heard);
        UpdateStatus::UNCHANGED
    }
}

/// Presses and clicks on an overlay's text, which lies over the scene, are the selection's alone,
/// so the scene's streams beneath never hear them, and presses on the scene's text still reach
/// them.
#[tokio::test(flavor = "current_thread")]
async fn presses_on_an_overlays_text_stay_with_the_selection() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let presses = EventStreamConfig {
        types: vec![
            SceneGraphEventType::MouseDown,
            SceneGraphEventType::Click,
            SceneGraphEventType::DoubleClick,
        ],
        ..Default::default()
    };
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
        vec![(presses, Arc::new(Heard(heard.clone())))];
    let selection = TextSelection::new();
    let state = State {
        title: TITLE.to_string(),
    };
    let mut app = AvengerApp::try_new(state, Arc::new(Texts), streams, bundled_label_engine())
        .await
        .unwrap()
        .with_overlay(Arc::new(Note(Arc::new(AtomicBool::new(true)))))
        .unwrap()
        .with_overlay(Arc::new(selection.clone()))
        .unwrap();
    let pinned = caret("pinned note", 160.0, 1);
    send(&mut app, &[click(pinned), click(pinned)].concat()).await;
    assert_eq!(selection.text().as_deref(), Some("pinned"));
    assert!(heard.lock().unwrap().is_empty());

    // A right click, which selects nothing, still reaches the scene's streams.
    let right = |state| {
        WindowEvent::MouseInput(WindowMouseInput {
            state,
            button: MouseButton::Right,
        })
    };
    let title = caret(TITLE, 10.0, 1);
    let right_click = [
        moved(title),
        right(ElementState::Pressed),
        right(ElementState::Released),
    ];
    send(&mut app, &right_click).await;
    assert_eq!(*heard.lock().unwrap(), ["press", "click"]);
    heard.lock().unwrap().clear();

    send(&mut app, &click(title)).await;
    assert_eq!(*heard.lock().unwrap(), ["press", "click"]);
}

/// A drag reads across labels in the order they draw, the labels between its ends whole: a
/// newline joins labels on different lines, and a tab those on one line. A drag back selects
/// the same, and a drag after a second press reads by word at each end.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_reads_across_labels_in_the_order_they_draw() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    let start = caret(TITLE, 10.0, 0);
    send(&mut app, &drag(start, &[caret("note", 80.0, 2)])).await;
    assert_eq!(selection.text().as_deref(), Some("one two three\nno"));

    let end = caret_at("side", SIDE, "side".len());
    send(&mut app, &drag(start, &[end])).await;
    assert_eq!(
        selection.text().as_deref(),
        Some("one two three\nnote\tside")
    );
    assert_eq!(shapes(&app.overlay().marks), 5);
    // The caret at the end lies just past the label, so the press goes down just inside it.
    send(&mut app, &drag([end[0] - 1.0, end[1]], &[start])).await;
    assert_eq!(
        selection.text().as_deref(),
        Some("one two three\nnote\tside")
    );

    let two = caret(TITLE, 10.0, "one t".len());
    send(&mut app, &click(two)).await;
    send(&mut app, &drag(two, &[caret("note", 80.0, 1)])).await;
    assert_eq!(selection.text().as_deref(), Some("two three\nnote"));
}

/// Off text, a drag's end falls in the nearest label.
#[tokio::test(flavor = "current_thread")]
async fn off_text_a_drag_ends_in_the_nearest_label() {
    let selection = TextSelection::new();
    let mut app = app(&selection).await;
    // Below the note's end, nearer it than "side" or the caption.
    send(&mut app, &drag(caret(TITLE, 10.0, 0), &[[40.0, 100.0]])).await;
    assert_eq!(selection.text().as_deref(), Some("one two three\nnote"));
}

/// A drag from an overlay's text stays among the overlays' labels, though it reaches the scene's.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_stays_in_its_layer() {
    let selection = TextSelection::new();
    let state = State {
        title: TITLE.to_string(),
    };
    let mut app = AvengerApp::try_new(state, Arc::new(Texts), Vec::new(), bundled_label_engine())
        .await
        .unwrap()
        .with_overlay(Arc::new(Note(Arc::new(AtomicBool::new(true)))))
        .unwrap()
        .with_overlay(Arc::new(selection.clone()))
        .unwrap();
    let pinned = caret("pinned note", 160.0, 1);
    send(&mut app, &drag(pinned, &[caret(TITLE, 10.0, 0)])).await;
    assert_eq!(selection.text().as_deref(), Some("p"));
}

/// A scene of text marks, drawn in the order given.
struct Placed(Vec<SceneTextMark>);

#[async_trait]
impl SceneGraphBuilder<State> for Placed {
    async fn build(&self, _: &mut State) -> Result<BuiltScene, AvengerAppError> {
        Ok(SceneGraph {
            marks: self.0.iter().cloned().map(SceneMark::from).collect(),
            width: 200.0,
            height: 200.0,
            origin: [0.0; 2],
        }
        .into())
    }
}

/// A one-line label of `text` at `at`.
fn plain(text: &str, at: [f32; 2]) -> SceneTextMark {
    SceneTextMark {
        width: LabelWidth::Auto.into(),
        ..text_at("", text, at)
    }
}

/// The point in the scene at `point` in the box of `mark`'s label.
fn in_label(mark: &SceneTextMark, point: impl Fn(TextBounds) -> [f32; 2]) -> [f32; 2] {
    let label = mark.labels().next().unwrap();
    let bounds = bundled_label_engine().bounds(&label.label).unwrap();
    label.placement(&bounds, [0.0; 2]).to_scene(point(bounds))
}

/// The text that a drag copies from the start of the first of `marks` to past the end of the
/// last.
async fn copied(marks: Vec<SceneTextMark>) -> String {
    let start = in_label(&marks[0], |_| [1.0, 2.0]);
    let end = in_label(marks.last().unwrap(), |b| [b.width + 2.0, b.height - 2.0]);
    let selection = TextSelection::new();
    let state = State {
        title: TITLE.to_string(),
    };
    let mut app = AvengerApp::try_new(
        state,
        Arc::new(Placed(marks)),
        Vec::new(),
        bundled_label_engine(),
    )
    .await
    .unwrap()
    .with_overlay(Arc::new(selection.clone()))
    .unwrap();
    send(&mut app, &drag(start, &[end])).await;
    selection.text().unwrap_or_default()
}

/// A tab joins two labels only where the second's first selected line goes on from the first's
/// last, across its middle and to its right as the first reads: not for a label to the left at
/// the same height, not beside a line that isn't the last, and not across a turn.
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
