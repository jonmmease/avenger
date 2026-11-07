//! The errors and the warnings of preparing and evaluating a chart.

use avenger_chart::ChartError;

/// Why a chart does not prepare or evaluate. A chart that the reader accepted is valid, so the
/// errors are the parts that the evaluator does not draw yet, the names that the renderer resolves,
/// and the failures of the parts that draw it.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// A part of the chart that this version of the evaluator does not draw, such as a concat.
    #[error("the evaluator does not draw {0} yet")]
    Unsupported(String),
    /// A custom scale whose type the registry does not name.
    #[error("the registry has no custom scale `{0}`")]
    UnknownScale(String),
    /// A format syntax of the chart that the label engine has no provider of.
    #[error("the chart formats {kind} in the syntax `{syntax}`, and the label engine's provider is `{provided}`")]
    MissingFormatter {
        kind: &'static str,
        syntax: String,
        provided: String,
    },
    #[error(transparent)]
    Chart(#[from] ChartError),
    #[error(transparent)]
    Arrow(#[from] arrow::error::ArrowError),
    #[error(transparent)]
    Scale(#[from] avenger_scales::error::AvengerScaleError),
    #[error(transparent)]
    Guide(#[from] avenger_guides::error::AvengerGuidesError),
    #[error(transparent)]
    Label(#[from] avenger_typst_label::LabelError),
    #[error(transparent)]
    Selection(#[from] avenger_selection::SelectionError),
    #[error(transparent)]
    Layout(#[from] avenger_layout::LayoutError),
}

/// A part of a chart that draws, but not as the chart says, such as a widget, which this version
/// of the evaluator does not draw, or a warning of the cascade.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalWarning {
    /// A value of the styles that does not draw as written.
    Style(avenger_style::StyleWarning),
    /// A part of the chart that the evaluator draws without, at `path`.
    NotDrawn { path: String, reason: String },
    /// Marks at `path` that reach past the edges of their plot, because no domain of a position
    /// scale keeps them inside at the plot's size.
    Fit { path: String, reason: String },
    /// A domain of the scale at `path` that takes its fallback, such as a selection domain whose
    /// selection's clauses give no values through its channel.
    Domain { path: String, reason: String },
}
