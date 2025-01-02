use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding, apply_numeric_encoding_optional};
use async_trait::async_trait;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;

pub struct RectMarkCompiler;

#[async_trait]
impl MarkCompiler for RectMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneRectMark::default();
        scene_mark.len = encoding_batches.len() as u32;
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding_optional!(mark, context, encoding_batches, scene_mark, x2);
        apply_numeric_encoding_optional!(mark, context, encoding_batches, scene_mark, y2);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, stroke_width);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, fill);

        Ok(CompiledMark {
            scene_marks: vec![SceneMark::Rect(scene_mark)],
            details: Default::default(),
        })
    }
}
