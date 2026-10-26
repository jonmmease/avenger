//! The tool that draws a set of widgets and routes events to them: which widget an event goes
//! to, which widget holds a press, which has keyboard focus, and which to draw again.

use async_trait::async_trait;
use avenger_app::app::{Scenes, Tool, ToolHandler, ToolInstall};
use avenger_app::error::AvengerAppError;
use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{Disposition, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{Key, NamedKey};
use avenger_layout::Rect;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::widget::{Look, Placed, WidgetContext, WidgetEvent};
use crate::widgets::{Erased, Widgets};

/// The [`Widgets`] tool's state: each widget's interaction state and last drawing, and which
/// widget is hovered, holds the press and has focus.
#[derive(Default)]
pub struct WidgetsState {
    /// One for each widget, in the order that they were added.
    entries: Vec<Entry>,
    hovered: Option<usize>,
    /// The pointer's last position over the window, which finds the hovered widget again when
    /// the widgets move.
    pointer: Option<[f32; 2]>,
    /// The widget that holds the current press.
    pressed: Option<usize>,
    /// Whether a press that began outside every enabled widget is down, such as a pan's.
    pressed_elsewhere: bool,
    focus: Option<Focus>,
    /// The app's label engine, from the first update, for handlers, which don't receive it.
    engine: Option<LabelEngine>,
    /// Whether a delivered event changed a widget's view.
    changed: bool,
}

struct Entry {
    own: Erased,
    drawn: Option<Drawn>,
}

/// What the tool last drew for a widget, and from what.
struct Drawn {
    rect: Rect,
    look: Look,
    view: Erased,
    marks: Vec<SceneMark>,
}

#[derive(Clone, Copy, PartialEq)]
struct Focus {
    widget: usize,
    /// Whether the focus came from the keyboard.
    visible: bool,
}

/// Where an event goes.
#[derive(Clone, Copy, PartialEq)]
enum Route {
    /// Not to the widgets.
    Pass,
    /// To the tool alone, which tracks hover, presses or focus and lets the event go on.
    Observe,
    /// To the tool alone, which keeps the event from the scenes below: a pointer event over a
    /// disabled widget.
    Block,
    /// To a widget.
    Widget(usize),
    /// A Tab press that moves focus to a widget.
    Tab(usize),
}

impl WidgetsState {
    /// Create an entry for each widget, the first time the tool sees them.
    fn ensure<State>(&mut self, widgets: &Widgets<State>) {
        if self.entries.len() != widgets.widgets.len() {
            self.entries = (widgets.widgets.iter())
                .map(|widget| Entry {
                    own: widget.new_state(),
                    drawn: None,
                })
                .collect();
        }
    }

    fn look(&self, widget: usize, enabled: bool) -> Look {
        let focus = self.focus.filter(|focus| focus.widget == widget);
        Look {
            enabled,
            hovered: enabled && self.hovered == Some(widget),
            pressed: enabled && self.pressed == Some(widget),
            focused: enabled && focus.is_some(),
            focus_visible: enabled && focus.is_some_and(|focus| focus.visible),
        }
    }

    /// Where and how the tool drew `widget`.
    fn placed(&self, widget: usize, enabled: bool) -> Placed {
        let rect =
            (self.entries[widget].drawn.as_ref()).map_or(Rect::default(), |drawn| drawn.rect);
        Placed {
            rect,
            look: self.look(widget, enabled),
        }
    }

    /// The topmost drawn widget whose rectangle holds `point`, skipping widgets that app state
    /// no longer places, which the next update removes.
    fn topmost<State>(
        &self,
        widgets: &Widgets<State>,
        app: &State,
        point: [f32; 2],
    ) -> Option<usize> {
        (self.entries.iter().enumerate().rev())
            .find(|(index, entry)| {
                let holds = (entry.drawn.as_ref()).is_some_and(|drawn| drawn.rect.contains(point));
                holds && widgets.shown(*index, app)
            })
            .map(|(index, _)| index)
    }

    /// The topmost drawn, enabled widget at `point`, which the pointer hovers there.
    fn hover_at<State>(
        &self,
        widgets: &Widgets<State>,
        app: &State,
        point: [f32; 2],
    ) -> Option<usize> {
        self.topmost(widgets, app, point)
            .filter(|&widget| widgets.widgets[widget].enabled(app))
    }

    /// The widget that the pointer hovers in `rects`, the rectangles of the next drawing: the
    /// widget that holds the press while the pointer is over it, no widget during a press that
    /// began elsewhere, and otherwise the topmost enabled widget under the pointer.
    fn hover_in<State>(
        &self,
        widgets: &Widgets<State>,
        app: &State,
        rects: &[Option<Rect>],
    ) -> Option<usize> {
        let pointer = self.pointer?;
        let holds = |index: usize| rects[index].is_some_and(|rect| rect.contains(pointer));
        match self.pressed {
            Some(pressed) => holds(pressed).then_some(pressed),
            None if self.pressed_elsewhere => None,
            None => (0..rects.len())
                .rev()
                .find(|&index| holds(index) && widgets.widgets[index].enabled(app)),
        }
    }

    /// The Tab stop after the focused widget, or before it when `forward` is false, or the
    /// first or last stop without focus.
    fn next_stop<State>(
        &self,
        widgets: &Widgets<State>,
        app: &State,
        forward: bool,
    ) -> Option<usize> {
        let current = self.focus.map(|focus| focus.widget);
        let mut stops = (0..self.entries.len()).filter(|&index| {
            let widget = &widgets.widgets[index];
            self.entries[index].drawn.is_some() && widget.focusable() && widget.enabled(app)
        });
        if forward {
            stops.find(|&stop| current.is_none_or(|current| stop > current))
        } else {
            stops
                .rev()
                .find(|&stop| current.is_none_or(|current| stop < current))
        }
    }

    fn route<State>(
        &self,
        widgets: &Widgets<State>,
        event: &SceneGraphEvent,
        app: &State,
    ) -> Route {
        use SceneGraphEvent::*;
        match event {
            KeyPress(press) if press.key == Key::Named(NamedKey::Tab) => {
                match self.next_stop(widgets, app, !press.modifiers.shift) {
                    Some(stop) => Route::Tab(stop),
                    None if self.focus.is_some() => Route::Observe,
                    None => Route::Pass,
                }
            }
            KeyPress(_) | KeyRelease(_) => match self.focus {
                Some(focus) => Route::Widget(focus.widget),
                None => Route::Pass,
            },
            MouseCancel(_) => match self.pressed {
                Some(widget) => Route::Widget(widget),
                None if self.pressed_elsewhere => Route::Observe,
                None => Route::Pass,
            },
            CursorLeft | WindowFocused(false) => Route::Observe,
            CursorMoved(_) | MouseDown(_) | MouseUp(_) | Click(_) | DoubleClick(_)
            | MouseWheel(_) => {
                if let Some(widget) = self.pressed {
                    return Route::Widget(widget);
                }
                if self.pressed_elsewhere {
                    return match event {
                        MouseUp(_) => Route::Observe,
                        _ => Route::Pass,
                    };
                }
                let Some(position) = event.position() else {
                    return Route::Pass;
                };
                match self.topmost(widgets, app, position) {
                    Some(widget) if widgets.widgets[widget].enabled(app) => Route::Widget(widget),
                    Some(_) => Route::Block,
                    None => match event {
                        CursorMoved(_) | MouseDown(_) => Route::Observe,
                        _ => Route::Pass,
                    },
                }
            }
            _ => Route::Pass,
        }
    }
}

/// Whether the tool tracks hover or presses from `event` even when its widget ignores it.
fn tracked(event: &SceneGraphEvent) -> bool {
    matches!(
        event,
        SceneGraphEvent::CursorMoved(_)
            | SceneGraphEvent::MouseDown(_)
            | SceneGraphEvent::MouseUp(_)
    )
}

/// Routes the tool's events to its widgets.
struct Router<State> {
    widgets: Widgets<State>,
}

impl<State: Send + Sync + 'static> Router<State> {
    /// Whether `widget` is still shown and enabled.
    fn alive(&self, widget: usize, app: &State) -> bool {
        self.widgets.shown(widget, app) && self.widgets.widgets[widget].enabled(app)
    }

    /// Whether the widget that holds the press or has focus lost its rectangle or became
    /// disabled, which the handler tells it about.
    fn stale(&self, app: &State, own: &WidgetsState) -> bool {
        let pressed = own.pressed.is_some_and(|widget| !self.alive(widget, app));
        pressed
            || own
                .focus
                .is_some_and(|focus| !self.alive(focus.widget, app))
    }

    /// Deliver `event` to `widget`, and note whether it changed the widget's view.
    fn deliver(
        &self,
        widget: usize,
        event: WidgetEvent<'_>,
        engine: &LabelEngine,
        app: &mut State,
        own: &mut WidgetsState,
    ) -> Rebuild {
        let erased = &self.widgets.widgets[widget];
        let placed = own.placed(widget, erased.enabled(app));
        let cx = WidgetContext {
            engine,
            style: &self.widgets.style,
        };
        let state = &mut own.entries[widget].own;
        let before = erased.view(app, state);
        let rebuild = erased.event(event, &placed, &cx, app, state);
        let after = erased.view(app, state);
        own.changed |= !erased.same_view(&before, &after);
        rebuild
    }

    /// Move focus to `widget`, telling the widget that loses it and the one that gains it.
    fn focus(
        &self,
        widget: usize,
        visible: bool,
        engine: &LabelEngine,
        app: &mut State,
        own: &mut WidgetsState,
    ) -> Rebuild {
        let previous = own.focus.replace(Focus { widget, visible });
        let mut rebuild = Rebuild::none();
        match previous {
            Some(previous) if previous.widget == widget => {}
            Some(previous) => {
                rebuild.merge(self.deliver(previous.widget, WidgetEvent::Blur, engine, app, own));
                rebuild.merge(self.deliver(widget, WidgetEvent::Focus, engine, app, own));
            }
            None => rebuild.merge(self.deliver(widget, WidgetEvent::Focus, engine, app, own)),
        }
        rebuild
    }

    /// Clear focus, telling the widget that loses it.
    fn blur(&self, engine: &LabelEngine, app: &mut State, own: &mut WidgetsState) -> Rebuild {
        match own.focus {
            Some(focus) => {
                let rebuild = self.deliver(focus.widget, WidgetEvent::Blur, engine, app, own);
                own.focus = None;
                rebuild
            }
            None => Rebuild::none(),
        }
    }
}

