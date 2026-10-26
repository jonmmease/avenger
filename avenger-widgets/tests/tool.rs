//! The Widgets tool's routing, press capture, focus and redrawing, with probe widgets that
//! record what they receive, over a chart scene that records what it hears.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{Disposition, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_layout::{Direction, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{bundled_label_engine, LabelError};
use avenger_widgets::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent, Widgets};

const CHART: &str = "chart";
const CONTROLS: &str = "controls";

#[derive(Default)]
struct State {
    /// Each probe's rectangle, by id. A probe without one is absent.
    rects: HashMap<String, Rect>,
    /// What each probe shows, by id. A negative value fails to draw.
    values: HashMap<String, i32>,
    disabled: HashSet<String>,
    /// What the chart's stream heard.
    heard: Arc<Mutex<Vec<String>>>,
}

/// What a probe received, how many times it drew, and the look it last drew with.
#[derive(Default)]
struct Log {
    events: Vec<String>,
    draws: usize,
    look: Look,
}

/// A widget that records its events and draws, consumes the K key, and shows a value from app
/// state.
struct Probe {
    id: String,
    log: Arc<Mutex<Log>>,
    focusable: bool,
}

impl Probe {
    fn new(id: &str) -> (Self, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        let probe = Self {
            id: id.to_string(),
            log: log.clone(),
            focusable: true,
        };
        (probe, log)
    }
}

/// A short name for `event`, for the logs.
fn name(event: &SceneGraphEvent) -> String {
    match event {
        SceneGraphEvent::MouseDown(_) => "press".into(),
        SceneGraphEvent::MouseUp(_) => "release".into(),
        SceneGraphEvent::Click(_) => "click".into(),
        SceneGraphEvent::CursorMoved(_) => "move".into(),
        SceneGraphEvent::KeyPress(press) => format!("key {:?}", press.key),
        SceneGraphEvent::KeyRelease(release) => format!("key up {:?}", release.key),
        other => format!("{other:?}"),
    }
}

impl Widget<State> for Probe {
    type State = ();
    type View = i32;

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, _direction: Direction, _cx: &WidgetContext<'_>) -> f32 {
        20.0
    }

    fn view(&self, app: &State, _own: &()) -> i32 {
        app.values.get(&self.id).copied().unwrap_or(0)
    }

    fn draw(
        &self,
        value: &i32,
        placed: &Placed,
        _cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let mut log = self.log.lock().unwrap();
        log.draws += 1;
        log.look = placed.look;
        if *value < 0 {
            return Err(LabelError::InvalidWidth { width: -1.0 }.into());
        }
        let r = placed.rect;
        let mark = SceneRectMark {
            x: r.x.into(),
            y: r.y.into(),
            width: Some(r.width.into()),
            height: Some(r.height.into()),
            ..Default::default()
        };
        Ok(vec![mark.into()])
    }

    fn enabled(&self, app: &State) -> bool {
        !app.disabled.contains(&self.id)
    }

    fn focusable(&self) -> bool {
        self.focusable
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &(),
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(press) if press.key == Key::Character('k') => {
                Disposition::Consume
            }
            SceneGraphEvent::KeyPress(_) | SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        _placed: &Placed,
        _cx: &WidgetContext<'_>,
        _app: &mut State,
        _own: &mut (),
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => name(event),
            WidgetEvent::Focus => "focus".into(),
            WidgetEvent::Blur => "blur".into(),
            WidgetEvent::Cancel => "cancel".into(),
        };
        self.log.lock().unwrap().events.push(event);
        Rebuild::none()
    }

    fn cursor(&self, _look: Look, _own: &()) -> Option<CursorIcon> {
        Some(CursorIcon::Pointer)
    }
}

/// Draws a chart that covers the window, with a stream that records presses, releases, moves and
/// keys. The R key redraws the chart, and the V key changes probe A's value.
struct Chart;

#[async_trait]
impl AppBuild<State> for Chart {
    fn scenes(&self) -> Vec<Scene<State>> {
        let heard = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::MouseUp,
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::KeyPress,
            ],
            ..Default::default()
        };
        vec![Scene::new(CHART, 0).stream(heard, Heard)]
    }

    async fn build(
        &self,
        _state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(CHART) {
            let background = SceneRectMark {
                width: Some(200.0.into()),
                height: Some(200.0.into()),
                ..Default::default()
            };
            let scene = SceneGraph {
                marks: vec![background.into()],
                width: 200.0,
                height: 200.0,
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
            match press.key {
                Key::Character('r') => return Rebuild::scene(CHART),
                Key::Character('v') => {
                    *state.values.entry("a".to_string()).or_default() += 1;
                    return Rebuild::scene(CHART);
                }
                _ => {}
            }
        }
        state.heard.lock().unwrap().push(name(event));
        Rebuild::none()
    }
}

/// Probes A, B and C stacked down the left of a 200 by 200 window, from y = 10, 40 and 70.
struct Fixture {
    app: AvengerApp<State>,
    logs: HashMap<&'static str, Arc<Mutex<Log>>>,
}

