use arrow::error::ArrowError;
use avenger_chart::ChartError;

/// An error of evaluating a chart.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// A chart that the evaluator does not draw, or a failure of a part that draws it.
    #[error("{0}")]
    Evaluation(String),
    /// A part of the chart that does not decode.
    #[error(transparent)]
    Chart(#[from] ChartError),
    #[error(transparent)]
    Arrow(#[from] ArrowError),
}
