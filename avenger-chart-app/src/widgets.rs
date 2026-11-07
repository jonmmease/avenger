//! The widgets of a chart in its app: a `Widgets` tool of slots, each of which draws one drawing of
//! one widget of the last evaluation at its rectangle, and writes the widget's clause to its
//! selection as its value changes.
//!
//! A `Widgets` tool holds a fixed set of widgets, and a chart update can change the widgets of the
//! chart, so the tool holds [`SLOTS`] slots, and the slot at a position draws the drawing at that
//! position among the drawings of the evaluation's widgets.

use std::sync::Arc;

use avenger_chart_eval::{Control, ControlState, ControlView, EvaluatedWidget, WidgetValue};
use avenger_eventstream::{
    scene::SceneGraphEvent,
    stream::{Disposition, Rebuild},
};
use avenger_layout::{Direction, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_selection::SelectionState;
use avenger_widgets::{
    Placed, Widget, WidgetContext, WidgetError, WidgetEvent, WidgetStyle, Widgets,
};

use crate::{
    state::{ChartAppState, ChartChange},
    CHART_SCENE, WIDGET_SCENE,
};

/// The number of drawings of widgets that an app draws, beyond which it draws none.
pub const SLOTS: usize = 64;

/// The controls of the widgets of the last evaluation, with the drawings that they draw.
#[derive(Default)]
pub(crate) struct WidgetControls {
    /// The widgets of the last evaluation.
    pub(crate) widgets: Vec<EvaluatedWidget>,
    /// The control of each widget, by its position.
    controls: Vec<Arc<Control<ChartAppState>>>,
    /// Each drawing: its widget and its rectangle.
    drawings: Vec<(usize, Rect)>,
    /// A description of the widgets that the controls were built from, which a new evaluation
    /// whose widgets differ changes.
    built: String,
    /// The count of the builds of the controls, which a slot's view holds so that a slot draws
    /// again with new controls.
    revision: u64,
}

impl WidgetControls {
    /// Takes the widgets of a new evaluation, and builds their controls again when they differ
    /// from the last.
    pub(crate) fn set(&mut self, widgets: Vec<EvaluatedWidget>) {
        let described = (widgets.iter())
            .map(|widget| {
                format!(
                    "{:?} {:?} {:?} {:?} {:?} {:?}",
                    widget.key,
                    widget.anchor,
                    widget.selection,
                    widget.title,
                    widget.spec,
                    widget.style
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.drawings = (widgets.iter().enumerate())
            .flat_map(|(index, widget)| {
                (widget.rects.iter())
                    .map(move |[x, y, width, height]| (index, Rect::new(*x, *y, *width, *height)))
            })
            .take(SLOTS)
            .collect();
        if described != self.built {
            self.controls = (widgets.iter().enumerate())
                .map(|(index, widget)| Arc::new(control(index, widget)))
                .collect();
            self.built = described;
            self.revision += 1;
        }
        self.widgets = widgets;
    }
}

/// The control of the widget `index` of the app, which reads its value from the widget's clause
/// and writes its clause.
fn control(index: usize, widget: &EvaluatedWidget) -> Control<ChartAppState> {
    Control::new(
        &widget.owner(),
        &widget.title,
        &widget.spec,
        move |state: &ChartAppState| state.widget_value(index),
        move |state: &mut ChartAppState, value| state.write_widget(index, value, false),
        move |state: &mut ChartAppState, value| state.write_widget(index, value, true),
    )
}

impl ChartAppState {
    /// The widgets of the last evaluation, with the rectangles of their drawings.
    pub fn widgets(&self) -> &[EvaluatedWidget] {
        &self.controls.widgets
    }

    /// The value of the widget `index`, from its clause.
    fn widget_value(&self, index: usize) -> WidgetValue {
        let widget = &self.controls.widgets[index];
        widget.value(self.evaluator.selections().get(&widget.selection))
    }

    /// Writes the clause of the widget `index` for `value` to its selection, and reports the
    /// selection, as settled when `settled` holds or the widget is not a slider, whose gesture
    /// settles at its end.
    fn write_widget(&mut self, index: usize, value: WidgetValue, settled: bool) -> Rebuild {
        let widget = &self.controls.widgets[index];
        let name = widget.selection.clone();
        let mut selections = self.evaluator.selections().clone();
        let resolution = self.resolution(&name);
        let state = (selections.entry(name.clone()))
            .or_insert_with(|| SelectionState::new(resolution, Vec::new()));
        widget.write(state, &value);
        let slider = matches!(value, WidgetValue::Number(_) | WidgetValue::Range(_));
        self.evaluator.set_selections(selections);
        self.touch(&name);
        self.report(ChartChange {
            settled: settled || !slider,
            ..ChartChange::default()
        });
        Rebuild::scene(CHART_SCENE)
    }
}

/// A slot of the tool: the drawing at its position.
struct Slot {
    id: String,
    position: usize,
}

/// What a slot shows: the control of its drawing and its view, the style of its widget, and the
/// revision of the controls, or nothing without a drawing.
struct SlotView {
    control: Option<Arc<Control<ChartAppState>>>,
    view: Option<ControlView>,
    style: WidgetStyle,
    revision: u64,
}

impl PartialEq for SlotView {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.view == other.view && self.style == other.style
    }
}

impl Slot {
    /// The control of the slot's drawing, with its widget's style.
    fn control<'a>(
        &self,
        state: &'a ChartAppState,
    ) -> Option<(&'a Arc<Control<ChartAppState>>, &'a WidgetStyle)> {
        let (widget, _) = state.controls.drawings.get(self.position)?;
        Some((
            state.controls.controls.get(*widget)?,
            &state.controls.widgets[*widget].style,
        ))
    }
}

impl Widget<ChartAppState> for Slot {
    type State = ControlState;
    type View = SlotView;

    fn id(&self) -> &str {
        &self.id
    }

    /// A slot stands at the rectangle of its drawing, which the evaluation measured.
    fn max_content(&self, _direction: Direction, _cx: &WidgetContext<'_>) -> f32 {
        0.0
    }

    fn view(&self, app: &ChartAppState, own: &ControlState) -> SlotView {
        match self.control(app) {
            Some((control, style)) => SlotView {
                view: Some(control.view(app, own)),
                control: Some(control.clone()),
                style: style.clone(),
                revision: app.controls.revision,
            },
            None => SlotView {
                control: None,
                view: None,
                style: WidgetStyle::default(),
                revision: app.controls.revision,
            },
        }
    }

    fn draw(
        &self,
        view: &SlotView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let (Some(control), Some(inner)) = (&view.control, &view.view) else {
            return Ok(Vec::new());
        };
        let cx = WidgetContext {
            engine: cx.engine,
            style: &view.style,
        };
        control.draw(inner, placed, &cx)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        placed: &Placed,
        app: &ChartAppState,
        own: &ControlState,
    ) -> Disposition {
        match self.control(app) {
            Some((control, _)) => control.disposition(event, placed, app, own),
            None => Disposition::Ignore,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut ChartAppState,
        own: &mut ControlState,
    ) -> Rebuild {
        let Some((control, style)) = self.control(app) else {
            return Rebuild::none();
        };
        let (control, style) = (control.clone(), style.clone());
        let cx = WidgetContext {
            engine: cx.engine,
            style: &style,
        };
        control.event(event, placed, &cx, app, own)
    }
}

/// The tool that draws the widgets of the chart and writes their clauses.
pub(crate) fn widgets() -> Widgets<ChartAppState> {
    let mut tool = Widgets::new(WIDGET_SCENE, 3, |state: &ChartAppState, id: &str| {
        let position: usize = id.strip_prefix("widget slot ")?.parse().ok()?;
        state.controls.drawings.get(position).map(|(_, rect)| *rect)
    });
    for position in 0..SLOTS {
        tool = tool.with(Slot {
            id: format!("widget slot {position}"),
            position,
        });
    }
    tool
}
