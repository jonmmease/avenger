#[macro_export]
macro_rules! apply_f32_encoding {
    ($mark:expr, $evaluted_scales:expr, $arrow_scales:expr, $encoding_batches:expr, $scene_mark:expr, $numeric_coercer:expr, $field:expr) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $evaluted_scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            let arrow_scale = $arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
                AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
            )?;
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = arrow_scale.scale_to_numeric(&evaluated_scale.config, &x)?;
            }
        } else {
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = $numeric_coercer.coerce_numeric(&x)?;
            }
        }
    };
}

#[macro_export]
macro_rules! apply_color_encoding {
    ($mark:expr, $evaluted_scales:expr, $arrow_scales:expr, $encoding_batches:expr, $scene_mark:expr, $interpolator:expr, $color_coercer:expr, $field:expr) => {
        if let Some(crate::types::mark::Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $evaluted_scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            let scale_impl = $arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
                AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
            )?;
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = scale_impl.scale_to_color(
                    &evaluated_scale.config,
                    &value,
                    $interpolator.as_ref(),
                )?;
            }
        } else {
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = $color_coercer.coerce_color(&value)?;
            }
        }
    };
}
