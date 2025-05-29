use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, UInt32Array},
    compute::kernels::take,
    datatypes::Float32Type,
};
use datafusion_common::ScalarValue;

use crate::error::AvengerScaleError;

use super::{
    linear::LinearScale, ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext,
    ScaleImpl,
};


#[derive(Debug, Clone)]
pub struct QuantizeScale;

impl QuantizeScale {
    pub fn new(domain: (f32, f32), range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range,
                options: HashMap::new(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f32, f32),
        count: Option<&ScalarValue>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Use nice method from linear scale
        LinearScale::apply_nice(domain, count)
    }
}

impl ScaleImpl for QuantizeScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Pre-compute scaling factors
        let n = config.range.len();
        let segments = config.range.len() as f32;

        let domain_span = QuantizeScale::apply_nice(
            config.numeric_interval_domain()?,
            config.options.get("nice"),
        )?;

        let indices = Arc::new(UInt32Array::from(
            values
                .as_primitive::<Float32Type>()
                .iter()
                .map(|x| match x {
                    Some(x) => {
                        if x.is_finite() {
                            let normalized = (x - domain_span.0) / domain_span.1;
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
        count: Option<f32>,
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
    use arrow::array::{Float32Array, StringArray};
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantize_scale_basic() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale;
        let config = ScaleConfig {
            domain: Arc::from(Float32Array::from(vec![0.0, 1.0])),
            range: Arc::from(StringArray::from(vec!["a", "b", "c"])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test array scaling with all test cases
        let values = Arc::from(Float32Array::from(vec![0.3, 0.5, 0.8])) as ArrayRef;
        let result = scale
            .scale_to_string(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert_eq!(result, vec!["a", "b", "c"]);

        Ok(())
    }


    #[test]
    fn test_quantize_scale_nice() -> Result<(), AvengerScaleError> {
        let (start, end) =
            QuantizeScale::apply_nice((1.1, 10.9), Some(&ScalarValue::from(5))).unwrap();
        assert_approx_eq!(f32, start, 0.0);
        assert_approx_eq!(f32, end, 12.0);

        Ok(())
    }
}
