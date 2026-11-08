//! The widgets of a chart: the controls that layers declare under keys, which write clauses of the
//! chart's selections or turn the chart's modes on and off. The copies of one key that share a widget are its
//! group, whose spec, domain, and title the evaluator prepares once, and whose value is the clause
//! of its identity in the state of its selection, or whether the chart's modes are on. A widget draws once on the
//! node that holds its group, or once beside each plot of the group when the node's part rule for
//! widgets keeps it.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{new_null_array, Array, ArrayRef, AsArray, Float64Array, RecordBatch, UInt32Array},
    compute::{cast, concat, sort_to_indices, take},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    proto::{self, widget, widget_domain},
    resolve::{Identity, Resolved, WidgetGroup},
    Chart,
};
use avenger_eventstream::{
    scene::SceneGraphEvent,
    stream::{Disposition, Rebuild},
};
use avenger_guides::GuideSide;
use avenger_layout::{Direction, Measurement, Rect};
use avenger_scales::scalar::Scalar;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_selection::{Clause, Condition, SelectionState};
use avenger_style::{ComputedStyle, Property};
use avenger_typst_label::LabelEngine;
use avenger_widgets::{
    Checkbox, CheckboxList, CheckboxListState, CheckboxListView, CheckboxState, CheckboxView, Look,
    Paint, Palette, Placed, RadioDirection, RadioGroup, RadioGroupState, RadioGroupView,
    RangeSlider, RangeSliderState, RangeSliderView, Slider, SliderState, SliderView, Widget,
    WidgetContext, WidgetError, WidgetEvent, WidgetStyle,
};

use crate::{
    marks::font_family,
    style::{color as style_color, number as style_number},
    EvalError, EvalWarning,
};

/// How a slider compares its target's column with its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderComparison {
    /// The rows in the bin of the slider's step at its value.
    Equal,
    /// The rows at or above its value.
    AtLeast,
    /// The rows at or below its value.
    AtMost,
}

/// What a widget is, with its domain fitted.
#[derive(Clone, Debug)]
pub enum WidgetSpec {
    /// One thumb over `domain`, which writes a range by its comparison.
    Slider {
        comparison: SliderComparison,
        domain: [f64; 2],
        step: Option<f64>,
        readout: String,
    },
    /// Two thumbs over `domain`, which write the range between them.
    RangeSlider {
        domain: [f64; 2],
        step: Option<f64>,
        readout: String,
    },
    /// One of `items`, with a label for each, and the label of the item that clears the widget.
    Radio {
        items: ArrayRef,
        labels: Vec<String>,
        clear: Option<String>,
    },
    /// Any of `items`, with a label for each.
    Checkboxes {
        items: ArrayRef,
        labels: Vec<String>,
    },
    /// One check box, which writes `value` while it is checked.
    Checkbox { value: ArrayRef },
    /// One of the `modes` of a group of the chart's modes, with a label for each, which turns on
    /// the chosen mode.
    ModeGroup {
        modes: Vec<String>,
        labels: Vec<String>,
    },
    /// One check box, which turns the toggle `mode` on and off.
    ModeToggle { mode: String },
}

/// The value that a widget shows: what the clause of its identity holds, or its clear state.
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetValue {
    /// The value of a slider.
    Number(f64),
    /// The ends of a range slider.
    Range([f64; 2]),
    /// The position of the item of a radio group, or `None` for its clearing item, or the position
    /// of the mode that is on in the group of a mode widget.
    Item(Option<usize>),
    /// The positions of the checked items of a check box list.
    Items(Vec<usize>),
    /// Whether a check box is checked.
    Checked(bool),
}

/// What a widget writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetTarget {
    /// The widget's clause of `selection`, over `column`.
    Selection { selection: String, column: String },
    /// The chart's modes.
    Modes,
}

/// A widget of an evaluation: its identity, what it writes, its title, its spec, its style, and
/// the rectangle of each of its drawings in the scene.
#[derive(Clone, Debug)]
pub struct EvaluatedWidget {
    pub key: String,
    /// The anchor of the widget's group, or `None` for a group that anchors at the root.
    pub anchor: Option<Identity>,
    pub target: WidgetTarget,
    /// The title, or empty for a widget without one.
    pub title: String,
    pub spec: WidgetSpec,
    pub style: WidgetStyle,
    /// The side of the plot or the concat that the widget stands on, along which the items of a
    /// radio group or a check box list run.
    pub side: GuideSide,
    /// The left, top, width, and height of each drawing.
    pub rects: Vec<[f32; 4]>,
}

impl EvaluatedWidget {
    /// The owner of the widget's clause: its key joined with the id of its anchor.
    pub fn owner(&self) -> String {
        Identity::owner_name(
            &self.key,
            self.anchor.as_ref().map_or("", |anchor| anchor.id.as_str()),
        )
    }

