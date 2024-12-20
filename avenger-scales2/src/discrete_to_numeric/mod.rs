pub mod band;
pub mod point;

use std::collections::HashMap;

use crate::{
    config::{DiscreteDomainConfig, ScaleConfig, ScaleConfigScalar, ScaleDomainConfig},
    error::AvengerScaleError,
};
use avenger_common::value::ScalarOrArray;
use std::fmt::Debug;

pub struct DiscreteToNumericScaleConfig {
    pub domain: DiscreteDomainConfig,
    pub range: (f32, f32),
    pub round: bool,
    pub range_offset: f32,
    pub options: HashMap<String, ScaleConfigScalar>,
}

impl Default for DiscreteToNumericScaleConfig {
    fn default() -> Self {
        Self {
            domain: DiscreteDomainConfig::Numbers(vec![1.0]),
            range: (0.0, 1.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        }
    }
}

impl TryFrom<ScaleConfig> for DiscreteToNumericScaleConfig {
    type Error = AvengerScaleError;

    fn try_from(config: ScaleConfig) -> Result<Self, Self::Error> {
        let domain = match config.clone().domain {
            ScaleDomainConfig::Discrete(domain) => domain,
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(
                    "domain".to_string(),
                ))
            }
        };

        Ok(Self {
            domain,
            range: config.get_numeric_interval_range()?,
            options: config.options,
            round: false,
            range_offset: 0.0,
        })
    }
}

pub trait DiscreteToNumericScale: Debug + Send + Sync + 'static {
    /// Scale from discrete domain to numeric range (e.g. band and point scales)
    ///
    /// Input indices correspond to the index of the domain vector in config.
    fn scale_numbers(
        &self,
        _config: &DiscreteToNumericScaleConfig,
        _values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_discrete_to_numeric".to_string(),
        ));
    }

    fn scale_strings(
        &self,
        _config: &DiscreteToNumericScaleConfig,
        _values: &[String],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_discrete_to_numeric".to_string(),
        ));
    }

    fn scale_indices(
        &self,
        _config: &DiscreteToNumericScaleConfig,
        _values: &[usize],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_discrete_to_numeric".to_string(),
        ));
    }

    /// Invert from numeric range interval to the discrete domain values that map to that interval
    fn invert(
        &self,
        _config: &DiscreteToNumericScaleConfig,
        _range: (f32, f32),
    ) -> Result<Vec<usize>, AvengerScaleError> {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_numeric_to_discrete".to_string(),
        ));
    }
}
