use crate::color_interpolator::ColorInterpolator;
use crate::error::AvengerScaleError;
use crate::formatter::Formatters;
use crate::scales::ordinal::OrdinalScale;
use crate::scales::{InferDomainFromDataMethod, ScaleConfig, ScaleImpl};
use crate::utils::ScalarValueUtils;
use arrow::array::{Array, AsArray, Float32Array, StringArray};
use arrow::compute::kernels::zip::zip;
use arrow::compute::{is_not_null, is_null};
use arrow::datatypes::{Float32Type, UInt32Type};
use arrow::{
    array::ArrayRef,
    compute::kernels::cast,
    datatypes::{DataType, Field},
};
use avenger_common::types::{AreaOrientation, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use css_color_parser::Color;
use paste::paste;
use std::f32::NAN;
use std::fmt::Debug;
use std::sync::Arc;
use strum::VariantNames;

pub trait ColorCoercer: Debug + Send + Sync + 'static {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError>;
}

pub trait NumericCoercer: Debug + Send + Sync + 'static {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError>;

    fn coerce_usize(&self, value: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError>;

    fn coerce_vec(&self, value: &ArrayRef) -> Result<ScalarOrArray<Vec<f32>>, AvengerScaleError>;
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CastNumericCoercer;

impl NumericCoercer for CastNumericCoercer {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::Float32)?;
        let result = cast_array.as_primitive::<Float32Type>();

        if result.null_count() > 0 {
            let mask = is_not_null(result)?;
            let fill_array = Float32Array::from(vec![default_value.unwrap_or(NAN); result.len()]);
            let filled = zip(&mask, &result, &fill_array)?;
            let result_vec = filled.as_primitive::<Float32Type>().values().to_vec();
            Ok(ScalarOrArray::new_array(result_vec))
        } else {
            Ok(ScalarOrArray::new_array(result.values().to_vec()))
        }
    }

    fn coerce_usize(&self, value: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::UInt32)?;
        Ok(ScalarOrArray::new_array(
            cast_array
                .as_primitive::<UInt32Type>()
                .values()
                .iter()
                .map(|el| *el as usize)
                .collect(),
        ))
    }

    fn coerce_vec(&self, value: &ArrayRef) -> Result<ScalarOrArray<Vec<f32>>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::new_list(DataType::Float32, false))?;
        let list_array = cast_array.as_list::<i32>();
        let mut result = Vec::new();
        for i in 0..list_array.len() {
            let values = list_array.value(i);
            let values = values.as_primitive::<Float32Type>().values().to_vec();
            result.push(values);
        }

        Ok(ScalarOrArray::new_array(result))
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CssColorCoercer;

impl ColorCoercer for CssColorCoercer {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let dtype = value.data_type();
        let default_value = default_value.unwrap_or(ColorOrGradient::transparent());
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
                            .unwrap_or_else(|_| default_value.clone()),
                        _ => default_value.clone(),
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
                        _ => default_value.clone(),
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

// Define the macro using paste
macro_rules! define_enum_coercer {
    ($enum_type:ty) => {
        paste! {
            pub fn [<to_ $enum_type:snake> ](
                &self,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$enum_type>, AvengerScaleError> {
                let domain = Arc::new(StringArray::from(Vec::from(<$enum_type>::VARIANTS))) as ArrayRef;
                let scale = OrdinalScale::new(domain.clone()).with_range(domain);
                scale.[<scale_to_ $enum_type:snake>](values)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct Coercer {
    pub color_coercer: Arc<dyn ColorCoercer>,
    pub number_coercer: Arc<dyn NumericCoercer>,
    pub formatters: Formatters,
}

impl Default for Coercer {
    fn default() -> Self {
        Self {
            color_coercer: Arc::new(CssColorCoercer),
            number_coercer: Arc::new(CastNumericCoercer),
            formatters: Formatters::default(),
        }
    }
}

impl Coercer {
    pub fn to_numeric(
        &self,
        values: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        self.number_coercer.coerce(values, default_value)
    }

    pub fn to_usize(&self, values: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError> {
        self.number_coercer.coerce_usize(values)
    }

    pub fn to_color(
        &self,
        values: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        self.color_coercer.coerce(values, default_value)
    }

    pub fn to_string(
        &self,
        values: &ArrayRef,
        default_value: Option<&str>,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        self.formatters.format(values, default_value)
    }

    pub fn to_numeric_vec(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<Vec<f32>>, AvengerScaleError> {
        self.number_coercer.coerce_vec(values)
    }

    define_enum_coercer!(StrokeCap);
    define_enum_coercer!(StrokeJoin);
    define_enum_coercer!(ImageAlign);
    define_enum_coercer!(ImageBaseline);
    define_enum_coercer!(AreaOrientation);
}