    /// The name of the selection that the widget writes, or `None` for a mode widget.
    pub fn selection(&self) -> Option<&str> {
        match &self.target {
            WidgetTarget::Selection { selection, .. } => Some(selection),
            WidgetTarget::Modes => None,
        }
    }

    /// The clause of the widget in `state`, the state of its selection.
    pub fn clause<'a>(&self, state: Option<&'a SelectionState>) -> Option<&'a Clause> {
        let position = self.position(state?)?;
        Some(&state?.clauses[position])
    }

    /// The value that the widget shows for `state`, the state of its selection, with `on`, which
    /// tells whether a mode of the chart is on.
    pub(crate) fn value(
        &self,
        state: Option<&SelectionState>,
        on: impl Fn(&str) -> bool,
    ) -> WidgetValue {
        match (&self.target, &self.spec) {
            (WidgetTarget::Modes, WidgetSpec::ModeGroup { modes, .. }) => {
                WidgetValue::Item(modes.iter().position(|name| on(name)))
            }
            (WidgetTarget::Modes, WidgetSpec::ModeToggle { mode }) => {
                WidgetValue::Checked(on(mode))
            }
            (WidgetTarget::Selection { column, .. }, spec) => {
                spec.value(column, self.clause(state))
            }
            (WidgetTarget::Modes, _) => WidgetValue::Item(None),
        }
    }

    /// The clause that the widget writes for `value`, or `None` in its clear state and for a mode
    /// widget.
    pub fn clause_for(&self, value: &WidgetValue) -> Option<Clause> {
        let WidgetTarget::Selection { column, .. } = &self.target else {
            return None;
        };
        let mut clause = Clause::new(self.owner());
        clause.partition = self.partition();
        clause.conditions = self.spec.conditions(column, value)?;
        Some(clause)
    }

    /// Writes the widget's clause for `value` to `state`, the state of its selection, in place of
    /// its clause, or removes its clause in its clear state.
    pub(crate) fn write(&self, state: &mut SelectionState, value: &WidgetValue) {
        let position = self.position(state);
        match (position, self.clause_for(value)) {
            (Some(position), Some(clause)) => state.clauses[position] = clause,
            (None, Some(clause)) => state.clauses.push(clause),
            (Some(_), None) => {
                let (owner, partition) = (self.owner(), self.partition());
                (state.clauses)
                    .retain(|clause| clause.owner != owner || clause.partition != partition);
            }
            (None, None) => {}
        }
    }

    /// The position of the widget's clause among the clauses of `state`.
    fn position(&self, state: &SelectionState) -> Option<usize> {
        let owner = self.owner();
        let partition = self.partition();
        (state.clauses.iter())
            .position(|clause| clause.owner == owner && clause.partition == partition)
    }

    /// The partition values of the widget's anchor.
    fn partition(&self) -> BTreeMap<String, Scalar> {
        (self.anchor.iter().flat_map(|anchor| &anchor.partition))
            .map(|(column, value)| (column.clone(), Scalar(value.clone())))
            .collect()
    }
}

