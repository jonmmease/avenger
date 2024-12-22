use std::collections::HashSet;

use avenger_scales3::error::AvengerScaleError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGuidesError {
    #[error("Invalid mix of legend encoding lengths: {0:?}")]
    InvalidLegendLength(HashSet<usize>),

    #[error("Invalid scale: {0}")]
    InvalidScale(#[from] AvengerScaleError),
}
