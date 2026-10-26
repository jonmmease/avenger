//! A widget collection and its layout measurement adapter.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_layout::{
    Column, Direction, Edges, Layout, Leaf, Measure, Measurement, Rect, Size, Track,
};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::LabelEngine;

use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

type Place<State> = Arc<dyn Fn(&State, &str) -> Option<Rect> + Send + Sync>;

/// A tool that draws a set of widgets in its own scene and routes pointer and key events to
/// them.
///
/// The app measures widgets through [`measure`](Widgets::measure). The `place` closure reads
/// their rectangles from app state. A widget without a rectangle is hidden. Widgets draw in
/// insertion order, so later widgets receive pointer events where rectangles overlap. Tab
/// visits widgets in insertion order within one tool. Keep the app's widgets in a single tool
/// for keyboard traversal.
///
/// Clones share widget definitions. Keep a clone for layout measurements and install the
/// original as a tool.
pub struct Widgets<State> {
    pub(crate) scene: String,
    pub(crate) z: i32,
    pub(crate) style: Arc<WidgetStyle>,
    pub(crate) place: Place<State>,
    pub(crate) widgets: Vec<Arc<dyn ErasedWidget<State>>>,
    index: HashMap<String, usize>,
}

impl<State> Widgets<State> {
    /// Whether app state places the widget at `index`.
    pub(crate) fn shown(&self, index: usize, app: &State) -> bool {
        (self.place)(app, self.widgets[index].id()).is_some()
    }
}

impl<State> Clone for Widgets<State> {
    fn clone(&self) -> Self {
        Self {
            scene: self.scene.clone(),
            z: self.z,
            style: self.style.clone(),
            place: self.place.clone(),
            widgets: self.widgets.clone(),
            index: self.index.clone(),
        }
    }
}

impl<State: Send + Sync + 'static> Widgets<State> {
    /// Create an empty tool with scene name `scene` and z-order `z`. The `place` closure reads
    /// each widget's rectangle from app state by id, in scene coordinates. Return `None` to
    /// hide a widget.
    pub fn new(
        scene: &str,
        z: i32,
        place: impl Fn(&State, &str) -> Option<Rect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            style: Arc::new(WidgetStyle::default()),
            place: Arc::new(place),
            widgets: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Add `widget` above the existing widgets.
    ///
    /// # Panics
    ///
    /// Panics if another widget has the same id.
    pub fn with(mut self, widget: impl Widget<State>) -> Self {
        let id = widget.id().to_string();
        assert!(
            !self.index.contains_key(&id),
            "two widgets are named {id:?}"
        );
        self.index.insert(id, self.widgets.len());
        self.widgets.push(Arc::new(widget));
        self
    }

    /// Set how the widgets look. Defaults to [`WidgetStyle::default`].
    pub fn with_style(mut self, style: WidgetStyle) -> Self {
        self.style = Arc::new(style);
        self
    }

    /// Build a column named `container` for the group box `id`, with its padding and a title
    /// leaf above `children`. Rows use their content's height, and the title sets the minimum
    /// column width. The column uses a separate layout table to preserve padding when nested.
    /// Place the group box in the container's region.
    ///
    /// # Panics
    ///
    /// Panics if no widget has the given `id`.
    pub fn group_box_layout<Id>(
        &self,
        id: &str,
        container: Id,
        engine: &LabelEngine,
        children: impl IntoIterator<Item = Layout<Id>>,
    ) -> Column<Id> {
        let widget = self
            .find(id)
            .unwrap_or_else(|| panic!("no widget is named {id:?}"));
        let cx = WidgetContext {
            engine,
            style: &self.style,
        };
        let title = Size::new(
            widget.max_content(Direction::Horizontal, &cx),
            widget.max_content(Direction::Vertical, &cx),
        );
        let title = Leaf::new().min_size(title).max_content(title);
        Column::new(std::iter::once(Layout::from(title)).chain(children))
            .id(container)
            .padding(widget.padding(&self.style))
            .tracks(Track::auto())
            .share(false)
    }

    /// Create a layout measurement adapter that looks up widgets by id.
    pub fn measure<'a>(&'a self, engine: &'a LabelEngine) -> WidgetMeasure<'a, State> {
        WidgetMeasure {
            widgets: self,
            cx: WidgetContext {
                engine,
                style: &self.style,
            },
        }
    }

    pub(crate) fn find(&self, id: &str) -> Option<&dyn ErasedWidget<State>> {
        let index = *self.index.get(id)?;
        Some(&*self.widgets[index])
    }
}

