use avenger_typst_label::LabelError;
use thiserror::Error;

/// Why a tooltip can't draw.
#[derive(Error, Debug)]
pub enum AvengerTooltipError {
    #[error("Text error: {0}")]
    Text(#[from] LabelError),

    #[error("A tooltip's field names the column {0:?}, which its focus's rows don't have")]
    Column(String),

    #[error("A group target's {marks} marks need as many titles, not {titles}")]
    Entries { marks: usize, titles: usize },
}
