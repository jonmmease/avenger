//! A group box, which frames widgets that the app's layout places inside it.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_eventstream::stream::Rebuild;
use avenger_layout::{Direction, Edges};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use lyon_path::geom::point;
use lyon_path::Path;

use crate::binding::{self, Get};
use crate::draw::{label_color, Text};
use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// A border with a title that breaks its top edge, around widgets that the app's layout places
/// inside it. It contains no widgets: the app makes a container node in its layout with the
/// group box's id and [`Widgets::padding`](crate::Widgets::padding) as its padding, and puts a
/// leaf for the title first in the container, which the app's measurer answers as the group box.
/// So a title wider than the widgets in the box widens the box.
///
/// Add a group box to the `Widgets` tool before the widgets inside it, so that it draws under
/// them and they are topmost for the pointer. It consumes presses inside it that miss its
/// widgets, so they don't reach the scenes below. It is not a Tab stop.
pub struct GroupBox<State> {
    id: String,
    title: Text,
    enabled: Option<Get<State, bool>>,
}

impl<State> GroupBox<State> {
    /// Create a group box named `id` with the plain-text `title`.
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: Text::plain(title),
            enabled: None,
        }
    }

    /// Read from app state whether the box draws enabled. Without it, it always does.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }
}

/// The space on each side of the title, where the top border breaks.
fn margin(style: &WidgetStyle) -> f32 {
    style.label_gap
}

impl<State: Send + Sync + 'static> Widget<State> for GroupBox<State> {
    type State = ();
    type View = ();

    fn id(&self) -> &str {
        &self.id
    }

    /// The title leaf's size: the title with its margins, and the gap below it.
    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let [width, height] = self.title.size(cx);
        match direction {
            Direction::Horizontal => width + 2.0 * margin(cx.style),
            Direction::Vertical => height + cx.style.label_gap,
        }
    }

    /// The border and the space inside it, on every side but the top, where the title leaf
    /// stands.
    fn padding(&self, style: &WidgetStyle) -> Edges {
        let side = style.border_width + style.group_padding;
        Edges::new(side, 0.0, side, side)
    }

    fn view(&self, _app: &State, _own: &()) {}

    fn draw(
        &self,
        _view: &(),
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let bounds = self.title.bounds(cx)?;
        let r = placed.rect;
        let inset = style.border_width / 2.0;
        let [left, right] = [r.x + inset, r.right() - inset];
        let [top, bottom] = [r.y + bounds.height / 2.0, r.bottom() - inset];
        let side = Widget::<State>::padding(self, style).left;
        let gap = [r.x + side, r.x + side + bounds.width + 2.0 * margin(style)];
        let radius = style
            .corner_radius
            .min((right - left) / 2.0)
            .min((bottom - top) / 2.0)
            .max(0.0);

        // From the end of the title's gap, clockwise around to its start.
        let mut path = Path::builder();
        path.begin(point(gap[1].min(right - radius), top));
        path.line_to(point(right - radius, top));
        path.quadratic_bezier_to(point(right, top), point(right, top + radius));
        path.line_to(point(right, bottom - radius));
        path.quadratic_bezier_to(point(right, bottom), point(right - radius, bottom));
        path.line_to(point(left + radius, bottom));
        path.quadratic_bezier_to(point(left, bottom), point(left, bottom - radius));
        path.line_to(point(left, top + radius));
        path.quadratic_bezier_to(point(left, top), point(left + radius, top));
        path.line_to(point(gap[0].max(left + radius), top));
        path.end(false);

        let enabled = Look {
            enabled: placed.look.enabled,
            ..Look::default()
        };
        let border = ScenePathMark {
            name: "group_box".to_string(),
            path: path.build().into(),
            fill: ColorOrGradient::Color([0.0; 4]).into(),
            stroke: ColorOrGradient::Color(style.neutral.paint(enabled).border).into(),
            stroke_width: Some(style.border_width),
            ..Default::default()
        };
        let at = [gap[0] + margin(style), r.y];
        let color = label_color(style, placed.look);
        Ok(vec![
            border.into(),
            self.title.mark("group_box_title", at, color, style).into(),
        ])
    }

    fn enabled(&self, app: &State) -> bool {
        binding::enabled(&self.enabled, app)
    }

    fn focusable(&self) -> bool {
        false
    }

    fn event(
        &self,
        _event: WidgetEvent<'_>,
        _placed: &Placed,
        _cx: &WidgetContext<'_>,
        _app: &mut State,
        _own: &mut (),
    ) -> Rebuild {
        Rebuild::none()
    }
}
