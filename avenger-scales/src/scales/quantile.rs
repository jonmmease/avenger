use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, UInt32Array},
    compute::{
        kernels::{cast, sort, take},
        SortOptions,
    },
    datatypes::{DataType, Float64Type},
};
use avenger_format::TickSpacing;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use crate::error::AvengerScaleError;

use super::domain_summary::{DomainFitError, DomainOptions, DomainSummary, Extents, SampleSummary};
use super::{ConfiguredScale, DomainKind, OptionDefinition, ScaleConfig, ScaleContext, ScaleImpl};

/// Quantile scale that maps continuous numeric input values to discrete range values
/// using quantile boundaries computed from the domain.
///
/// The domain should contain sample data values. The scale computes n-1 quantile
/// thresholds that divide the sorted domain into n equal-sized groups, where n is
/// the number of values in the range. Each input value is then mapped to the
/// corresponding range value based on which quantile it falls into.
///
/// # Config Options
///
/// This scale does not currently support any configuration options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantileScale;

impl QuantileScale {
    /// A quantile scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: ArrayRef) -> ConfiguredScale {
        Self::configured(Vec::new(), range)
    }

    pub fn configured(domain: Vec<f64>, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(domain)),
                range,
                options: HashMap::new(),
                context: ScaleContext::default(),
            },
        }
    }
}

#[typetag::serde(name = "quantile")]
impl ScaleImpl for QuantileScale {
    /// Quantiles are arbitrary breaks, so each label keeps its own precision.
    fn tick_spacing(&self) -> TickSpacing {
        TickSpacing::Varying
    }

    fn scale_type(&self) -> &'static str {
        "quantile"
    }

    fn summarize(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        if !matches!(extents, Extents::None) {
            return Err(DomainFitError::InvalidInput(
                "quantile scales do not position marks, so they take no extents".into(),
            )
            .into());
        }
        let values = cast(values, &DataType::Float64)?;
        let values: Vec<f64> = values
            .as_primitive::<Float64Type>()
            .iter()
            .flatten()
            .filter(|value| value.is_finite())
            .collect();
        Ok(DomainSummary::Sample(SampleSummary::new(&values)?))
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        if !matches!(options, DomainOptions::Sample) {
            return Err(DomainFitError::InvalidInput(
                "a quantile scale fits with sample options".into(),
            )
            .into());
        }
        let DomainSummary::Sample(sample) = summary else {
            return Err(DomainFitError::InvalidInput(
                "a quantile scale fits a sample summary".into(),
            )
            .into());
        };
        // The domain is the whole sample, empty without data.
        let domain: ArrayRef = Arc::new(Float64Array::from(sample.values().to_vec()));
        Ok(ScaleConfig {
            domain,
            ..config.clone()
        })
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    /// The range's values.
    fn output_type(&self, range_type: &DataType) -> Result<DataType, AvengerScaleError> {
        Ok(range_type.clone())
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                // Quantile scale supports no custom options currently
                // But default option is allowed for consistency
                OptionDefinition::optional("default", super::OptionConstraint::String),
            ];
        }

        &DEFINITIONS
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let n = config.range.len();
        if n <= 1 || config.domain.is_empty() {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "Quantile scale requires a non-empty domain and range".to_string(),
            ));
        }

        let thresholds = quantile_thresholds(&config.domain, n)?;
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

        // Compute range indices
        let indices = UInt32Array::from(
            values
                .iter()
                .map(|x| {
                    x.and_then(|x| {
                        if x.is_finite() {
                            let idx =
                                match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
                                    Ok(i) => (i + 1) as u32,
                                    Err(i) => i as u32,
                                };
                            Some(idx)
                        } else {
                            None
                        }
                    })
                })
                .collect::<Vec<_>>(),
        );

        Ok(take::take(&config.range, &indices, None)?)
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        _count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // The quantile boundaries are the ticks
        Ok(crate::array::tick_array(quantile_thresholds(
            &config.domain,
            config.range.len(),
        )?))
    }
}

fn quantile_thresholds(domain: &ArrayRef, n: usize) -> Result<Vec<f64>, AvengerScaleError> {
    // Sort domain and cast to f64 array
    let domain = sort::sort(
        domain,
        Some(SortOptions {
            descending: false,
            nulls_first: false,
        }),
    )?;
    let domain = cast(&domain, &DataType::Float64)?;
    let domain = domain.as_primitive::<Float64Type>();

    // Compute n-1 quantile thresholds
    let domain_len = domain.len();
    let thresholds = (1..n)
        .map(|i| {
            let k = (domain_len * i) / n;
            domain.value(k)
        })
        .collect::<Vec<_>>();

    Ok(thresholds)
}

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;
    use arrow::{array::StringArray, datatypes::Float64Type};

    #[test]
    fn test_quantile_scale_basic() -> Result<(), AvengerScaleError> {
        // Create sample population with skewed distribution
        let domain = Arc::new(Float64Array::from(vec![
            1.0, 1.0, 2.0, 3.0, 3.0, 3.0, 4.0, 4.0, 5.0,
        ]));
        let config = ScaleConfig {
            domain,
            range: Arc::new(StringArray::from(vec!["small", "medium", "large"])),
            options: vec![("default".to_string(), "default".into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let scale = QuantileScale;

        // Check quantile thresholds
        let thresholds = scale.ticks(&config, None)?;
        let thresholds = thresholds.as_primitive::<Float64Type>();
        assert_eq!(thresholds.len(), 2);
        assert_eq!(thresholds.value(0), 3.0); // First third of values: [1,1,2]
        assert_eq!(thresholds.value(1), 4.0); // Second third: [3,3,3]
                                              // Last third: [4,4,5]

        // Test mapping values
        let values = Arc::new(Float64Array::from(vec![1.5, 3.0, 4.5, f64::NAN])) as ArrayRef;
        let result = scale.scale_to_string(&config, &values)?;
        let result_vec = result.as_vec(values.len(), None);

        assert_eq!(result_vec, vec!["small", "medium", "large", "default"]);

        Ok(())
    }
}
