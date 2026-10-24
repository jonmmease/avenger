use avenger_typst_label::LabelError;

/// Why a tooltip can't describe its instance.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AvengerTooltipError {
    #[error("the instance's rows have no column {0:?}")]
    MissingColumn(String),
    #[error(transparent)]
    Label(#[from] LabelError),
}
