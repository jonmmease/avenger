use std::{collections::HashMap, sync::Arc};

use super::{
    ConfiguredScale, InferDomainFromDataMethod, OptionDefinition, ScaleConfig, ScaleContext,
    ScaleImpl,
};
use crate::error::AvengerScaleError;
use lazy_static::lazy_static;

use crate::scalar::Scalar;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array, UInt32Array},
    compute::kernels::cast,
    datatypes::{DataType, UInt32Type},
};
use avenger_common::{
    types::{AreaOrientation, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use serde::de::DeserializeOwned;

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
                ordinal_scale_to(values, &config.domain, range_vec, default_value)
            }
        }
    };
}

/// Ordinal scale that maps discrete domain values to discrete range values by position.
///
/// Each unique value in the domain is mapped to the corresponding value in the range
/// array by index. The first domain value maps to the first range value, the second
/// to the second, and so on. Values not present in the domain produce null/default outputs.
///
/// The range can contain any type of values: numbers, strings, colors, or even enum
/// values like StrokeCap or ImageAlign. The domain and range must have the same length.
///
/// # Config Options
///
/// This scale does not currently support any configuration options.
#[derive(Debug, Clone)]
pub struct OrdinalScale;

impl OrdinalScale {
    pub fn configured(domain: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float32Array::from(Vec::<f32>::new())),
                options: HashMap::new(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for OrdinalScale {
    fn scale_type(&self) -> &'static str {
        "ordinal"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                // Ordinal scale supports no custom options currently
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
        // Get dictionary array with range indices
        let range_dict_array =
            range_dict_array_for_values(&config.domain, config.range.len(), values)?;

        // The dictionary now has indices that need to be used to take from the range
        let dict_array = range_dict_array.as_any_dictionary();
        let indices_array = dict_array.values(); // These are the range indices

        // Use take to get the actual range values in the correct order
        use arrow::compute::kernels::take;
        let range_values = take::take(&config.range, &indices_array, None)?;

        // Replace the dictionary values with the actual range values
        let range_dict_with_values = dict_array.with_values(range_values);

        Ok(range_dict_with_values)
    }

    // Enums
    impl_ordinal_enum_scale_method!(StrokeCap);
    impl_ordinal_enum_scale_method!(StrokeJoin);
    impl_ordinal_enum_scale_method!(ImageAlign);
    impl_ordinal_enum_scale_method!(ImageBaseline);
    impl_ordinal_enum_scale_method!(AreaOrientation);
}

/// Helper function to get dictionary array with range indices corresponding to values
fn range_dict_array_for_values(
    domain: &ArrayRef,
    range_length: usize,
    values: &ArrayRef,
) -> Result<ArrayRef, AvengerScaleError> {
    // If values is already a dictionary array, we need to handle it specially
    let values_to_process = if let DataType::Dictionary(_, value_type) = values.data_type() {
        // If domain type matches the dictionary's value type, cast the dictionary values
        if domain.data_type() == value_type.as_ref() {
            cast(values, domain.data_type())?
        } else {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "dictionary value type does not match domain type".to_string(),
            ));
        }
    } else if values.data_type() != domain.data_type() {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "values and domain have different types".to_string(),
        ));
    } else {
        values.clone()
    };

    if range_length != domain.len() {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "range length does not match domain length".to_string(),
        ));
    }

    // Convert domain and range to vectors of Scalars
    let domain_values = (0..domain.len())
        .map(|i| Scalar::try_from_array(domain.as_ref(), i).unwrap())
        .collect::<Vec<_>>();

    // Cast values to dictionary array
    let dict_type = DataType::Dictionary(
        Box::new(DataType::Int16),
        Box::new(domain.data_type().clone()),
    );
    let dict_array = cast(&values_to_process, &dict_type)?;

    // Downcast to dictionary with erased types
    let dict_array = dict_array.as_any_dictionary();

    // Get array of unique domain values that are observed in the values
    let observed_domain_array = dict_array.values();
    let observed_domain_values = (0..observed_domain_array.len())
        .map(|i| Scalar::try_from_array(observed_domain_array, i))
        .collect::<Result<Vec<_>, AvengerScaleError>>()?;

    // Create a mapping from domain values to indices into range values
    let mapping = domain_values
        .into_iter()
        .enumerate()
        .map(|(i, v)| (v, i as u32))
        .collect::<HashMap<_, _>>();

    // Build corresponding array of range value indices that correspond to the observed domain values
    let observed_range_indices = Arc::new(UInt32Array::from(
        observed_domain_values
            .iter()
            .map(|d| mapping.get(d).cloned())
            .collect::<Vec<_>>(),
    )) as ArrayRef;

    // Replace domain values with range indices
    let range_dict_array = dict_array.with_values(observed_range_indices);

    // Return the dictionary array
    Ok(range_dict_array)
}

