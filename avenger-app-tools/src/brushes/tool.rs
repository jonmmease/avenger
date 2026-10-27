//! The tool that draws every brush in one scene and routes presses to them.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use avenger_app::app::{Scenes, Tool, ToolHandler, ToolInstall};
use avenger_app::error::AvengerAppError;
use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType, SceneMouseDownEvent};
use avenger_eventstream::stream::{DebounceConfig, Disposition, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_scenegraph::scene_graph::SceneGraph;

use super::brush::{Brush, Claim, Clicked, ErasedBrush, Own};
use super::style::BrushStyle;
use crate::CLICK_DISTANCE;

/// Draws brushes, the selections that an app's charts make by dragging and clicking, in one
/// scene, and routes presses to them. See the [module documentation](super) for the gestures.
pub struct Brushes<State> {
    scene: String,
    z: i32,
    brushes: Vec<Arc<dyn ErasedBrush<State>>>,
    style: Arc<BrushStyle>,
    /// The least time between a gesture's reports through `on_change`, if any.
    change_interval: Option<Duration>,
}

impl<State> Clone for Brushes<State> {
    fn clone(&self) -> Self {
        Self {
            scene: self.scene.clone(),
            z: self.z,
            brushes: self.brushes.clone(),
            style: self.style.clone(),
            change_interval: self.change_interval,
        }
    }
}

/// Each brush's gesture and drawings, and the press and pointer that the tool follows.
#[derive(Default)]
pub struct BrushesState {
    /// One state for each brush, in definition order.
    owns: Vec<Own>,
    /// The brush whose gesture holds the current press.
    held: Option<Held>,
    /// Whether a press that no brush claimed is active, such as a pan below.
    pressed_elsewhere: bool,
    /// The current left-button press, whichever scene took it.
    press: Option<Press>,
    /// Whether the current press's click selected an instance, which keeps a box under it.
    click_hit: bool,
    pointer: Option<[f32; 2]>,
}

struct Held {
    brush: usize,
    /// Whether Escape ended the gesture before its release.
    canceled: bool,
}

struct Press {
    at: [f32; 2],
    /// Whether the press can still end as a click: the pointer stays within the click distance
    /// of the press, and Escape hasn't canceled the press's gesture.
    click: bool,
}

impl BrushesState {
    /// Whether the current press is still a click.
    fn clicking(&self) -> bool {
        self.press.as_ref().is_some_and(|press| press.click)
    }

    /// Create each brush's state, or create them again if the brush count changed.
    fn ensure<State>(&mut self, tool: &Brushes<State>) {
        if self.owns.len() != tool.brushes.len() {
            self.owns = tool.brushes.iter().map(|brush| brush.new_state()).collect();
        }
    }
}

impl<State: Send + Sync + 'static> Brushes<State> {
    /// Create a tool without brushes, with scene name `scene` and z-order `z`. Place the scene
    /// above the chart scenes that the brushes select from and below tooltips.
    pub fn new(scene: &str, z: i32) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            brushes: Vec::new(),
            style: Arc::new(BrushStyle::default()),
            change_interval: None,
        }
    }

    /// Add `brush` after the existing brushes.
    pub fn with(mut self, brush: impl Into<Brush<State>>) -> Self {
        self.brushes.push(brush.into().0);
        self
    }

    /// Set how the brushes look. Defaults to [`BrushStyle::default`].
    pub fn with_style(mut self, style: BrushStyle) -> Self {
        self.style = Arc::new(style);
        self
    }

    /// Report each gesture's value through `on_change` at most once per `interval`, so an app
    /// that rebuilds a large chart for each change rebuilds less often. The brushes still draw
    /// every move. A drag that pauses for `interval` reports its latest value, and the release
    /// reports the last one before `on_settle`. Without an interval, every move reports.
    pub fn with_change_interval(mut self, interval: Duration) -> Self {
        self.change_interval = Some(interval);
        self
    }

    /// The brush that a press at `at` goes to, and what the press does: the last brush that
    /// would edit a selection there, or else the first brush that would start one.
    fn claim(&self, app: &State, at: [f32; 2]) -> Option<(usize, Claim)> {
        let claims: Vec<Option<Claim>> = (self.brushes.iter())
            .map(|brush| brush.claim(app, at))
            .collect();
        let edit = claims
            .iter()
            .rposition(|claim| matches!(claim, Some(Claim::Edit(_))));
        let index = edit.or_else(|| claims.iter().position(Option::is_some))?;
        Some((index, claims[index]?))
    }
}

/// Whether the brushes can claim `press`. The second press of a double click passes down, so that
/// the double click reaches the scenes below.
fn claimable(press: &SceneMouseDownEvent) -> bool {
    press.button == MouseButton::Left && press.count != 2
}

/// Routes the tool's events to its brushes.
struct Router<State> {
    tool: Brushes<State>,
}

