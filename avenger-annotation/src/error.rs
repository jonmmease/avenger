use avenger_typst_label::LabelError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerAnnotationError {
    #[error("Text error: {0}")]
    Text(#[from] LabelError),

    #[error("Leaders need one entry per label: {labels} labels, {leaders} entries")]
    LeaderCount { labels: usize, leaders: usize },
}
