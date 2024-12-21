pub mod band;
pub mod linear;
pub mod log;
pub mod ordinal;
pub mod point;
pub mod pow;
pub mod quantile;
pub mod quantize;
pub mod symlog;
pub mod threshold;

use std::{collections::HashMap, fmt::Debug};

use arrow::{
    array::{ArrayRef, AsArray},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::{AreaOrientation, ColorOrGradient, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use datafusion_common::ScalarValue;

use crate::{
    color_interpolator::ColorInterpolator, error::AvengerScaleError, utils::ScalarValueUtils,
};
use crate::coerce::{ColorCoercer, CssColorCoercer};

/// Macro to generate scale_to_X trait methods that return a default error implementation
#[macro_export]
macro_rules! declare_enum_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            fn [<scale_to_ $type_name:snake>](
                &self,
                _config: &ScaleConfig,
                _values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                Err(AvengerScaleError::ScaleOperationNotSupported(
                    stringify!([<scale_to_ $type_name:snake>]).to_string(),
                ))
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct ScaleConfig {
    pub domain: ArrayRef,
    pub range: ArrayRef,
    pub options: HashMap<String, ScalarValue>,
}

impl ScaleConfig {
    pub fn numeric_interval_domain(&self) -> Result<(f32, f32), AvengerScaleError> {
        if self.domain.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_domain".to_string(),
            ));
        }
        let domain = cast(self.domain.as_ref(), &DataType::Float32)?;
        let domain = domain.as_primitive::<Float32Type>();
        Ok((domain.value(0), domain.value(1)))
    }

    pub fn numeric_interval_range(&self) -> Result<(f32, f32), AvengerScaleError> {
        if self.range.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_range".to_string(),
            ));
        }
        let range = cast(self.range.as_ref(), &DataType::Float32)?;
        let range = range.as_primitive::<Float32Type>();
        Ok((range.value(0), range.value(1)))
    }

    pub fn color_range(&self) -> Result<Vec<[f32; 4]>, AvengerScaleError> {

        let coercer = CssColorCoercer;
        let range_colors = coercer.coerce_color(&self.range)?;
        let range_colors_vec: Vec<_> = range_colors.as_iter(range_colors.len(), None).map(
            |c| c.color_or_transparent()
        ).collect();
        Ok(range_colors_vec)
    }

    pub fn f32_option(&self, key: &str, default: f32) -> f32 {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(ScalarValue::from(default))
            .as_f32()
            .unwrap_or(default)
    }

    pub fn boolean_option(&self, key: &str, default: bool) -> bool {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(ScalarValue::from(default))
            .as_boolean()
            .unwrap_or(default)
    }

    pub fn i32_option(&self, key: &str, default: i32) -> i32 {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(ScalarValue::from(default))
            .as_i32()
            .unwrap_or(default)
    }

    pub fn string_option(&self, key: &str, default: &str) -> String {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(ScalarValue::from(default))
            .as_string()
            .unwrap_or(default.to_string())
    }
}

/// Method that should be used to infer a scale's domain from the data that it will scale
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferDomainFromDataMethod {
    /// Use the min and max values of the data
    /// In this case the domain will be a two element array
    Interval,
    /// Use the unique values of the data
    /// In this case the domain will be an array of unique values
    Unique,
    /// Use all values of the data
    /// In this case the domain will be an array of all values
    All,
}

pub trait ArrowScale: Debug + Send + Sync + 'static {
    /// Method that should be used to infer a scale's domain from the data that it will scale
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod;

    /// Scale to numeric values
    fn scale_to_numeric(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_to_numeric".to_string(),
        ))
    }

    fn scale_scalar_to_numeric(
        &self,
        config: &ScaleConfig,
        value: &ScalarValue,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let array = value.to_array()?;
        Ok(self.scale_to_numeric(config, &array)?.to_scalar_if_len_one())
    }

    fn invert_from_numeric(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_from_numeric".to_string(),
        ))
    }

    /// Invert a range interval to a subset of the domain
    fn invert_range_interval(
        &self,
        _config: &ScaleConfig,
        _range: (f32, f32),
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_range_interval".to_string(),
        ))
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels
    fn ticks(
        &self,
        _config: &ScaleConfig,
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks".to_string(),
        ))
    }

    /// Scale to color values
    fn scale_to_color(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
        _interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_to_color".to_string(),
        ))
    }

    fn scale_scalar_to_color(
        &self,
        config: &ScaleConfig,
        value: &ScalarValue,
        interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let array = value.to_array()?;
        Ok(self.scale_to_color(config, &array, interpolator)?.to_scalar_if_len_one())
    }


    /// Scale to string values
    fn scale_to_string(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale_to_string".to_string(),
        ))
    }

    fn scale_scalar_to_string(
        &self,
        config: &ScaleConfig,
        value: &ScalarValue,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let array = value.to_array()?;
        Ok(self.scale_to_string(config, &array)?.to_scalar_if_len_one())
    }


    // Scale to enums
    declare_enum_scale_method!(StrokeCap);
    declare_enum_scale_method!(StrokeJoin);
    declare_enum_scale_method!(ImageAlign);
    declare_enum_scale_method!(ImageBaseline);
    declare_enum_scale_method!(AreaOrientation);
}

/// Make sure the trait object safe by defining a struct
#[allow(dead_code)]
struct MakeSureItsObjectSafe {
    pub scales: Box<dyn ArrowScale>,
}
