use std::{any::Any, collections::HashMap, sync::Arc};

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
    concat::{GridConcat, HConcat, VConcat, WrapConcat, concat_coord_ref},
    coords::CoordinateSystemTransformCore,
    error::AvengerChartError,
    facet::coord::{apply_measurement_edge_targets, retarget_measurement_plot_area_no_remeasure},
    layout::Orientation,
    marks::{
        ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState,
        CompiledSubplotPayload, SubplotContainerCoordinateSystem, SubplotDataSource,
        SubplotMarkCore, compile_subplot_payload, compile_subplot_payload_with_context,
    },
    plot::{
        CompiledPlot,
        compiled::{
            compiled_subplot_payload_child_plot, compiled_subplot_payload_child_plot_arc,
            rendering::{
                apply_domain_overrides_to_scales, has_raw_domain_scale,
                resolve_raw_domain_overrides,
            },
        },
    },
    render::{
        EvaluatedChildFrameKind, EvaluatedChildFrameSegment, EvaluationContext, RenderContext,
        context::plot_area_pattern_reference_frame,
    },
    task::ChartFuture,
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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SubplotContainerCoordinateSystem for HConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("HConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new(
            ConcatContainerKind::HConcat,
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
            ConcatContainerKind::HConcat,
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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SubplotContainerCoordinateSystem for VConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("VConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new(
            ConcatContainerKind::VConcat,
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
            ConcatContainerKind::VConcat,
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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SubplotContainerCoordinateSystem for WrapConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("WrapConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new(
            ConcatContainerKind::WrapConcat,
            compile_subplot_payload(subplot, compiled_state, session_context).await?,
        )))
    }

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("WrapConcat")?;
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
            ConcatContainerKind::WrapConcat,
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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SubplotContainerCoordinateSystem for GridConcat {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("GridConcat")?;

        Ok(Arc::new(CompiledConcatSubplot::new_with_grid_placement(
            ConcatContainerKind::GridConcat,
            compile_subplot_payload(subplot, compiled_state, session_context).await?,
            GridPlacementConfig::from_subplot(subplot)?,
        )))
    }

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("GridConcat")?;
        let child_tool_context;
        let compile_context =
            if let Some(tool_context) = compile_context.and_then(ToolCompileContext::downcast) {
                child_tool_context =
                    tool_context.with_coord_node_path_appended(compiled_state.mark_index());
                Some(&child_tool_context as CompileContext<'_>)
            } else {
                compile_context
            };

        Ok(Arc::new(CompiledConcatSubplot::new_with_grid_placement(
            ConcatContainerKind::GridConcat,
            compile_subplot_payload_with_context(
                subplot,
                compiled_state,
                session_context,
                compile_context,
            )
            .await?,
            GridPlacementConfig::from_subplot(subplot)?,
        )))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GridPlacementConfig {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) row_span: usize,
    pub(crate) column_span: usize,
}