/// The numbers of `values`, without nulls.
fn numbers(values: &ArrayRef) -> Vec<f64> {
    match cast(values, &DataType::Float64) {
        Ok(values) => values
            .as_primitive::<Float64Type>()
            .iter()
            .flatten()
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The text of each value of `values`, empty for a null.
fn texts(values: &ArrayRef) -> Vec<String> {
    match cast(values, &DataType::Utf8) {
        Ok(text) => (text.as_string::<i32>().iter())
            .map(|value| value.unwrap_or_default().to_string())
            .collect(),
        Err(_) => vec![String::new(); values.len()],
    }
}

impl WidgetSpec {
    /// The value that the widget shows for `clause`, its clause over `column`: the clause's range
    /// or set, or the clear state without a clause. A mode widget, whose value is whether the
    /// chart's modes are on and not a clause, shows no mode on.
    pub fn value(&self, column: &str, clause: Option<&Clause>) -> WidgetValue {
        let condition = clause.and_then(|clause| {
            (clause.conditions.iter()).find(|condition| condition.columns() == [column])
        });
        let [low, high] = match condition {
            Some(Condition::Range { ends, .. }) => match cast(ends, &DataType::Float64) {
                Ok(ends) => {
                    let ends = ends.as_primitive::<Float64Type>();
                    [0, 1].map(|end| {
                        (ends.len() > end && ends.is_valid(end)).then(|| ends.value(end))
                    })
                }
                Err(_) => [None, None],
            },
            _ => [None, None],
        };
        let set = match condition {
            Some(Condition::Set { values, .. }) => Some(texts(&values[0])),
            _ => None,
        };
        match self {
            Self::Slider {
                comparison, domain, ..
            } => WidgetValue::Number(match comparison {
                SliderComparison::Equal | SliderComparison::AtLeast => low.unwrap_or(domain[0]),
                SliderComparison::AtMost => high.unwrap_or(domain[1]),
            }),
            Self::RangeSlider { domain, .. } => {
                WidgetValue::Range([low.unwrap_or(domain[0]), high.unwrap_or(domain[1])])
            }
            Self::Radio { items, .. } => WidgetValue::Item(set.and_then(|set| {
                let first = set.first()?;
                texts(items).iter().position(|item| item == first)
            })),
            Self::Checkboxes { items, .. } => WidgetValue::Items(match set {
                Some(set) => (texts(items).iter().enumerate())
                    .filter(|(_, item)| set.contains(item))
                    .map(|(index, _)| index)
                    .collect(),
                // Every box is checked in the clear state, which keeps every row.
                None => (0..items.len()).collect(),
            }),
            Self::Checkbox { .. } => WidgetValue::Checked(set.is_some()),
            Self::ModeGroup { .. } => WidgetValue::Item(None),
            Self::ModeToggle { .. } => WidgetValue::Checked(false),
        }
    }

    /// The conditions that the widget writes over `column` for `value`, or `None` in its clear
    /// state and for a mode widget. A slider writes a range whose end at the end of its domain is
    /// open, so the range keeps the rows past the domain, and the other widgets write a set.
    pub fn conditions(&self, column: &str, value: &WidgetValue) -> Option<Vec<Condition>> {
        let range = |low: Option<f64>, high: Option<f64>, exclude_end: bool| {
            (low.is_some() || high.is_some()).then(|| Condition::Range {
                column: column.to_string(),
                channel: None,
                ends: Arc::new(Float64Array::from(vec![low, high])) as ArrayRef,
                exclude_end,
                cells: None,
            })
        };
        let set = |values: ArrayRef| Condition::values(column.to_string(), None, values);
        let condition = match (self, value) {
            (
                Self::Slider {
                    comparison,
                    domain,
                    step,
                    ..
                },
                WidgetValue::Number(value),
            ) => match comparison {
                // Without a step, the bin is the value alone.
                SliderComparison::Equal => match step.filter(|step| *step > 0.0) {
                    Some(step) => range(Some(*value), Some(value + step), true),
                    None => range(Some(*value), Some(*value), false),
                },
                SliderComparison::AtLeast => {
                    range((*value > domain[0]).then_some(*value), None, false)
                }
                SliderComparison::AtMost => {
                    range(None, (*value < domain[1]).then_some(*value), false)
                }
            },
            (Self::RangeSlider { domain, .. }, WidgetValue::Range([low, high])) => range(
                (*low > domain[0]).then_some(*low),
                (*high < domain[1]).then_some(*high),
                false,
            ),
            (Self::Radio { items, .. }, WidgetValue::Item(Some(item))) if *item < items.len() => {
                Some(set(items.slice(*item, 1)))
            }
            (Self::Checkboxes { items, .. }, WidgetValue::Items(checked))
                if checked.len() < items.len() =>
            {
                let indices: UInt32Array = checked.iter().map(|item| *item as u32).collect();
                take(items.as_ref(), &indices, None).ok().map(set)
            }
            (Self::Checkbox { value }, WidgetValue::Checked(true)) => Some(set(value.clone())),
            _ => None,
        };
        condition.map(|condition| vec![condition])
    }
}

/// The name of a widget's kind, the `type` of its `widget` style node.
pub(crate) fn kind_name(kind: &widget::Kind) -> &'static str {
    match kind {
        widget::Kind::Slider(_) => "slider",
        widget::Kind::RangeSlider(_) => "range_slider",
        widget::Kind::Radio(_) => "radio",
        widget::Kind::Checkboxes(_) => "checkboxes",
        widget::Kind::Checkbox(_) => "checkbox",
        widget::Kind::Mode(_) => "mode",
    }
}

// ---------------------------------------------------------------------------------------------
// Controls

type Read<State> = Arc<dyn Fn(&State) -> WidgetValue + Send + Sync>;
type Write<State> = Arc<dyn Fn(&mut State, WidgetValue) -> Rebuild + Send + Sync>;

/// The avenger-widgets control of a widget's spec, over a value that `State` holds: the
/// evaluator draws one over the value alone, and an app one over its own state.
pub enum Control<State> {
    Slider(Slider<State>),
    Range(RangeSlider<State>),
    Radio(RadioGroup<State, Option<usize>>),
    Checkboxes(CheckboxList<State, usize>),
    Checkbox(Checkbox<State>),
}

/// The interaction state of a [`Control`].
#[derive(Debug, Default)]
pub enum ControlState {
    #[default]
    None,
    Slider(SliderState),
    Range(RangeSliderState),
    Radio(RadioGroupState),
    Checkboxes(CheckboxListState),
    Checkbox(CheckboxState),
}

/// What a [`Control`] shows.
#[derive(Debug, PartialEq)]
pub enum ControlView {
    Slider(SliderView),
    Range(RangeSliderView),
    Radio(RadioGroupView),
    Checkboxes(CheckboxListView),
    Checkbox(CheckboxView),
}

impl<State: Send + Sync + 'static> Control<State> {
    /// The control of `widget`, named by its owner and titled by its title, without a title when
    /// its title is empty, whose items, for a radio group or a check box list, run along its side.
    /// It reads its value from the state with `read` and writes each new value with `write`. A slider's gesture, which writes many
    /// values, ends with `settle` and its last value, and each change of the other kinds is a
    /// gesture of its own, which `write` alone ends.
    pub fn new(
        widget: &EvaluatedWidget,
        read: impl Fn(&State) -> WidgetValue + Send + Sync + 'static,
        write: impl Fn(&mut State, WidgetValue) -> Rebuild + Send + Sync + 'static,
        settle: impl Fn(&mut State, WidgetValue) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        let owner = widget.owner();
        let (id, title, spec) = (owner.as_str(), widget.title.as_str(), &widget.spec);
        let direction = match widget.side.is_vertical() {
            true => RadioDirection::Column,
            false => RadioDirection::Row,
        };
        let read: Read<State> = Arc::new(read);
        let write: Write<State> = Arc::new(write);
        let settle: Write<State> = Arc::new(settle);
        match spec {
            WidgetSpec::Slider {
                domain,
                step,
                readout,
                ..
            } => {
                let mut slider = Slider::new(id, domain[0]..=domain[1]);
                if !title.is_empty() {
                    slider = slider.label(title);
                }
                if !readout.is_empty() {
                    slider = slider.readout(readout);
                }
                if let Some(step) = step {
                    slider = slider.step(*step);
                }
                let start = domain[0];
                Self::Slider(
                    slider
                        .value(move |state| match read(state) {
                            WidgetValue::Number(value) => value,
                            _ => start,
                        })
                        .on_change(move |state, value| write(state, WidgetValue::Number(value)))
                        .on_settle(move |state, value| settle(state, WidgetValue::Number(value))),
                )
            }
            WidgetSpec::RangeSlider {
                domain,
                step,
                readout,
            } => {
                let mut slider = RangeSlider::new(id, domain[0]..=domain[1]);
                if !title.is_empty() {
                    slider = slider.label(title);
                }
                if !readout.is_empty() {
                    slider = slider.readout(readout);
                }
                if let Some(step) = step {
                    slider = slider.step(*step);
                }
                let whole = *domain;
                Self::Range(
                    slider
                        .value(move |state| match read(state) {
                            WidgetValue::Range(range) => range,
                            _ => whole,
                        })
                        .on_change(move |state, range| write(state, WidgetValue::Range(range)))
                        .on_settle(move |state, range| settle(state, WidgetValue::Range(range))),
                )
            }
            WidgetSpec::Radio { labels, clear, .. } => {
                let clear = clear.as_deref();
                Self::Radio(radio(id, title, labels, clear, direction, read, write))
            }
            WidgetSpec::ModeGroup { labels, .. } => {
                Self::Radio(radio(id, title, labels, None, direction, read, write))
            }
            WidgetSpec::Checkboxes { labels, .. } => {
                let mut list = CheckboxList::new(id, labels.iter().cloned().enumerate());
                if !title.is_empty() {
                    list = list.label(title);
                }
                Self::Checkboxes(
                    list.direction(direction)
                        .value(move |state| match read(state) {
                            WidgetValue::Items(items) => items,
                            _ => Vec::new(),
                        })
                        .on_change(move |state, items| write(state, WidgetValue::Items(items))),
                )
            }
            WidgetSpec::Checkbox { .. } | WidgetSpec::ModeToggle { .. } => {
                let mut checkbox = Checkbox::new(id);
                if !title.is_empty() {
                    checkbox = checkbox.label(title);
                }
                Self::Checkbox(
                    checkbox
                        .value(move |state| matches!(read(state), WidgetValue::Checked(true)))
                        .on_change(move |state, checked| {
                            write(state, WidgetValue::Checked(checked))
                        }),
                )
            }
        }
    }
}

