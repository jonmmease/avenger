#[macro_export]
macro_rules! apply_numeric_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) =
            $mark.encodings.get(stringify!($field))
        {
            let evaluated_scale = $context.scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
                $scene_mark.$field = evaluated_scale.scale.scale_to_numeric(&value)?;
            }
        } else {
            if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
                $scene_mark.$field = $context.coercer.to_numeric(&value)?;
            }
        }
    };
}

#[macro_export]
macro_rules! apply_color_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:ident) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) =
            $mark.encodings.get(stringify!($field))
        {
            let evaluated_scale = $context.scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
                $scene_mark.$field = evaluated_scale.scale.scale_to_color(&value)?;
            }
        } else {
            if let Some(value) = $encoding_batches.array_for_field(stringify!($field)) {
                $scene_mark.$field = $context.coercer.to_color(&value)?;
            }
        }
    };
}