impl GridPlacementConfig {
    fn from_subplot(subplot: &dyn SubplotMarkCore) -> Result<Self, AvengerChartError> {
        let row = subplot.grid_row_config().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridConcat subplots require `.grid_cell(row, column)`".to_string(),
            )
        })?;
        let column = subplot.grid_column_config().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridConcat subplots require `.grid_cell(row, column)`".to_string(),
            )
        })?;
        let row_span = subplot.grid_row_span_config();
        let column_span = subplot.grid_column_span_config();
        if row_span == 0 || column_span == 0 {
            return Err(AvengerChartError::InvalidArgument(
                "GridConcat subplot spans must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            row,
            column,
            row_span,
            column_span,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ConcatContainerKind {
    HConcat,
    VConcat,
    GridConcat,
    WrapConcat,
}

impl ConcatContainerKind {
    fn evaluated(self) -> EvaluatedChildFrameKind {
        match self {
            Self::HConcat => EvaluatedChildFrameKind::HConcat,
            Self::VConcat => EvaluatedChildFrameKind::VConcat,
            Self::GridConcat => EvaluatedChildFrameKind::GridConcat,
            Self::WrapConcat => EvaluatedChildFrameKind::WrapConcat,
        }
    }
}

/// Compiled child-plot mark for concat coordinate systems.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledConcatSubplot {
    kind: ConcatContainerKind,
    payload: CompiledSubplotPayload,
    grid_placement: Option<GridPlacementConfig>,
}

impl CompiledConcatSubplot {
    fn new(kind: ConcatContainerKind, payload: CompiledSubplotPayload) -> Self {
        Self {
            kind,
            payload,
            grid_placement: None,
        }
    }

    fn new_with_grid_placement(
        kind: ConcatContainerKind,
        payload: CompiledSubplotPayload,
        grid_placement: GridPlacementConfig,
    ) -> Self {
        Self {
            kind,
            payload,
            grid_placement: Some(grid_placement),
        }
    }

    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub(crate) fn compiled_subplot_arc(&self) -> Arc<CompiledPlot> {
        compiled_subplot_payload_child_plot_arc(&self.payload)
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

    fn kind(&self) -> ConcatContainerKind {
        self.kind
    }

    pub(crate) fn grid_placement(&self) -> Option<GridPlacementConfig> {
        self.grid_placement
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

    fn interaction_child_frame_segment(
        &self,
        concat_measurement: &crate::concat::ConcatCoordMeasurement,
        child: &crate::concat::ConcatChildMeasurement,
        child_count: usize,
    ) -> Result<EvaluatedChildFrameSegment, AvengerChartError> {
        let child_index = self.child_index();
        let key = self.key().map(ToOwned::to_owned);
        let label = self.label().map(ToOwned::to_owned);
        let (row, column, row_count, column_count, row_span, column_span) =
            match concat_measurement.band_direction() {
                Some(Orientation::Horizontal) => (
                    Some(0),
                    Some(child_index),
                    Some(1),
                    Some(child_count),
                    Some(1),
                    Some(1),
                ),
                Some(Orientation::Vertical) => (
                    Some(child_index),
                    Some(0),
                    Some(child_count),
                    Some(1),
                    Some(1),
                    Some(1),
                ),
                None => {
                    let grid_shape = concat_measurement.grid_shape().ok_or_else(|| {
                        AvengerChartError::InternalError(
                            "Grid/wrap concat measurement missing grid shape".to_string(),
                        )
                    })?;
                    let placement = child.grid_placement.ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Missing grid/wrap placement for child {child_index}"
                        ))
                    })?;
                    (
                        Some(placement.row),
                        Some(placement.column),
                        Some(grid_shape.rows),
                        Some(grid_shape.columns),
                        Some(placement.row_span),
                        Some(placement.column_span),
                    )
                }
            };
        let slot_index = match (row, column, column_count) {
            (Some(row), Some(column), Some(column_count)) => Some(row * column_count + column),
            _ => Some(child_index),
        };

        Ok(EvaluatedChildFrameSegment {
            kind: self.kind().evaluated(),
            child_index,
            key,
            label,
            row,
            column,
            row_count,
            column_count,
            row_span,
            column_span,
            slot_index,
        })
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
        data: Option<&'a RecordBatch>,
        context: &'a RenderContext<'a>,
    ) -> ChartFuture<'a, Result<Vec<SceneMark>, AvengerChartError>> {
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
            let child_frame_region = concat_measurement.child_frame_region(self.child_index())?;

            let mut params = self.compiled_subplot().get_default_params().clone();
            params.extend(context.eval.params.clone());
            let sharing_levels = child.sharing_levels.clone();
            let mut sharing_levels = sharing_levels.into_iter();
            let first_level = sharing_levels.next().ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Concat subplot rendering requires a sharing level".to_string(),
                )
            })?;
            let mut child_eval_ctx = context
                .eval
                .with_params(params)
                .with_child_frame_sharing_level_appended(first_level);
            for level in sharing_levels {
                child_eval_ctx = child_eval_ctx.with_child_frame_sharing_level_appended(level);
            }
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
            if let Some(content_size_override) = child_frame_region.content_size_override {
                retarget_measurement_plot_area_no_remeasure(
                    &mut child_measurement,
                    self.compiled_subplot(),
                    &child_eval_ctx,
                    child_facet_path,
                    content_size_override.width,
                    content_size_override.height,
                )?;
            }
            if let Some(edge_targets) = child_frame_region.edge_targets {
                apply_measurement_edge_targets(&mut child_measurement, edge_targets);
            }
            if has_raw_domain_scale(self.compiled_subplot()) {
                let raw_domain_overrides = resolve_raw_domain_overrides(
                    self.compiled_subplot(),
                    child_eval_ctx.session_context.as_ref(),
                    &child_measurement.params,
                )
                .await?;
                if !raw_domain_overrides.is_empty() {
                    apply_domain_overrides_to_scales(
                        &mut child_measurement.scales,
                        &raw_domain_overrides,
                    );
                }
            }
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
                let child_count = concat_measurement.children().len();
                let child_frame_segment =
                    self.interaction_child_frame_segment(concat_measurement, child, child_count)?;
                let translated = child_scopes.into_iter().map(|mut scope| {
                    scope.prepend_coord_node_path(self.child_index());
                    scope.prepend_subplot_id(self.compiled_state().id.as_deref());
                    scope.prepend_child_frame_segment(child_frame_segment.clone());
                    scope.bounds.x += child_frame_region.content.x;
                    scope.bounds.y += child_frame_region.content.y;
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
                    rows.prepend_subplot_id(self.compiled_state().id.as_deref());
                    rows
                });
                context.eval.push_event_datums(translated);
            }
            let child_chrome_event_datums = std::mem::take(&mut components.chrome_event_datums);
            if !child_chrome_event_datums.is_empty() {
                let translated = child_chrome_event_datums.into_iter().map(|mut rows| {
                    let mut path = Vec::with_capacity(rows.mark_path.len() + 1);
                    path.push(0);
                    path.extend(rows.mark_path);
                    rows.mark_path = path;
                    rows.prepend_subplot_id(self.compiled_state().id.as_deref());
                    rows
                });
                context.eval.push_event_datums(translated);
            }

            let data_marks_group = SceneGroup {
                origin: [0.0, 0.0],
                pattern_reference_frame: plot_area_pattern_reference_frame(
                    components.plot_bounds.width,
                    components.plot_bounds.height,
                ),
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
                origin: child_frame_region.plot_origin(),
                pattern_reference_frame: None,
                clip: avenger_scenegraph::marks::group::Clip::None,
                marks: all_marks,
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
                interactive: true,
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
    avenger_chart_core::impl_mark_with_data_context!();

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
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
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
