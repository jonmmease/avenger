pub mod marks;
pub mod scale;

use std::{collections::HashMap, sync::Arc};

use crate::{
    error::AvengerChartError,
    types::group::{Group, MarkOrGroup},
    utils::ExprHelpers,
};
use async_recursion::async_recursion;
use avenger_scales::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    scales::{linear::LinearScale, ScaleImpl},
};
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
use avenger_scales::scales::coerce::{CastNumericCoercer, ColorCoercer, CssColorCoercer, NumericCoercer};
use marks::{ArcMarkCompiler, MarkCompiler};
use scale::evaluate_scale;

pub struct AvengerRuntime {
    ctx: SessionContext,
    // scale_compilers: HashMap<String, Box<dyn ScaleCompiler>>,
    mark_compilers: HashMap<String, Box<dyn MarkCompiler>>,

    scales: HashMap<String, Box<dyn ScaleImpl>>,
    interpolator: Box<dyn ColorInterpolator>,
    color_coercer: Box<dyn ColorCoercer>,
    numeric_coercer: Box<dyn NumericCoercer>,
}

impl AvengerRuntime {
    pub fn new(ctx: SessionContext) -> Self {
        let mut mark_compilers: HashMap<String, Box<dyn MarkCompiler>> = HashMap::new();
        mark_compilers.insert("arc".to_string(), Box::new(ArcMarkCompiler));

        let mut scales: HashMap<String, Box<dyn ScaleImpl>> = HashMap::new();
        scales.insert("linear".to_string(), Box::new(LinearScale));

        Self {
            ctx,
            mark_compilers,
            scales,
            interpolator: Box::new(SrgbaColorInterpolator),
            color_coercer: Box::new(CssColorCoercer),
            numeric_coercer: Box::new(CastNumericCoercer),
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

        // Collect and evaluate scales
        let mut evaluated_scales = HashMap::new();

        for (key, value) in group.get_scales() {
            let evaluated_scale =
                evaluate_scale(&value, &key, &self.ctx, &query_values, &self.scales).await?;
            evaluated_scales.insert(key.clone(), evaluated_scale);
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
                        .compile(
                            mark,
                            &self.ctx,
                            &query_values,
                            &evaluated_scales,
                            &self.scales,
                            &self.interpolator,
                            &self.color_coercer,
                            &self.numeric_coercer,
                        )
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
