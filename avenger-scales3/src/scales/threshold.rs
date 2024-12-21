use arrow::{
    array::{ArrayRef, AsArray},
    compute::kernels::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::{AreaOrientation, ColorOrGradient, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};

use crate::{color_interpolator::ColorInterpolator, error::AvengerScaleError};

use super::{
    ordinal::{
        prep_discrete_color_range, prep_discrete_enum_range, prep_discrete_numeric_range,
        prep_discrete_string_range,
    },
    ArrowScale, InferDomainFromDataMethod, ScaleConfig,
};

/// Macro to generate scale_to_X trait methods for threshold enum scaling
#[macro_export]
macro_rules! impl_threshold_enum_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            fn [<scale_to_ $type_name:snake>](
                &self,
                config: &ScaleConfig,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                let (range_vec, default_value) = prep_discrete_enum_range::<$type_name>(config)?;
                threshold_scale(values, &config.domain, range_vec, default_value)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct ThresholdScale;

impl ArrowScale for ThresholdScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    /// Scale to numeric values
    fn scale_to_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_numeric_range(config)?;
        threshold_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_string_range(config)?;
        threshold_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        _color_interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_color_range(config)?;
        threshold_scale(values, &config.domain, range_vec, default_value)
    }

    // Enums
    impl_threshold_enum_scale_method!(StrokeCap);
    impl_threshold_enum_scale_method!(StrokeJoin);
    impl_threshold_enum_scale_method!(ImageAlign);
    impl_threshold_enum_scale_method!(ImageBaseline);
    impl_threshold_enum_scale_method!(AreaOrientation);
}

/// Generic helper function for evaluating threshold scales
fn threshold_scale<R: Sync + Clone>(
    values: &ArrayRef,
    domain: &ArrayRef,
    range: Vec<R>,
    default_value: R,
) -> Result<ScalarOrArray<R>, AvengerScaleError> {
    let thresholds = validate_extract_thresholds(domain)?;

    // Validate the range has the correct number of elements
    if range.len() != thresholds.len() + 1 {
        return Err(AvengerScaleError::ThresholdDomainMismatch {
            domain_len: thresholds.len(),
            range_len: range.len(),
        });
    }

    let indices = threshold_indices(values, &thresholds)?;
    let range = indices
        .into_iter()
        .map(|i| {
            i.map(|i| range[i].clone())
                .unwrap_or_else(|| default_value.clone())
        })
        .collect();

    Ok(ScalarOrArray::new_array(range))
}

fn threshold_indices(
    values: &ArrayRef,
    thresholds: &[f32],
) -> Result<Vec<Option<usize>>, AvengerScaleError> {
    Ok(values
        .as_primitive::<Float32Type>()
        .iter()
        .map(|x| match x {
            Some(x) => {
                if x.is_finite() {
                    let idx = match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
                        Ok(i) => (i + 1) as usize,
                        Err(i) => i as usize,
                    };
                    Some(idx)
                } else {
                    None
                }
            }
            None => None,
        })
        .collect())
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

    use arrow::array::{Float32Array, StringArray};
    use float_cmp::assert_approx_eq;

    use super::*;

    #[test]
    fn test_threshold_scale_basic() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![30.0, 70.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0])),
            options: HashMap::new(),
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

    #[test]
    fn test_validate_range_length() -> Result<(), AvengerScaleError> {
        // Tese are fine
        threshold_scale(
            &(Arc::new(Float32Array::from(vec![-1.0, 1.0])) as ArrayRef),
            &(Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0])) as ArrayRef),
            vec![0.0, 1.0, 2.0, 3.0],
            0.0,
        )?;
        threshold_scale(
            &(Arc::new(Float32Array::from(vec![-1.0, 1.0, 3.0, 3.0])) as ArrayRef),
            &(Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0])) as ArrayRef),
            vec![0, 1, 2, 3, 4, 5],
            0,
        )?;

        // Invalid number of elements between thresholds and range
        let err = threshold_scale(
            &(Arc::new(Float32Array::from(vec![-1.0, 1.0, 3.0, 3.0])) as ArrayRef),
            &(Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0, 3.0])) as ArrayRef),
            vec![0, 1, 2, 3],
            0,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AvengerScaleError::ThresholdDomainMismatch {
                domain_len: 4,
                range_len: 4,
            }
        ));

        // Non-ascending thresholds
        let err = threshold_scale(
            &(Arc::new(Float32Array::from(vec![-1.0, 1.0, 4.0, 3.0])) as ArrayRef),
            &(Arc::new(Float32Array::from(vec![0.0, 1.0, 4.0, 3.0])) as ArrayRef),
            vec![0, 1, 2, 3, 4],
            0,
        )
        .unwrap_err();
        assert!(matches!(err, AvengerScaleError::ThresholdsNotAscending(_)));

        Ok(())
    }
}
