//! The trait that every widget implements, and what the [`Widgets`](crate::Widgets) tool passes
//! to it.

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_layout::{Direction, Edges, Measurement, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{LabelEngine, LabelError};

use crate::style::WidgetStyle;

/// A control that a [`Widgets`](crate::Widgets) tool draws in its scene and routes events to.
///
/// The widget reads its value from app state and reports changes through app state, so it keeps
/// only interaction state of its own. The app's layout places it: the widget measures itself for
/// the layout, and draws in the rectangle that the layout gives it.
pub trait Widget<State>: Send + Sync + 'static {
    /// Interaction state that the widget keeps between events, such as a drag's starting value.
    type State: Default + Send + Sync + 'static;

    /// Everything that [`draw`](Widget::draw) shows from app state and the widget's own state.
    /// The tool draws the widget again only when its view, rectangle, look or style changes.
    type View: PartialEq + Send + Sync + 'static;

    /// The widget's name, unique within its `Widgets` tool.
    fn id(&self) -> &str;

    /// The widget's size in `direction` without a constraint. Measurements depend on the
    /// widget's configuration, the style and the space it gets, never on app state, so that a
    /// change of value does not move the layout.
    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32;

    /// The widget's minimum height at its final `width`.
    fn height(&self, _width: f32, cx: &WidgetContext<'_>) -> Measurement {
        Measurement::always(self.max_content(Direction::Vertical, cx))
    }

    /// The widget's minimum width at its solved `height`.
    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> Measurement {
        Measurement::always(self.max_content(Direction::Horizontal, cx))
    }

    /// The space between the widget's rectangle and the content that the app's layout places
    /// inside it, for a widget that frames other widgets.
    fn padding(&self, _style: &WidgetStyle) -> Edges {
        Edges::default()
    }

    /// Collect what the widget shows.
    fn view(&self, app: &State, own: &Self::State) -> Self::View;

    /// Draw `view` in the widget's rectangle, in scene coordinates. When a text selection scene
    /// is above the widgets, a label that no interactive mark drawn after it covers is
    /// selectable, as a title should be. Cover a label that a press acts on, such as a button's.
    fn draw(
        &self,
        view: &Self::View,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError>;

    /// Whether the widget takes input. A disabled widget draws with the style's disabled paint,
    /// receives no events and is not a Tab stop.
    fn enabled(&self, _app: &State) -> bool {
        true
    }

    /// Whether the widget is a Tab stop while it is placed and enabled.
    fn focusable(&self) -> bool {
        true
    }

    /// Decide what happens to an input event that the tool routes to the widget, as
    /// [`ToolHandler::disposition`](avenger_app::app::ToolHandler::disposition) does for a
    /// tool's stream. By default, the widget consumes pointer events, so the scenes below it
    /// don't hear them, and ignores key events, so they reach the app.
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &Self::State,
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(_) | SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    /// Handle an event that the tool routes to the widget, and return the scenes to rebuild.
    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut Self::State,
    ) -> Rebuild;

    /// The cursor while the widget is hovered or holds the press, or `None` for the default.
    fn cursor(&self, _look: Look, _own: &Self::State) -> Option<CursorIcon> {
        None
    }
}

/// An event that the [`Widgets`](crate::Widgets) tool routes to one widget.
#[derive(Debug, Clone, Copy)]
pub enum WidgetEvent<'a> {
    /// A pointer event over the widget or during its press, or a key event while it has focus.
    Input(&'a SceneGraphEvent),
    /// The widget gained keyboard focus.
    Focus,
    /// The widget lost keyboard focus.
    Blur,
    /// The widget's press or held key ended without its release: the window lost focus, or the
    /// widget lost its rectangle or became disabled.
    Cancel,
}

/// How a widget looks apart from its value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    /// The widget takes input.
    pub enabled: bool,
    /// The pointer is over the widget, and no press that began elsewhere is in progress.
    pub hovered: bool,
    /// A press that began on the widget is in progress.
    pub pressed: bool,
    /// The widget has keyboard focus.
    pub focused: bool,
    /// The widget's focus came from the keyboard, so the widget draws a focus ring.
    pub focus_visible: bool,
}

/// Where and how the tool draws a widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub rect: Rect,
    pub look: Look,
}

/// What every widget measures and draws with.
#[derive(Clone, Copy)]
pub struct WidgetContext<'a> {
    pub engine: &'a LabelEngine,
    pub style: &'a WidgetStyle,
}

/// A widget's failure to draw, such as a slider readout's invalid format pattern or markup.
#[derive(Debug, thiserror::Error)]
pub enum WidgetError {
    #[error(transparent)]
    Label(#[from] LabelError),
}