/// A radio group named `id` and titled `title`, without a title when `title` is empty, of an item
/// for each of `labels`, after an item labeled `clear` that clears it, in `direction`, which reads
/// its value with `read` and writes it with `write`.
fn radio<State: Send + Sync + 'static>(
    id: &str,
    title: &str,
    labels: &[String],
    clear: Option<&str>,
    direction: RadioDirection,
    read: Read<State>,
    write: Write<State>,
) -> RadioGroup<State, Option<usize>> {
    let clearing = clear.map(|label| (None, label.to_string()));
    let items = (labels.iter().enumerate()).map(|(index, label)| (Some(index), label.clone()));
    let mut radio = RadioGroup::new(id, clearing.into_iter().chain(items));
    if !title.is_empty() {
        radio = radio.label(title);
    }
    radio
        .direction(direction)
        .value(move |state| match read(state) {
            WidgetValue::Item(item) => item,
            _ => None,
        })
        .on_change(move |state, item| write(state, WidgetValue::Item(item)))
}

/// Evaluates `$body` with the control of `$control` bound to `$inner`.
macro_rules! each {
    ($control:expr, $inner:ident => $body:expr) => {
        match $control {
            Control::Slider($inner) => $body,
            Control::Range($inner) => $body,
            Control::Radio($inner) => $body,
            Control::Checkboxes($inner) => $body,
            Control::Checkbox($inner) => $body,
        }
    };
}

