use std::collections::HashSet;

use avenger_scales::error::AvengerScaleError;
use avenger_typst_label::LabelError;
use chrono_tz::Tz;
use thiserror::Error;

use crate::GuideSide;

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

    #[error("An attribution stands above or below a plot, not on its {0:?} side")]
    AttributionSide(GuideSide),

    #[error("An attribution's {name} must be finite and non-negative, not {value}")]
    AttributionSpacing { name: &'static str, value: f32 },

    #[error("A facet guide's {name} must be finite and non-negative, not {value}")]
    FacetGuideSpacing { name: &'static str, value: f32 },

    #[error("A facet guide's label angle must be finite, not {0}")]
    FacetGuideAngle(f32),

    #[error(
        "A polar axis runs from its start angle to an end angle past it, at most a full turn \
         later, not from {start} to {end}"
    )]
    PolarAngles { start: f32, end: f32 },
}
