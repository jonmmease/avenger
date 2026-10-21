//! Text selection selects a label's text under a drag, by position, word, or line, across the
//! label's lines, draws it in the overlay, and gives the app a text cursor and the selected text
//! to copy.

use std::sync::Arc;

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, Changes, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_common::types::TextBaseline;
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text_selection::TextSelection;
use avenger_typst_label::{bundled_label_engine, Affinity, LabelWidth, TextPosition};
use lyon_path::PathEvent;

const TITLE: &str = "one two three";

#[derive(Clone)]
struct State {
    title: String,
}

/// A wrapped title at (10, 10), a note at (10, 80), and a non-interactive caption at (10, 120).
struct Texts;

fn text(name: &str, text: &str, y: f32) -> SceneTextMark {
    SceneTextMark {
        name: name.into(),
        len: 1,
        text: text.to_string().into(),
        x: 10.0.into(),
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
    let label = text("", source, y);
    let label = label.labels().next().unwrap();
    let layout = bundled_label_engine().text_layout(&label.label).unwrap();
    let caret = layout.caret(TextPosition::new(offset, Affinity::Downstream));
    [10.0 + caret.x + 0.5, y + caret.y + caret.height / 2.0]
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
