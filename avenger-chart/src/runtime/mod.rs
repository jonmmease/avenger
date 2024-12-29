pub mod app;
pub mod context;
pub mod controller;
pub mod marks;
pub mod scale;

use std::{collections::HashMap, sync::Arc};

use crate::runtime::app::AvengerChartState;
use crate::{
    error::AvengerChartError,
    param::Param,
    types::group::{Group, MarkOrGroup},
    types::scales::Scale,
    utils::ExprHelpers,
};
use async_recursion::async_recursion;
use async_trait::async_trait;
use avenger_app::app::{AvengerApp, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::UpdateStatus;
use avenger_geometry::rtree::SceneGraphRTree;
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
use avenger_scenegraph::scene_graph::SceneGraph;
use context::CompilationContext;
use controller::param_stream::ParamStream;
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
use marks::{arc::ArcMarkCompiler, symbol::SymbolMarkCompiler, MarkCompiler};

#[derive(Clone)]
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
        mark_compilers.insert("symbol".to_string(), Arc::new(SymbolMarkCompiler));

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
        param_values: ParamValues,
    ) -> Result<SceneGroup, AvengerChartError> {
        // Build compilation context
        let context = CompilationContext {
            ctx: self.ctx.clone(),
            coercer: self.coercer.clone(),
            param_values: param_values.clone(),
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
                    let group = self.compile_group(group, param_values.clone()).await?;
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

    pub async fn build_app(
        &self,
        chart: Group,
    ) -> Result<AvengerApp<AvengerChartState>, AvengerChartError> {
        let chart_state = AvengerChartState::new(chart, Arc::new(self.clone()));

        // Build event streams that wrap param streams (need to revisit naming these).
        let param_streams = chart_state
            .chart
            .controllers
            .iter()
            .flat_map(|c| c.param_streams())
            .collect::<Vec<_>>();
        let mut stream_callbacks = Vec::new();
        for param_stream in param_streams {
            let input_param_names = Vec::from(param_stream.input_params());
            let input_scales = Vec::from(param_stream.input_scales());

            let stream_config = param_stream.stream_config().clone();
            let stream_callback: Arc<dyn EventStreamHandler<AvengerChartState>> =
                Arc::new(ParamEventStreamHandler {
                    input_param_names,
                    input_scales,
                    param_stream,
                });

            stream_callbacks.push((stream_config, stream_callback));
        }

        let avenger_app = AvengerApp::try_new(
            chart_state,
            Arc::new(SceneGraphBuilderImpl),
            stream_callbacks,
        )
        .await?;
        Ok(avenger_app)
    }
}

struct ParamEventStreamHandler {
    input_param_names: Vec<String>,
    input_scales: Vec<Scale>,
    param_stream: Arc<dyn ParamStream>,
}

#[async_trait]
impl EventStreamHandler<AvengerChartState> for ParamEventStreamHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AvengerChartState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        // Build param values to pass to param stream
        let input_params = self
            .input_param_names
            .iter()
            .map(|name| (name.clone(), state.param_values[name].clone()))
            .collect::<HashMap<_, _>>();

        // Evaluate scales to pass to param stream
        let mut scales = Vec::new();
        for scale in self.input_scales.iter() {
            scales.push(state.eval_scale(scale).await);
        }

        // Get group path (where should this come from?)
        let group_path = vec![0 as usize];

        let (new_params, update_status) =
            self.param_stream
                .update(event, &input_params, &scales, &group_path, rtree);

        // Store params
        for (name, value) in new_params {
            state.param_values.insert(name, value);
        }

        update_status
    }
}

struct SceneGraphBuilderImpl;

#[async_trait]
impl SceneGraphBuilder<AvengerChartState> for SceneGraphBuilderImpl {
    async fn build(&self, state: &AvengerChartState) -> Result<SceneGraph, AvengerAppError> {
        Ok(state.compile_scene_graph().await?)
    }
}
