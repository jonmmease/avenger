use avenger_scales::utils::ScalarValueUtils;
use datafusion::{
    prelude::{lit, Expr},
    scalar::ScalarValue,
};

use crate::error::AvengerChartError;

#[macro_export]
macro_rules! apply_numeric_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
            $scene_mark.$field = $context
                .coercer
                .to_numeric(&value, None)?
                .to_scalar_if_len_one();
        }
    };
}

#[macro_export]
macro_rules! apply_usize_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
            $scene_mark.$field = $context.coercer.to_usize(&value)?.to_scalar_if_len_one();
        }
    };
}

#[macro_export]
macro_rules! apply_numeric_encoding_optional {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
            $scene_mark.$field = Some(
                $context
                    .coercer
                    .to_numeric(&value, None)?
                    .to_scalar_if_len_one(),
            );
        }
    };
}

#[macro_export]
macro_rules! apply_string_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
            $scene_mark.$field = $context
                .coercer
                .to_string(&value, None)?
                .to_scalar_if_len_one();
        }
    };
}

#[macro_export]
macro_rules! apply_color_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
            $scene_mark.$field = $context
                .coercer
                .to_color(&value, None)?
                .to_scalar_if_len_one();
        }
    };
}

pub fn css_color(color: &str) -> Result<Expr, AvengerChartError> {
    let rgba = ScalarValue::from(color).as_rgba()?;
    Ok(lit(ScalarValue::make_rgba(
        rgba[0], rgba[1], rgba[2], rgba[3],
    )))
}