const A: [f32; 2] = [20.0, 20.0];
const B: [f32; 2] = [20.0, 50.0];
const C: [f32; 2] = [20.0, 80.0];
const OUTSIDE: [f32; 2] = [150.0, 150.0];

async fn fixture_with(state: State, unfocusable: &[&str]) -> Result<Fixture, AvengerAppError> {
    let mut widgets = Widgets::new(CONTROLS, 10, |s: &State, id| s.rects.get(id).copied());
    let mut logs = HashMap::new();
    for id in ["a", "b", "c"] {
        let (mut probe, log) = Probe::new(id);
        probe.focusable = !unfocusable.contains(&id);
        widgets = widgets.with(probe);
        logs.insert(id, log);
    }
    let app = AvengerApp::builder(state, Chart, bundled_label_engine())
        .size([200.0, 200.0])
        .tool(widgets)
        .build()
        .await?;
    Ok(Fixture { app, logs })
}

fn placed() -> State {
    let mut state = State::default();
    for (id, y) in [("a", 10.0), ("b", 40.0), ("c", 70.0)] {
        state
            .rects
            .insert(id.to_string(), Rect::new(10.0, y, 50.0, 20.0));
    }
    state
}

async fn fixture() -> Fixture {
    fixture_with(placed(), &[]).await.unwrap()
}

impl Fixture {
    async fn send(&mut self, events: impl IntoIterator<Item = WindowEvent>) -> Vec<usize> {
        let mut scenes = Vec::new();
        for event in events {
            scenes.extend(self.app.update(&event, Instant::now()).await.scenes);
        }
        scenes
    }

    /// Take the events that probe `id` received.
    fn events(&self, id: &str) -> Vec<String> {
        std::mem::take(&mut self.logs[id].lock().unwrap().events)
    }

    fn draws(&self, id: &str) -> usize {
        self.logs[id].lock().unwrap().draws
    }

    /// Take what the chart heard.
    fn heard(&mut self) -> Vec<String> {
        std::mem::take(&mut self.app.state_mut().heard.lock().unwrap())
    }

    /// Clear every log.
    fn forget(&mut self) {
        for id in ["a", "b", "c"] {
            self.events(id);
        }
        self.heard();
    }

    /// The look that probe `id` last drew with.
    fn look(&self, id: &str) -> Look {
        self.logs[id].lock().unwrap().look
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

fn press_at(at: [f32; 2]) -> Vec<WindowEvent> {
    vec![moved(at), left(ElementState::Pressed)]
}

fn release() -> WindowEvent {
    left(ElementState::Released)
}

fn key(key: Key, state: ElementState) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput { key, state })
}

/// A press and release of `key`.
fn tap(pressed: Key) -> Vec<WindowEvent> {
    vec![
        key(pressed, ElementState::Pressed),
        key(pressed, ElementState::Released),
    ]
}

const TAB: Key = Key::Named(NamedKey::Tab);
const SHIFT: Key = Key::Named(NamedKey::Shift);

#[test]
fn a_press_on_a_probe_reaches_only_the_probe() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(A)).await;
        f.send([moved([30.0, 22.0]), release()]).await;
        // The manager sends a release's click before the release.
        let events = ["move", "focus", "press", "move", "click", "release"];
        assert_eq!(f.events("a"), events);
        assert!(f.heard().is_empty());

        f.send(press_at(OUTSIDE)).await;
        f.send([release()]).await;
        assert_eq!(f.heard(), ["move", "press", "release"]);
    });
}

#[test]
fn a_probe_keeps_its_drag_outside_it() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(A)).await;
        f.forget();
        f.send([moved(B), moved(OUTSIDE), release()]).await;
        assert_eq!(f.events("a"), ["move", "move", "release"]);
        assert!(f.events("b").is_empty());
        assert!(!f.look("b").hovered);
        assert!(f.heard().is_empty());
    });
}

#[test]
fn a_chart_press_moving_over_a_probe_leaves_it_alone() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(OUTSIDE)).await;
        f.forget();
        f.send([moved(A)]).await;
        assert!(!f.look("a").hovered);
        assert_eq!(f.app.cursor(), CursorIcon::Default);
        f.send([release()]).await;
        assert!(f.events("a").is_empty());
        assert_eq!(f.heard(), ["move", "release"]);
    });
}

#[test]
fn keys_reach_the_focused_probe_and_the_rest_reach_the_chart() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(B)).await;
        f.send([release()]).await;
        f.forget();
        f.send(tap(Key::Character('k'))).await;
        f.send(tap(Key::Character('x'))).await;
        assert_eq!(f.events("b"), ["key Character('k')"]);
        assert_eq!(f.heard(), ["key Character('x')"]);
    });
}

