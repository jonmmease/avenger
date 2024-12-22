use std::{collections::HashMap, sync::Arc};

use crate::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    error::AvengerScaleError,
    formatter::Formatters,
    utils::ScalarValueUtils,
};
use arrow::{
    array::{ArrayRef, AsArray, DictionaryArray, UInt32Array},
    compute::kernels::{cast, take},
    datatypes::{DataType, Float32Type, UInt32Type, Utf8Type},
};
use avenger_common::{
    types::{AreaOrientation, ColorOrGradient, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use datafusion_common::{DataFusionError, ScalarValue};
use serde::{de::DeserializeOwned, Deserialize};

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleImpl};

/// Macro to generate scale_to_X trait methods for ordinal enum scaling
#[macro_export]
macro_rules! impl_ordinal_enum_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            fn [<scale_to_ $type_name:snake>](
                &self,
                config: &ScaleConfig,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                let (range_vec, default_value) = prep_discrete_enum_range::<$type_name>(config)?;
                ordinal_scale(values, &config.domain, range_vec, default_value)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct OrdinalScale;

impl OrdinalScale {
    pub fn new(domain: ArrayRef, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range,
                options: HashMap::new(),
            },
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            formatters: Formatters::default(),
        }
    }
}

impl ScaleImpl for OrdinalScale {
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
        ordinal_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_string_range(config)?;
        ordinal_scale(values, &config.domain, range_vec, default_value)
    }

    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        _color_interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let (range_vec, default_value) = prep_discrete_color_range(config)?;
        ordinal_scale(values, &config.domain, range_vec, default_value)
    }

    // Enums
    impl_ordinal_enum_scale_method!(StrokeCap);
    impl_ordinal_enum_scale_method!(StrokeJoin);
    impl_ordinal_enum_scale_method!(ImageAlign);
    impl_ordinal_enum_scale_method!(ImageBaseline);
    impl_ordinal_enum_scale_method!(AreaOrientation);
}

pub(crate) fn prep_discrete_numeric_range(
    config: &ScaleConfig,
) -> Result<(Vec<f32>, f32), AvengerScaleError> {
    let range = config.range.as_primitive::<Float32Type>();
    let default_value = config.f32_option("default", f32::NAN);
    Ok((
        range.iter().map(|i| i.unwrap_or(default_value)).collect(),
        default_value,
    ))
}

pub(crate) fn prep_discrete_string_range(
    config: &ScaleConfig,
) -> Result<(Vec<String>, String), AvengerScaleError> {
    // Try to convert range to Utf8
    let range = cast(&config.range, &DataType::Utf8).map_err(|_| {
        AvengerScaleError::ScaleOperationNotSupported(
            "ordinal scale range is not a string array".to_string(),
        )
    })?;
    let default_value = config.string_option("default", "");
    let range_vec = range
        .as_string::<i32>()
        .iter()
        .map(|i| {
            i.map(|v| v.to_string())
                .unwrap_or_else(|| default_value.clone())
        })
        .collect::<Vec<_>>();
    Ok((range_vec, default_value))
}

pub(crate) fn prep_discrete_color_range(
    config: &ScaleConfig,
) -> Result<(Vec<ColorOrGradient>, ColorOrGradient), AvengerScaleError> {
    // Get default color
    let default_color = config
        .options
        .get("default")
        .cloned()
        .unwrap_or("transparent".into())
        .as_rgba()?;

    // Get range colors
    let range_vec = (0..config.range.len())
        .map(|i| {
            let rgba = ScalarValue::try_from_array(&config.range, i)
                .map(|v| v.as_rgba().unwrap_or(default_color))?;
            Ok(ColorOrGradient::Color(rgba))
        })
        .collect::<Result<Vec<_>, DataFusionError>>()?;
    Ok((range_vec, ColorOrGradient::Color(default_color)))
}

pub(crate) fn prep_discrete_enum_range<R: Sync + Clone + DeserializeOwned + Default>(
    config: &ScaleConfig,
) -> Result<(Vec<R>, R), AvengerScaleError> {
    // Try to convert range to Utf8
    let range = cast(&config.range, &DataType::Utf8).map_err(|_| {
        AvengerScaleError::ScaleOperationNotSupported(
            "ordinal scale range is not a string array".to_string(),
        )
    })?;
    let default_value = R::default();
    let mut range_vec = Vec::with_capacity(range.len());
    for s in range.as_string::<i32>().iter() {
        match s {
            Some(s) => {
                let v: R = serde_json::from_value(serde_json::Value::String((s.to_string())))
                    .unwrap_or(default_value.clone());
                range_vec.push(v);
            }
            None => {
                range_vec.push(default_value.clone());
            }
        }
    }
    Ok((range_vec, default_value))
}

