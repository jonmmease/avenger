use crate::error::AvengerChartError;
use crate::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::runtime::context::CompilationContext;
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding, apply_usize_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use std::collections::HashMap;

pub struct PathMarkCompiler;

#[async_trait]
impl MarkCompiler for PathMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;

        // Create a new default SceneArcMark
        let mut scene_mark = ScenePathMark::default();
        scene_mark.len = encoding_batches.len() as u32;

        // name
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // z-index
        scene_mark.zindex = mark.zindex;

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, fill);
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);

        // Apply scalars
        if let Some(stroke_width) = encoding_batches.numeric_scalar("stroke_width")? {
            scene_mark.stroke_width = Some(stroke_width);
        }

        // transforms
        if let Some(value) = encoding_batches.array_for_field("transform") {
            scene_mark.transform = context
                .coercer
                .to_path_transform(&value)?
                .to_scalar_if_len_one();
        }

        // path
        if let Some(value) = encoding_batches.array_for_field("path") {
            scene_mark.path = context.coercer.to_path(&value)?.to_scalar_if_len_one();
        }

        if let Some(value) = encoding_batches.stroke_cap_scalar("stroke_cap")? {
            scene_mark.stroke_cap = value;
        }
        if let Some(value) = encoding_batches.stroke_join_scalar("stroke_join")? {
            scene_mark.stroke_join = value;
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
            scene_marks: vec![SceneMark::Path(scene_mark)],
            details: details.unwrap_or_default(),
        })
    }
}
