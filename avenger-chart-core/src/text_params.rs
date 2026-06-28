use avenger_text::{LabelParamValue, LabelParams};
use datafusion::{arrow::array::Array, common::ScalarValue, error::DataFusionError};
use indexmap::IndexMap;

use crate::{ArrayRefHelpers, AvengerChartError};

pub fn scalar_params_to_label_params(
    params: &IndexMap<String, ScalarValue>,
) -> Result<LabelParams, AvengerChartError> {
    params
        .iter()
        .map(|(name, value)| Ok((name.clone(), scalar_value_to_label_param(value)?)))
        .collect()
}

pub fn scalar_value_to_label_param(
    value: &ScalarValue,
) -> Result<LabelParamValue, AvengerChartError> {
    Ok(match value {
        ScalarValue::Null => LabelParamValue::None,
        ScalarValue::Boolean(value) => value
            .map(LabelParamValue::Bool)
            .unwrap_or(LabelParamValue::None),
        ScalarValue::Int8(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::Int16(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::Int32(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::Int64(value) => value
            .map(LabelParamValue::Int)
            .unwrap_or(LabelParamValue::None),
        ScalarValue::UInt8(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::UInt16(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::UInt32(value) => value
            .map(|value| LabelParamValue::Int(value as i64))
            .unwrap_or(LabelParamValue::None),
        ScalarValue::UInt64(value) => match value {
            Some(value) => LabelParamValue::Int(i64::try_from(*value).map_err(|_| {
                AvengerChartError::InvalidArgument(format!(
                    "Typst label parameter unsigned integer {value} does not fit in i64"
                ))
            })?),
            None => LabelParamValue::None,
        },
        ScalarValue::Float32(value) => match value {
            Some(value) if value.is_finite() => LabelParamValue::Float(*value as f64),
            Some(value) => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Typst label parameter float must be finite, got {value}"
                )));
            }
            None => LabelParamValue::None,
        },
        ScalarValue::Float64(value) => match value {
            Some(value) if value.is_finite() => LabelParamValue::Float(*value),
            Some(value) => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Typst label parameter float must be finite, got {value}"
                )));
            }
            None => LabelParamValue::None,
        },
        ScalarValue::Utf8(value) | ScalarValue::LargeUtf8(value) | ScalarValue::Utf8View(value) => {
            value
                .as_ref()
                .map(|value| LabelParamValue::Str(value.clone()))
                .unwrap_or(LabelParamValue::None)
        }
        ScalarValue::List(array) => {
            if array.is_null(0) {
                LabelParamValue::None
            } else {
                LabelParamValue::Array(
                    array
                        .value(0)
                        .to_scalar_vec()
                        .map_err(datafusion_to_chart_error)?
                        .iter()
                        .map(scalar_value_to_label_param)
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
        }
        ScalarValue::Struct(array) => {
            if array.is_null(0) {
                LabelParamValue::None
            } else {
                let mut dict = IndexMap::new();
                for (index, field) in array.fields().iter().enumerate() {
                    let value = ScalarValue::try_from_array(array.column(index), 0)
                        .map_err(datafusion_to_chart_error)?;
                    dict.insert(field.name().clone(), scalar_value_to_label_param(&value)?);
                }
                LabelParamValue::Dict(dict)
            }
        }
        other if other.is_null() => LabelParamValue::None,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "ScalarValue variant {other:?} is not supported as a Typst label parameter"
            )));
        }
    })
}

fn datafusion_to_chart_error(error: DataFusionError) -> AvengerChartError {
    AvengerChartError::DataFusionError(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::common::ScalarValue;

    #[test]
    fn converts_basic_scalar_params() {
        let mut params = IndexMap::new();
        params.insert(
            "name".to_string(),
            ScalarValue::Utf8(Some("Revenue".into())),
        );
        params.insert("count".to_string(), ScalarValue::Int64(Some(42)));
        params.insert("ratio".to_string(), ScalarValue::Float64(Some(0.5)));
        params.insert("active".to_string(), ScalarValue::Boolean(Some(true)));
        params.insert("missing".to_string(), ScalarValue::Null);

        let converted = scalar_params_to_label_params(&params).unwrap();

        assert_eq!(
            converted.get("name"),
            Some(&LabelParamValue::Str("Revenue".to_string()))
        );
        assert_eq!(converted.get("count"), Some(&LabelParamValue::Int(42)));
        assert_eq!(converted.get("ratio"), Some(&LabelParamValue::Float(0.5)));
        assert_eq!(converted.get("active"), Some(&LabelParamValue::Bool(true)));
        assert_eq!(converted.get("missing"), Some(&LabelParamValue::None));
    }

    #[test]
    fn rejects_non_finite_float_param() {
        let err = scalar_value_to_label_param(&ScalarValue::Float64(Some(f64::NAN))).unwrap_err();
        assert!(matches!(err, AvengerChartError::InvalidArgument(_)));
    }

    #[test]
    fn rejects_out_of_range_unsigned_param() {
        let err = scalar_value_to_label_param(&ScalarValue::UInt64(Some(u64::MAX))).unwrap_err();
        assert!(matches!(err, AvengerChartError::InvalidArgument(_)));
    }
}
