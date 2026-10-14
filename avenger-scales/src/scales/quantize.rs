use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, Float64Array, UInt32Array};
use arrow::compute::kernels::{cast::cast, take};
use arrow::datatypes::{DataType, Float64Type};
use lazy_static::lazy_static;

use crate::error::AvengerScaleError;

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents, NiceStep};
use super::no_domain;
use super::{
    linear::LinearScale, ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition,
    ScaleConfig, ScaleContext, ScaleImpl,
};

/// Quantize scale that divides a continuous numeric domain into uniform segments,
/// mapping each segment to a discrete value from the range.
///
/// The scale divides the domain into n equal-sized bins where n is the number of
/// values in the range array. Each input value is mapped to the range value
/// corresponding to its bin.
#[derive(Debug, Clone)]
pub struct QuantizeScale;

impl QuantizeScale {
    /// A quantize scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: ArrayRef) -> ConfiguredScale {
        Self::configured((0.0, 1.0), range).with_domain(no_domain())
    }

    pub fn configured(domain: (f64, f64), range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range,
                options: Default::default(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for QuantizeScale {
    fn scale_type(&self) -> &'static str {
        "quantize"
    }

    fn summarize(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let values = cast(values, &DataType::Float64)?;
        fit::interval_summary(values.as_primitive::<Float64Type>(), extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let options = options.interval()?;
        fit::fit_continuous(
            config,
            options,
            summary,
            fit::IntervalFit {
                nice: options.nice.map(NiceStep::Auto),
                zero: options.zero.then_some(0.0),
                single_value_span: fit::linear_single_value_span,
            },
            |value| value,
            (0.0, 1.0),
        )
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![OptionDefinition::optional(
                "default",
                OptionConstraint::String
            ),];
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

        let (domain_start, domain_end) = config.numeric_interval_domain()?;
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
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::error::AvengerScaleError;
    use arrow::array::{Float64Array, StringArray};

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
}
