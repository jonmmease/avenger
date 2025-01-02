use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_color_encoding, apply_numeric_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;

pub struct RuleMarkCompiler;

#[async_trait]
impl MarkCompiler for RuleMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneRuleMark::default();
        scene_mark.len = encoding_batches.len() as u32;
        if let Some(name) = mark.name.clone() {
            scene_mark.name = name;
        }

        // Apply numeric encodings
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x2);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y2);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, stroke_width);

        // Apply color encoding
        apply_color_encoding!(mark, context, encoding_batches, scene_mark, stroke);

        // Enums
        if let Some(value) = encoding_batches.array_for_field("stroke_cap") {
            scene_mark.stroke_cap = context
                .coercer
                .to_stroke_cap(&value)?
                .to_scalar_if_len_one();
        }

        // Stroke Dash
        if let Some(value) = encoding_batches.array_for_field("stroke_dash") {
            scene_mark.stroke_dash = Some(context.coercer.to_numeric_vec(&value)?);
        }

        Ok(CompiledMark {
            scene_marks: vec![SceneMark::Rule(scene_mark)],
            details: Default::default(),
        })
    }
}