/// Evaluates `$body` with the control of `$control` bound to `$inner` and its interaction state,
/// from `$own`, bound to `$state`, or with a fresh state when `$own` holds another kind's, as after
/// the spec of an app's widget changes.
macro_rules! with_state {
    ($control:expr, $own:expr, $inner:ident, $state:ident => $body:expr) => {
        match ($control, $own) {
            (Control::Slider($inner), ControlState::Slider($state)) => $body,
            (Control::Range($inner), ControlState::Range($state)) => $body,
            (Control::Radio($inner), ControlState::Radio($state)) => $body,
            (Control::Checkboxes($inner), ControlState::Checkboxes($state)) => $body,
            (Control::Checkbox($inner), ControlState::Checkbox($state)) => $body,
            (control, _) => each!(control, $inner => {
                let $state = &mut Default::default();
                $body
            }),
        }
    };
}

impl<State: Send + Sync + 'static> Control<State> {
    /// `own` as the interaction state of the control's kind, fresh when it holds another kind's.
    fn fit_state(&self, own: &mut ControlState) {
        let fits = matches!(
            (self, &*own),
            (Control::Slider(_), ControlState::Slider(_))
                | (Control::Range(_), ControlState::Range(_))
                | (Control::Radio(_), ControlState::Radio(_))
                | (Control::Checkboxes(_), ControlState::Checkboxes(_))
                | (Control::Checkbox(_), ControlState::Checkbox(_))
        );
        if !fits {
            *own = match self {
                Control::Slider(_) => ControlState::Slider(Default::default()),
                Control::Range(_) => ControlState::Range(Default::default()),
                Control::Radio(_) => ControlState::Radio(Default::default()),
                Control::Checkboxes(_) => ControlState::Checkboxes(Default::default()),
                Control::Checkbox(_) => ControlState::Checkbox(Default::default()),
            };
        }
    }
}

impl<State: Send + Sync + 'static> Widget<State> for Control<State> {
    type State = ControlState;
    type View = ControlView;

    fn id(&self) -> &str {
        each!(self, inner => Widget::<State>::id(inner))
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        each!(self, inner => inner.max_content(direction, cx))
    }

    fn height(&self, width: f32, cx: &WidgetContext<'_>) -> Measurement {
        each!(self, inner => inner.height(width, cx))
    }

    fn width(&self, height: f32, cx: &WidgetContext<'_>) -> Measurement {
        each!(self, inner => inner.width(height, cx))
    }

    fn view(&self, app: &State, own: &ControlState) -> ControlView {
        match (self, own) {
            (Control::Slider(inner), ControlState::Slider(own)) => {
                ControlView::Slider(inner.view(app, own))
            }
            (Control::Range(inner), ControlState::Range(own)) => {
                ControlView::Range(inner.view(app, own))
            }
            (Control::Radio(inner), ControlState::Radio(own)) => {
                ControlView::Radio(inner.view(app, own))
            }
            (Control::Checkboxes(inner), ControlState::Checkboxes(own)) => {
                ControlView::Checkboxes(inner.view(app, own))
            }
            (Control::Checkbox(inner), ControlState::Checkbox(own)) => {
                ControlView::Checkbox(inner.view(app, own))
            }
            (Control::Slider(inner), _) => {
                ControlView::Slider(inner.view(app, &Default::default()))
            }
            (Control::Range(inner), _) => ControlView::Range(inner.view(app, &Default::default())),
            (Control::Radio(inner), _) => ControlView::Radio(inner.view(app, &Default::default())),
            (Control::Checkboxes(inner), _) => {
                ControlView::Checkboxes(inner.view(app, &Default::default()))
            }
            (Control::Checkbox(inner), _) => {
                ControlView::Checkbox(inner.view(app, &Default::default()))
            }
        }
    }

    fn draw(
        &self,
        view: &ControlView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        match (self, view) {
            (Control::Slider(inner), ControlView::Slider(view)) => inner.draw(view, placed, cx),
            (Control::Range(inner), ControlView::Range(view)) => inner.draw(view, placed, cx),
            (Control::Radio(inner), ControlView::Radio(view)) => inner.draw(view, placed, cx),
            (Control::Checkboxes(inner), ControlView::Checkboxes(view)) => {
                inner.draw(view, placed, cx)
            }
            (Control::Checkbox(inner), ControlView::Checkbox(view)) => inner.draw(view, placed, cx),
            // A view of another kind draws nothing until the control's next view.
            _ => Ok(Vec::new()),
        }
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        placed: &Placed,
        app: &State,
        own: &ControlState,
    ) -> Disposition {
        match (self, own) {
            (Control::Slider(inner), ControlState::Slider(own)) => {
                inner.disposition(event, placed, app, own)
            }
            (Control::Range(inner), ControlState::Range(own)) => {
                inner.disposition(event, placed, app, own)
            }
            (Control::Radio(inner), ControlState::Radio(own)) => {
                inner.disposition(event, placed, app, own)
            }
            (Control::Checkboxes(inner), ControlState::Checkboxes(own)) => {
                inner.disposition(event, placed, app, own)
            }
            (Control::Checkbox(inner), ControlState::Checkbox(own)) => {
                inner.disposition(event, placed, app, own)
            }
            _ => each!(self, inner => inner.disposition(event, placed, app, &Default::default())),
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut ControlState,
    ) -> Rebuild {
        self.fit_state(own);
        with_state!(self, own, inner, state => inner.event(event, placed, cx, app, state))
    }
}

