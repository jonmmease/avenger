use std::collections::HashMap;
use std::sync::Arc;
use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding, apply_string_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;

pub struct TextMarkCompiler;

#[async_trait]
impl MarkCompiler for TextMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;

        // Create a new default SceneArcMark
        let mut scene_mark = SceneTextMark::default();
        scene_mark.len = encoding_batches.len() as u32;
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, angle);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, font_size);

        // text
        apply_string_encoding!(mark, context, encoding_batches, scene_mark, text);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, color);

        // Enums
        if let Some(value) = encoding_batches.array_for_field("align") {
            scene_mark.align = context.coercer.to_text_align(&value)?;
        }
        if let Some(value) = encoding_batches.array_for_field("baseline") {
            scene_mark.baseline = context.coercer.to_text_baseline(&value)?;
        }
        if let Some(value) = encoding_batches.array_for_field("font_weight") {
            scene_mark.font_weight = context.coercer.to_font_weight(&value)?;
        }
        if let Some(value) = encoding_batches.array_for_field("font_style") {
            scene_mark.font_style = context.coercer.to_font_style(&value)?;
        }

        let details = if let Some(details_batch) = encoding_batches.details_batch {
            Some([(Vec::<usize>::new(), details_batch)].into_iter().collect::<HashMap<_, _>>())
        } else {
            None
        };
        Ok(CompiledMark{
            scene_marks: vec![SceneMark::Text(Arc::new(scene_mark))],
            details: details.unwrap_or_default(),
        })
    }
}
