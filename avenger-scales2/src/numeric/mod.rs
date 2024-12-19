pub mod color;
pub mod linear;
pub mod pow;

use std::collections::HashMap;

use avenger_common::value::ScalarOrArray;

use crate::{
    config::{ScaleConfig, ScaleConfigScalar},
    error::AvengerScaleError,
};

/// Config for numeric scales
#[derive(Debug, Clone)]
pub struct NumericScaleConfig {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub round: bool,
    pub clamp: bool,
    pub range_offset: f32,

    /// Additional scale specific options
    pub options: HashMap<String, ScaleConfigScalar>,
}

impl Default for NumericScaleConfig {
    fn default() -> Self {
        Self {
            domain: (0.0, 1.0),
            range: (0.0, 1.0),
            round: false,
            clamp: false,
            range_offset: 0.0,
            options: HashMap::new(),
        }
    }
}

impl TryFrom<ScaleConfig> for NumericScaleConfig {
    type Error = AvengerScaleError;

    fn try_from(config: ScaleConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            domain: config.get_numeric_interval_domain()?,
            range: config.get_numeric_interval_range()?,
            round: config.round.unwrap_or(false),
            clamp: config.clamp.unwrap_or(false),
            range_offset: config.range_offset.unwrap_or(0.0),
            options: config.options,
        })
    }
}

/// Trait for all continuous numeric-to-numeric scales
pub trait NumericScale: Send + Sync + 'static {
    /// Scale numeric values from continuous domain to continuous range
    /// e.g. linear with numeric range
    fn scale(
        &self,
        _config: &NumericScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError>;

    fn scale_scalar(
        &self,
        config: &NumericScaleConfig,
        value: f32,
    ) -> Result<f32, AvengerScaleError> {
        let scaled = self.scale(config, &[value])?;
        Ok(scaled.as_vec(1, None)[0])
    }

    /// Invert numeric values from continuous range to continuous domain
    fn invert(
        &self,
        _config: &NumericScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert".to_string(),
        ));
    }

    fn invert_scalar(
        &self,
        config: &NumericScaleConfig,
        value: f32,
    ) -> Result<f32, AvengerScaleError> {
        let inverted = self.invert(config, &[value])?;
        Ok(inverted.as_vec(1, None)[0])
    }

    /// Nice scale domain
    fn nice(
        &self,
        config: NumericScaleConfig,
        _count: Option<usize>,
    ) -> Result<NumericScaleConfig, AvengerScaleError> {
        return Ok(config);
    }

    /// Compute ticks for a scale with numeric domain
    /// Ticks are in the domain space of the scale
    fn ticks(
        &self,
        _config: NumericScaleConfig,
        _count: Option<f32>,
    ) -> Result<Vec<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks".to_string(),
        ));
    }
}