/// Generic helper function for evaluating ordinal scales
fn ordinal_scale<R: Sync + Clone>(
    values: &ArrayRef,
    domain: &ArrayRef,
    range: Vec<R>,
    default_value: R,
) -> Result<ScalarOrArray<R>, AvengerScaleError> {
    // values and domain should have the same type
    if values.data_type() != domain.data_type() {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "values and domain have different types".to_string(),
        ));
    }

    if range.len() != domain.len() {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "range length does not match domain length".to_string(),
        ));
    }

    // Convert domain and range to vectors of ScalarValues
    let domain_values = (0..domain.len())
        .map(|i| ScalarValue::try_from_array(domain.as_ref(), i).unwrap())
        .collect::<Vec<_>>();

    // Create a mapping from domain values to indices into range values
    let mapping = domain_values
        .into_iter()
        .enumerate()
        .map(|(i, v)| (v, i as u32))
        .collect::<HashMap<_, _>>();

    // Cast values to dictionary array
    let dict_type = DataType::Dictionary(
        Box::new(DataType::Int16),
        Box::new(domain.data_type().clone()),
    );
    let dict_array = cast(values, &dict_type)?;

    // Downcast to dictionary with erased types
    let dict_array = dict_array.as_any_dictionary();

    // Get array of unique domain values that are observed in the values
    let observed_domain_array = dict_array.values();
    let observed_domain_values = (0..observed_domain_array.len())
        .map(|i| ScalarValue::try_from_array(observed_domain_array, i))
        .collect::<Result<Vec<_>, DataFusionError>>()?;

    // Build corresponding array of range value indices that correspond to the observed domain values
    let observed_range_indices = Arc::new(UInt32Array::from(
        observed_domain_values
            .iter()
            .map(|d| mapping.get(d).cloned())
            .collect::<Vec<_>>(),
    )) as ArrayRef;

    // Replace domain values with range indices
    let range_dict_array = dict_array.with_values(observed_range_indices);

    // Cast range indices to flat u32 array
    let range_array = cast(&range_dict_array, &DataType::UInt32)?;
    let range_indices = range_array.as_primitive::<UInt32Type>();
    let scaled_values = range_indices
        .iter()
        .map(|i| {
            i.map(|v| range[v as usize].clone())
                .unwrap_or_else(|| default_value.clone())
        })
        .collect::<Vec<_>>();

    Ok(ScalarOrArray::new_array(scaled_values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float32Array, StringArray};
    use std::sync::Arc;

    #[test]
    fn test_simple_ordinal_scale() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
        let range = Arc::new(Float32Array::from(vec![1.4, 2.5, 3.6])) as ArrayRef;

        // Create scale
        let scale = OrdinalScale;
        let config = ScaleConfig {
            domain,
            range,
            options: HashMap::new(),
        };

        // Create input values to scale
        let values = Arc::new(StringArray::from(vec!["b", "a", "d", "b", "d"])) as ArrayRef;

        // Apply scale
        let result = scale.scale_to_numeric(&config, &values)?;

        // Convert to string array and verify results
        let result = result.as_vec(values.len(), None);
        assert_eq!(result[0], 2.5);
        assert_eq!(result[1], 1.4);
        assert!(result[2].is_nan());
        assert_eq!(result[3], 2.5);
        assert!(result[4].is_nan());

        Ok(())
    }

    #[test]
    fn test_ordinal_stroke_cap() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
        let range = Arc::new(StringArray::from(vec!["butt", "round", "square"])) as ArrayRef;

        // Create scale
        let scale = OrdinalScale;
        let config = ScaleConfig {
            domain,
            range,
            options: HashMap::new(),
        };

        // Create input values to scale
        let values = Arc::new(StringArray::from(vec!["b", "a", "d", "b", "d"])) as ArrayRef;

        // Apply scale
        let result = scale.scale_to_stroke_cap(&config, &values)?;

        println!("{:?}", result);

        // Convert to string array and verify results
        let result = result.as_vec(values.len(), None);
        assert_eq!(result[0], StrokeCap::Round);
        assert_eq!(result[1], StrokeCap::Butt);
        assert_eq!(result[2], StrokeCap::default());
        assert_eq!(result[3], StrokeCap::Round);
        assert_eq!(result[4], StrokeCap::default());

        Ok(())
    }
}
