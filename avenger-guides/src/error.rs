use std::collections::HashSet;

use arrow::datatypes::DataType;
use avenger_format::{FormatError, ValueKind};
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

    #[error("A {formatter} formatter cannot label {ticks} ticks")]
    FormatMismatch {
        formatter: ValueKind,
        ticks: DataType,
    },

    #[error("Axes cannot label {0} ticks")]
    UnsupportedTicks(DataType),

    #[error("The formatter displays times in {formatter}, but the scale places ticks in {scale}")]
    TimezoneMismatch { scale: Tz, formatter: Tz },

    #[error("Invalid label: {0}")]
    Format(FormatError),

    #[error("Text error: {0}")]
    Text(#[from] LabelError),

    #[error("Invalid axis ticks: {0}")]
    InvalidAxisTicks(String),
}