impl<State: Send + Sync + 'static> Router<State> {
    /// The app's requests, with the tool's scene when the brush's drawing changed.
    fn redraw(&self, changed: Option<Rebuild>) -> Rebuild {
        match changed {
            Some(rebuild) => rebuild.and_tree(&self.tool.scene),
            None => Rebuild::none(),
        }
    }

    /// Note what a brush did with a click, and return its rebuild requests with the tool's scene.
    fn clicked(&self, own: &mut BrushesState, clicked: Option<Clicked>) -> Rebuild {
        let Some(clicked) = clicked else {
            return Rebuild::none();
        };
        own.click_hit |= clicked.hit;
        self.redraw(Some(clicked.rebuild))
    }

    /// Let every brush follow the pointer at `at`, and request the tool's scene if a drawing
    /// changed.
    fn hover(&self, app: &State, own: &mut BrushesState, at: Option<[f32; 2]>) -> Rebuild {
        let mut changed = false;
        for (brush, state) in self.tool.brushes.iter().zip(&mut own.owns) {
            changed |= brush.hover(app, state, at);
        }
        self.redraw(changed.then(Rebuild::none))
    }
}

/// Reports the value that a held gesture kept, as moves settle under a change interval.
struct Flush<State> {
    tool: Brushes<State>,
}

#[async_trait]
impl<State: Send + Sync + 'static> ToolHandler<State, BrushesState> for Flush<State> {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        app: &mut State,
        own: &mut BrushesState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let Some(held) = own.held.as_ref().filter(|held| !held.canceled) else {
            return Rebuild::none();
        };
        let brush = held.brush;
        let kept = self.tool.brushes[brush].flush(app, &mut own.owns[brush]);
        kept.map(|rebuild| rebuild.and_tree(&self.tool.scene))
            .unwrap_or_default()
    }
}

/// Whether `event` is a press of Escape.
fn escape(event: &SceneGraphEvent) -> bool {
    matches!(event, SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape))
}

#[async_trait]
impl<State: Send + Sync + 'static> ToolHandler<State, BrushesState> for Router<State> {
    /// Consume claimed presses and the Escape that ends a gesture, and pass every other event on.
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        app: &State,
        own: &BrushesState,
        _trees: &SceneTrees<'_>,
    ) -> Disposition {
        let live = own.held.as_ref().is_some_and(|held| !held.canceled);
        match event {
            SceneGraphEvent::MouseDown(press)
                if own.held.is_none()
                    && claimable(press)
                    && self.tool.claim(app, press.position).is_some() =>
            {
                Disposition::Consume
            }
            SceneGraphEvent::KeyPress(_) if live && escape(event) => Disposition::Consume,
            SceneGraphEvent::KeyPress(_) => Disposition::Ignore,
            _ => Disposition::Handle,
        }
    }

    async fn handle(
        &self,
        event: &SceneGraphEvent,
        app: &mut State,
        own: &mut BrushesState,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        own.ensure(&self.tool);
        let brushes = &self.tool.brushes;
        match event {
            SceneGraphEvent::MouseDown(press) => {
                if own.held.is_some() || press.button != MouseButton::Left {
                    return Rebuild::none();
                }
                own.press = Some(Press {
                    at: press.position,
                    click: true,
                });
                own.click_hit = false;
                let claim = claimable(press)
                    .then(|| self.tool.claim(app, press.position))
                    .flatten();
                let Some((brush, claim)) = claim else {
                    own.pressed_elsewhere = true;
                    return self.hover(app, own, None);
                };
                let shift = press.modifiers.shift;
                let state = &mut own.owns[brush];
                brushes[brush].press(app, state, claim, press.position, shift, trees);
                own.held = Some(Held {
                    brush,
                    canceled: false,
                });
                self.redraw(Some(Rebuild::none()))
            }
            SceneGraphEvent::CursorMoved(moved) => {
                own.pointer = Some(moved.position);
                if let Some(press) = &mut own.press {
                    let [dx, dy] = [0, 1].map(|axis| moved.position[axis] - press.at[axis]);
                    press.click &= dx.hypot(dy) <= CLICK_DISTANCE;
                }
                match &own.held {
                    Some(_) if own.clicking() => Rebuild::none(),
                    Some(held) if !held.canceled => {
                        let (brush, at) = (held.brush, moved.position);
                        let report = self.tool.change_interval.is_none();
                        let changed = brushes[brush].drag(app, &mut own.owns[brush], at, report);
                        self.redraw(changed)
                    }
                    Some(_) => Rebuild::none(),
                    None if own.pressed_elsewhere => Rebuild::none(),
                    None => self.hover(app, own, Some(moved.position)),
                }
            }
            SceneGraphEvent::Click(click) if click.button == MouseButton::Left => {
                if !own.clicking() {
                    return Rebuild::none();
                }
                let mut rebuild = Rebuild::none();
                let shift = click.modifiers.shift;
                for (index, brush) in brushes.iter().enumerate() {
                    let state = &mut own.owns[index];
                    let clicked = brush.click(app, state, click.position, shift, trees);
                    rebuild.merge(self.clicked(own, clicked));
                }
                rebuild
            }
            SceneGraphEvent::MouseUp(release) if release.button == MouseButton::Left => {
                own.pressed_elsewhere = false;
                let click = own.clicking();
                own.press = None;
                let mut rebuild = match own.held.take() {
                    Some(held) if !held.canceled => {
                        let (brush, state) = (&brushes[held.brush], &mut own.owns[held.brush]);
                        // A click that selected an instance keeps the box under it.
                        let ended = if own.click_hit {
                            brush.cancel(app, state)
                        } else {
                            brush.release(app, state, release.position, click)
                        };
                        self.redraw(ended)
                    }
                    _ => Rebuild::none(),
                };
                rebuild.merge(self.hover(app, own, Some(release.position)));
                rebuild
            }
            SceneGraphEvent::MouseCancel(_) => {
                own.pressed_elsewhere = false;
                own.press = None;
                match own.held.take() {
                    Some(held) if !held.canceled => {
                        let brush = held.brush;
                        self.redraw(brushes[brush].cancel(app, &mut own.owns[brush]))
                    }
                    _ => Rebuild::none(),
                }
            }
            SceneGraphEvent::KeyPress(_) => match &mut own.held {
                Some(held) if !held.canceled => {
                    held.canceled = true;
                    // The release of a canceled press is no click, for any brush.
                    if let Some(press) = &mut own.press {
                        press.click = false;
                    }
                    let brush = held.brush;
                    self.redraw(brushes[brush].cancel(app, &mut own.owns[brush]))
                }
                _ => Rebuild::none(),
            },
            SceneGraphEvent::CursorLeft => {
                own.pointer = None;
                self.hover(app, own, None)
            }
            _ => Rebuild::none(),
        }
    }
}

