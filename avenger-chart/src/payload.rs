//! The Arrow fields of a chart message other than its tables: one-row and one-column IPC streams
//! of constants, partition values, domains, ranges, and selection values, domain summaries, domain
//! fits, and scale options.
//!
//! Each decoder reads its field without a copy of aligned buffers, as the tables do. These fields
//! are small, so a renderer decodes the ones that it needs one time, when it prepares a chart.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, StringArray},
    datatypes::{DataType, Field, Float64Type, Schema},
    record_batch::RecordBatch,
};
use avenger_scales::{
    scalar::Scalar,
    scales::domain_summary::{
        CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, IntervalOptions,
    },
};
use bytes::Bytes;

use crate::{
    codec::{decode_ipc, encode_ipc},
    proto::{self, category_fit, domain_fit, option_value},
    ChartError,
};

/// The column of a one-column IPC stream, such as an explicit domain or a range.
pub fn array(bytes: &Bytes) -> Result<ArrayRef, ChartError> {
    let batch = decode_ipc(bytes)?;
    if batch.num_columns() != 1 {
        return Err(ChartError::Payload(format!(
            "the field holds one column, not {}",
            batch.num_columns()
        )));
    }
    Ok(batch.column(0).clone())
}

/// The value of a one-row and one-column IPC stream, such as a constant or a partition value, as
/// an array of one element.
pub fn value(bytes: &Bytes) -> Result<ArrayRef, ChartError> {
    let array = array(bytes)?;
    if array.len() != 1 {
        return Err(ChartError::Payload(format!(
            "the field holds one value, not {}",
            array.len()
        )));
    }
    Ok(array)
}

/// The one-column IPC stream of `array`, as `array` decodes it.
pub fn encode_array(array: &ArrayRef) -> Result<Bytes, ChartError> {
    let schema = Schema::new(vec![Field::new("value", array.data_type().clone(), true)]);
    let batch = RecordBatch::try_new(Arc::new(schema), vec![array.clone()])?;
    Ok(Bytes::from(encode_ipc(&batch)?))
}

/// The domain summary of a summary domain: the struct of `DomainSummary::to_arrow` in one row.
pub fn summary(bytes: &Bytes) -> Result<DomainSummary, ChartError> {
    let value = value(bytes)?;
    DomainSummary::from_arrow(value.as_ref(), 0)
        .map_err(|error| ChartError::Payload(format!("the summary does not decode: {error}")))
}

/// The bytes of a summary domain, as [`summary`] decodes them.
pub fn encode_summary(summary: &DomainSummary) -> Result<Bytes, ChartError> {
    encode_array(&(Arc::new(summary.to_arrow()) as ArrayRef))
}

/// The `DomainOptions` of avenger-scales for a domain fit. A preferred order of categories fails,
/// because avenger-scales has no preferred order yet.
pub fn fit(fit: &proto::DomainFit) -> Result<DomainOptions, ChartError> {
    match &fit.fit {
        Some(domain_fit::Fit::Interval(interval)) => Ok(DomainOptions::Interval(IntervalOptions {
            zero: interval.zero,
            nice: interval.nice,
            empty: if interval.empty.is_empty() {
                None
            } else {
                Some(array(&interval.empty)?)
            },
        })),
        Some(domain_fit::Fit::Categories(categories)) => {
            let order = match &categories.order {
                Some(category_fit::Order::Sort(sort)) => {
                    match proto::CategorySort::try_from(*sort) {
                        Ok(proto::CategorySort::Ascending) => CategoryOrder::Ascending,
                        Ok(proto::CategorySort::Descending) => CategoryOrder::Descending,
                        _ => {
                            return Err(ChartError::Payload(format!(
                                "the category sort {sort} is not ascending or descending"
                            )))
                        }
                    }
                }
                Some(category_fit::Order::Preferred(_)) => {
                    return Err(ChartError::Payload(
                        "avenger-scales has no preferred order of categories yet".to_string(),
                    ))
                }
                None => {
                    return Err(ChartError::Payload(
                        "a category fit has an order".to_string(),
                    ))
                }
            };
            Ok(DomainOptions::Categories(CategoryOptions { order }))
        }
        None => Err(ChartError::Payload("a domain fit has a kind".to_string())),
    }
}

/// The domain fit of `options`, or `None` for the options of a quantile scale, which has no fit.
/// An explicit order of categories fails, because a chart holds it as an explicit domain.
pub fn encode_fit(options: &DomainOptions) -> Result<Option<proto::DomainFit>, ChartError> {
    let fit = match options {
        DomainOptions::Interval(interval) => domain_fit::Fit::Interval(proto::IntervalFit {
            zero: interval.zero,
            nice: interval.nice,
            empty: match &interval.empty {
                Some(empty) => encode_array(empty)?,
                None => Bytes::new(),
            },
        }),
        DomainOptions::Categories(categories) => {
            let sort = match &categories.order {
                CategoryOrder::Ascending => proto::CategorySort::Ascending,
                CategoryOrder::Descending => proto::CategorySort::Descending,
                CategoryOrder::Explicit(_) => {
                    return Err(ChartError::Payload(
                        "a chart holds an explicit order of categories as an explicit domain"
                            .to_string(),
                    ))
                }
            };
            domain_fit::Fit::Categories(proto::CategoryFit {
                order: Some(category_fit::Order::Sort(sort as i32)),
            })
        }
        DomainOptions::Sample => return Ok(None),
    };
    Ok(Some(proto::DomainFit { fit: Some(fit) }))
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
