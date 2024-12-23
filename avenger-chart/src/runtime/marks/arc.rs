use crate::apply_f32_encoding;
use crate::error::AvengerChartError;
use crate::runtime::marks::{eval_encoding_exprs, ArcMarkCompiler, MarkCompiler};
use crate::runtime::scale::EvaluatedScale;
use crate::types::mark::Mark;
use async_trait::async_trait;
use avenger_scales::color_interpolator::ColorInterpolator;
use avenger_scales::scales::ScaleImpl;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::common::ParamValues;
use datafusion::prelude::SessionContext;
use std::collections::HashMap;
use avenger_scales::scales::coerce::{ColorCoercer, NumericCoercer};

#[async_trait]
impl MarkCompiler for ArcMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        ctx: &SessionContext,
        params: &ParamValues,
        evaluted_scales: &HashMap<String, EvaluatedScale>,
        arrow_scales: &HashMap<String, Box<dyn ScaleImpl>>,
        interpolator: &Box<dyn ColorInterpolator>,
        color_coercer: &Box<dyn ColorCoercer>,
        numeric_coercer: &Box<dyn NumericCoercer>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, ctx, params).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneArcMark::default();
        scene_mark.len = encoding_batches.len() as u32;

        // Apply f32 encodings
        apply_f32_encoding!(
            mark,
            evaluted_scales,
            arrow_scales,
            encoding_batches,
            scene_mark,
            numeric_coercer,
            "x"
        );
        // if let Some(Encoding::Scaled(scaled)) = mark.encodings.get("x") {
        //     let evaluated_scale = evaluted_scales.get(scaled.get_scale()).ok_or(
        //         AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
        //     )?;
        //     let arrow_scale = arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
        //         AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
        //     )?;
        //     if let Some(x) = encoding_batches.array_for_field("x") {
        //         scene_mark.x = arrow_scale.scale_to_numeric(&evaluated_scale.config, &x)?;
        //     }
        // } else {
        //     if let Some(x) = encoding_batches.f32_scalar_or_array_for_field("x")? {
        //         scene_mark.x = x;
        //     }
        // }
        // apply_f32_encoding!(mark, evaluted_scales, encoding_batches, scene_mark, "y");
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "start_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "end_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "outer_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "inner_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "pad_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "corner_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "stroke_width"
        // );

        // Apply color encoding
        crate::apply_color_encoding!(
            mark,
            evaluted_scales,
            arrow_scales,
            encoding_batches,
            scene_mark,
            interpolator,
            color_coercer,
            "fill"
        );

        Ok(vec![SceneMark::Arc(scene_mark)])
    }
}