/// Generic helper function for evaluating ordinal scales
fn ordinal_scale_to<R: Sync + Clone>(
    values: &ArrayRef,
    domain: &ArrayRef,
    range: Vec<R>,
    default_value: R,
) -> Result<ScalarOrArray<R>, AvengerScaleError> {
    // Get dictionary array with range indices
    let range_dict_array = range_dict_array_for_values(domain, range.len(), values)?;

    // Use dictionary array efficiently without casting
    let dict_array = range_dict_array.as_any_dictionary();

    // Get the unique range indices from the dictionary values
    let unique_indices = dict_array.values();
    let unique_indices = unique_indices.as_primitive::<UInt32Type>();

    // Create mapping from unique values to range values
    let mut unique_range_values = Vec::with_capacity(unique_indices.len());
    for i in 0..unique_indices.len() {
        let idx = unique_indices.value(i) as usize;
        unique_range_values.push(range[idx].clone());
    }

    // Get the dictionary keys using normalized_keys()
    let keys = dict_array.normalized_keys();

    // Map keys to final values
    let mut scaled_values = Vec::with_capacity(values.len());
    for (i, key) in keys.into_iter().enumerate() {
        if dict_array.is_null(i) {
            scaled_values.push(default_value.clone());
        } else {
            scaled_values.push(unique_range_values[key].clone());
        }
    }

    Ok(ScalarOrArray::new_array(scaled_values))
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
                let v: R = serde_json::from_value(serde_json::Value::String(s.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float32Array, StringArray};
    use arrow::compute::kernels::cast::cast;
    use arrow::datatypes::Float32Type;
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
            context: ScaleContext::default(),
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
            context: ScaleContext::default(),
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

    #[test]
    fn test_ordinal_scale_returns_dictionary() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
        let range = Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef;

        // Create scale
        let scale = OrdinalScale;
        let config = ScaleConfig {
            domain,
            range,
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Create input values with repetitions
        let values =
            Arc::new(StringArray::from(vec!["b", "a", "c", "b", "a", "c", "b"])) as ArrayRef;

        // Apply scale
        let result = scale.scale(&config, &values)?;

        // Verify the result is a dictionary array
        assert!(matches!(result.data_type(), DataType::Dictionary(_, _)));

        // Check the dictionary type
        if let DataType::Dictionary(key_type, value_type) = result.data_type() {
            assert_eq!(key_type.as_ref(), &DataType::Int16);
            assert_eq!(value_type.as_ref(), &DataType::Float32);
        }

        // Verify we can cast it back to get the correct values
        let cast_result = cast(&result, &DataType::Float32)?;
        let float_array = cast_result.as_primitive::<Float32Type>();

        assert_eq!(float_array.value(0), 2.0); // "b" -> 2.0
        assert_eq!(float_array.value(1), 1.0); // "a" -> 1.0
        assert_eq!(float_array.value(2), 3.0); // "c" -> 3.0
        assert_eq!(float_array.value(3), 2.0); // "b" -> 2.0
        assert_eq!(float_array.value(4), 1.0); // "a" -> 1.0
        assert_eq!(float_array.value(5), 3.0); // "c" -> 3.0
        assert_eq!(float_array.value(6), 2.0); // "b" -> 2.0

        Ok(())
    }

    #[test]
    fn test_ordinal_scale_with_colors_returns_dictionary() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = Arc::new(StringArray::from(vec!["red", "green", "blue"])) as ArrayRef;
        let range = Arc::new(StringArray::from(vec!["#ff0000", "#00ff00", "#0000ff"])) as ArrayRef;

        // Create scale
        let scale = OrdinalScale;
        let config = ScaleConfig {
            domain,
            range,
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Create input values with repetitions
        let values = Arc::new(StringArray::from(vec![
            "red", "blue", "green", "red", "red", "blue", "green",
        ])) as ArrayRef;

        // Apply scale
        let result = scale.scale(&config, &values)?;

        // Verify the result is a dictionary array
        assert!(matches!(result.data_type(), DataType::Dictionary(_, _)));

        // Check the dictionary type
        if let DataType::Dictionary(key_type, value_type) = result.data_type() {
            assert_eq!(key_type.as_ref(), &DataType::Int16);
            assert_eq!(value_type.as_ref(), &DataType::Utf8);
        }

        // Get the dictionary array
        let dict_array = result.as_any_dictionary();

        // Check that the dictionary values are unique (should be 3 unique colors)
        let dict_values = dict_array.values();
        assert_eq!(dict_values.len(), 3); // Should only have 3 unique values

        // Verify we can cast it back to get the correct values
        let cast_result = cast(&result, &DataType::Utf8)?;
        let string_array = cast_result.as_string::<i32>();

        assert_eq!(string_array.value(0), "#ff0000"); // "red" -> "#ff0000"
        assert_eq!(string_array.value(1), "#0000ff"); // "blue" -> "#0000ff"
        assert_eq!(string_array.value(2), "#00ff00"); // "green" -> "#00ff00"
        assert_eq!(string_array.value(3), "#ff0000"); // "red" -> "#ff0000"
        assert_eq!(string_array.value(4), "#ff0000"); // "red" -> "#ff0000"
        assert_eq!(string_array.value(5), "#0000ff"); // "blue" -> "#0000ff"
        assert_eq!(string_array.value(6), "#00ff00"); // "green" -> "#00ff00"

        Ok(())
    }

    #[test]
    fn test_ordinal_scale_with_nulls() -> Result<(), AvengerScaleError> {
        // Test that our optimized ordinal_scale_to handles nulls correctly
        // Create domain and range arrays
        let domain = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
        let range = Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0])) as ArrayRef;

        // Create scale
        let scale = OrdinalScale;
        let config = ScaleConfig {
            domain,
            range,
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Create input values with nulls
        let values = Arc::new(StringArray::from(vec![
            Some("a"),
            None,
            Some("b"),
            Some("d"), // not in domain
            None,
            Some("c"),
        ])) as ArrayRef;

        // Apply scale
        let result = scale.scale_to_numeric(&config, &values)?;
        let result = result.as_vec(values.len(), None);

        assert_eq!(result[0], 10.0); // "a" -> 10.0
        assert!(result[1].is_nan()); // null -> NaN
        assert_eq!(result[2], 20.0); // "b" -> 20.0
        assert!(result[3].is_nan()); // "d" (not in domain) -> NaN
        assert!(result[4].is_nan()); // null -> NaN
        assert_eq!(result[5], 30.0); // "c" -> 30.0

        Ok(())
    }
}
