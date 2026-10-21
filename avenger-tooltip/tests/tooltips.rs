//! Tooltips show the focus at the pointer in the app's overlay, and build the overlay alone:
//! never the scene or its hit-testing tree. With pinning, a click pins a focus, and a pinned or
//! hovered focus is found again when the app rebuilds the tree.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_tooltip::{
    FocusStrategy, GroupOrder, GroupTarget, InstanceTarget, TooltipAnchor, TooltipField, Tooltips,
};

#[derive(Clone, Default)]
struct State {
    builds: usize,
    /// How far right of their places the points lie.
    shift: f32,
    /// Whether the scene draws only the first point.
    one: bool,
    /// Whether a region with a row lies beneath the points, across the scene's top half.
    region: bool,
    /// Where a pan's drag started, less the shift then, while it lasts.
    drag_from: Option<f32>,
    /// How much the points' values exceed 1.
    raise: f64,
    /// Whether the points' rows come in reverse order, with the points still in their places.
    reverse: bool,
}

/// Pans the points along x with a drag, as a map's drag does: each move builds the scene and not
/// its tree, and the release builds both.
struct Pan;

#[async_trait]
impl EventStreamHandler<State> for Pan {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        match (event, state.drag_from) {
            (SceneGraphEvent::MouseDown(down), _) => {
                state.drag_from = Some(down.position[0] - state.shift);
                UpdateStatus::UNCHANGED
            }
            (SceneGraphEvent::CursorMoved(moved), Some(from)) => {
                state.shift = moved.position[0] - from;
                UpdateStatus::SCENE
            }
            (SceneGraphEvent::MouseUp(_), Some(_)) => {
                state.drag_from = None;
                UpdateStatus::SCENE_AND_TREE
            }
            _ => UpdateStatus::UNCHANGED,
        }
    }
}

/// Raises the points' values by 1 for each key press, building the scene and not its tree.
struct Raise;

#[async_trait]
impl EventStreamHandler<State> for Raise {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        state.raise += 1.0;
        UpdateStatus::SCENE
    }
}

/// Two points, at x = 20 and 80 past the state's shift and y = 50, with a row each, of an id
/// and a value. Each build builds new rows.
struct Points;

#[async_trait]
impl SceneGraphBuilder<State> for Points {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError> {
        state.builds += 1;
        let count = if state.one { 1 } else { 2 };
        let mut xs: Vec<f32> = [20.0, 80.0][..count]
            .iter()
            .map(|x| x + state.shift)
            .collect();
        let mut ids: Vec<i64> = [0, 1][..count].to_vec();
        if state.reverse {
            xs.reverse();
            ids.reverse();
        }
        let ids: ArrayRef = Arc::new(Int64Array::from(ids));
        let values: ArrayRef = Arc::new(Float64Array::from(vec![1.0 + state.raise; count]));
        let mut data = SceneData::new();
        data.insert(
            "points",
            Arc::new(RecordBatch::try_from_iter([("id", ids), ("value", values)]).unwrap()),
        );
        let points = SceneSymbolMark {
            name: "points".into(),
            len: count as u32,
            x: xs.into(),
            y: 50.0.into(),
            ..Default::default()
        };
        let mut marks: Vec<SceneMark> = Vec::new();
        if state.region {
            let values: ArrayRef = Arc::new(Float64Array::from(vec![2.0]));
            data.insert(
                "region",
                Arc::new(RecordBatch::try_from_iter([("value", values)]).unwrap()),
            );
            let region = SceneRectMark {
                name: "region".into(),
                width: Some(100.0.into()),
                height: Some(60.0.into()),
                ..Default::default()
            };
            marks.push(region.into());
        }
        marks.push(points.into());
        let scene = SceneGraph {
            marks,
            width: 100.0,
            height: 100.0,
            origin: [0.0; 2],
        };
        Ok(BuiltScene { scene, data })
    }
}

async fn app(tooltips: &Tooltips) -> AvengerApp<State> {
    app_with(tooltips, false).await
}

