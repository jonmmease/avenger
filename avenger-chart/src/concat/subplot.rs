use std::{any::Any, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use avenger_scales::scales::ScaleImpl;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, dataframe::DataFrame, logical_expr::Expr,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use avenger_chart_core::{
    CompileContext, MarkRuntimeContext, RadiusExpression, ResolvedDomain, ScaleRange,
    ScaleTypePreference,
};

use crate::{
    concat::{HConcat, VConcat, concat_coord_ref},
    coords::CoordinateSystemTransformCore,
    error::AvengerChartError,
    layout::BandDirection,
    marks::{
        ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState,
        CompiledSubplotPayload, SubplotContainerCoordinateSystem, SubplotDataSource,
        SubplotMarkCore, compile_subplot_payload, compile_subplot_payload_with_context,
    },
    plot::{
        CompiledPlot,
        compiled::{ChildFrameSharingLevel, compiled_subplot_payload_child_plot},
    },
    render::{EvaluationContext, RenderContext},
    theme::Theme,
    tools::ToolCompileContext,
};

fn refresh_measurement_params_for_child(
    measurement: &mut crate::plot::compiled::ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) {
    let width = measurement.params.get("width").cloned();
    let height = measurement.params.get("height").cloned();
    let mut params = measurement.params.clone();
    params.extend(eval_ctx.params().clone());
    if let Some(width) = width {
        params.insert("width".to_string(), width);
    }
    if let Some(height) = height {
        params.insert("height".to_string(), height);
    }
    measurement.params = params;
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for HConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("HConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new(
            compile_subplot_payload(subplot, compiled_state, session_context).await?,
        )))
    }

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("HConcat")?;
        let child_tool_context;
        let compile_context =
            if let Some(tool_context) = compile_context.and_then(ToolCompileContext::downcast) {
                child_tool_context =
                    tool_context.with_coord_node_path_appended(compiled_state.mark_index());
                Some(&child_tool_context as CompileContext<'_>)
            } else {
                compile_context
            };

        Ok(Arc::new(CompiledConcatSubplot::new(
            compile_subplot_payload_with_context(
                subplot,
                compiled_state,
                session_context,
                compile_context,
            )
            .await?,
        )))
    }
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for VConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("VConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new(
            compile_subplot_payload(subplot, compiled_state, session_context).await?,
        )))
    }

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("VConcat")?;
        let child_tool_context;
        let compile_context =
            if let Some(tool_context) = compile_context.and_then(ToolCompileContext::downcast) {
                child_tool_context =
                    tool_context.with_coord_node_path_appended(compiled_state.mark_index());
                Some(&child_tool_context as CompileContext<'_>)
            } else {
                compile_context
            };

        Ok(Arc::new(CompiledConcatSubplot::new(
            compile_subplot_payload_with_context(
                subplot,
                compiled_state,
                session_context,
                compile_context,
            )
            .await?,
        )))
    }
}

/// Compiled child-plot mark for concat coordinate systems.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledConcatSubplot {
    payload: CompiledSubplotPayload,
}

impl CompiledConcatSubplot {
    pub(crate) fn new(payload: CompiledSubplotPayload) -> Self {
        Self { payload }
    }

    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    pub fn label(&self) -> Option<&str> {
        self.payload.label()
    }

    pub fn key(&self) -> Option<&str> {
        self.payload.key()
    }

    pub fn data_source(&self) -> SubplotDataSource {
        self.payload.data_source()
    }

    pub fn child_index(&self) -> usize {
        self.payload.mark_index()
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.payload.inherits_parent_data()
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.payload.has_explicit_child_data()
    }

    fn group_name(&self) -> String {
        match self.key() {
            Some(key) => format!("concat_subplot_{}_{}", self.child_index(), key),
            None => format!("concat_subplot_{}", self.child_index()),
        }
    }

    fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        context: &RenderContext<'_>,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        self.payload
            .inherited_data_override(data, context.session_context().as_ref())
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
        data: Option<&'a RecordBatch>,
        context: &'a RenderContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'a>> {
        Box::pin(async move {
            let concat_measurement =
                concat_coord_ref(context.coord_measurement()).ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Subplot marks require ConcatCoordMeasurement in coord_measurement"
                            .to_string(),
                    )
                })?;
            let child = concat_measurement
                .child(self.child_index())
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing concat child measurement for subplot child index {}",
                        self.child_index()
                    ))
                })?;
            let child_frame_placement = concat_measurement.child_frame_placement();
            let render_placement =
                child_frame_placement
                    .child(self.child_index())
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Missing concat child-frame placement for subplot child index {}",
                            self.child_index()
                        ))
                    })?;

            let mut params = self.compiled_subplot().get_default_params().clone();
            params.extend(context.eval.params.clone());
            let child_count = concat_measurement.children().len();
            let sharing_level = match concat_measurement.child_band_layout.direction {
                BandDirection::Horizontal => ChildFrameSharingLevel::hconcat_child(
                    self.child_index(),
                    child_count,
                    self.key(),
                ),
                BandDirection::Vertical => ChildFrameSharingLevel::vconcat_child(
                    self.child_index(),
                    child_count,
                    self.key(),
                ),
            };
            let mut child_eval_ctx = context
                .eval
                .with_params(params)
                .with_child_frame_sharing_level_appended(sharing_level);
            let local_facet_path;
            let child_facet_path = if let Some(facet_tree) = &child.local_facet_tree {
                child_eval_ctx = child_eval_ctx
                    .with_facet_tree(facet_tree.clone())
                    .with_facet_data_root(child.facet_data_root.clone());
                local_facet_path = Vec::new();
                local_facet_path.as_slice()
            } else {
                context.facet_path
            };
            let data_override = self.inherited_data_override(data, context)?;
            let mut child_measurement = child.measurement.clone();
            refresh_measurement_params_for_child(&mut child_measurement, &child_eval_ctx);
            let mut components = Box::pin(self.compiled_subplot().build_plot_components(
                &child_eval_ctx,
                &child_measurement,
                data_override.as_ref(),
                true,
                child_facet_path,
            ))
            .await?;

            let child_scopes = std::mem::take(&mut components.interaction_scopes);
            if !child_scopes.is_empty() {
                let translated = child_scopes.into_iter().map(|mut scope| {
                    scope.prepend_coord_node_path(self.child_index());
                    scope.bounds.x += render_placement.origin[0];
                    scope.bounds.y += render_placement.origin[1];
                    scope
                });
                context.eval.push_interaction_scopes(translated);
            }
            let child_event_datums = std::mem::take(&mut components.event_datums);
            if !child_event_datums.is_empty() {
                let translated = child_event_datums.into_iter().map(|mut rows| {
                    let mut path = Vec::with_capacity(rows.mark_path.len() + 2);
                    path.push(0);
                    path.push(0);
                    path.extend(rows.mark_path);
                    rows.mark_path = path;
                    rows
                });
                context.eval.push_event_datums(translated);
            }

            let data_marks_group = SceneGroup {
                origin: [0.0, 0.0],
                marks: components.data_marks,
                clip: components.clip,
                zindex: Some(0),
                ..Default::default()
            };
            let mut all_marks = vec![SceneMark::Group(data_marks_group)];
            all_marks.extend(components.guide_marks);
            all_marks.extend(components.legend_marks);
            all_marks.extend(components.title_marks);
            all_marks.extend(components.subtitle_marks);
            all_marks.extend(components.debug_marks);

            Ok(vec![SceneMark::Group(SceneGroup {
                name: self.group_name(),
                origin: render_placement.origin,
                clip: avenger_scenegraph::marks::group::Clip::None,
                marks: all_marks,
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
            })])
        })
    }
}

pub fn compiled_subplot(mark: &dyn CompiledMark) -> Option<&CompiledConcatSubplot> {
    if mark.mark_type() != "subplot" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledConcatSubplot>()
}

impl CompiledMarkCore for CompiledConcatSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<ScaleTypePreference> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> HashMap<String, Expr> {
        HashMap::new()
    }

    fn default_channel_range(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &datafusion::arrow::datatypes::DataType,
        _theme: &Theme,
        _params: &indexmap::IndexMap<String, datafusion::scalar::ScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledConcatSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Concat subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}
