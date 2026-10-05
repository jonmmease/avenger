use std::collections::HashSet;

use arrow::datatypes::DataType;
use avenger_scales::error::AvengerScaleError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGuidesError {
    #[error("Invalid mix of legend encoding lengths: {0:?}")]
    InvalidLegendLength(HashSet<usize>),

    #[error("Invalid scale: {0}")]
    InvalidScale(#[from] AvengerScaleError),

    #[error("Numeric axes need numeric ticks, not {0}")]
    NonNumericTicks(DataType),
}