#[async_trait]
impl<State: Send + Sync + 'static> ToolHandler<State, WidgetsState> for Router<State> {
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        app: &State,
        own: &WidgetsState,
        _trees: &SceneTrees<'_>,
    ) -> Disposition {
        if own.engine.is_none() {
            return Disposition::Ignore;
        }
        let disposition = match own.route(&self.widgets, event, app) {
            Route::Pass => Disposition::Ignore,
            Route::Observe => Disposition::Handle,
            Route::Block | Route::Tab(_) => Disposition::Consume,
            Route::Widget(_) if matches!(event, SceneGraphEvent::MouseCancel(_)) => {
                Disposition::Handle
            }
            Route::Widget(widget) => {
                let erased = &self.widgets.widgets[widget];
                let placed = own.placed(widget, erased.enabled(app));
                match erased.disposition(event, &placed, app, &own.entries[widget].own) {
                    Disposition::Ignore if tracked(event) => Disposition::Handle,
                    disposition => disposition,
                }
            }
        };
        // The handler tells a widget that lost its rectangle or became disabled that it lost
        // its press or focus, even when the event is for no widget.
        match disposition {
            Disposition::Ignore if self.stale(app, own) => Disposition::Handle,
            disposition => disposition,
        }
    }

    async fn handle(
        &self,
        event: &SceneGraphEvent,
        app: &mut State,
        own: &mut WidgetsState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let Some(engine) = own.engine.clone() else {
            return Rebuild::none();
        };
        let engine = &engine;
        let before = (own.hovered, own.pressed, own.focus);
        own.changed = false;
        match event {
            SceneGraphEvent::CursorLeft => own.pointer = None,
            event => own.pointer = event.position().or(own.pointer),
        }
        let mut rebuild = Rebuild::none();

        // A widget that lost its rectangle or became disabled loses its press and focus, here,
        // because only handlers can change app state.
        if let Some(widget) = own.pressed.filter(|&widget| !self.alive(widget, app)) {
            rebuild.merge(self.deliver(widget, WidgetEvent::Cancel, engine, app, own));
            own.pressed = None;
        }
        if own
            .focus
            .is_some_and(|focus| !self.alive(focus.widget, app))
        {
            rebuild.merge(self.blur(engine, app, own));
        }

        use SceneGraphEvent::*;
        match own.route(&self.widgets, event, app) {
            Route::Pass => {}
            Route::Tab(widget) => rebuild.merge(self.focus(widget, true, engine, app, own)),
            Route::Observe | Route::Block => match event {
                MouseDown(_) => {
                    rebuild.merge(self.blur(engine, app, own));
                    own.pressed_elsewhere = true;
                    own.hovered = None;
                }
                MouseUp(up) => {
                    own.pressed_elsewhere = false;
                    own.hovered = own.hover_at(&self.widgets, app, up.position);
                }
                MouseCancel(_) => own.pressed_elsewhere = false,
                KeyPress(_) => rebuild.merge(self.blur(engine, app, own)),
                // The focused widget keeps its focus, but loses a key held across the change.
                WindowFocused(false) => {
                    if let Some(focus) = own.focus {
                        let cancel = WidgetEvent::Cancel;
                        rebuild.merge(self.deliver(focus.widget, cancel, engine, app, own));
                    }
                }
                _ => own.hovered = None,
            },
            Route::Widget(widget) if matches!(event, MouseCancel(_)) => {
                rebuild.merge(self.deliver(widget, WidgetEvent::Cancel, engine, app, own));
                own.pressed = None;
            }
            Route::Widget(widget) => {
                let erased = &self.widgets.widgets[widget];
                let placed = own.placed(widget, erased.enabled(app));
                let wanted = erased.disposition(event, &placed, app, &own.entries[widget].own)
                    != Disposition::Ignore;
                let input = WidgetEvent::Input(event);
                match event {
                    MouseDown(_) if wanted => {
                        if erased.focusable() {
                            rebuild.merge(self.focus(widget, false, engine, app, own));
                        } else {
                            rebuild.merge(self.blur(engine, app, own));
                        }
                        own.pressed = Some(widget);
                        own.hovered = Some(widget);
                        rebuild.merge(self.deliver(widget, input, engine, app, own));
                    }
                    MouseDown(_) => {
                        rebuild.merge(self.blur(engine, app, own));
                        own.pressed_elsewhere = true;
                        own.hovered = None;
                    }
                    MouseUp(up) => {
                        if wanted {
                            rebuild.merge(self.deliver(widget, input, engine, app, own));
                        }
                        own.pressed = None;
                        own.hovered = own.hover_at(&self.widgets, app, up.position);
                    }
                    CursorMoved(moved) => {
                        own.hovered = match own.pressed {
                            Some(_) => placed.rect.contains(moved.position).then_some(widget),
                            None => Some(widget),
                        };
                        if wanted {
                            rebuild.merge(self.deliver(widget, input, engine, app, own));
                        }
                    }
                    _ if wanted => rebuild.merge(self.deliver(widget, input, engine, app, own)),
                    _ => {}
                }
            }
        }

        if own.changed || (own.hovered, own.pressed, own.focus) != before {
            rebuild.merge(Rebuild::tree(&self.widgets.scene));
        }
        rebuild
    }
}

