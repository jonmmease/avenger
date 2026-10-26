//! A radio group that selects one value from a fixed list.

use std::sync::Arc;

use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_layout::{Direction, Measurement, Rect};
use avenger_scenegraph::marks::mark::SceneMark;

use crate::binding::{self, Get, Set};
use crate::draw::{cover, focus_ring, holds, label_color, rect, Bounds, Text};
use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// How a radio group arranges its items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RadioDirection {
    /// A row when the row fits the group's width, and a column otherwise.
    #[default]
    Auto,
    Row,
    Column,
}

/// A group of radio buttons that selects one value from a fixed list. Pressing and releasing on
/// the same item selects it. Arrow keys move the selection and wrap at either end. The group
/// is a single Tab stop.
pub struct RadioGroup<State, T> {
    id: String,
    label: Option<Text>,
    items: Vec<(T, Text)>,
    value: Option<Get<State, T>>,
    on_change: Option<Set<State, T>>,
    enabled: Option<Get<State, bool>>,
    direction: RadioDirection,
}

impl<State, T: Clone + PartialEq + Send + Sync + 'static> RadioGroup<State, T> {
    /// Create a group named `id` from value and plain-text label pairs in `items`.
    pub fn new(id: &str, items: impl IntoIterator<Item = (T, impl Into<String>)>) -> Self {
        Self {
            id: id.to_string(),
            label: None,
            items: (items.into_iter())
                .map(|(value, label)| (value, Text::plain(label)))
                .collect(),
            value: None,
            on_change: None,
            enabled: None,
            direction: RadioDirection::Auto,
        }
    }

    /// Show the plain-text `label` above the items. Omit it when a group box supplies the title.
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(Text::plain(label));
        self
    }

    /// Read the selected value from app state. No item is selected without this callback or
    /// when its value does not match an item.
    pub fn value(mut self, value: impl Fn(&State) -> T + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Change app state for a newly selected value, and return the scenes to rebuild.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, T) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Read the group's enabled state from app state. Defaults to enabled.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    /// Arrange items in a row, a column, or according to the available width. Defaults to
    /// [`RadioDirection::Auto`].
    pub fn direction(mut self, direction: RadioDirection) -> Self {
        self.direction = direction;
        self
    }

    fn selected(&self, app: &State) -> Option<usize> {
        let value = self.value.as_ref()?(app);
        self.items.iter().position(|(item, _)| *item == value)
    }

    fn select(&self, item: usize, app: &mut State) -> Rebuild {
        match &self.on_change {
            Some(on_change) if self.selected(app) != Some(item) => {
                on_change(app, self.items[item].0.clone())
            }
            _ => Rebuild::none(),
        }
    }

    fn sizes(&self, cx: &WidgetContext<'_>) -> Sizes {
        let style = cx.style;
        Sizes {
            label: self.label.as_ref().map(|label| label.size(cx)),
            items: (self.items.iter())
                .map(|(_, label)| {
                    let [width, height] = label.size(cx);
                    [
                        style.control_size + style.label_gap + width,
                        height.max(style.control_size),
                    ]
                })
                .collect(),
        }
    }

    /// Whether the group arranges its items in a row at `width`.
    fn in_row(&self, width: f32, sizes: &Sizes, style: &WidgetStyle) -> bool {
        match self.direction {
            RadioDirection::Row => true,
            RadioDirection::Column => false,
            RadioDirection::Auto => width + TOLERANCE >= sizes.size(true, style)[0],
        }
    }

    /// Each item's rectangle in the arrangement selected by `rect`'s width.
    fn arrange(&self, rect: Rect, sizes: &Sizes, style: &WidgetStyle) -> Vec<Bounds> {
        let row = self.in_row(rect.width, sizes, style);
        let mut at = [rect.x, rect.y + sizes.label_height(style)];
        let [across, down] = style.item_gap;
        (sizes.items.iter())
            .map(|&[width, height]| {
                let bounds = [at[0], at[1], width, height];
                if row {
                    at[0] += width + across;
                } else {
                    at[1] += height + down;
                }
                bounds
            })
            .collect()
    }

    fn item_at(&self, rect: Rect, point: [f32; 2], cx: &WidgetContext<'_>) -> Option<usize> {
        let items = self.arrange(rect, &self.sizes(cx), cx.style);
        items.iter().position(|&item| holds(item, point))
    }
}

/// Width tolerance for fitting a row despite rounding during layout.
const TOLERANCE: f32 = 0.01;

/// The sizes of a group's label and items.
struct Sizes {
    label: Option<[f32; 2]>,
    items: Vec<[f32; 2]>,
}

impl Sizes {
    /// The height of the label and the gap below it.
    fn label_height(&self, style: &WidgetStyle) -> f32 {
        self.label
            .map_or(0.0, |[_, height]| height + style.label_gap)
    }

    /// The group's size with its items in a row or a column.
    fn size(&self, row: bool, style: &WidgetStyle) -> [f32; 2] {
        let [across, down] = style.item_gap;
        let gaps = self.items.len().saturating_sub(1) as f32;
        let widths = self.items.iter().map(|[width, _]| *width);
        let heights = self.items.iter().map(|[_, height]| *height);
        let [width, height] = if row {
            [
                widths.sum::<f32>() + gaps * across,
                heights.fold(0.0, f32::max),
            ]
        } else {
            [
                widths.fold(0.0, f32::max),
                heights.sum::<f32>() + gaps * down,
            ]
        };
        let label_width = self.label.map_or(0.0, |[width, _]| width);
        [width.max(label_width), height + self.label_height(style)]
    }
}