/// An app with `tooltips`, whose scene has a region beneath its points when `region` is set.
async fn app_with(tooltips: &Tooltips, region: bool) -> AvengerApp<State> {
    app_streams(tooltips, region, Vec::new()).await
}

/// An app with `tooltips` and `streams`, whose scene has a region beneath its points when `region`
/// is set.
async fn app_streams(
    tooltips: &Tooltips,
    region: bool,
    streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)>,
) -> AvengerApp<State> {
    let state = State {
        region,
        ..Default::default()
    };
    let engine = avenger_typst_label::bundled_label_engine().with_number_formatting(Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ));
    AvengerApp::try_new(state, Arc::new(Points), streams, engine)
        .await
        .unwrap()
        .with_overlay(Arc::new(tooltips.clone()))
        .unwrap()
}

fn moved(x: f32, y: f32) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: [x, y] })
}

fn button(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

const CLICK: [ElementState; 2] = [ElementState::Pressed, ElementState::Released];

fn escape() -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Named(NamedKey::Escape),
        state: ElementState::Pressed,
    })
}

/// Send `events`, and return the last new overlay's marks, if any: none when the tooltip hides.
async fn send(app: &mut AvengerApp<State>, events: &[WindowEvent]) -> Option<Vec<SceneMark>> {
    let mut marks = None;
    for event in events {
        let changes = app.update(event, Instant::now()).await.unwrap();
        if let Some(overlay) = changes.overlay {
            marks = Some(overlay.marks.clone());
        }
    }
    marks
}

/// The number of marks in the last new overlay, if any.
async fn sent(app: &mut AvengerApp<State>, events: &[WindowEvent]) -> Option<usize> {
    send(app, events).await.map(|marks| marks.len())
}

async fn click(app: &mut AvengerApp<State>) {
    send(app, &CLICK.map(button)).await;
}

/// The x of the focus's one member, or `None` without a focus.
fn focused_x(tooltips: &Tooltips) -> Option<f32> {
    Some(tooltips.focus()?.members[0].position[0])
}

/// The text of every text mark in `marks` and in their groups.
fn texts(marks: &[SceneMark]) -> Vec<String> {
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Group(group) => texts(&group.marks),
            SceneMark::Text(text) => text.text_iter().cloned().collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// A move to a new focus shows it, a move within it changes nothing, and a move away or a press
/// hides it. Each change builds the overlay, and none builds the scene.
#[tokio::test(flavor = "current_thread")]
async fn a_hover_builds_the_overlay_alone() {
    let tooltips = Tooltips::new();
    let mut app = app(&tooltips).await;
    let builds = app.app_state_mut().builds;

    let changes = app
        .update(&moved(22.0, 52.0), Instant::now())
        .await
        .unwrap();
    assert!(changes.scene.is_none());
    assert_eq!(changes.overlay.map(|overlay| overlay.marks.len()), Some(1));
    let changes = app
        .update(&moved(24.0, 52.0), Instant::now())
        .await
        .unwrap();
    assert!(changes.scene.is_none() && changes.overlay.is_none());

    assert_eq!(sent(&mut app, &[moved(50.0, 0.0)]).await, Some(0));
    sent(&mut app, &[moved(78.0, 52.0)]).await;
    let press = button(ElementState::Pressed);
    assert_eq!(sent(&mut app, &[press]).await, Some(0));
    // A drag shows nothing.
    assert_eq!(sent(&mut app, &[moved(80.0, 50.0)]).await, None);
    assert_eq!(app.app_state_mut().builds, builds);
}

/// A tooltip shows its target's fields: their titles, and their values in their formats.
#[tokio::test(flavor = "current_thread")]
async fn a_tooltip_shows_its_targets_fields() {
    let target = InstanceTarget::default().with_fields([TooltipField::new("value")
        .with_title("Value")
        .with_value_format(".2f")]);
    let tooltips = Tooltips::over([target]);
    let mut app = app(&tooltips).await;
    let marks = send(&mut app, &[moved(22.0, 52.0)]).await.unwrap();
    assert_eq!(texts(&marks), ["Value", "1.00"]);
}

/// A scene that the app builds without its tree shows new values in the tooltip.
#[tokio::test(flavor = "current_thread")]
async fn a_tooltip_shows_the_scenes_values() {
    let target = InstanceTarget::default()
        .with_fields([TooltipField::new("value").with_value_format(".2f")]);
    let tooltips = Tooltips::over([target]);
    let raise = EventStreamConfig {
        types: vec![SceneGraphEventType::KeyPress],
        ..Default::default()
    };
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
        vec![(raise, Arc::new(Raise))];
    let mut app = app_streams(&tooltips, false, streams).await;
    send(&mut app, &[moved(22.0, 52.0)]).await;
    let key = WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Character('r'),
        state: ElementState::Pressed,
    });
    let marks = send(&mut app, &[key]).await.unwrap();
    assert_eq!(texts(&marks), ["value", "2.00"]);
}

