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
