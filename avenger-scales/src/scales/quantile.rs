use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{
        kernels::{cast, sort},
        SortOptions,
    },
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::{AreaOrientation, ColorOrGradient, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use datafusion_common::ScalarValue;

use crate::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    error::AvengerScaleError, formatter::Formatters,
};

use super::{
    linear::LinearScale,
    ordinal::{
        prep_discrete_color_range, prep_discrete_enum_range, prep_discrete_numeric_range,
        prep_discrete_string_range,
    },
    ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleImpl,
};

/// Macro to generate scale_to_X trait methods for threshold enum scaling
#[macro_export]
macro_rules! impl_quantile_enum_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            fn [<scale_to_ $type_name:snake>](
                &self,
                config: &ScaleConfig,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                let (range_vec, default_value) = prep_discrete_enum_range::<$type_name>(config)?;
                quantile_scale(values, &config.domain, range_vec, default_value)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct QuantileScale;

impl QuantileScale {
    pub fn new(domain: Vec<f32>, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(domain)),
                range,
                options: HashMap::new(),
            },
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            formatters: Formatters::default(),
        }
    }
}

impl ScaleImpl for QuantileScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    /// Scale to numeric values
    fn scale_to_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // let domain_span = config.numeric_interval_domain()?;
        let (range_vec, default_value) = prep_discrete_numeric_range(config)?;
        quantile_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_string_range(config)?;
        quantile_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        _color_interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_color_range(config)?;
        quantile_scale(values, &config.domain, range_vec, default_value)
    }

    // Enums
    impl_quantile_enum_scale_method!(StrokeCap);
    impl_quantile_enum_scale_method!(StrokeJoin);
    impl_quantile_enum_scale_method!(ImageAlign);
    impl_quantile_enum_scale_method!(ImageBaseline);
    impl_quantile_enum_scale_method!(AreaOrientation);

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

/// Generic helper function for evaluating quantize scales
fn quantile_scale<R: Sync + Clone>(
    values: &ArrayRef,
    domain: &ArrayRef,
    range: Vec<R>,
    default_value: R,
) -> Result<ScalarOrArray<R>, AvengerScaleError> {
    let n = range.len();
    if n <= 1 || domain.is_empty() {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "Quantile scale requires a non-empty domain and range".to_string(),
        ));
    }

    let thresholds = quantile_thresholds(domain, n)?;
    Ok(ScalarOrArray::new_array(
        values
            .as_primitive::<Float32Type>()
            .iter()
            .map(|x| match x {
                Some(x) => {
                    if x.is_finite() {
                        let idx = match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap())
                        {
                            Ok(i) => (i + 1) as usize,
                            Err(i) => i as usize,
                        };
                        range[idx].clone()
                    } else {
                        default_value.clone()
                    }
                }
                None => default_value.clone(),
            })
            .collect(),
    ))
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