// ---------------------------------------------------------------------------------------------
// Preparing and drawing

/// The style of a widget from the computed style of its `widget` node: its font, the color of its
/// text, and with an `accent-color` other than `auto`, the color of its checked and filled parts.
pub(crate) fn widget_style(style: &ComputedStyle) -> WidgetStyle {
    let mut widget = WidgetStyle {
        font_family: font_family(style),
        font_size: style.font_size(),
        ..WidgetStyle::default()
    };
    let text = style_color(style, Property::Color);
    for paint in [
        &mut widget.neutral.normal,
        &mut widget.neutral.hovered,
        &mut widget.neutral.pressed,
    ] {
        paint.foreground = text;
    }
    if let Some(accent) = style.color(Property::AccentColor) {
        let shade = |amount: f32| {
            let [r, g, b, a] = accent;
            [r * amount, g * amount, b * amount, a]
        };
        widget.accent = Palette::shaded(
            Paint {
                fill: accent,
                border: accent,
                foreground: widget.accent.normal.foreground,
            },
            shade(0.85),
            shade(0.72),
            widget.background,
        );
    }
    widget
}

/// A widget prepared without a size: its evaluated form without rectangles, where it stands, its
/// measurements, and its control over its value alone.
pub(crate) struct PreparedWidget {
    pub(crate) evaluated: EvaluatedWidget,
    pub(crate) side: GuideSide,
    /// The gap between the widget and what stands inside it.
    pub(crate) offset: f32,
    /// The path of the node that holds the widget's group, whose part rule for widgets decides how
    /// many times it draws.
    pub(crate) owner: String,
    /// The plots of the layers of the group, by their positions in `Tree::plots`.
    pub(crate) plots: Vec<usize>,
    /// The widget's length along its side.
    pub(crate) length: f32,
    /// The widget's thickness across its side.
    pub(crate) thickness: f32,
    control: Control<WidgetValue>,
}

/// What preparing the widgets reads.
pub(crate) struct WidgetInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) drawn: &'a [RecordBatch],
    /// The computed style of the `widget` node of each group, by its position in
    /// `Resolved::widgets`.
    pub(crate) styles: Vec<&'a ComputedStyle>,
    pub(crate) engine: &'a LabelEngine,
}

/// The values that the domain of `group` fits, with its fit and whether they are explicit: its
/// explicit values, the summaries of its copies, or its targets' columns in the drawn rows of the
/// layers of its copies.
fn domain_values(
    chart: &Chart,
    group: &WidgetGroup,
    drawn: &[RecordBatch],
) -> Option<(ArrayRef, Option<proto::DomainFit>, bool)> {
    let joined = |values: Vec<ArrayRef>| {
        let refs: Vec<&dyn Array> = values.iter().map(|values| values.as_ref()).collect();
        concat(&refs).ok()
    };
    Some(match group.widget.domain()? {
        widget_domain::Domain::Explicit(values) => (chart.list_elements(values), None, true),
        widget_domain::Domain::Summary(summary) => {
            let values = (group.members.iter())
                .filter_map(|member| member.summary.as_ref())
                .map(|values| chart.list_elements(values))
                .collect();
            (joined(values)?, summary.fit, false)
        }
        widget_domain::Domain::Data(data) => {
            let columns = (group.members.iter())
                .filter_map(|member| drawn[member.layer].column_by_name(&member.column).cloned())
                .collect();
            (joined(columns)?, data.fit, false)
        }
    })
}

