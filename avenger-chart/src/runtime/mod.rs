pub mod app;
pub mod context;
pub mod controller;
pub mod scale;

use crate::marks::area::AreaMarkCompiler;
use crate::marks::image::ImageMarkCompiler;
use crate::marks::line::LineMarkCompiler;
use crate::marks::path::PathMarkCompiler;
use crate::marks::rect::RectMarkCompiler;
use crate::marks::rule::RuleMarkCompiler;
use crate::marks::text::TextMarkCompiler;
use crate::marks::trail::TrailMarkCompiler;
use crate::marks::{arc::ArcMarkCompiler, symbol::SymbolMarkCompiler, MarkCompiler};
use crate::runtime::app::AvengerChartState;
use crate::runtime::controller::param_stream::ParamStreamContext;
use crate::types::guide::GuideCompilationContext;
use crate::{
    error::AvengerChartError,
    param::Param,
    types::group::{Group, MarkOrGroup},
    types::scales::Scale,
    utils::ExprHelpers,
};
use arrow::array::RecordBatch;
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
use scale::eval_scale;
use std::{collections::HashMap, sync::Arc};

pub struct CompiledChart {
    pub scene_group: SceneGroup,
    pub details: HashMap<Vec<usize>, RecordBatch>,
}

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
        mark_compilers.insert("area".to_string(), Arc::new(AreaMarkCompiler));
        mark_compilers.insert("image".to_string(), Arc::new(ImageMarkCompiler));
        mark_compilers.insert("line".to_string(), Arc::new(LineMarkCompiler));
        mark_compilers.insert("path".to_string(), Arc::new(PathMarkCompiler));
        mark_compilers.insert("rect".to_string(), Arc::new(RectMarkCompiler));
        mark_compilers.insert("rule".to_string(), Arc::new(RuleMarkCompiler));
        mark_compilers.insert("symbol".to_string(), Arc::new(SymbolMarkCompiler));
        mark_compilers.insert("text".to_string(), Arc::new(TextMarkCompiler));
        mark_compilers.insert("trail".to_string(), Arc::new(TrailMarkCompiler));

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
        parent_path: Vec<usize>,
        param_values: ParamValues,
    ) -> Result<CompiledChart, AvengerChartError> {
        let mut details: HashMap<Vec<usize>, RecordBatch> = HashMap::new();

        // Build compilation context
        let context = CompilationContext {
            ctx: self.ctx.clone(),
            coercer: self.coercer.clone(),
            param_values: param_values.clone(),
        };

        // Collect and compile scene marks
        let mut scene_marks: Vec<SceneMark> = Vec::new();
        let mut marks_and_groups = group.get_marks_and_groups().clone();

        // Add controller marks
        for controller in group.controllers.iter() {
            marks_and_groups.extend(controller.marks());
        }

        // Compile scene marks
        for (idx, mark_or_group) in marks_and_groups.iter().enumerate() {
            let mut mark_path = parent_path.clone();
            mark_path.push(idx);

            match mark_or_group {
                MarkOrGroup::Mark(mark) => {
                    let mark_type = mark.get_mark_type();
                    let mark_compiler = self.mark_compilers.get(mark_type).ok_or(
                        AvengerChartError::MarkTypeLookupError(mark_type.to_string()),
                    )?;

                    let compiled_mark = mark_compiler.compile(mark, &context).await?;
                    scene_marks.extend(compiled_mark.scene_marks);

                    // Perpend mark path and merge details
                    for (child_path, batch) in compiled_mark.details {
                        let mut path = mark_path.clone();
                        path.extend(child_path);
                        details.insert(path, batch);
                    }
                }
                MarkOrGroup::Group(group) => {
                    // process groups recursively
                    let compiled_group = self
                        .compile_group(group, mark_path, param_values.clone())
                        .await?;
                    scene_marks.push(SceneMark::Group(compiled_group.scene_group));

                    // Merge details
                    details.extend(compiled_group.details);
                }
            }
        }

        // Build initial group with all of the non-guide marks
        let mut scene_group = SceneGroup {
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

        // Add guide marks
        for guide in group.get_guides() {
            // Evaluate scales
            let mut configured_scales = Vec::new();
            for scale in guide.scales() {
                configured_scales.push(
                    eval_scale(scale, &context.ctx, Some(&context.param_values))
                        .await
                        .unwrap(),
                );
            }

            let guide_context = GuideCompilationContext {
                size: group.get_size(),
                origin: [group.get_x(), group.get_y()],
                group: &scene_group,
                scales: &configured_scales,
            };
            let compiled_guide = guide.compile(&guide_context)?;
            scene_group.marks.extend(compiled_guide);
        }

        Ok(CompiledChart {
            scene_group,
            details,
        })
    }

    pub async fn build_app(
        &self,
        chart: Group,
    ) -> Result<AvengerApp<AvengerChartState>, AvengerChartError> {
        let chart_state = AvengerChartState::new(chart, Arc::new(self.clone()));

        // Build event streams that wrap param streams (need to revisit naming these).
        let mut param_streams = chart_state
            .chart
            .controllers
            .iter()
            .flat_map(|c| c.param_streams())
            .collect::<Vec<_>>();

        // Collect param streams from params
        for param in &chart_state.chart.params {
            if let Some(stream) = &param.stream {
                param_streams.push(stream.clone());
            }
        }

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

        let context = ParamStreamContext {
            event,
            params: &input_params,
            scales: &scales,
            group_path: &group_path,
            rtree,
            details: &state.details,
        };

        let (new_params, update_status) = self.param_stream.update(context);

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
    async fn build(&self, state: &mut AvengerChartState) -> Result<SceneGraph, AvengerAppError> {
        Ok(state.compile_scene_graph().await?)
    }
}
