use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, ArcMarkCompiler, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_f32_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;

#[async_trait]
impl MarkCompiler for ArcMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneArcMark::default();
        scene_mark.len = encoding_batches.len() as u32;

        // Apply numeric encodings
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "x");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "y");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "start_angle");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "end_angle");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "outer_radius");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "inner_radius");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "pad_angle");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "corner_radius");
        apply_f32_encoding!(mark, context, encoding_batches, scene_mark, "stroke_width");

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, "fill");
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, "stroke");

        Ok(vec![SceneMark::Arc(scene_mark)])
    }
}
