//! # avenger-chart-eval
//!
//! The evaluation of an avenger [`Chart`](avenger_chart::Chart) to a scene graph at any size. A
//! [`ChartEvaluator`] prepares a chart once with the cascade of a style table, a label engine, and
//! a [`Registry`] of custom scales. It builds the chart's style tree, whose nodes stand for the
//! parts of the chart and carry the format's style fields as inline styles, and it computes the
//! tree's styles for the host inputs.

mod convert;
mod error;
mod evaluator;
mod registry;
mod style;

pub use error::{EvalError, EvalWarning};
pub use evaluator::ChartEvaluator;
pub use registry::Registry;
#[doc(hidden)]
pub use style::inline::STYLE_FIELDS;
