use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;

pub struct ArcMarkCompiler;

#[async_trait]
impl MarkCompiler for ArcMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneArcMark::default();
        scene_mark.len = encoding_batches.len() as u32;

        // name
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // z-index
        scene_mark.zindex = mark.zindex;

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, start_angle);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, end_angle);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, outer_radius);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, inner_radius);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, pad_angle);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, corner_radius);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, stroke_width);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, fill);
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);

        Ok(CompiledMark{
            scene_marks: vec![SceneMark::Arc(scene_mark)],
            details: Default::default()
        })
    }
}
