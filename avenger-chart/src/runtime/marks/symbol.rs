use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;

pub struct SymbolMarkCompiler;

#[async_trait]
impl MarkCompiler for SymbolMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneSymbolMark::default();
        scene_mark.len = encoding_batches.len() as u32;

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, size);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, angle);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, fill);
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);

        Ok(vec![SceneMark::Symbol(scene_mark)])
    }
}