/// A tooltip at the pointer follows it within its focus, building the overlay alone, and
/// reports its anchor while it shows.
#[tokio::test(flavor = "current_thread")]
async fn a_tooltip_at_the_pointer_follows_it() {
    let pointer = InstanceTarget::default().with_anchor(TooltipAnchor::Pointer);
    let tooltips = Tooltips::over([pointer]);
    let mut app = app(&tooltips).await;
    let builds = app.app_state_mut().builds;
    // An outline of the point, and the tooltip.
    assert_eq!(sent(&mut app, &[moved(22.0, 52.0)]).await, Some(2));
    assert_eq!(sent(&mut app, &[moved(24.0, 52.0)]).await, Some(2));
    assert_eq!(tooltips.focus().unwrap().pointer, [24.0, 52.0]);
    assert_eq!(tooltips.anchor(), Some(TooltipAnchor::Pointer));
    assert_eq!(app.app_state_mut().builds, builds);
    sent(&mut app, &[moved(50.0, 0.0)]).await;
    assert_eq!(tooltips.anchor(), None);
}

/// A pinned tooltip takes the presses and clicks on its box from the scene's streams beneath: a
/// drag from it doesn't pan, and a click on it keeps it pinned, though it covers another focus.
/// The box shows the arrow, and a drag elsewhere still pans.
#[tokio::test(flavor = "current_thread")]
async fn a_pinned_tooltip_takes_presses_on_its_box() {
    // The pan's release builds new rows, which the key finds the pinned row in.
    let target = InstanceTarget::default().with_key("id");
    let tooltips = Tooltips::over([target]).with_pinning();
    let pan = EventStreamConfig {
        types: vec![
            SceneGraphEventType::MouseDown,
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseUp,
        ],
        ..Default::default()
    };
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
        vec![(pan, Arc::new(Pan))];
    let mut app = app_streams(&tooltips, false, streams)
        .await
        .with_cursor(|_: &State| CursorStyle::Grab);
    send(&mut app, &[moved(22.0, 52.0)]).await;
    click(&mut app).await;
    assert!(tooltips.pinned());
    // The tooltip beside the point at x = 20 covers the point at x = 80.
    send(&mut app, &[moved(78.0, 52.0)]).await;
    assert_eq!(app.cursor(), CursorStyle::Default);
    let drag = [
        button(ElementState::Pressed),
        moved(60.0, 52.0),
        button(ElementState::Released),
    ];
    send(&mut app, &drag).await;
    assert_eq!(app.app_state_mut().shift, 0.0);
    click(&mut app).await;
    assert!(tooltips.pinned());
    assert_eq!(focused_x(&tooltips), Some(20.0));

    send(&mut app, &[moved(10.0, 90.0)]).await;
    assert_eq!(app.cursor(), CursorStyle::Grab);
    let drag = [
        button(ElementState::Pressed),
        moved(20.0, 90.0),
        button(ElementState::Released),
    ];
    send(&mut app, &drag).await;
    assert_eq!(app.app_state_mut().shift, 10.0);
}

