//! The widget interface and the data supplied by the [`Widgets`](crate::Widgets) tool.

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_layout::{Direction, Edges, Measurement, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{LabelEngine, LabelError};

use crate::style::WidgetStyle;

/// A control that a [`Widgets`](crate::Widgets) tool draws in its scene and routes events to.
///
/// Values belong to app state. A widget keeps its own interaction state, provides layout
/// measurements, and draws in the rectangle assigned by the app.
pub trait Widget<State>: Send + Sync + 'static {
    /// Interaction state that the widget keeps between events, such as a drag's starting value.
    type State: Default + Send + Sync + 'static;

    /// Everything that [`draw`](Widget::draw) shows from app state and the widget's own state.
    /// The tool reuses the drawing while the view, rectangle and look are unchanged.
    type View: PartialEq + Send + Sync + 'static;

    /// The widget's name, unique within its `Widgets` tool.
    fn id(&self) -> &str;

    /// The widget's unconstrained size in `direction`. Measurements depend on configuration,
    /// style and available space, but not app state. A value change does not alter the measured
    /// size.
    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32;

    /// The widget's minimum height at its final `width`.
    fn height(&self, _width: f32, cx: &WidgetContext<'_>) -> Measurement {
        Measurement::always(self.max_content(Direction::Vertical, cx))
    }

    /// The widget's minimum width at its solved `height`.
    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> Measurement {
        Measurement::always(self.max_content(Direction::Horizontal, cx))
    }

    /// Insets for content placed inside a widget that frames other widgets.
    fn padding(&self, _style: &WidgetStyle) -> Edges {
        Edges::default()
    }

    /// Collect what the widget shows.
    fn view(&self, app: &State, own: &Self::State) -> Self::View;

    /// Draw `view` in the widget's rectangle, in scene coordinates. A text selection scene above
    /// the widgets can select labels unless a later interactive mark covers them. Cover labels
    /// that activate a control, such as button labels, and leave titles selectable.
    fn draw(
        &self,
        view: &Self::View,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError>;

    /// Whether the widget accepts input. The tool excludes disabled widgets from input routing
    /// and Tab traversal, but can still send `Cancel` and `Blur`. Built-ins use disabled paint.
    fn enabled(&self, _app: &State) -> bool {
        true
    }

    /// Whether the widget is a Tab stop while it is placed and enabled.
    fn focusable(&self) -> bool {
        true
    }

    /// Decide what happens to an input event that the tool routes to the widget, as
    /// [`ToolHandler::disposition`](avenger_app::app::ToolHandler::disposition) does for a
    /// tool's stream. By default, the widget consumes pointer events to prevent scenes below
    /// from receiving them. It ignores key, paste and input method events so they reach the
    /// app.
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &Self::State,
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(_)
            | SceneGraphEvent::KeyRelease(_)
            | SceneGraphEvent::Paste(_)
            | SceneGraphEvent::Composed(_)
            | SceneGraphEvent::Composing(_) => Disposition::Ignore,
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

    /// The text in `view` that the copy shortcut copies while the widget has focus, such as its
    /// selected text, or `None` for none. The view is the latest drawing's, so the text matches
    /// what the widget shows after app state changes its value.
    fn copy_text<'a>(&self, _view: &'a Self::View) -> Option<&'a str> {
        None
    }

    /// While the widget has focus, the area of its caret in `view`, as `[x, y, width, height]`
    /// in scene coordinates, if it takes typed text. The host then lets input methods compose
    /// text, such as accented letters from dead keys, and places their candidate windows beside
    /// the caret. `None`, the default, for a widget that takes no typed text.
    fn text_entry(
        &self,
        _view: &Self::View,
        _placed: &Placed,
        _cx: &WidgetContext<'_>,
    ) -> Option<[f32; 4]> {
        None
    }
}

/// An event that the [`Widgets`](crate::Widgets) tool routes to one widget.
#[derive(Debug, Clone, Copy)]
pub enum WidgetEvent<'a> {
    /// A pointer event over the widget or during its press, or a key, paste or input method
    /// event while it has focus.
    Input(&'a SceneGraphEvent),
    /// The widget gained keyboard focus.
    Focus,
    /// The widget lost keyboard focus.
    Blur,
    /// The widget's active input ended without its release. Window focus loss sends this to the
    /// focused widget, which keeps its focus, and to the widget that holds the press. A widget
    /// that becomes hidden or disabled during its press also receives it.
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
