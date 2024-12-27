pub mod context;
pub mod marks;
pub mod scale;

use std::{collections::HashMap, sync::Arc};

use crate::{
    error::AvengerChartError,
    param::Param,
    types::group::{Group, MarkOrGroup},
    utils::ExprHelpers,
};
use async_recursion::async_recursion;
use avenger_scales::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    scales::{coerce::Coercer, linear::LinearScale, ScaleImpl},
};
use avenger_scales::{
    formatter::Formatters,
    scales::{
        coerce::{CastNumericCoercer, ColorCoercer, CssColorCoercer, NumericCoercer},
        ConfiguredScale, ScaleConfig,
    },
};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
};
use context::CompilationContext;
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
use marks::{ArcMarkCompiler, MarkCompiler};

pub struct AvengerRuntime {
    ctx: SessionContext,
    mark_compilers: HashMap<String, Arc<dyn MarkCompiler>>,
    scales: HashMap<String, Arc<dyn ScaleImpl>>,
    interpolator: Arc<dyn ColorInterpolator>,
    color_coercer: Arc<dyn ColorCoercer>,
    numeric_coercer: Arc<dyn NumericCoercer>,
    coercer: Arc<Coercer>,
}

impl AvengerRuntime {
    pub fn new(ctx: SessionContext) -> Self {
        let mut mark_compilers: HashMap<String, Arc<dyn MarkCompiler>> = HashMap::new();
        mark_compilers.insert("arc".to_string(), Arc::new(ArcMarkCompiler));

        let mut scales: HashMap<String, Arc<dyn ScaleImpl>> = HashMap::new();
        scales.insert("linear".to_string(), Arc::new(LinearScale));

        Self {
            ctx,
            mark_compilers,
            scales,
            interpolator: Arc::new(SrgbaColorInterpolator),
            color_coercer: Arc::new(CssColorCoercer),
            numeric_coercer: Arc::new(CastNumericCoercer),
            coercer: Arc::new(Coercer::default()),
        }
    }

    pub fn ctx(&self) -> &SessionContext {
        &self.ctx
    }

    #[async_recursion]
    pub async fn compile_group(
        &self,
        group: &Group,
        params: Vec<Param>,
    ) -> Result<SceneGroup, AvengerChartError> {
        // Build compilation context
        let param_values = ParamValues::Map(
            params
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect(),
        );
        let context = CompilationContext {
            ctx: self.ctx.clone(),
            coercer: self.coercer.clone(),
            param_values,
        };

        // Collect and compile scene marks
        let mut scene_marks: Vec<SceneMark> = Vec::new();
        for mark_or_group in group.get_marks_and_groups() {
            match mark_or_group {
                MarkOrGroup::Mark(mark) => {
                    let mark_type = mark.get_mark_type();
                    let mark_compiler = self.mark_compilers.get(mark_type).ok_or(
                        AvengerChartError::MarkTypeLookupError(mark_type.to_string()),
                    )?;

                    let new_marks = mark_compiler.compile(mark, &context).await?;
                    scene_marks.extend(new_marks);
                }
                MarkOrGroup::Group(group) => {
                    // process groups recursively
                    let group = self.compile_group(group, params.clone()).await?;
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