/// A click on a focus pins it through moves and the pointer's leaving, a click off every focus
/// unpins and hides it, and Escape hides a pinned tooltip. A press that drags pins nothing.
#[tokio::test(flavor = "current_thread")]
async fn a_click_pins_a_focus() {
    let tooltips = Tooltips::new().with_pinning();
    let mut app = app(&tooltips).await;
    send(&mut app, &[moved(22.0, 52.0)]).await;
    click(&mut app).await;
    assert!(tooltips.pinned());
    let away = [moved(50.0, 0.0), WindowEvent::CursorLeft, moved(80.0, 50.0)];
    send(&mut app, &away).await;
    assert_eq!(focused_x(&tooltips), Some(20.0), "the pinned focus stays");

    send(&mut app, &[moved(50.0, 0.0)]).await;
    click(&mut app).await;
    assert!(!tooltips.pinned());
    assert_eq!(focused_x(&tooltips), None);

    send(&mut app, &[moved(78.0, 50.0)]).await;
    click(&mut app).await;
    assert_eq!(sent(&mut app, &[escape()]).await, Some(0));
    assert!(!tooltips.pinned());

    let drag = [
        moved(22.0, 52.0),
        button(ElementState::Pressed),
        moved(60.0, 52.0),
        button(ElementState::Released),
    ];
    send(&mut app, &drag).await;
    assert!(!tooltips.pinned());
}

async fn rebuild(app: &mut AvengerApp<State>) {
    app.waker().request(Refresh::Rebuild);
    app.refresh().await.unwrap();
}

/// When the app rebuilds the tree, a hovered focus is found again at the pointer, and a pinned
/// focus by its key, until its row is gone.
#[tokio::test(flavor = "current_thread")]
async fn a_focus_is_found_again_after_the_tree_rebuilds() {
    let tooltips = Tooltips::over([InstanceTarget::default().with_key("id")]).with_pinning();
    let mut app = app(&tooltips).await;
    send(&mut app, &[moved(78.0, 50.0)]).await;
    app.app_state_mut().shift = -40.0;
    rebuild(&mut app).await;
    assert_eq!(
        focused_x(&tooltips),
        Some(40.0),
        "the point that now lies nearest"
    );

    send(&mut app, &[moved(40.0, 50.0)]).await;
    click(&mut app).await;
    app.app_state_mut().shift = 0.0;
    rebuild(&mut app).await;
    assert_eq!(
        focused_x(&tooltips),
        Some(80.0),
        "the pinned point, where it now lies"
    );

    app.app_state_mut().one = true;
    rebuild(&mut app).await;
    assert_eq!(focused_x(&tooltips), None);
    assert!(!tooltips.pinned());
}

/// Tooltips try their targets in order: a point within reach wins over the region beneath it,
/// and elsewhere in the region, the region's tooltip follows the pointer, over its outline.
#[tokio::test(flavor = "current_thread")]
async fn targets_are_tried_in_order() {
    let tooltips = Tooltips::over([
        InstanceTarget::new(FocusStrategy::Nearest { max_radius: 8.0 }).with_marks(["points"]),
        InstanceTarget::new(FocusStrategy::Exact)
            .with_marks(["region"])
            .with_anchor(TooltipAnchor::Pointer),
    ]);
    let mut app = app_with(&tooltips, true).await;
    send(&mut app, &[moved(24.0, 52.0)]).await;
    assert_eq!(tooltips.target(), Some(0));
    assert_eq!(focused_x(&tooltips), Some(20.0));
    send(&mut app, &[moved(50.0, 52.0)]).await;
    assert_eq!(tooltips.target(), Some(1));
    assert_eq!(sent(&mut app, &[moved(52.0, 40.0)]).await, Some(2));
    send(&mut app, &[moved(50.0, 80.0)]).await;
    assert_eq!(tooltips.target(), None);
}

/// The wheel hides the tooltip, and the move to where the pointer is that macOS reports before
/// each wheel step doesn't show it again, so a zoom doesn't flash it. A real move does.
#[tokio::test(flavor = "current_thread")]
async fn a_wheel_steps_move_shows_nothing() {
    let tooltips = Tooltips::new();
    let mut app = app(&tooltips).await;
    let wheel = WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
    });
    assert_eq!(sent(&mut app, &[moved(22.0, 52.0)]).await, Some(1));
    assert_eq!(sent(&mut app, std::slice::from_ref(&wheel)).await, Some(0));
    assert_eq!(sent(&mut app, &[moved(22.0, 52.0), wheel]).await, None);
    assert_eq!(focused_x(&tooltips), None);
    assert_eq!(sent(&mut app, &[moved(23.0, 52.0)]).await, Some(1));
}

