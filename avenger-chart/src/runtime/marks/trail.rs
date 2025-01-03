use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_boolean_encoding, apply_color_encoding, apply_numeric_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use std::collections::HashMap;

pub struct TrailMarkCompiler;

#[async_trait]
impl MarkCompiler for TrailMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;

        // Create a new default SceneArcMark
        let mut scene_mark = SceneTrailMark::default();
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
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, size);

        // boolean encoding
        apply_boolean_encoding!(mark, context, encoding_batches, scene_mark, defined);

        // stroke
        if let Some(color) = encoding_batches.color_scalar("stroke")? {
            scene_mark.stroke = color;
        }

        let details = if let Some(details_batch) = encoding_batches.details_batch {
            Some(
                [(Vec::<usize>::new(), details_batch)]
                    .into_iter()
                    .collect::<HashMap<_, _>>(),
            )
        } else {
            None
        };
        Ok(CompiledMark {
            scene_marks: vec![SceneMark::Trail(scene_mark)],
            details: details.unwrap_or_default(),
        })
    }
}