/// Layout measurements for widgets by id. Unknown ids return `None`.
pub struct WidgetMeasure<'a, State> {
    widgets: &'a Widgets<State>,
    cx: WidgetContext<'a>,
}

impl<State: Send + Sync + 'static> Measure<String> for WidgetMeasure<'_, State> {
    fn height(&mut self, id: &String, width: f32) -> Option<Measurement> {
        Some(self.widgets.find(id)?.height(width, &self.cx))
    }

    fn width(&mut self, id: &String, height: f32) -> Option<Measurement> {
        Some(self.widgets.find(id)?.width(height, &self.cx))
    }

    fn max_content(&mut self, id: &String, direction: Direction) -> Option<f32> {
        Some(self.widgets.find(id)?.max_content(direction, &self.cx))
    }
}

/// Interaction state or a view, with its type erased.
pub(crate) type Erased = Box<dyn Any + Send + Sync>;

/// A widget, with the types of its state and view erased.
pub(crate) trait ErasedWidget<State>: Send + Sync {
    fn id(&self) -> &str;
    fn new_state(&self) -> Erased;
    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32;
    fn height(&self, width: f32, cx: &WidgetContext<'_>) -> Measurement;
    fn width(&self, height: f32, cx: &WidgetContext<'_>) -> Measurement;
    fn padding(&self, style: &WidgetStyle) -> Edges;
    fn view(&self, app: &State, own: &Erased) -> Erased;
    fn same_view(&self, view: &Erased, other: &Erased) -> bool;
    fn draw(
        &self,
        view: &Erased,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError>;
    fn enabled(&self, app: &State) -> bool;
    fn focusable(&self) -> bool;
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        placed: &Placed,
        app: &State,
        own: &Erased,
    ) -> Disposition;
    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut Erased,
    ) -> Rebuild;
    fn cursor(&self, look: Look, own: &Erased) -> Option<CursorIcon>;
}

fn own<W: Widget<State>, State>(own: &Erased) -> &W::State {
    own.downcast_ref().expect("a widget's state has its type")
}

fn view<W: Widget<State>, State>(view: &Erased) -> &W::View {
    view.downcast_ref().expect("a widget's view has its type")
}

impl<State, W: Widget<State>> ErasedWidget<State> for W {
    fn id(&self) -> &str {
        Widget::id(self)
    }

    fn new_state(&self) -> Erased {
        Box::new(W::State::default())
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        Widget::max_content(self, direction, cx)
    }

    fn height(&self, width: f32, cx: &WidgetContext<'_>) -> Measurement {
        Widget::height(self, width, cx)
    }

    fn width(&self, height: f32, cx: &WidgetContext<'_>) -> Measurement {
        Widget::width(self, height, cx)
    }

    fn padding(&self, style: &WidgetStyle) -> Edges {
        Widget::padding(self, style)
    }

    fn view(&self, app: &State, state: &Erased) -> Erased {
        Box::new(Widget::view(self, app, own::<W, State>(state)))
    }

    fn same_view(&self, a: &Erased, b: &Erased) -> bool {
        view::<W, State>(a) == view::<W, State>(b)
    }

    fn draw(
        &self,
        drawn: &Erased,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        Widget::draw(self, view::<W, State>(drawn), placed, cx)
    }

    fn enabled(&self, app: &State) -> bool {
        Widget::enabled(self, app)
    }

    fn focusable(&self) -> bool {
        Widget::focusable(self)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        placed: &Placed,
        app: &State,
        state: &Erased,
    ) -> Disposition {
        Widget::disposition(self, event, placed, app, own::<W, State>(state))
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        state: &mut Erased,
    ) -> Rebuild {
        let state = state.downcast_mut().expect("a widget's state has its type");
        Widget::event(self, event, placed, cx, app, state)
    }

    fn cursor(&self, look: Look, state: &Erased) -> Option<CursorIcon> {
        Widget::cursor(self, look, own::<W, State>(state))
    }
}
