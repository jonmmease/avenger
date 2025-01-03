use crate::error::AvengerChartError;
use crate::runtime::context::CompilationContext;
use crate::runtime::marks::{eval_encoding_exprs, CompiledMark, MarkCompiler};
use crate::types::mark::Mark;
use crate::{apply_boolean_encoding, apply_color_encoding, apply_numeric_encoding};
use async_trait::async_trait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::mark::SceneMark;

pub struct AreaMarkCompiler;

#[async_trait]
impl MarkCompiler for AreaMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError> {
        let encoding_batches = eval_encoding_exprs(&mark.from, &mark.encodings, &mark.details, &context).await?;

        // Create a new default SceneAreaMark
        let mut scene_mark = SceneAreaMark::default();
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
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, x2);
        apply_numeric_encoding!(mark, context, encoding_batches, scene_mark, y2);

        // boolean encoding
        apply_boolean_encoding!(mark, context, encoding_batches, scene_mark, defined);

        // Apply scalars
        if let Some(color) = encoding_batches.color_scalar("fill")? {
            scene_mark.fill = color;
        }
        if let Some(color) = encoding_batches.color_scalar("stroke")? {
            scene_mark.stroke = color;
        }
        if let Some(stroke_width) = encoding_batches.numeric_scalar("stroke_width")? {
            scene_mark.stroke_width = stroke_width;
        }
        if let Some(value) = encoding_batches.stroke_cap_scalar("stroke_cap")? {
            scene_mark.stroke_cap = value;
        }
        if let Some(value) = encoding_batches.stroke_join_scalar("stroke_join")? {
            scene_mark.stroke_join = value;
        }
        if let Some(value) = encoding_batches.stroke_dash_scalar("stroke_dash")? {
            scene_mark.stroke_dash = Some(value);
        }

        Ok(CompiledMark{
            scene_marks: vec![SceneMark::Area(scene_mark)],
            details: Default::default()
        })
    }
}
