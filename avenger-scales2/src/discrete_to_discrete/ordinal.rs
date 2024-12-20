use crate::{
    config::{
        DiscreteDomainConfig, DiscreteRangeConfig, ScaleConfig, ScaleConfigScalar,
        ScaleDomainConfig, ScaleRangeConfig,
    },
    error::AvengerScaleError,
};

use indexmap::IndexMap;
use ordered_float::OrderedFloat;
use std::fmt::Debug;
use std::hash::Hash;

use super::{DiscreteToDiscreteScale, DiscreteToDiscreteScaleConfig};

/// A discrete scale that maps input values to a fixed set of output values.
/// Supports default values for inputs not found in the domain.
#[derive(Debug, Clone)]
struct OrdinalScaleImpl<D>
where
    D: Clone + Hash + Eq + Debug + Sync + 'static,
{
    mapping: IndexMap<D, usize>,
}

impl<D> OrdinalScaleImpl<D>
where
    D: Clone + Hash + Eq + Debug + Sync + 'static,
{
    /// Creates a new ordinal scale from domain and range arrays with a required default value
    pub fn new(domain: &[D]) -> Result<Self, AvengerScaleError> {
        // Create a mapping from domain values to range values
        let mapping = domain
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, domain_value)| (domain_value, index))
            .collect::<IndexMap<_, _>>();

        Ok(Self { mapping })
    }

    /// Maps input values to their corresponding range values using the ordinal mapping
    pub fn scale<'a>(&self, values: impl IntoIterator<Item = D>) -> Vec<Option<usize>> {
        values
            .into_iter()
            .map(|v| self.mapping.get(&v).cloned())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct OrdinalScale;

impl DiscreteToDiscreteScale for OrdinalScale {
    fn scale_numbers(
        &self,
        config: &DiscreteToDiscreteScaleConfig,
        values: &[f32],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError> {
        if config.domain.len() != config.range.len() {
            return Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: config.domain.len(),
                range_len: config.range.len(),
            });
        }
        let domain: Vec<_> = match &config.domain {
            DiscreteDomainConfig::Numbers(vec) => vec.iter().map(|v| OrderedFloat(*v)).collect(),
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(format!(
                    "scale_numbers expects a numeric domain, received {:?}",
                    config.domain
                )))
            }
        };
        let scale_impl = OrdinalScaleImpl::new(&domain)?;
        Ok(scale_impl.scale(values.iter().map(|v| OrderedFloat(*v))))
    }

    fn scale_strings(
        &self,
        config: &super::DiscreteToDiscreteScaleConfig,
        values: &[String],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError> {
        if config.domain.len() != config.range.len() {
            return Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: config.domain.len(),
                range_len: config.range.len(),
            });
        }
        let domain: Vec<_> = match &config.domain {
            DiscreteDomainConfig::Strings(vec) => vec.iter().cloned().collect(),
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(format!(
                    "scale_strings expects a string domain, received {:?}",
                    config.domain
                )))
            }
        };
        let scale_impl = OrdinalScaleImpl::new(&domain)?;
        Ok(scale_impl.scale(values.iter().cloned()))
    }

    fn scale_indices(
        &self,
        config: &super::DiscreteToDiscreteScaleConfig,
        values: &[usize],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError> {
        if config.domain.len() != config.range.len() {
            return Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: config.domain.len(),
                range_len: config.range.len(),
            });
        }
        let domain: Vec<_> = match &config.domain {
            DiscreteDomainConfig::Indices(vec) => vec.iter().cloned().collect(),
            _ => {
                return Err(AvengerScaleError::ScaleOperationNotSupported(format!(
                    "scale_indices expects an index domain, received {:?}",
                    config.domain
                )))
            }
        };
        let scale_impl = OrdinalScaleImpl::new(&domain)?;
        Ok(scale_impl.scale(values.iter().cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_ordinal_scale() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let range = vec!["red".to_string(), "green".to_string(), "blue".to_string()];

        // Create scale with a default value
        let scale = OrdinalScale;
        let config = DiscreteToDiscreteScaleConfig {
            domain: DiscreteDomainConfig::Strings(domain),
            range: DiscreteRangeConfig::Strings(range),
            default_value: Some(ScaleConfigScalar::String("gray".to_string())),
        };

        let result = scale.scale_strings(
            &config,
            &[
                "b".to_string(),
                "a".to_string(),
                "d".to_string(),
                "b".to_string(),
                "d".to_string(),
            ],
        )?;
        assert_eq!(result, vec![Some(1), Some(0), None, Some(1), None]);

        Ok(())
    }

    #[test]
    fn test_domain_range_mismatch() {
        let domain = vec![1.0, 2.0, 3.0];
        let range = vec!["red".to_string(), "green".to_string()];

        let scale = OrdinalScale;
        let config = DiscreteToDiscreteScaleConfig {
            domain: DiscreteDomainConfig::Numbers(domain),
            range: DiscreteRangeConfig::Strings(range),
            default_value: Some(ScaleConfigScalar::String("gray".to_string())),
        };
        assert!(matches!(
            scale.scale_numbers(&config, &[1.0, 2.0, 3.0, 4.0, 5.0]),
            Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: 3,
                range_len: 2,
            })
        ));
    }
}
