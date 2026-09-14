use std::collections::HashSet;

use avenger_scales::error::AvengerScaleError;
use avenger_text::error::AvengerTextError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGuidesError {
    #[error("Invalid mix of legend encoding lengths: {0:?}")]
    InvalidLegendLength(HashSet<usize>),

    #[error("Invalid scale: {0}")]
    InvalidScale(#[from] AvengerScaleError),

    #[error("Text error: {0}")]
    Text(#[from] AvengerTextError),

    #[error("Invalid axis ticks: {0}")]
    InvalidAxisTicks(String),

    #[error("Invalid axis label format: {0}")]
    InvalidAxisLabelFormat(String),
}
