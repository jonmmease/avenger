use std::sync::Arc;

use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding, apply_numeric_encoding_optional};
use async_trait::async_trait;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::mark::SceneMark;

pub struct ImageMarkCompiler;

#[async_trait]
impl MarkCompiler for ImageMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneImageMark::default();
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
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, width);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, height);

        // image encoding
        if let Some(value) = encoding_batches.array_for_field("image") {
            scene_mark.image = context.coercer.to_image(&value)?.to_scalar_if_len_one();
        }

        // enum encodings
        if let Some(value) = encoding_batches.array_for_field("align") {
            scene_mark.align = context
                .coercer
                .to_image_align(&value)?
                .to_scalar_if_len_one();
        }
        if let Some(value) = encoding_batches.array_for_field("baseline") {
            scene_mark.baseline = context
                .coercer
                .to_image_baseline(&value)?
                .to_scalar_if_len_one();
        }

        Ok(CompiledMark {
            scene_marks: vec![SceneMark::Image(Arc::new(scene_mark))],
            details: Default::default(),
        })
    }
}
