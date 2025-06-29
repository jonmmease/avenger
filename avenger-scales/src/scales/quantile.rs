use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, UInt32Array},
    compute::{
        kernels::{cast, sort, take},
        SortOptions,
    },
    datatypes::{DataType, Float32Type},
};

use crate::error::AvengerScaleError;

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl};

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
#[derive(Debug, Clone)]
pub struct QuantileScale;

impl QuantileScale {
    pub fn configured(domain: Vec<f32>, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(domain)),
                range,
                options: HashMap::new(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for QuantileScale {
    fn scale_type(&self) -> &'static str {
        "quantile"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
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
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

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
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // The quantile boundaries are the ticks
        let thresholds = Arc::new(Float32Array::from(quantile_thresholds(
            &config.domain,
            config.range.len(),
        )?)) as ArrayRef;
        Ok(thresholds)
    }
}

fn quantile_thresholds(domain: &ArrayRef, n: usize) -> Result<Vec<f32>, AvengerScaleError> {
    // Sort domain and cast to f32 array
    let domain = sort::sort(
        domain,
        Some(SortOptions {
            descending: false,
            nulls_first: false,
        }),
    )?;
    let domain = cast(&domain, &DataType::Float32)?;
    let domain = domain.as_primitive::<Float32Type>();

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
    use arrow::array::StringArray;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantile_scale_basic() -> Result<(), AvengerScaleError> {
        // Create sample population with skewed distribution
        let domain = Arc::new(Float32Array::from(vec![
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
        let thresholds = thresholds.as_primitive::<Float32Type>();
        assert_eq!(thresholds.len(), 2);
        assert_approx_eq!(f32, thresholds.value(0), 3.0); // First third of values: [1,1,2]
        assert_approx_eq!(f32, thresholds.value(1), 4.0); // Second third: [3,3,3]
                                                          // Last third: [4,4,5]

        // Test mapping values
        let values = Arc::new(Float32Array::from(vec![1.5, 3.0, 4.5, f32::NAN])) as ArrayRef;
        let result = scale.scale_to_string(&config, &values)?;
        let result_vec = result.as_vec(values.len(), None);

        assert_eq!(result_vec, vec!["small", "medium", "large", "default"]);

        Ok(())
    }
}