/// The interval of `values` for a slider: the two explicit ends, or the least and the greatest
/// number, with zero when the fit asks for it. `None` without numbers.
fn interval(values: &ArrayRef, explicit: bool, fit: Option<&proto::DomainFit>) -> Option<[f64; 2]> {
    let numbers = numbers(values);
    if explicit {
        return (numbers.len() == 2).then(|| [numbers[0], numbers[1]]);
    }
    let low = numbers.iter().copied().fold(f64::INFINITY, f64::min);
    let high = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(low.is_finite() && high.is_finite()) {
        return None;
    }
    let zero = match fit.and_then(|fit| fit.fit.as_ref()) {
        Some(proto::domain_fit::Fit::Interval(interval)) => interval.zero.unwrap_or(false),
        _ => false,
    };
    Some(match zero {
        true => [low.min(0.0), high.max(0.0)],
        false => [low, high],
    })
}

/// The items of a radio group or a check box list from `values`: the explicit values in their
/// order, or the distinct values that are not null, in ascending order. `None` without items.
fn items(values: &ArrayRef, explicit: bool) -> Option<ArrayRef> {
    if explicit {
        return (!values.is_empty()).then(|| values.clone());
    }
    let order = sort_to_indices(values.as_ref(), None, None).ok()?;
    let sorted = take(values.as_ref(), &order, None).ok()?;
    let labels = texts(&sorted);
    let keep: UInt32Array = (0..sorted.len())
        .filter(|&row| sorted.is_valid(row) && (row == 0 || labels[row] != labels[row - 1]))
        .map(|row| row as u32)
        .collect();
    let items = take(sorted.as_ref(), &keep, None).ok()?;
    (!items.is_empty()).then_some(items)
}

/// The side of a widget: its declaration's, or the computed `orient` of its node.
fn side_of(side: i32, style: &ComputedStyle) -> GuideSide {
    match proto::Side::try_from(side).unwrap_or_default() {
        proto::Side::Top => GuideSide::Top,
        proto::Side::Bottom => GuideSide::Bottom,
        proto::Side::Left => GuideSide::Left,
        proto::Side::Right => GuideSide::Right,
        _ => match style.keyword(Property::Orient) {
            Some("bottom" | "bottom-left" | "bottom-right") => GuideSide::Bottom,
            Some("left") => GuideSide::Left,
            Some("right") => GuideSide::Right,
            _ => GuideSide::Top,
        },
    }
}

/// The spec of `group`, or the reason that the widget does not draw.
fn spec(chart: &Chart, group: &WidgetGroup, drawn: &[RecordBatch]) -> Result<WidgetSpec, String> {
    let kind = (group.widget.kind.as_ref()).ok_or("the widget has no kind")?;
    let domain = domain_values(chart, group, drawn);
    let step = |step: Option<&proto::ValueRef>| {
        step.and_then(|step| numbers(chart.value(step)).first().copied())
    };
    let format = |labels: Option<&proto::Labels>| {
        labels.map_or_else(String::new, |labels| labels.format.clone())
    };
    let interval = || {
        (domain.as_ref())
            .and_then(|(values, fit, explicit)| interval(values, *explicit, fit.as_ref()))
    };
    let items = || (domain.as_ref()).and_then(|(values, _, explicit)| items(values, *explicit));
    Ok(match kind {
        widget::Kind::Slider(slider) => WidgetSpec::Slider {
            comparison: match proto::SliderComparison::try_from(slider.comparison) {
                Ok(proto::SliderComparison::AtLeast) => SliderComparison::AtLeast,
                Ok(proto::SliderComparison::AtMost) => SliderComparison::AtMost,
                _ => SliderComparison::Equal,
            },
            domain: interval().ok_or("the slider's domain holds no numbers")?,
            step: step(slider.step.as_ref()),
            readout: format(slider.readout.as_ref()),
        },
        widget::Kind::RangeSlider(slider) => WidgetSpec::RangeSlider {
            domain: interval().ok_or("the range slider's domain holds no numbers")?,
            step: step(slider.step.as_ref()),
            readout: format(slider.readout.as_ref()),
        },
        widget::Kind::Radio(radio) => {
            let items = items().ok_or("the radio group's domain holds no items")?;
            WidgetSpec::Radio {
                labels: texts(&items),
                items,
                clear: (!radio.clear_label.is_empty()).then(|| radio.clear_label.clone()),
            }
        }
        widget::Kind::Checkboxes(_) => {
            let items = items().ok_or("the check box list's domain holds no items")?;
            WidgetSpec::Checkboxes {
                labels: texts(&items),
                items,
            }
        }
        widget::Kind::Mode(widget) => {
            match widget.modes.as_ref().ok_or("the widget has no modes")? {
                proto::mode_widget::Modes::Group(group) => {
                    let modes: Vec<&proto::Mode> = (chart.message().modes.iter())
                        .filter(|mode| &mode.group == group)
                        .collect();
                    WidgetSpec::ModeGroup {
                        modes: modes.iter().map(|mode| mode.name.clone()).collect(),
                        labels: modes.iter().map(|mode| mode_label(mode)).collect(),
                    }
                }
                proto::mode_widget::Modes::Toggle(toggle) => WidgetSpec::ModeToggle {
                    mode: toggle.clone(),
                },
            }
        }
        widget::Kind::Checkbox(checkbox) => WidgetSpec::Checkbox {
            value: checkbox.value.as_ref().map_or_else(
                || new_null_array(&DataType::Null, 1),
                |value| chart.value(value).clone(),
            ),
        },
    })
}

