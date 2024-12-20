use std::collections::HashMap;

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use palette::Srgba;

use crate::error::AvengerScaleError;

/// General scale configuration to use across all scales
#[derive(Debug, Clone)]
pub struct ScaleConfig {
    pub domain: ScaleDomainState,
    pub range: ScaleRangeState,
    pub round: Option<bool>,
    pub clamp: Option<bool>,
    pub range_offset: Option<f32>,
    pub nice: Option<usize>,

    /// Additional scale specific options
    pub options: HashMap<String, f32>,
}

#[derive(Debug, Clone)]
pub enum ScaleDomainState {
    // Intervals
    Interval(f32, f32),
    // Discrete values
    DiscreteNumeric(Vec<f32>),
    // Discrete values
    DiscreteString(Vec<String>),
}

impl ScaleDomainState {
    pub fn new_interval(start: f32, end: f32) -> Self {
        Self::Interval(start, end)
    }
}

#[derive(Debug, Clone)]
pub enum ScaleRangeState {
    Numeric(f32, f32),
    Color(Vec<Srgba>),
    Enum(Vec<String>),
}

impl ScaleRangeState {
    /// Whether the scale is considered discrete
    pub fn is_discrete(&self) -> bool {
        matches!(self, ScaleRangeState::Enum(_) | ScaleRangeState::Color(_))
    }
}

pub trait ScaleTrait {
    /// Scale numeric values from continuous domain to continuous range
    /// e.g. linear with numeric range
    fn scale_numeric(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_numeric".to_string(),
        ));
    }

    /// Invert numeric values from continuous range to continuous domain
    fn invert_numeric(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_numeric".to_string(),
        ));
    }

    fn scale_date_to_numeric(
        _config: &ScaleConfig,
        _values: &[NaiveDate],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_date_to_numeric".to_string(),
        ));
    }

    fn invert_date_to_numeric(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<NaiveDate>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_date_to_numeric".to_string(),
        ));
    }

    fn scale_timestamp_to_numeric(
        _config: &ScaleConfig,
        _values: &[NaiveDateTime],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_timestamp_to_numeric".to_string(),
        ));
    }

    fn invert_timestamp_to_numeric(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<NaiveDateTime>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_timestamp_to_numeric".to_string(),
        ));
    }

    fn scale_timestamptz_to_numeric(
        _config: &ScaleConfig,
        _values: &[DateTime<Utc>],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_timestamptz_to_numeric".to_string(),
        ));
    }

    fn invert_timestamptz_to_numeric(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<DateTime<Utc>>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_timestamptz_to_numeric".to_string(),
        ));
    }

    /// Scale numeric values from continuous domain to continuous color range
    /// e.g. linear with color range
    fn scale_numeric_color(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_numeric_color".to_string(),
        ));
    }

    /// Scale from discrete domain to numeric range (e.g. band and point scales)
    ///
    /// Domain must be discrete, and input indices correspond to the index
    /// of the domain.
    fn scale_discrete_to_numeric(
        _config: &ScaleConfig,
        _values: &[usize],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_discrete_to_numeric".to_string(),
        ));
    }

    /// Invert from numeric range interval to discrete domain
    fn invert_numeric_to_discrete(
        _config: &ScaleConfig,
        _range: (f32, f32),
    ) -> Result<Vec<usize>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_numeric_to_discrete".to_string(),
        ));
    }

    /// Scale from numeric range to discrete domain.
    /// e.g. quantize scale
    ///
    /// Requires a discrete domain, and returned indices correspond to the index
    /// of the range.
    fn scale_numeric_to_discrete(
        _config: &ScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<usize>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_numeric_to_discrete".to_string(),
        ));
    }

    /// Nice scale domain
    fn nice(
        &self,
        config: ScaleConfig,
        _count: Option<usize>,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        return Ok(config);
    }

    /// Compute ticks for a scale with numeric domain
    /// Ticks are in the domain space of the scale
    fn ticks_numeric(&self, config: ScaleConfig) -> Result<Vec<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks_numeric".to_string(),
        ));
    }

    /// Compute ticks for a scale with date domain
    fn ticks_date(&self, config: ScaleConfig) -> Result<Vec<NaiveDate>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks_date".to_string(),
        ));
    }

    /// Compute ticks for a scale with timestamp domain
    fn ticks_timestamp(
        &self,
        config: ScaleConfig,
    ) -> Result<Vec<NaiveDateTime>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks_timestamp".to_string(),
        ));
    }

    /// Compute ticks for a scale with timestamptz domain
    fn ticks_timestamptz(
        &self,
        _config: ScaleConfig,
    ) -> Result<Vec<DateTime<Utc>>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks_timestamptz".to_string(),
        ));
    }

    /// Format a vector of tick values as strings
    fn format_ticks_numeric(
        &self,
        _config: ScaleConfig,
        _ticks: Vec<f32>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "format_ticks_numeric".to_string(),
        ));
    }

    fn format_ticks_date(
        &self,
        _config: ScaleConfig,
        _ticks: Vec<NaiveDate>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "format_ticks_date".to_string(),
        ));
    }

    fn format_ticks_timestamp(
        &self,
        _config: ScaleConfig,
        _ticks: Vec<NaiveDateTime>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "format_ticks_timestamp".to_string(),
        ));
    }

    fn format_ticks_timestamptz(
        &self,
        _config: ScaleConfig,
        _ticks: Vec<DateTime<Utc>>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "format_ticks_timestamptz".to_string(),
        ));
    }
}
