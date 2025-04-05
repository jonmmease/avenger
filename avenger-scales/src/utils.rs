use std::sync::Arc;

use crate::error::AvengerScaleError;
use arrow::{
    array::{Array, AsArray, ListArray},
    compute::kernels::cast,
    datatypes::{DataType, Float32Type},
};
use css_color_parser::Color;
use datafusion_common::ScalarValue;

pub trait ScalarValueUtils {
    fn as_f32(&self) -> Result<f32, AvengerScaleError>;
    fn as_boolean(&self) -> Result<bool, AvengerScaleError>;
    fn as_i32(&self) -> Result<i32, AvengerScaleError>;
    fn as_string(&self) -> Result<String, AvengerScaleError>;
    fn as_f32_2(&self) -> Result<[f32; 2], AvengerScaleError>;
    fn as_f32_4(&self) -> Result<[f32; 4], AvengerScaleError>;
    fn as_rgba(&self) -> Result<[f32; 4], AvengerScaleError>;
    fn make_rgba(r: f32, g: f32, b: f32, a: f32) -> ScalarValue;
}

impl ScalarValueUtils for ScalarValue {
    fn as_f32(&self) -> Result<f32, AvengerScaleError> {
        match self {
            ScalarValue::Int16(Some(val)) => Ok(*val as f32),
            ScalarValue::Int32(Some(val)) => Ok(*val as f32),
            ScalarValue::Int64(Some(val)) => Ok(*val as f32),
            ScalarValue::Float32(Some(val)) => Ok(*val),
            ScalarValue::Float64(Some(val)) => Ok(*val as f32),
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to f32: {:?}",
                self
            ))),
        }
    }

    fn as_boolean(&self) -> Result<bool, AvengerScaleError> {
        match self {
            ScalarValue::Boolean(Some(val)) => Ok(*val),
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to boolean: {:?}",
                self
            ))),
        }
    }

    fn as_i32(&self) -> Result<i32, AvengerScaleError> {
        match self {
            ScalarValue::Int16(Some(val)) => Ok(*val as i32),
            ScalarValue::Int32(Some(val)) => Ok(*val),
            ScalarValue::Int64(Some(val)) => Ok(*val as i32),
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to i32: {:?}",
                self
            ))),
        }
    }

    fn as_string(&self) -> Result<String, AvengerScaleError> {
        match self {
            ScalarValue::Utf8(Some(val))
            | ScalarValue::Utf8View(Some(val))
            | ScalarValue::LargeUtf8(Some(val)) => Ok(val.to_string()),
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to utf8: {:?}",
                self
            ))),
        }
    }

    fn as_f32_2(&self) -> Result<[f32; 2], AvengerScaleError> {
        match self {
            ScalarValue::List(list) if list.data_type().is_numeric() => {
                let element = list.value(0);
                let element = cast(&element, &DataType::Float32)?;
                let array = element.as_primitive::<Float32Type>();
                if array.len() != 2 {
                    return Err(AvengerScaleError::InternalError(format!(
                        "ScalarValue is not convertable to [f32; 2]: {:?}",
                        self
                    )));
                }

                let min = array.value(0);
                let max = array.value(1);
                Ok([min, max])
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to f32: {:?}",
                self
            ))),
        }
    }

    fn as_f32_4(&self) -> Result<[f32; 4], AvengerScaleError> {
        match self {
            ScalarValue::List(list) => {
                let element = list.value(0);
                let element = cast(&element, &DataType::Float32)?;
                let array = element.as_primitive::<Float32Type>();
                if array.len() != 4 {
                    return Err(AvengerScaleError::InternalError(format!(
                        "ScalarValue is not convertable to [f32; 4]: {:?}",
                        self
                    )));
                }

                Ok([
                    array.value(0),
                    array.value(1),
                    array.value(2),
                    array.value(3),
                ])
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "ScalarValue is not convertable to [f32; 4]: {:?}",
                self
            ))),
        }
    }

    fn as_rgba(&self) -> Result<[f32; 4], AvengerScaleError> {
        if let Ok(rgba) = self.as_f32_4() {
            // assume already in rgba format
            Ok(rgba)
        } else {
            match self {
                ScalarValue::Utf8(Some(s))
                | ScalarValue::Utf8View(Some(s))
                | ScalarValue::LargeUtf8(Some(s)) => match s.parse::<Color>() {
                    Ok(color) => Ok([
                        color.r as f32 / 255.0,
                        color.g as f32 / 255.0,
                        color.b as f32 / 255.0,
                        color.a,
                    ]),
                    Err(e) => Err(AvengerScaleError::InternalError(format!(
                        "ScalarValue is not convertable to an rgba color: {:?}\n{e:?}",
                        self
                    ))),
                },
                _ => Err(AvengerScaleError::InternalError(format!(
                    "ScalarValue is not convertable to an rgba color: {:?}",
                    self
                ))),
            }
        }
    }

    fn make_rgba(r: f32, g: f32, b: f32, a: f32) -> ScalarValue {
        let list_array = ListArray::from_iter_primitive::<Float32Type, _, _>(vec![Some(vec![
            Some(r),
            Some(g),
            Some(b),
            Some(a),
        ])]);

        ScalarValue::List(Arc::new(list_array))
    }
}
