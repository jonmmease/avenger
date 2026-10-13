use std::collections::HashSet;

use avenger_scales::error::AvengerScaleError;
use avenger_typst_label::LabelError;
use chrono_tz::Tz;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGuidesError {
    #[error("Invalid mix of legend encoding lengths: {0:?}")]
    InvalidLegendLength(HashSet<usize>),

    #[error("Invalid scale: {0}")]
    InvalidScale(#[from] AvengerScaleError),

    #[error("The text engine shows times in {formatter}, but the scale places ticks in {scale}")]
    TimezoneMismatch { scale: Tz, formatter: Tz },

    #[error("Text error: {0}")]
    Text(#[from] LabelError),
}