#[async_trait]
impl<State: Send + Sync + 'static> Tool<State> for Brushes<State> {
    type State = BrushesState;

    fn install(&self, tool: &mut ToolInstall<'_, State, BrushesState>) {
        let events = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseUp,
                SceneGraphEventType::MouseCancel,
                SceneGraphEventType::Click,
                SceneGraphEventType::KeyPress,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        let router = Router { tool: self.clone() };
        tool.scene(&self.scene, self.z)
            .stream(&self.scene, events, router);
        // Moves settle into a report at most once per interval, and once more when they stop.
        if let Some(interval) = self.change_interval {
            let wait = interval.as_millis() as u64;
            let settled_moves = EventStreamConfig {
                types: vec![SceneGraphEventType::CursorMoved],
                debounce: Some(DebounceConfig {
                    max_wait: Some(wait),
                    ..DebounceConfig::new(wait)
                }),
                ..Default::default()
            };
            let flush = Flush { tool: self.clone() };
            tool.stream(&self.scene, settled_moves, flush);
        }
    }

    /// Draw the brushes again when the tool's scene is requested or a scene below it changes, as
    /// after a pan below moves the data under a box. Each brush reuses the drawing of an area
    /// whose rectangle and selection pixels are unchanged. Return the first drawing error, such
    /// as a scale that does not invert, after updating the scene.
    async fn update(
        &self,
        app: &State,
        own: &mut BrushesState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let requested = scenes.requested(&self.scene);
        if !requested && !scenes.below(&self.scene).any(|below| below.changed()) {
            return Ok(());
        }
        own.ensure(self);
        let mut marks = Vec::new();
        let mut changed = false;
        let mut error = None;
        for (brush, state) in self.brushes.iter().zip(&mut own.owns) {
            let drawing = brush.draw(app, state, &self.style);
            marks.extend(drawing.marks);
            changed |= drawing.changed;
            error = error.or(drawing.error);
        }
        if changed || requested {
            if marks.is_empty() {
                scenes.clear(&self.scene)?;
            } else {
                let [width, height] = scenes.size();
                let scene = SceneGraph {
                    marks,
                    width,
                    height,
                    origin: [0.0; 2],
                };
                scenes.set(&self.scene, scene.into())?;
            }
        }
        match error {
            Some(error) => Err(AvengerAppError::InvalidApp(error)),
            None => Ok(()),
        }
    }

    /// The gesture's cursor during a press that a brush holds, and otherwise the cursor of what
    /// a press under the pointer would do: move, resize, or a crosshair for a new selection.
    fn cursor(&self, app: &State, own: &BrushesState) -> Option<CursorIcon> {
        if let Some(held) = &own.held {
            return (!held.canceled)
                .then(|| self.brushes[held.brush].cursor(&own.owns[held.brush]))
                .flatten();
        }
        if own.pressed_elsewhere {
            return None;
        }
        match self.claim(app, own.pointer?)?.1 {
            Claim::Edit(cursor) => Some(cursor),
            Claim::New => Some(CursorIcon::Crosshair),
        }
    }
}