#[test]
fn tab_visits_the_stops_in_order() {
    pollster::block_on(async {
        let mut state = placed();
        state.disabled.insert("b".to_string());
        let mut f = fixture_with(state, &[]).await.unwrap();
        f.send(tap(TAB)).await;
        assert_eq!(f.events("a"), ["focus"]);
        f.send(tap(TAB)).await;
        assert_eq!(f.events("a"), ["blur"]);
        assert!(f.events("b").is_empty());
        assert_eq!(f.events("c"), ["focus"]);
        assert!(f.heard().is_empty());

        // Past the last stop, focus clears and the Tab goes on to the chart.
        f.send(tap(TAB)).await;
        assert_eq!(f.events("c"), ["blur"]);
        assert_eq!(f.heard(), ["key Named(Tab)"]);

        f.send([key(SHIFT, ElementState::Pressed)]).await;
        f.send(tap(TAB)).await;
        assert_eq!(f.events("c"), ["focus"]);
    });
}

#[test]
fn tab_skips_unplaced_and_unfocusable_probes() {
    pollster::block_on(async {
        let mut state = placed();
        state.rects.remove("a");
        let mut f = fixture_with(state, &["b"]).await.unwrap();
        f.send(tap(TAB)).await;
        assert!(f.events("a").is_empty());
        assert!(f.events("b").is_empty());
        assert_eq!(f.events("c"), ["focus"]);
    });
}

#[test]
fn only_keyboard_focus_is_visible() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(A)).await;
        f.send([release()]).await;
        assert!(f.look("a").focused);
        assert!(!f.look("a").focus_visible);
        f.send(tap(TAB)).await;
        assert!(f.look("b").focus_visible);
    });
}

#[test]
fn a_press_outside_the_probes_blurs_and_reaches_the_chart() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(A)).await;
        f.send([release()]).await;
        f.forget();
        f.send(press_at(OUTSIDE)).await;
        assert_eq!(f.events("a"), ["blur"]);
        assert_eq!(f.heard(), ["move", "press"]);
    });
}

#[test]
fn a_probe_that_disappears_gets_cancel_and_blur_with_the_next_event() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send(press_at(A)).await;
        f.forget();
        f.app.state_mut().rects.remove("a");
        f.send([moved([20.0, 21.0])]).await;
        assert_eq!(f.events("a"), ["cancel", "blur"]);
        f.send([release()]).await;

        f.send(press_at(B)).await;
        f.send([release()]).await;
        f.forget();
        f.app.state_mut().disabled.insert("b".to_string());
        f.send(tap(Key::Character('x'))).await;
        assert_eq!(f.events("b"), ["blur"]);
        assert_eq!(f.heard(), ["key Character('x')"]);
    });
}

#[test]
fn only_changed_probes_draw_again() {
    pollster::block_on(async {
        let mut f = fixture().await;
        let controls = f.app.scenes().len() - 1;
        let draws = |f: &Fixture| ["a", "b", "c"].map(|id| f.draws(id));
        let before = draws(&f);

        // A chart rebuild that changes no probe's view, rectangle or look draws nothing.
        let scenes = f.send(tap(Key::Character('r'))).await;
        assert!(!scenes.contains(&controls));
        assert_eq!(draws(&f), before);

        // A value change from the chart's handler draws only the probe that shows it.
        let scenes = f.send(tap(Key::Character('v'))).await;
        assert!(scenes.contains(&controls));
        assert_eq!(draws(&f), [before[0] + 1, before[1], before[2]]);
    });
}

#[test]
fn the_hovered_probes_cursor_is_the_apps() {
    pollster::block_on(async {
        let mut f = fixture().await;
        f.send([moved(C)]).await;
        assert_eq!(f.app.cursor(), CursorIcon::Pointer);
        f.send([moved(OUTSIDE)]).await;
        assert_eq!(f.app.cursor(), CursorIcon::Default);
    });
}

#[test]
fn a_probe_that_fails_to_draw_fails_the_first_build() {
    pollster::block_on(async {
        let mut state = placed();
        state.values.insert("b".to_string(), -1);
        let error = fixture_with(state, &[]).await.err().unwrap();
        assert!(error.to_string().contains("widget \"b\" failed to draw"));
    });
}

#[test]
fn a_probe_that_fails_later_draws_nothing_and_the_others_stay() {
    pollster::block_on(async {
        let mut f = fixture().await;
        let controls = f.app.scenes().len() - 1;
        f.app.state_mut().values.insert("a".to_string(), -2);
        f.send(tap(Key::Character('v'))).await;
        let drawn = [
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            ("c".to_string(), 2),
        ];
        assert_eq!(groups(&f, controls), drawn);

        // It tries again only when its view changes.
        let failed = f.draws("a");
        f.send(tap(Key::Character('r'))).await;
        assert_eq!(f.draws("a"), failed);
    });
}

/// Each widget group in the scene at `index`, by name, with its number of marks: the backing
/// box and what the widget drew.
fn groups(f: &Fixture, index: usize) -> Vec<(String, usize)> {
    (f.app.scenes()[index].marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Group(group) => Some((group.name.clone(), group.marks.len())),
            _ => None,
        })
        .collect()
}
