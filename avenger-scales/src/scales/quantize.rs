use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, Float64Array, UInt32Array};
use arrow::compute::kernels::{cast::cast, take};
use arrow::datatypes::{DataType, Float64Type};
use lazy_static::lazy_static;

use crate::{error::AvengerScaleError, scalar::Scalar};

use super::{
    linear::{LinearScale, NormalizationConfig},
    ConfiguredScale, DomainKind, InferDomainFromDataMethod, OptionConstraint, OptionDefinition,
    ScaleConfig, ScaleContext, ScaleImpl,
};

/// Quantize scale that divides a continuous numeric domain into uniform segments,
/// mapping each segment to a discrete value from the range.
///
/// The scale divides the domain into n equal-sized bins where n is the number of
/// values in the range array. Each input value is mapped to the range value
/// corresponding to its bin.
///
/// # Config Options
///
/// - **nice** (boolean or f64, default: false): When true or a number, extends the domain to nice round values
///   before quantization. If true, uses a default count of 10. If a number, uses that as the target tick count
///   for determining nice values. This ensures bins align with human-friendly boundaries.
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. If both min and max
///   are positive, sets min to zero. If both min and max are negative, sets max to zero. If the domain already
///   spans zero, no change is made. Zero extension is applied before nice calculations.
#[derive(Debug, Clone)]
pub struct QuantizeScale;

impl QuantizeScale {
    pub fn configured(domain: (f64, f64), range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range,
                options: vec![
                    ("nice".to_string(), false.into()),
                    ("zero".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f64, f64),
        count: Option<&Scalar>,
    ) -> Result<(f64, f64), AvengerScaleError> {
        // Use nice method from linear scale
        LinearScale::apply_nice(domain, count)
    }

    /// Apply normalization (zero and nice) to domain
    pub fn apply_normalization(
        domain: (f64, f64),
        zero: Option<&Scalar>,
        nice: Option<&Scalar>,
    ) -> Result<(f64, f64), AvengerScaleError> {
        // Use LinearScale normalization since quantize scale works with linear domains
        // Quantize scale doesn't use padding, so we pass dummy range and None for padding
        LinearScale::apply_normalization(NormalizationConfig {
            domain,
            range: (0.0, 1.0),
            clip_padding_lower: None,
            clip_padding_upper: None,
            zero,
            nice,
        })
    }
}

impl ScaleImpl for QuantizeScale {
    fn scale_type(&self) -> &'static str {
        "quantize"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional("zero", OptionConstraint::Boolean),
                OptionDefinition::optional("default", OptionConstraint::String),
            ];
        }

        &DEFINITIONS
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Pre-compute scaling factors
        let n = config.range.len();
        let segments = config.range.len() as f64;

        let (domain_start, domain_end) = QuantizeScale::apply_nice(
            config.numeric_interval_domain()?,
            config.options.get("nice"),
        )?;
        let domain_span = domain_end - domain_start;

        let values = cast(values, &DataType::Float64)?;
        let indices = Arc::new(UInt32Array::from(
            values
                .as_primitive::<Float64Type>()
                .iter()
                .map(|x| match x {
                    Some(x) => {
                        if x.is_finite() {
                            let normalized = (x - domain_start) / domain_span;
                            let idx = ((normalized * segments).floor() as usize).clamp(0, n - 1);
                            Some(idx as u32)
                        } else {
                            None
                        }
                    }
                    None => None,
                })
                .collect::<Vec<_>>(),
        )) as ArrayRef;

        Ok(take::take(&config.range, &indices, None)?)
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Ticks are the same as for a linear scale
        let linear_scale = LinearScale;
        linear_scale.ticks(config, count)
    }

    fn compute_normalized_domain(
        &self,
        config: &ScaleConfig,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = QuantizeScale::apply_normalization(
            config.numeric_interval_domain()?,
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        Ok(Arc::new(Float64Array::from(vec![domain_start, domain_end])) as ArrayRef)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::error::AvengerScaleError;
    use arrow::array::{Float64Array, StringArray};
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantize_scale_basic() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale;
        let config = ScaleConfig {
            domain: Arc::from(Float64Array::from(vec![0.0, 1.0])),
            range: Arc::from(StringArray::from(vec!["a", "b", "c"])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test array scaling with all test cases
        let values = Arc::from(Float64Array::from(vec![0.3, 0.5, 0.8])) as ArrayRef;
        let result = scale
            .scale_to_string(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert_eq!(result, vec!["a", "b", "c"]);

        Ok(())
    }

    /// A domain that doesn't start at 0 splits into segments of equal width.
    #[test]
    fn test_quantize_scale_offset_domain() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::from(Float64Array::from(vec![10.0, 20.0])),
            range: Arc::from(StringArray::from(vec!["a", "b"])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let values = Arc::from(Float64Array::from(vec![12.0, 18.0])) as ArrayRef;
        let result = QuantizeScale
            .scale_to_string(&config, &values)?
            .as_vec(values.len(), None);
        assert_eq!(result, vec!["a", "b"]);
        Ok(())
    }

    #[test]
    fn test_quantize_scale_nice() -> Result<(), AvengerScaleError> {
        let (start, end) = QuantizeScale::apply_nice((1.1, 10.9), Some(&Scalar::from(5))).unwrap();
        assert_approx_eq!(f64, start, 0.0);
        assert_approx_eq!(f64, end, 12.0);

        Ok(())
    }
}
