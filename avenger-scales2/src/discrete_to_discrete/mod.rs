pub mod ordinal;

use crate::config::{
    DiscreteDomainConfig, DiscreteRangeConfig, ScaleConfig, ScaleConfigScalar, ScaleDomainConfig,
    ScaleRangeConfig,
};
use crate::error::AvengerScaleError;

#[cfg(feature = "temporal")]
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::fmt::Debug;

pub struct DiscreteToDiscreteScaleConfig {
    pub domain: DiscreteDomainConfig,
    pub range: DiscreteRangeConfig,
    pub default_value: Option<ScaleConfigScalar>,
}

impl TryFrom<ScaleConfig> for DiscreteToDiscreteScaleConfig {
    type Error = AvengerScaleError;

    fn try_from(config: ScaleConfig) -> Result<Self, Self::Error> {
        let domain = match config.domain {
            ScaleDomainConfig::Discrete(domain) => domain,
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(
                    "ordinal".to_string(),
                ));
            }
        };

        let range = match config.range {
            ScaleRangeConfig::Discrete(range) => range,
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(
                    "ordinal".to_string(),
                ));
            }
        };

        let default_value = config.options.get("default_value").cloned();

        Ok(Self {
            domain,
            range,
            default_value,
        })
    }
}

pub trait DiscreteToDiscreteScale: Debug + Send + Sync + 'static {
    /// Scale a vector of domain numbers to a vector of indices into the range array
    fn scale_numbers(
        &self,
        config: &DiscreteToDiscreteScaleConfig,
        values: &[f32],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError>;

    /// Scale a vector of domain indices to a vector of indices into the range array
    fn scale_indices(
        &self,
        config: &DiscreteToDiscreteScaleConfig,
        values: &[usize],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError>;

    /// Scale a vector of domain strings to a vector of indices into the range array
    fn scale_strings(
        &self,
        config: &DiscreteToDiscreteScaleConfig,
        values: &[String],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError>;
}
