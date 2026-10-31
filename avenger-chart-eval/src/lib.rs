//! # avenger-chart-eval
//!
//! The evaluation of an avenger [`Chart`](avenger_chart::Chart) to a scene graph. A
//! [`ChartEvaluator`] prepares a chart one time with a label engine and a [`Theme`], and
//! [`ChartEvaluator::evaluate`] evaluates it at any size to an [`EvaluatedChart`]: its scene, the
//! rows behind its marks, its plots, and the warnings of the evaluation.
//!
//! The `geo` feature draws map plots, through avenger-geo and avenger-guides-geo. Without it,
//! preparing a chart with a map plot fails with an error that names the feature.

mod encode;
mod error;
mod evaluator;
mod fit;
#[cfg(feature = "geo")]
mod geo;
#[cfg(not(feature = "geo"))]
#[path = "no_geo.rs"]
mod geo;
mod guides;
mod layout;
mod scale;
mod select;
mod theme;

pub use error::EvalError;
pub use evaluator::{ChartEvaluator, EvaluatedChart, EvaluatedPlot, EvaluationWarning, PlotCoord};
pub use theme::{MapStyle, Theme, TitleAlign, TitleStyle};