#[async_trait]
impl<State: Send + Sync + 'static> Tool<State> for Widgets<State> {
    type State = WidgetsState;

    fn install(&self, tool: &mut ToolInstall<'_, State, WidgetsState>) {
        let events = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::MouseUp,
                SceneGraphEventType::MouseCancel,
                SceneGraphEventType::Click,
                SceneGraphEventType::DoubleClick,
                SceneGraphEventType::MouseWheel,
                SceneGraphEventType::KeyPress,
                SceneGraphEventType::KeyRelease,
                SceneGraphEventType::WindowFocused,
            ],
            ..Default::default()
        };
        let router = Router {
            widgets: self.clone(),
        };
        tool.scene(&self.scene, self.z)
            .stream(&self.scene, events, router);
    }

    /// Draw each widget whose view, rectangle or look changed since the tool last drew it, and
    /// set the scene when a widget changed, appeared or disappeared, or when the scene was
    /// requested. A widget that fails to draw shows nothing until its inputs change, and the
    /// update returns the first failure after it sets the scene.
    async fn update(
        &self,
        app: &State,
        own: &mut WidgetsState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let engine = scenes.text_engine();
        own.engine.get_or_insert_with(|| engine.clone());
        own.ensure(self);
        let cx = WidgetContext {
            engine,
            style: &self.style,
        };
        let rects: Vec<Option<Rect>> = (self.widgets.iter())
            .map(|widget| (self.place)(app, widget.id()))
            .collect();
        // The widgets can move under a still pointer, as when the window resizes.
        own.hovered = own.hover_in(self, app, &rects);
        let mut changed = false;
        let mut failure = None;
        for (index, widget) in self.widgets.iter().enumerate() {
            let Some(rect) = rects[index] else {
                changed |= own.entries[index].drawn.take().is_some();
                continue;
            };
            let look = own.look(index, widget.enabled(app));
            let entry = &mut own.entries[index];
            let view = widget.view(app, &entry.own);
            let same = (entry.drawn.as_ref()).is_some_and(|drawn| {
                drawn.rect == rect && drawn.look == look && widget.same_view(&drawn.view, &view)
            });
            if same {
                continue;
            }
            let marks = match widget.draw(&view, &Placed { rect, look }, &cx) {
                Ok(marks) => marks,
                Err(error) => {
                    failure
                        .get_or_insert(format!("widget {:?} failed to draw: {error}", widget.id()));
                    Vec::new()
                }
            };
            entry.drawn = Some(Drawn {
                rect,
                look,
                view,
                marks,
            });
            changed = true;
        }
        if changed || scenes.requested(&self.scene) {
            let marks = (own.entries.iter().zip(&self.widgets))
                .filter_map(|(entry, widget)| {
                    let drawn = entry.drawn.as_ref()?;
                    Some(group(widget.id(), drawn).into())
                })
                .collect();
            let [width, height] = scenes.size();
            let scene = SceneGraph {
                marks,
                width,
                height,
                origin: [0.0; 2],
            };
            scenes.set(&self.scene, scene.into())?;
        }
        match failure {
            Some(failure) => Err(AvengerAppError::InvalidApp(failure)),
            None => Ok(()),
        }
    }

    fn cursor(&self, _app: &State, own: &WidgetsState) -> Option<CursorIcon> {
        let widget = own.pressed.or(own.hovered)?;
        let look = own.look(widget, true);
        self.widgets[widget].cursor(look, &own.entries[widget].own)
    }
}

/// A widget's marks over an invisible box that covers its rectangle, so that the scenes below
/// find the widget as the topmost hit there.
fn group(id: &str, drawn: &Drawn) -> SceneGroup {
    let Rect {
        x,
        y,
        width,
        height,
    } = drawn.rect;
    let backing = SceneRectMark {
        name: "widget".to_string(),
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        ..Default::default()
    };
    let mut marks = vec![backing.into()];
    marks.extend(drawn.marks.iter().cloned());
    SceneGroup {
        name: id.to_string(),
        interactive: Some(true),
        marks,
        ..Default::default()
    }
}
