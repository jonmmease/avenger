pub mod mark;
pub mod scale;
use std::{collections::HashMap, sync::Arc};

use async_recursion::async_recursion;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
};
use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRewriter},
        ParamValues,
    },
    datasource::ViewTable,
    error::DataFusionError,
    logical_expr::{expr::ScalarFunction, lit},
    prelude::{DataFrame, Expr, SessionContext},
    scalar::ScalarValue,
};
use mark::{ArcMarkCompiler, MarkCompiler};
use scale::{LinearScaleCompiler, ScaleCompiler};

use crate::{
    error::AvengerChartError,
    scales::ScaleImpl,
    types::{
        group::{Group, MarkOrGroup},
        mark::Mark,
        scales::Scale,
    },
    utils::ExprHelpers,
};

pub struct AvengerRuntime {
    ctx: SessionContext,
    scale_compilers: HashMap<String, Box<dyn ScaleCompiler>>,
    mark_compilers: HashMap<String, Box<dyn MarkCompiler>>,
}

impl AvengerRuntime {
    pub fn new(ctx: SessionContext) -> Self {
        // Initialize scale compilers
        let mut scale_compilers: HashMap<String, Box<dyn ScaleCompiler>> = HashMap::new();
        scale_compilers.insert("linear".to_string(), Box::new(LinearScaleCompiler));

        let mut mark_compilers: HashMap<String, Box<dyn MarkCompiler>> = HashMap::new();
        mark_compilers.insert("arc".to_string(), Box::new(ArcMarkCompiler));

        Self {
            ctx,
            scale_compilers,
            mark_compilers,
        }
    }

    #[async_recursion]
    pub async fn compile_group(
        &self,
        group: &Group,
        params: Option<&ParamValues>,
    ) -> Result<SceneGroup, AvengerChartError> {
        // Eval params to ScalarValues
        // treat as already in topological order, consider supporting out-of-order params later
        let mut query_values: HashMap<String, ScalarValue> = HashMap::new();

        // Add parent params
        if let Some(ParamValues::Map(params)) = params {
            query_values.extend(params.clone().into_iter());
        }

        // Add group params after parent params to then take precedence
        for (key, value) in group.get_params() {
            let scalar_value = value
                .eval_to_scalar(&self.ctx, Some(&ParamValues::Map(query_values.clone())))
                .await?;
            query_values.insert(key.clone(), scalar_value);
        }
        let query_values = ParamValues::Map(query_values);

        // Register DataFrames with context with params applied
        for (key, value) in group.get_datasets() {
            let df = value.clone().with_param_values(query_values.clone())?;
            let view_table = ViewTable::try_new(df.into_optimized_plan()?, None)?;
            self.ctx.register_table(key.clone(), Arc::new(view_table))?;
        }

        // Collect and compile scales
        let mut scales = HashMap::new();
        for (key, value) in group.get_scales() {
            let scale_type = value
                .get_scale_type()
                .cloned()
                .unwrap_or_else(|| "linear".to_string());

            let scale_compiler = self
                .scale_compilers
                .get(&scale_type)
                .ok_or(AvengerChartError::ScaleTypeLookupError(scale_type))?;
            let scale_impl = scale_compiler
                .compile(&value, &self.ctx, &query_values)
                .await?;
            scales.insert(key.clone(), scale_impl);
        }

        // Collect and compile scene marks
        let mut scene_marks: Vec<SceneMark> = Vec::new();
        for mark_or_group in group.get_marks_and_groups() {
            match mark_or_group {
                MarkOrGroup::Mark(mark) => {
                    let mark_type = mark.get_mark_type();
                    let mark_compiler = self.mark_compilers.get(mark_type).ok_or(
                        AvengerChartError::MarkTypeLookupError(mark_type.to_string()),
                    )?;

                    let new_marks = mark_compiler
                        .compile(mark, &self.ctx, &query_values, &scales)
                        .await?;
                    scene_marks.extend(new_marks);
                }
                MarkOrGroup::Group(group) => {
                    // process groups recursively
                    let group = self.compile_group(group, Some(&query_values)).await?;
                    scene_marks.push(SceneMark::Group(group));
                }
            }
        }

        let scene_group = SceneGroup {
            name: group.get_name().cloned().unwrap_or_default(),
            origin: [group.get_x(), group.get_y()],
            clip: Clip::default(),
            marks: scene_marks,
            gradients: vec![],
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        };

        Ok(scene_group)
    }
}