/// A radio group's item under the pointer and the item that a press began on.
#[derive(Debug, Default)]
pub struct RadioGroupState {
    hovered: Option<usize>,
    pressed: Option<usize>,
}

/// What a radio group shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioGroupView {
    selected: Option<usize>,
    hovered: Option<usize>,
    pressed: Option<usize>,
}

impl<State: Send + Sync + 'static, T: Clone + PartialEq + Send + Sync + 'static> Widget<State>
    for RadioGroup<State, T>
{
    type State = RadioGroupState;
    type View = RadioGroupView;

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let row = self.direction != RadioDirection::Column;
        let size = self.sizes(cx).size(row, cx.style);
        match direction {
            Direction::Horizontal => size[0],
            Direction::Vertical => size[1],
        }
    }

    /// The row height if the row fits `width`, or the column height otherwise.
    fn height(&self, width: f32, cx: &WidgetContext<'_>) -> Measurement {
        let sizes = self.sizes(cx);
        let style = cx.style;
        let row_width = sizes.size(true, style)[0] - TOLERANCE;
        match self.direction {
            RadioDirection::Auto if self.in_row(width, &sizes, style) => {
                Measurement::over(sizes.size(true, style)[1], row_width..=f32::INFINITY)
            }
            RadioDirection::Auto => {
                let below = f32::NEG_INFINITY..=row_width.next_down();
                Measurement::over(sizes.size(false, style)[1], below)
            }
            direction => {
                Measurement::always(sizes.size(direction == RadioDirection::Row, style)[1])
            }
        }
    }

    /// The column's width, which is the narrowest the group can be.
    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> Measurement {
        let row = self.direction == RadioDirection::Row;
        Measurement::always(self.sizes(cx).size(row, cx.style)[0])
    }

    fn view(&self, app: &State, own: &RadioGroupState) -> RadioGroupView {
        RadioGroupView {
            selected: self.selected(app),
            hovered: own.hovered,
            pressed: own.pressed,
        }
    }

    fn draw(
        &self,
        view: &RadioGroupView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let look = placed.look;
        let color = label_color(style, look);
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(label) = &self.label {
            label.bounds(cx)?;
            let at = [placed.rect.x, placed.rect.y];
            marks.push(label.mark("radio_group_label", at, color, style).into());
        }
        let sizes = self.sizes(cx);
        let items = self.arrange(placed.rect, &sizes, style);
        let size = style.control_size;
        for (index, ((_, label), [x, y, width, height])) in self.items.iter().zip(items).enumerate()
        {
            let bounds = label.bounds(cx)?;
            let hovered = look.hovered && view.hovered == Some(index);
            let item = Look {
                hovered,
                pressed: look.pressed && hovered && view.pressed == Some(index),
                ..look
            };
            let selected = view.selected == Some(index);
            let palette = if selected {
                &style.accent
            } else {
                &style.neutral
            };
            let paint = palette.paint(item);
            let circle = [x, y + (height - size) / 2.0, size, size];
            marks.push(
                rect(
                    "radio",
                    circle,
                    paint.fill,
                    paint.border,
                    style.border_width,
                    size / 2.0,
                )
                .into(),
            );
            if selected {
                let dot = size * 0.4;
                let [mid_x, mid_y] = [circle[0] + size / 2.0, circle[1] + size / 2.0];
                let bounds = [mid_x - dot / 2.0, mid_y - dot / 2.0, dot, dot];
                marks.push(
                    rect(
                        "radio_dot",
                        bounds,
                        paint.foreground,
                        [0.0; 4],
                        0.0,
                        dot / 2.0,
                    )
                    .into(),
                );
            }
            let at = [
                x + size + style.label_gap,
                y + (height - bounds.height) / 2.0,
            ];
            marks.push(label.mark("radio_label", at, color, style).into());
            marks.push(cover([x, y, width, height]));
            if look.focus_visible && index == view.selected.unwrap_or(0) {
                marks.push(focus_ring(circle, size / 2.0, style));
            }
        }
        Ok(marks)
    }

    fn enabled(&self, app: &State) -> bool {
        binding::enabled(&self.enabled, app)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &RadioGroupState,
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(press) if step(press.key).is_some() => Disposition::Consume,
            SceneGraphEvent::KeyPress(_) | SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut RadioGroupState,
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => event,
            WidgetEvent::Cancel => {
                own.pressed = None;
                return Rebuild::none();
            }
            WidgetEvent::Focus | WidgetEvent::Blur => return Rebuild::none(),
        };
        match event {
            SceneGraphEvent::CursorMoved(moved) => {
                own.hovered = self.item_at(placed.rect, moved.position, cx);
            }
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                own.pressed = self.item_at(placed.rect, down.position, cx);
            }
            SceneGraphEvent::MouseUp(up) if up.button == MouseButton::Left => {
                let pressed = own.pressed.take();
                let released = self.item_at(placed.rect, up.position, cx);
                if let Some(item) = pressed.filter(|_| pressed == released) {
                    return self.select(item, app);
                }
            }
            SceneGraphEvent::KeyPress(press) => {
                let (Some(step), false) = (step(press.key), self.items.is_empty()) else {
                    return Rebuild::none();
                };
                let count = self.items.len() as isize;
                let item = match self.selected(app) {
                    Some(selected) => (selected as isize + step).rem_euclid(count),
                    None if step > 0 => 0,
                    None => count - 1,
                };
                return self.select(item as usize, app);
            }
            _ => {}
        }
        Rebuild::none()
    }
}

/// How far an arrow key moves the selection: forward for right and down, back for left and up.
fn step(key: Key) -> Option<isize> {
    match key {
        Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => Some(1),
        Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => Some(-1),
        _ => None,
    }
}
