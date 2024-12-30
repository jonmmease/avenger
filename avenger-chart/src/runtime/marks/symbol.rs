use std::collections::HashMap;
use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
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
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;

        // Create a new default SceneArcMark
        let mut scene_mark = SceneSymbolMark::default();
        scene_mark.len = encoding_batches.len() as u32;
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, size);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, angle);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, fill);
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);

        let details = if let Some(details_batch) = encoding_batches.details_batch {
            Some([(Vec::<usize>::new(), details_batch)].into_iter().collect::<HashMap<_, _>>())
        } else {
            None
        };
        Ok(CompiledMark{
            scene_marks: vec![SceneMark::Symbol(scene_mark)],
            details: details.unwrap_or_default(),
        })
    }
}