/// The tooltip's group in an overlay's marks.
fn tooltip_origin(marks: &[SceneMark]) -> [f32; 2] {
    marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == "tooltip" => Some(group.origin),
            _ => None,
        })
        .expect("the overlay has a tooltip")
}

/// A pinned tooltip moves with its data while a drag pans it, though the drag builds the scene
/// and not its tree, and stays with its data when the release builds the tree, beside its data
/// or beside the pointer.
#[tokio::test(flavor = "current_thread")]
async fn a_pinned_tooltip_moves_with_a_drag() {
    for anchor in [TooltipAnchor::Data, TooltipAnchor::Pointer] {
        let target = InstanceTarget::default().with_anchor(anchor).with_key("id");
        let tooltips = Tooltips::over([target]).with_pinning();
        let pan = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseUp,
            ],
            ..Default::default()
        };
        let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
            vec![(pan, Arc::new(Pan))];
        let mut app = app_streams(&tooltips, false, streams).await;
        send(&mut app, &[moved(22.0, 52.0)]).await;
        let pinned = send(&mut app, &CLICK.map(button)).await.unwrap();
        assert!(tooltips.pinned());
        let before = tooltip_origin(&pinned);

        send(&mut app, &[button(ElementState::Pressed)]).await;
        let dragged = send(&mut app, &[moved(32.0, 52.0)]).await.unwrap();
        assert_eq!(
            tooltip_origin(&dragged),
            [before[0] + 10.0, before[1]],
            "{anchor:?}"
        );

        let released = send(&mut app, &[button(ElementState::Released)])
            .await
            .unwrap();
        assert_eq!(
            tooltip_origin(&released),
            [before[0] + 10.0, before[1]],
            "{anchor:?}"
        );
        assert_eq!(focused_x(&tooltips), Some(30.0));
    }
}

/// A group's rows come in its entries' order, or down the scene.
#[tokio::test(flavor = "current_thread")]
async fn a_groups_rows_come_in_its_order() {
    let names = |tooltips: &Tooltips| -> Vec<String> {
        let focus = tooltips.focus().expect("a focus");
        focus
            .members
            .iter()
            .map(|m| m.mark().name.clone())
            .collect()
    };
    let group = GroupTarget::new(["points", "region"], ["Points", "Region"]).unwrap();
    let orders = [
        (group.clone(), ["points", "region"]),
        (group.with_order(GroupOrder::Position), ["region", "points"]),
    ];
    for (target, expected) in orders {
        let tooltips = Tooltips::over([target]);
        let mut app = app_with(&tooltips, true).await;
        send(&mut app, &[moved(22.0, 52.0)]).await;
        assert_eq!(names(&tooltips), expected);
    }
}

/// A pinned focus follows its key when its rows change order, and without a key, it hides when
/// its rows change, rather than show another row at its index.
#[tokio::test(flavor = "current_thread")]
async fn a_pinned_focus_keeps_its_data() {
    for key in [Some("id"), None] {
        let target = InstanceTarget::default();
        let target = match key {
            Some(key) => target.with_key(key),
            None => target,
        };
        let tooltips = Tooltips::over([target]).with_pinning();
        let mut app = app(&tooltips).await;
        send(&mut app, &[moved(22.0, 52.0)]).await;
        click(&mut app).await;
        app.app_state_mut().reverse = true;
        rebuild(&mut app).await;
        let focus = tooltips.focus();
        let member = focus.as_ref().map(|focus| &focus.members[0]);
        match key {
            Some(_) => {
                let member = member.expect("the pinned point");
                assert_eq!(member.position[0], 20.0);
                assert_eq!(member.mark().instance_index, Some(1));
            }
            None => {
                assert!(member.is_none());
                assert!(!tooltips.pinned());
            }
        }
    }
}
