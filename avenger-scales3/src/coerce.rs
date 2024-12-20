use arrow::array::AsArray;
use arrow::datatypes::Float32Type;
use arrow::{
    array::ArrayRef,
    compute::kernels::cast,
    datatypes::{DataType, Field},
};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use css_color_parser::Color;

use crate::error::AvengerScaleError;

pub trait ColorCoercer: Send + Sync + 'static {
    fn coerce_color(
        &self,
        value: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError>;
}

pub trait NumericCoercer: Send + Sync + 'static {
    fn coerce_numeric(&self, value: &ArrayRef) -> Result<ScalarOrArray<f32>, AvengerScaleError>;
}

#[derive(Default, Clone, Copy)]
pub struct CastNumericCoercer;

impl NumericCoercer for CastNumericCoercer {
    fn coerce_numeric(&self, value: &ArrayRef) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::Float32)?;
        let result = cast_array.as_primitive::<Float32Type>();
        Ok(ScalarOrArray::new_array(result.values().to_vec()))
    }
}

#[derive(Default, Clone, Copy)]
pub struct CssColorCoercer;

impl ColorCoercer for CssColorCoercer {
    fn coerce_color(
        &self,
        value: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let dtype = value.data_type();
        match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // cast to normalize to utf8
                let cast_array = cast(value, &DataType::Utf8)?;
                let string_array = cast_array.as_string::<i32>();
                let result = string_array
                    .iter()
                    .map(|el| match el {
                        Some(el) => el
                            .parse::<Color>()
                            .map(|color| {
                                ColorOrGradient::Color([
                                    color.r as f32 / 255.0,
                                    color.g as f32 / 255.0,
                                    color.b as f32 / 255.0,
                                    color.a,
                                ])
                            })
                            .unwrap_or_else(|_| ColorOrGradient::transparent()),
                        _ => ColorOrGradient::transparent(),
                    })
                    .collect::<Vec<_>>();
                Ok(ScalarOrArray::new_array(result))
            }

            DataType::List(field)
            | DataType::ListView(field)
            | DataType::FixedSizeList(field, _)
            | DataType::LargeList(field)
            | DataType::LargeListView(field)
                if field.data_type().is_numeric() =>
            {
                // Cast to normalize to list of f32 arrays
                let cast_type = DataType::List(Field::new("item", DataType::Float32, true).into());
                let cast_array = cast(value, &cast_type)?;
                let list_array = cast_array.as_list::<i32>();
                let result = list_array
                    .iter()
                    .map(|el| match el {
                        Some(el) if el.len() == 4 => {
                            let values = el.as_primitive::<Float32Type>();
                            ColorOrGradient::Color([
                                values.value(0),
                                values.value(1),
                                values.value(2),
                                values.value(3),
                            ])
                        }
                        _ => ColorOrGradient::transparent(),
                    })
                    .collect::<Vec<_>>();
                Ok(ScalarOrArray::new_array(result))
            }
            _ => {
                return Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to color: {:?}",
                    dtype
                )))
            }
        }
    }
}
