use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, UInt32Array},
    compute::kernels::{cast, take},
    datatypes::{DataType, Float32Type},
};

use crate::error::AvengerScaleError;

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl};

#[derive(Debug, Clone)]
pub struct ThresholdScale;

impl ThresholdScale {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(domain: Vec<f32>, range: ArrayRef) -> ConfiguredScale {
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

impl ScaleImpl for ThresholdScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let thresholds = validate_extract_thresholds(&config.domain)?;

        // Validate the range has the correct number of elements
        if config.range.len() != thresholds.len() + 1 {
            return Err(AvengerScaleError::ThresholdDomainMismatch {
                domain_len: thresholds.len(),
                range_len: config.range.len(),
            });
        }

        let indices = Arc::new(UInt32Array::from(
            values
                .as_primitive::<Float32Type>()
                .iter()
                .map(|x| match x {
                    Some(x) => {
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
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Ticks are the same as the domain values
        Ok(config.domain.clone())
    }
}

fn validate_extract_thresholds(domain: &ArrayRef) -> Result<Vec<f32>, AvengerScaleError> {
    // Try to convert range to f32
    let thresholds = cast(&domain, &DataType::Float32).map_err(|_| {
        AvengerScaleError::ScaleOperationNotSupported(
            "threshold scale domain is not numeric".to_string(),
        )
    })?;

    let thresholds = thresholds
        .as_primitive::<Float32Type>()
        .values()
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    // Validate the thresholds are in ascending order
    if !thresholds.windows(2).all(|w| w[0] <= w[1]) {
        return Err(AvengerScaleError::ThresholdsNotAscending(
            thresholds.clone(),
        ));
    }

    Ok(thresholds)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use arrow::array::{Float32Array, StringArray};
    use avenger_common::types::ImageAlign;

    #[test]
    fn test_threshold_scale_basic() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![30.0, 70.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = ThresholdScale;

        let values = Arc::new(Float32Array::from(vec![20.0, 50.0, 80.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_eq!(result, vec![0.0, 1.0, 2.0]);

        Ok(())
    }

    #[test]
    fn test_threshold_scale_enum() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![30.0, 70.0])),
            range: Arc::new(StringArray::from(vec!["left", "center", "right"])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = ThresholdScale;

        let values = Arc::new(Float32Array::from(vec![50.0, 20.0, 80.0])) as ArrayRef;
        let result = scale
            .scale_to_image_align(&config, &values)?
            .as_vec(values.len(), None);

        assert_eq!(
            result,
            vec![ImageAlign::Center, ImageAlign::Left, ImageAlign::Right]
        );

        Ok(())
    }
}
