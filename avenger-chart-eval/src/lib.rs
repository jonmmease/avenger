//! # avenger-chart-eval
//!
//! The evaluation of an avenger [`Chart`](avenger_chart::Chart) to a scene graph at any size. A
//! [`ChartEvaluator`] prepares a chart once with the cascade of a style table, a label engine, and
//! a [`Registry`] of custom scales. It builds the chart's style tree, whose nodes stand for the
//! parts of the chart and carry the format's style fields as inline styles, and it computes the
//! tree's styles for the host inputs.

mod bins;
mod convert;
mod encode;
mod error;
mod evaluator;
mod fit;
mod geo;
mod guides;
mod layout;
mod marks;
mod polar;
mod polyline;
mod reach;
mod registry;
mod state;
mod style;
mod widgets;

pub use error::{EvalError, EvalWarning};
pub use evaluator::{ChartEvaluator, EvaluatedChart, EvaluatedLayer, EvaluatedPlot, PreparedChart};
pub use geo::{ProjectionNavigation, ProjectionView};
pub use registry::Registry;
pub use state::{
    Carry, Fit, Fitted, ProjectionIdentity, ScaleIdentity, ScaleNavigation, UpdateOptions, Views,
};
pub use widgets::{
    Control, ControlState, ControlView, EvaluatedWidget, SliderComparison, WidgetSpec,
    WidgetTarget, WidgetValue,
};

/// Compiles the README's examples.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
#[doc(hidden)]
pub use style::inline::STYLE_FIELDS;
