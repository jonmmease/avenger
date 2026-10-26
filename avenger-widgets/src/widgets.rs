//! The set of widgets that one tool draws, and the measurer that the app's layout asks about
//! them.

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
/// The app places the widgets: its layout measures each widget through
/// [`measure`](Widgets::measure), and [`place`](Widgets::place) reads each widget's rectangle
/// back from app state. A widget without a rectangle is absent. Widgets draw in the order that
/// they were added, so a later widget is on top for the pointer, and Tab visits them in that
/// order.
///
/// Clones share the widgets, so an app keeps a clone to measure the widgets in its build and
/// installs the original as a tool.
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
    /// Create a tool whose scene is named `scene`, at z-order `z`, with no widgets.
    pub fn new(scene: &str, z: i32) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            style: Arc::new(WidgetStyle::default()),
            place: Arc::new(|_: &State, _: &str| None),
            widgets: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Add `widget`, above the widgets added before it. Panics if a widget with the same id was
    /// added.
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

    /// Read each widget's rectangle, in scene coordinates, from app state by its id, or `None`
    /// for a widget that isn't shown.
    pub fn place(
        mut self,
        place: impl Fn(&State, &str) -> Option<Rect> + Send + Sync + 'static,
    ) -> Self {
        self.place = Arc::new(place);
        self
    }

    /// Set how the widgets look. Defaults to [`WidgetStyle::default`].
    pub fn with_style(mut self, style: WidgetStyle) -> Self {
        self.style = Arc::new(style);
        self
    }

    /// Lay out `children` inside the group box named `id`: a column named `container`, with the
    /// box's padding, a leaf for the box's title above the children, and rows that take their
    /// content's height. Place the group box in the container's region. The title leaf keeps the
    /// column at least as wide as the title, and the column starts its own table, so it takes
    /// its padding wherever the app nests it. Panics if no widget is named `id`.
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

    /// A measurer for the app's layout, which answers for each widget by its id.
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

/// Answers the app's layout about widgets by their ids. Ids of no widget answer `None`.
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
