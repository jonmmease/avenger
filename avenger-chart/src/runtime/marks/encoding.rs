#[macro_export]
macro_rules! apply_f32_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:expr) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $context.scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = evaluated_scale.scale.scale_to_numeric(&x)?;
            }
        } else {
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = $context.coerce_scale.scale_to_numeric(&x)?;
            }
        }
    };
}

#[macro_export]
macro_rules! apply_color_encoding {
    ($mark:expr, $context:expr, $encoding_batches:expr, $scene_mark:expr, $field:expr) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $context.scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = evaluated_scale.scale.scale_to_color(&value)?;
            }
        } else {
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = $context.coerce_scale.scale_to_color(&value)?;
            }
        }
    };
}
