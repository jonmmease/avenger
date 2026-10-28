//! Conversions between the fields of a chart message and the values of Arrow and avenger-scales:
//! list values and scale options.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, StringArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Float64Type},
};
use avenger_scales::scalar::Scalar;

use crate::{
    proto::{self, option_value},
    ChartError,
};

/// `values` as a list value: an array of one `List` element, the form of the fields that hold a
/// list, such as an explicit domain, a range, or tick values.
pub fn list(values: ArrayRef) -> ArrayRef {
    let field = Arc::new(Field::new_list_field(values.data_type().clone(), true));
    Arc::new(ListArray::new(
        field,
        OffsetBuffer::from_lengths([values.len()]),
        values,
        None,
    ))
}

/// The scale options, in their order, as the one-element `Scalar`s of avenger-scales.
pub fn options(options: &[proto::NamedOption]) -> Result<Vec<(String, Scalar)>, ChartError> {
    options
        .iter()
        .map(|option| {
            let value = option
                .value
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .ok_or_else(|| {
                    ChartError::Payload(format!("the option `{}` has no value", option.name))
                })?;
            let scalar = match value {
                option_value::Value::Boolean(value) => Scalar::from_bool(*value),
                option_value::Value::Float(value) => Scalar::from_f64(*value),
                option_value::Value::Integer(value) => {
                    Scalar(Arc::new(Int64Array::from(vec![*value])))
                }
                option_value::Value::String(value) => Scalar::from_string(value),
                option_value::Value::Floats(list) => Scalar::from_f64_list(&list.values),
                option_value::Value::Strings(list) => {
                    let values: Vec<&str> = list.values.iter().map(String::as_str).collect();
                    Scalar::from_string_list(&values)
                }
            };
            Ok((option.name.clone(), scalar))
        })
        .collect()
}

/// The option value of a one-element `Scalar` of `Boolean`, `Float64`, `Int64`, `Utf8`, or a list
/// of `Float64` or `Utf8`.
pub fn encode_option(scalar: &Scalar) -> Result<proto::OptionValue, ChartError> {
    let array = &scalar.0;
    if array.len() != 1 || array.is_null(0) {
        return Err(ChartError::Payload(
            "an option holds one value that is not null".to_string(),
        ));
    }
    let value = match array.data_type() {
        DataType::Boolean => option_value::Value::Boolean(
            array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("a Boolean array")
                .value(0),
        ),
        DataType::Float64 => option_value::Value::Float(
            array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("a Float64 array")
                .value(0),
        ),
        DataType::Int64 => option_value::Value::Integer(
            array
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("an Int64 array")
                .value(0),
        ),
        DataType::Utf8 => option_value::Value::String(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a Utf8 array")
                .value(0)
                .to_string(),
        ),
        DataType::List(field) if field.data_type() == &DataType::Float64 => {
            let list = array
                .as_any()
                .downcast_ref::<ListArray>()
                .expect("a list array")
                .value(0);
            let values = arrow::array::AsArray::as_primitive::<Float64Type>(&list);
            option_value::Value::Floats(proto::FloatList {
                values: values.values().to_vec(),
            })
        }
        DataType::List(field) if field.data_type() == &DataType::Utf8 => {
            let list = array
                .as_any()
                .downcast_ref::<ListArray>()
                .expect("a list array")
                .value(0);
            let values = arrow::array::AsArray::as_string::<i32>(&list);
            option_value::Value::Strings(proto::StringList {
                values: values
                    .iter()
                    .map(|value| value.unwrap_or("").to_string())
                    .collect(),
            })
        }
        other => {
            return Err(ChartError::Payload(format!(
                "an option is a Boolean, a Float64, an Int64, a Utf8, or a list of Float64 or \
                 Utf8, not {other}"
            )))
        }
    };
    Ok(proto::OptionValue { value: Some(value) })
}