/// The label of `mode`: its own, or its name.
fn mode_label(mode: &proto::Mode) -> String {
    match mode.label.is_empty() {
        true => mode.name.clone(),
        false => mode.label.clone(),
    }
}

/// The widgets of a chart, with a warning for each widget that does not draw, such as a slider
/// whose domain holds no numbers. A widget whose node is hidden does not draw.
pub(crate) fn widgets(
    inputs: &WidgetInputs,
    warnings: &mut Vec<EvalWarning>,
) -> Vec<PreparedWidget> {
    let mut prepared = Vec::new();
    for (index, group) in inputs.resolved.widgets.iter().enumerate() {
        let style = inputs.styles[index];
        if style.is_hidden() {
            continue;
        }
        let spec = match spec(inputs.chart, group, inputs.drawn) {
            Ok(spec) => spec,
            Err(reason) => {
                let member = &group.members[0];
                warnings.push(EvalWarning::NotDrawn {
                    path: format!(
                        "{}.widgets[{}]",
                        inputs.resolved.tree.layers[member.layer].path, member.index
                    ),
                    reason,
                });
                continue;
            }
        };
        let target = match group.widget.target() {
            Some(target) => WidgetTarget::Selection {
                selection: target.selection.clone(),
                column: target.column.clone(),
            },
            None => WidgetTarget::Modes,
        };
        // A widget without a title takes the name of its column, a mode widget of a toggle takes
        // the toggle's label, and a mode widget of a group has none.
        let title = match (&group.widget.title, &target, &spec) {
            (Some(title), _, _) => title.text.clone(),
            (None, WidgetTarget::Selection { column, .. }, _) => column.clone(),
            (None, WidgetTarget::Modes, WidgetSpec::ModeToggle { mode }) => {
                (inputs.chart.message().modes.iter())
                    .find(|declared| &declared.name == mode)
                    .map_or_else(|| mode.clone(), mode_label)
            }
            (None, WidgetTarget::Modes, _) => String::new(),
        };
        let side = side_of(group.widget.side, style);
        let evaluated = EvaluatedWidget {
            key: group.key.clone(),
            anchor: group.anchor.clone(),
            target,
            title,
            spec,
            style: widget_style(style),
            side,
            rects: Vec::new(),
        };
        let control = Control::new(
            &evaluated,
            WidgetValue::clone,
            |_, _| Rebuild::none(),
            |_, _| Rebuild::none(),
        );
        let cx = WidgetContext {
            engine: inputs.engine,
            style: &evaluated.style,
        };
        let [along, across] = match side.is_vertical() {
            true => [Direction::Vertical, Direction::Horizontal],
            false => [Direction::Horizontal, Direction::Vertical],
        };
        let mut plots: Vec<usize> = (group.members.iter())
            .map(|member| inputs.resolved.tree.layers[member.layer].plot)
            .collect();
        plots.dedup();
        prepared.push(PreparedWidget {
            side,
            offset: style_number(style, Property::Offset),
            owner: group.owner.clone(),
            plots,
            length: control.max_content(along, &cx),
            thickness: control.max_content(across, &cx),
            control,
            evaluated,
        });
    }
    prepared
}

impl PreparedWidget {
    /// The marks of the widget at `value` in `slot`, its slab's rectangle past its offset, from the
    /// start of the slot along its side, with the rectangle that it draws in.
    pub(crate) fn draw(
        &self,
        engine: &LabelEngine,
        value: &WidgetValue,
        slot: [f32; 4],
    ) -> Result<(Vec<SceneMark>, [f32; 4]), EvalError> {
        let cx = WidgetContext {
            engine,
            style: &self.evaluated.style,
        };
        let [x, y, width, height] = slot;
        let rect = match self.side.is_vertical() {
            true => [x, y, width, height.min(self.length)],
            false => [x, y, width.min(self.length), height],
        };
        let placed = Placed {
            rect: Rect::new(rect[0], rect[1], rect[2], rect[3]),
            look: Look {
                enabled: true,
                ..Look::default()
            },
        };
        let view = self.control.view(value, &ControlState::None);
        let marks = (self.control.draw(&view, &placed, &cx))
            .map_err(|error| EvalError::Unsupported(format!("a widget that draws: {error}")))?;
        Ok((marks, rect))
    }
}
