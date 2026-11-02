use arrow::error::ArrowError;

/// An error of a selection: of a pixel grid or of a condition.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    /// A pixel grid that cannot compute cells: a cell that is not positive and finite, a scale
    /// without a numeric range, or values or box edges that the scale does not map.
    #[error("{0}")]
    Grid(String),
    /// A condition that breaks a rule of its kind, such as a set whose lists of values differ in
    /// length.
    #[error("{0}")]
    Condition(String),
    #[error(transparent)]
    Arrow(#[from] ArrowError),
}
