use crate::facet::coord::{
    FacetBandCoordMeasurement, FacetColumn, FacetRow, FacetWrap, facet_band_ref,
};
use crate::facet::marks::facet_config::{
    FacetColChannelConfig, FacetRowChannelConfig, FacetWrapChannelConfig,
};
use crate::facet::ownership_policy::{
    cell_requires_invalid_path_axis_fallback_hidden, has_holes_from_cells,
    resolve_facet_ownership_policy,
};
use crate::facet::placement::{FacetBandPlacement, facet_child_frame_placement_from_band};
use crate::plot::CompiledPlot;
use crate::plot::compiled::{
    compiled_subplot_payload_child_plot, compiled_subplot_payload_child_plot_arc,
};
use crate::render::{EvaluationContext, RenderContext};
use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, ColumnDimensionConfig, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CompiledSubplotPayload, CoordinateGuide,
    CoordinateSystemTransformCore, DefaultLogicalExprNodeExt, FacetAxis, FacetDimensionConfig,
    FacetEmptyCellPolicy, FacetWrapColumnMode, MarkRuntimeContext, RowDimensionConfig,
    ScaleSharing, ScaleTypePreference, SerializableExpr, Size2D, SubplotContainerCoordinateSystem,
    SubplotDataSource, SubplotMarkCore, WrapDimensionConfig, channel_value::expr_to_string,
    default_scale_type_for_data_type,
};
use avenger_chart_marks::Subplot;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::{future::Future, pin::Pin, sync::Arc};
use tracing::trace;

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug)]
struct FacetBandRenderOps {
    axis: FacetAxis,
    label: &'static str,
    group_prefix: &'static str,
}

impl FacetBandRenderOps {
    fn row() -> Self {
        Self {
            axis: FacetAxis::Row,
            label: "FacetRow",
            group_prefix: "facet_row_",
        }
    }

    fn col() -> Self {
        Self {
            axis: FacetAxis::Column,
            label: "FacetCol",
            group_prefix: "facet_col_",
        }
    }

    fn group_name(self, idx: usize, is_empty: bool) -> String {
        if is_empty {
            format!("{}{idx}_empty", self.group_prefix)
        } else {
            format!("{}{idx}", self.group_prefix)
        }
    }

    fn trace_position(self, idx: usize, subplot_origin: [f32; 2], position: f32, band_size: f32) {
        match self.axis {
            FacetAxis::Column => trace!(
                cell_index = idx,
                origin_x = subplot_origin[0],
                origin_y = subplot_origin[1],
                x = position,
                width = band_size,
                "{} render position",
                self.label
            ),
            FacetAxis::Row => trace!(
                cell_index = idx,
                origin_x = subplot_origin[0],
                origin_y = subplot_origin[1],
                y = position,
                height = band_size,
                "{} render position",
                self.label
            ),
        }
    }
}

fn facet_subplot_eval_ctx(
    compiled_subplot: &CompiledPlot,
    context: &RenderContext,
    axis_owner_ignore_empty_cells: bool,
) -> EvaluationContext {
    let mut params = compiled_subplot.get_default_params().clone();
    params.extend(context.eval.params.clone());
    let eval_ctx = context.eval.with_params(params);
    eval_ctx.with_axis_owner_ignore_empty_cells(axis_owner_ignore_empty_cells)
}

async fn render_facet_band_with_placement(
    ops: FacetBandRenderOps,
    compiled_subplot: &CompiledPlot,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    context: &RenderContext<'_>,
    facet_measurement: &FacetBandCoordMeasurement,
    placement: FacetBandPlacement,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    if facet_measurement.cells.is_empty() {
        return Ok(Vec::new());
    }

    let ownership_policy = resolve_facet_ownership_policy(
        facet_empty_cell_policy,
        has_holes_from_cells(
            facet_measurement
                .cells
                .iter()
                .map(|cell| cell.plan.is_empty),
        ),
    );
    let subplot_eval_ctx = facet_subplot_eval_ctx(
        compiled_subplot,
        context,
        ownership_policy.axis_owner_ignore_empty_cells,
    );

    let child_frame_placement = facet_child_frame_placement_from_band(
        facet_measurement,
        &placement,
        Size2D::new(context.plot_width(), context.plot_height()),
    )?;
    trace!(
        axis = ?placement.axis,
        main_axis_extent = placement.main_axis_extent,
        cross_axis_extent = ?placement.cross_axis_extent,
        child_content_width = child_frame_placement.content_size.width,
        child_content_height = child_frame_placement.content_size.height,
        cell_count = placement.cell_count(),
        "{} render placement resolved",
        ops.label
    );

    let mut scene_marks = Vec::with_capacity(facet_measurement.cells.len());
    for (idx, cell) in facet_measurement.cells.iter().enumerate() {
        let cell_placement = placement.cell(idx).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing facet render placement for cell index {idx}"
            ))
        })?;
        let child_render_placement = child_frame_placement.child(idx).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing facet child-frame placement for cell index {idx}"
            ))
        })?;
        let position = cell_placement.main_axis_start;
        let subplot_origin = child_render_placement.origin;
        let is_empty_cell = cell.plan.is_empty;
        let band_size = cell_placement.main_axis_size;

        if is_empty_cell
            && matches!(
                ownership_policy.effective_empty_cell_policy,
                FacetEmptyCellPolicy::Hole
            )
        {
            let empty_group = SceneGroup {
                name: ops.group_name(idx, true),
                origin: subplot_origin,
                clip: avenger_scenegraph::marks::group::Clip::None,
                marks: Vec::new(),
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
            };
            scene_marks.push(SceneMark::Group(empty_group));
            continue;
        }

        let cell_eval_ctx = if cell_requires_invalid_path_axis_fallback_hidden(
            is_empty_cell,
            cell.plan.in_domain_slot,
        ) {
            subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
        } else {
            subplot_eval_ctx.clone()
        }
        .with_facet_coord_node_path_appended(idx)
        .with_child_frame_container_path_appended(
            crate::container::ContainerPathSegment::facet_value(
                facet_measurement.axis,
                facet_measurement.facet_depth,
                cell.plan.value.clone(),
            ),
        );

        let is_terminal_cell =
            facet_band_ref(cell.measurement.coord_measurement.as_ref()).is_none();
        let cached_profile = if is_terminal_cell {
            cell_eval_ctx.layout_profile().and_then(|profile| {
                let source_measurement = profile.facet_cell_measurement(
                    cell_eval_ctx.facet_tree.as_ref(),
                    &cell.plan.full_path,
                    compiled_subplot,
                    cell_eval_ctx.session_context().as_ref(),
                    cell_eval_ctx.params(),
                )?;
                let components = profile.facet_cell_rendered_components(
                    cell_eval_ctx.facet_tree.as_ref(),
                    &cell.plan.full_path,
                    compiled_subplot,
                    cell_eval_ctx.session_context().as_ref(),
                    cell_eval_ctx.params(),
                )?;
                Some((source_measurement, components))
            })
        } else {
            None
        };
        let components = if let Some((source_measurement, cached_components)) = cached_profile {
            match Box::pin(compiled_subplot.build_plot_components_reusing_data_marks(
                &cell_eval_ctx,
                &source_measurement,
                &cell.measurement,
                Some(&cell.data_override),
                true,
                &cell.plan.full_path,
                &cached_components,
            ))
            .await?
            {
                Some(components) => {
                    cell_eval_ctx.record_preview_data_mark_reuse();
                    components
                }
                None => {
                    cell_eval_ctx.record_preview_data_mark_reuse_miss();
                    Box::pin(compiled_subplot.build_plot_components(
                        &cell_eval_ctx,
                        &cell.measurement,
                        Some(&cell.data_override),
                        true,
                        &cell.plan.full_path,
                    ))
                    .await?
                }
            }
        } else {
            if is_terminal_cell && cell_eval_ctx.layout_profile().is_some() {
                cell_eval_ctx.record_preview_data_mark_reuse_miss();
            }
            Box::pin(compiled_subplot.build_plot_components(
                &cell_eval_ctx,
                &cell.measurement,
                Some(&cell.data_override),
                true,
                &cell.plan.full_path,
            ))
            .await?
        };

        if is_terminal_cell
            && let Some(capture) = cell_eval_ctx.facet_cell_rendered_components_capture()
        {
            capture
                .lock()
                .expect("facet cell rendered components profile lock poisoned")
                .insert_for_cell(
                    cell_eval_ctx.facet_tree.as_ref(),
                    &cell.plan.full_path,
                    compiled_subplot,
                    cell_eval_ctx.session_context().as_ref(),
                    cell_eval_ctx.params(),
                    components.clone(),
                );
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

        let subplot_group = SceneGroup {
            name: ops.group_name(idx, false),
            origin: subplot_origin,
            clip: avenger_scenegraph::marks::group::Clip::None,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        };
        scene_marks.push(SceneMark::Group(subplot_group));
        ops.trace_position(idx, subplot_origin, position, band_size);
    }

    Ok(scene_marks)
}

async fn render_facet_band_common(
    ops: FacetBandRenderOps,
    compiled_subplot: &CompiledPlot,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let facet_measurement = context
        .coord_measurement()
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected FacetBandCoordMeasurement in coord_measurement".into(),
            )
        })?;

    if facet_measurement.cells.is_empty() {
        return Ok(Vec::new());
    }

    let placement = facet_measurement.resolved_placement_from_scale_specs(context.scales())?;
    render_facet_band_with_placement(
        ops,
        compiled_subplot,
        facet_empty_cell_policy,
        context,
        facet_measurement,
        placement,
    )
    .await
}

/// Facet row channel builder methods for `Subplot<FacetRow>`.
pub trait FacetRowSubplotChannels: Sized {
    /// Set the faceting channel for rows.
    fn row<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure row faceting, including ordering, slot sharing, and guide options.
    fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig;
}

impl FacetRowSubplotChannels for Subplot<FacetRow> {
    fn row<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(RowDimensionConfig::channel_name(), value.into())
    }

    fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        s.set_facet_row_options(
            cfg.title,
            cfg.slot_sharing,
            cfg.position,
            cfg.visible,
            cfg.empty_cell_policy,
            cfg.order_expr,
            cfg.order_descending,
        );
        s
    }
}

/// Facet column channel builder methods for `Subplot<FacetColumn>`.
pub trait FacetColumnSubplotChannels: Sized {
    /// Set the faceting channel for columns.
    fn column<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure column faceting, including ordering, slot sharing, and guide options.
    fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig;

    /// Configure column faceting, including ordering, slot sharing, and guide options.
    fn column_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig;
}

impl FacetColumnSubplotChannels for Subplot<FacetColumn> {
    fn column<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(ColumnDimensionConfig::channel_name(), value.into())
    }

    fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.column(value);
        let cfg = f(FacetColChannelConfig::default());
        s.set_facet_col_options(
            cfg.title,
            cfg.slot_sharing,
            cfg.position,
            cfg.visible,
            cfg.empty_cell_policy,
            cfg.order_expr,
            cfg.order_descending,
        );
        s
    }

    fn column_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        self.col_with(value, f)
    }
}

/// Facet wrap channel builder methods for `Subplot<FacetWrap>`.
pub trait FacetWrapSubplotChannels: Sized {
    /// Set the wrapped facet channel.
    fn wrap<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure wrapped faceting, including physical columns, ordering, slot
    /// sharing, and guide options.
    fn wrap_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetWrapChannelConfig) -> FacetWrapChannelConfig;
}

impl FacetWrapSubplotChannels for Subplot<FacetWrap> {
    fn wrap<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(WrapDimensionConfig::channel_name(), value.into().no_scale())
    }

    fn wrap_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetWrapChannelConfig) -> FacetWrapChannelConfig,
    {
        let mut s = self.wrap(value);
        let cfg = f(FacetWrapChannelConfig::default());
        s.set_facet_wrap_options(
            cfg.title,
            cfg.slot_sharing,
            cfg.position,
            cfg.visible,
            cfg.empty_cell_policy,
            cfg.order_expr,
            cfg.order_descending,
            cfg.column_mode,
        );
        s
    }
}

fn facet_title_for_channel(
    explicit_title: Option<&str>,
    channel_name: &str,
    compiled_state: &CompiledMarkState,
    session_context: &SessionContext,
) -> Option<String> {
    match explicit_title {
        Some("") => None,
        Some(title) => Some(title.to_string()),
        None => compiled_state
            .data
            .channels()
            .get(channel_name)
            .and_then(|cv| cv.expr(session_context))
            .map(|expr| expr_to_string(&expr)),
    }
}

async fn compile_facet_subplot_child(
    subplot: &dyn SubplotMarkCore,
    session_context: &SessionContext,
) -> Result<Arc<CompiledPlot>, AvengerChartError> {
    if subplot.has_plot_level_data() {
        return Err(AvengerChartError::InvalidArgument(
            "Nested facet plots should not have their own data attached. \
             Data flows from the parent facet to child plots. \
             Remove the .data() call from the inner Plot."
                .to_string(),
        ));
    }

    subplot
        .compile_child_plot(session_context)
        .await?
        .into_any_arc()
        .downcast::<CompiledPlot>()
        .map_err(|_| {
            AvengerChartError::InternalError(
                "facet subplot child plot did not compile to avenger-chart CompiledPlot"
                    .to_string(),
            )
        })
}

/// Compiled subplot mark specialized for the FacetRow outer coordinate system.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRowSubplot {
    pub(crate) payload: CompiledSubplotPayload,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) facet_guide_visible: bool,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub(crate) facet_order_expr: Option<LogicalExprNode>,
    #[serde(default)]
    pub(crate) facet_order_descending: bool,
}

impl CompiledFacetRowSubplot {
    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub(crate) fn compiled_subplot_arc(&self) -> Arc<CompiledPlot> {
        compiled_subplot_payload_child_plot_arc(&self.payload)
    }
    pub fn compiled_state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }
    pub fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    pub fn facet_slot_sharing(&self) -> Option<ScaleSharing> {
        self.facet_slot_sharing
    }
    pub fn facet_position(&self) -> Option<&str> {
        self.facet_position.as_deref()
    }
    pub fn facet_guide_visible(&self) -> bool {
        self.facet_guide_visible
    }
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
    }
    pub fn facet_order_expr(&self) -> Option<&LogicalExprNode> {
        self.facet_order_expr.as_ref()
    }
    pub fn facet_order_descending(&self) -> bool {
        self.facet_order_descending
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'a>> {
        Box::pin(render_facet_band_common(
            FacetBandRenderOps::row(),
            self.compiled_subplot(),
            self.facet_empty_cell_policy,
            context,
        ))
    }
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for FacetRow {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_channel(ColumnDimensionConfig::channel_name(), "FacetRow")?;
        let compiled_subplot = compile_facet_subplot_child(subplot, session_context).await?;
        let channel_name = RowDimensionConfig::channel_name();
        let facet_title = facet_title_for_channel(
            subplot.facet_row_title_config(),
            channel_name,
            &compiled_state,
            session_context,
        );

        Ok(Arc::new(CompiledFacetRowSubplot {
            payload: CompiledSubplotPayload::new(
                compiled_state,
                compiled_subplot,
                subplot.label_config().map(ToOwned::to_owned),
                subplot.key_config().map(ToOwned::to_owned),
                SubplotDataSource::InheritParent,
            ),
            facet_title,
            facet_slot_sharing: subplot.facet_row_slot_sharing_config(),
            facet_position: subplot.facet_row_position_config().map(ToOwned::to_owned),
            facet_guide_visible: subplot.facet_row_guide_visible_config().unwrap_or(true),
            facet_empty_cell_policy: subplot
                .facet_row_empty_cell_policy_config()
                .unwrap_or_default(),
            facet_order_expr: subplot.facet_row_order_expr_config().cloned(),
            facet_order_descending: subplot.facet_row_order_descending_config(),
        }))
    }
}

impl CompiledMarkCore for CompiledFacetRowSubplot {
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
        "facet_row"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            name: RowDimensionConfig::channel_name(),
            required: true,
            default_value: None,
            allow_column_ref: true,
        }]
    }

    fn wants_full_data_batch(&self) -> bool {
        true // Facets need full data for nested filtering
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<ScaleTypePreference> {
        if channel == RowDimensionConfig::channel_name() {
            // Use band scale for row faceting regardless of domain type (categorical input expected)
            Some(ScaleTypePreference::Band)
        } else {
            default_scale_type_for_data_type(data_type)
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> std::collections::HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        let mut options = HashMap::new();

        // Configure band scale padding for facet row channel
        // Note: padding_inner is set by the FacetRow coordinate system (default 0.1)
        // We only set padding_outer and alignment here
        if channel == RowDimensionConfig::channel_name() && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Align bands flush to the top so the first row's
            // band_start is 0.0. Mirrors FacetCol behavior to avoid
            // 1px vertical offsets in debug overlays.
            options.insert("align".to_string(), lit(0.0f32));
            // Disable band rounding - we handle rounding manually in closures for better control
            options.insert("round".to_string(), lit(false));
        }

        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetRowSubplot {
    /// Render faceted row layout
    ///
    /// Uses the coordinate-system measurement from RenderContext (computed by FacetRow)
    /// and the adjusted row scale to resolve deterministic band positions.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Facet row subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}

// ============================================================================
// FacetCol Implementation
// ============================================================================

/// Compiled subplot mark specialized for the FacetColumn outer coordinate system.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetColumnSubplot {
    pub(crate) payload: CompiledSubplotPayload,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) facet_guide_visible: bool,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub(crate) facet_order_expr: Option<LogicalExprNode>,
    #[serde(default)]
    pub(crate) facet_order_descending: bool,
}

impl CompiledFacetColumnSubplot {
    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub(crate) fn compiled_subplot_arc(&self) -> Arc<CompiledPlot> {
        compiled_subplot_payload_child_plot_arc(&self.payload)
    }
    pub fn compiled_state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }
    pub fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    pub fn facet_slot_sharing(&self) -> Option<ScaleSharing> {
        self.facet_slot_sharing
    }
    pub fn facet_position(&self) -> Option<&str> {
        self.facet_position.as_deref()
    }
    pub fn facet_guide_visible(&self) -> bool {
        self.facet_guide_visible
    }
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
    }
    pub fn facet_order_expr(&self) -> Option<&LogicalExprNode> {
        self.facet_order_expr.as_ref()
    }
    pub fn facet_order_descending(&self) -> bool {
        self.facet_order_descending
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'a>> {
        Box::pin(render_facet_band_common(
            FacetBandRenderOps::col(),
            self.compiled_subplot(),
            self.facet_empty_cell_policy,
            context,
        ))
    }
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for FacetColumn {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_channel(RowDimensionConfig::channel_name(), "FacetColumn")?;
        let compiled_subplot = compile_facet_subplot_child(subplot, session_context).await?;
        let channel_name = ColumnDimensionConfig::channel_name();
        let facet_title = facet_title_for_channel(
            subplot.facet_col_title_config(),
            channel_name,
            &compiled_state,
            session_context,
        );

        Ok(Arc::new(CompiledFacetColumnSubplot {
            payload: CompiledSubplotPayload::new(
                compiled_state,
                compiled_subplot,
                subplot.label_config().map(ToOwned::to_owned),
                subplot.key_config().map(ToOwned::to_owned),
                SubplotDataSource::InheritParent,
            ),
            facet_title,
            facet_slot_sharing: subplot.facet_col_slot_sharing_config(),
            facet_position: subplot.facet_col_position_config().map(ToOwned::to_owned),
            facet_guide_visible: subplot.facet_col_guide_visible_config().unwrap_or(true),
            facet_empty_cell_policy: subplot
                .facet_col_empty_cell_policy_config()
                .unwrap_or_default(),
            facet_order_expr: subplot.facet_col_order_expr_config().cloned(),
            facet_order_descending: subplot.facet_col_order_descending_config(),
        }))
    }
}

// ============================================================================
// FacetWrap Implementation
// ============================================================================

/// Compiled subplot mark specialized for the FacetWrap outer coordinate system.
///
/// The public mark owns the authored wrapped value, while `physical_subplot`
/// is an internal `FacetColumn` child nested inside synthetic row bands. This
/// keeps rendering/guide code shared with row and column facets without making
/// the synthetic row a user-visible facet level.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetWrapSubplot {
    pub(crate) payload: CompiledSubplotPayload,
    pub(crate) physical_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) facet_guide_visible: bool,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub(crate) facet_order_expr: Option<LogicalExprNode>,
    #[serde(default)]
    pub(crate) facet_order_descending: bool,
    #[serde(default)]
    pub(crate) facet_column_mode: FacetWrapColumnMode,
}

impl CompiledFacetWrapSubplot {
    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub(crate) fn physical_subplot(&self) -> &CompiledPlot {
        &self.physical_subplot
    }

    pub(crate) fn physical_subplot_arc(&self) -> Arc<CompiledPlot> {
        self.physical_subplot.clone()
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    pub fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }

    pub fn facet_slot_sharing(&self) -> Option<ScaleSharing> {
        self.facet_slot_sharing
    }

    pub fn facet_position(&self) -> Option<&str> {
        self.facet_position.as_deref()
    }

    pub fn facet_guide_visible(&self) -> bool {
        self.facet_guide_visible
    }

    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
    }

    pub fn facet_order_expr(&self) -> Option<&LogicalExprNode> {
        self.facet_order_expr.as_ref()
    }

    pub fn facet_order_descending(&self) -> bool {
        self.facet_order_descending
    }

    pub fn facet_column_mode(&self) -> FacetWrapColumnMode {
        self.facet_column_mode.clone()
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'a>> {
        Box::pin(render_facet_band_common(
            FacetBandRenderOps::row(),
            self.physical_subplot(),
            self.facet_empty_cell_policy,
            context,
        ))
    }
}

fn synthetic_column_state_for_wrap(
    compiled_state: &CompiledMarkState,
    session_context: &SessionContext,
) -> Result<CompiledMarkState, AvengerChartError> {
    let wrap_channel = compiled_state
        .data
        .channels()
        .get(WrapDimensionConfig::channel_name())
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "FacetWrap requires a `wrap` channel. Use `.wrap(...)` or `.wrap_with(...)`."
                    .to_string(),
            )
        })?;
    let wrap_expr = wrap_channel.expr(session_context).ok_or_else(|| {
        AvengerChartError::InvalidArgument(
            "FacetWrap `wrap` channel must be a single expression".to_string(),
        )
    })?;
    let mut channels = compiled_state.data.channels().clone();
    channels.swap_remove(WrapDimensionConfig::channel_name());
    channels.insert(
        ColumnDimensionConfig::channel_name().to_string(),
        ChannelValue::from(wrap_expr),
    );

    let mut state = compiled_state.clone();
    state.data = CompiledDataContext::from_logical_plan_node(
        compiled_state.data.logical_plan_node().cloned(),
        channels,
    );
    Ok(state)
}

fn build_physical_wrap_subplot(
    compiled_state: &CompiledMarkState,
    compiled_subplot: Arc<CompiledPlot>,
    facet_title: Option<String>,
    facet_position: Option<String>,
    facet_guide_visible: bool,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    session_context: &SessionContext,
) -> Result<Arc<CompiledPlot>, AvengerChartError> {
    let synthetic_state = synthetic_column_state_for_wrap(compiled_state, session_context)?;
    let synthetic_column_mark: Arc<dyn CompiledMark> = Arc::new(CompiledFacetColumnSubplot {
        payload: CompiledSubplotPayload::new(
            synthetic_state,
            compiled_subplot.clone(),
            None,
            None,
            SubplotDataSource::InheritParent,
        ),
        facet_title,
        facet_slot_sharing: Some(ScaleSharing::Free),
        facet_position,
        facet_guide_visible,
        facet_empty_cell_policy,
        facet_order_expr: None,
        facet_order_descending: false,
    });
    let synthetic_marks = vec![synthetic_column_mark];
    let mut guide = crate::facet::guide::FacetColGuideConfig::default();
    guide.set_compiled_marks(&synthetic_marks, session_context);
    let compiled_guide = Arc::from(guide.build());

    Ok(Arc::new(CompiledPlot {
        coord_transform: Box::new(FacetColumn),
        compiled_guide: Some(compiled_guide),
        marks: synthetic_marks,
        axis_specs: Default::default(),
        legends: Default::default(),
        layout_spec: Default::default(),
        title: None,
        subtitle: None,
        theme: compiled_subplot.theme.clone(),
        scale_to_coord_channel: Default::default(),
        scale_specs: Default::default(),
        data: None,
        default_params: compiled_subplot.default_params.clone(),
    }))
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for FacetWrap {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_channel(RowDimensionConfig::channel_name(), "FacetWrap")?;
        subplot.validate_no_channel(ColumnDimensionConfig::channel_name(), "FacetWrap")?;
        let compiled_subplot = compile_facet_subplot_child(subplot, session_context).await?;
        let channel_name = WrapDimensionConfig::channel_name();
        let facet_title = facet_title_for_channel(
            subplot.facet_wrap_title_config(),
            channel_name,
            &compiled_state,
            session_context,
        );
        let facet_position = subplot.facet_wrap_position_config().map(ToOwned::to_owned);
        let facet_guide_visible = subplot.facet_wrap_guide_visible_config().unwrap_or(true);
        let facet_empty_cell_policy = subplot
            .facet_wrap_empty_cell_policy_config()
            .unwrap_or_default();
        let physical_subplot = build_physical_wrap_subplot(
            &compiled_state,
            compiled_subplot.clone(),
            facet_title.clone(),
            facet_position.clone(),
            facet_guide_visible,
            facet_empty_cell_policy,
            session_context,
        )?;

        Ok(Arc::new(CompiledFacetWrapSubplot {
            payload: CompiledSubplotPayload::new(
                compiled_state,
                compiled_subplot,
                subplot.label_config().map(ToOwned::to_owned),
                subplot.key_config().map(ToOwned::to_owned),
                SubplotDataSource::InheritParent,
            ),
            physical_subplot,
            facet_title,
            facet_slot_sharing: subplot.facet_wrap_slot_sharing_config(),
            facet_position,
            facet_guide_visible,
            facet_empty_cell_policy,
            facet_order_expr: subplot.facet_wrap_order_expr_config().cloned(),
            facet_order_descending: subplot.facet_wrap_order_descending_config(),
            facet_column_mode: subplot.facet_wrap_column_mode_config(),
        }))
    }
}

impl CompiledMarkCore for CompiledFacetWrapSubplot {
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
        "facet_wrap"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            name: WrapDimensionConfig::channel_name(),
            required: true,
            default_value: None,
            allow_column_ref: true,
        }]
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetWrapSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Facet wrap subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}

/// Typed view over compiled facet subplot marks.
pub enum FacetSubplotRef<'a> {
    Row(&'a CompiledFacetRowSubplot),
    Col(&'a CompiledFacetColumnSubplot),
    Wrap(&'a CompiledFacetWrapSubplot),
}

impl<'a> FacetSubplotRef<'a> {
    pub fn compiled_subplot(self) -> &'a CompiledPlot {
        match self {
            Self::Row(mark) => mark.compiled_subplot(),
            Self::Col(mark) => mark.compiled_subplot(),
            Self::Wrap(mark) => mark.compiled_subplot(),
        }
    }

    pub fn physical_compiled_subplot(self) -> &'a CompiledPlot {
        match self {
            Self::Row(mark) => mark.compiled_subplot(),
            Self::Col(mark) => mark.compiled_subplot(),
            Self::Wrap(mark) => mark.physical_subplot(),
        }
    }

    pub fn facet_slot_sharing(self) -> Option<ScaleSharing> {
        match self {
            Self::Row(mark) => mark.facet_slot_sharing(),
            Self::Col(mark) => mark.facet_slot_sharing(),
            Self::Wrap(mark) => mark.facet_slot_sharing(),
        }
    }

    pub fn facet_order_expr(
        self,
        ctx: &SessionContext,
    ) -> Result<Option<datafusion::logical_expr::Expr>, AvengerChartError> {
        match self {
            Self::Row(mark) => mark
                .facet_order_expr()
                .map(|expr| expr.to_expr(ctx))
                .transpose(),
            Self::Col(mark) => mark
                .facet_order_expr()
                .map(|expr| expr.to_expr(ctx))
                .transpose(),
            Self::Wrap(mark) => mark
                .facet_order_expr()
                .map(|expr| expr.to_expr(ctx))
                .transpose(),
        }
    }

    pub fn facet_order_descending(self) -> bool {
        match self {
            Self::Row(mark) => mark.facet_order_descending(),
            Self::Col(mark) => mark.facet_order_descending(),
            Self::Wrap(mark) => mark.facet_order_descending(),
        }
    }

    pub fn facet_column_mode(self) -> FacetWrapColumnMode {
        match self {
            Self::Wrap(mark) => mark.facet_column_mode(),
            Self::Row(_) | Self::Col(_) => FacetWrapColumnMode::Auto,
        }
    }

    pub fn facet_empty_cell_policy(self) -> FacetEmptyCellPolicy {
        match self {
            Self::Row(mark) => mark.facet_empty_cell_policy(),
            Self::Col(mark) => mark.facet_empty_cell_policy(),
            Self::Wrap(mark) => mark.facet_empty_cell_policy(),
        }
    }

    pub(crate) fn render_with_context<'b>(
        self,
        context: &'b RenderContext<'b>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'b>>
    where
        'a: 'b,
    {
        match self {
            Self::Row(mark) => mark.render_with_context(context),
            Self::Col(mark) => mark.render_with_context(context),
            Self::Wrap(mark) => mark.render_with_context(context),
        }
    }
}

/// Downcast a compiled mark into a typed facet subplot reference.
pub fn facet_subplot_ref(mark: &dyn CompiledMark) -> Option<FacetSubplotRef<'_>> {
    match mark.mark_type() {
        "facet_col" => mark
            .as_any()
            .downcast_ref::<CompiledFacetColumnSubplot>()
            .map(FacetSubplotRef::Col),
        "facet_row" => mark
            .as_any()
            .downcast_ref::<CompiledFacetRowSubplot>()
            .map(FacetSubplotRef::Row),
        "facet_wrap" => mark
            .as_any()
            .downcast_ref::<CompiledFacetWrapSubplot>()
            .map(FacetSubplotRef::Wrap),
        _ => None,
    }
}

impl CompiledMarkCore for CompiledFacetColumnSubplot {
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
        "facet_col"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            name: "column",
            required: true,
            default_value: None,
            allow_column_ref: true,
        }]
    }

    fn wants_full_data_batch(&self) -> bool {
        true // Facets need full data for nested filtering
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<ScaleTypePreference> {
        if channel == ColumnDimensionConfig::channel_name() {
            // Use band scale for column faceting regardless of domain type (categorical input expected)
            Some(ScaleTypePreference::Band)
        } else {
            default_scale_type_for_data_type(data_type)
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> std::collections::HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        let mut options = HashMap::new();

        // Configure band scale padding for facet col channel
        // Note: padding_inner is set by the FacetCol coordinate system (default 0.1)
        // We only set padding_outer and alignment here
        if channel == ColumnDimensionConfig::channel_name() && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Align bands flush to the left so band_start of the first
            // subplot is exactly 0. This prevents a residual 1px offset
            // from split rounding when distributing leftover space.
            options.insert("align".to_string(), lit(0.0f32));
            // Disable band rounding - we handle rounding manually in closures for better control
            options.insert("round".to_string(), lit(false));
        }

        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetColumnSubplot {
    /// Render faceted column layout
    ///
    /// Uses the coordinate-system measurement from RenderContext (computed by FacetColumn)
    /// and the already-adjusted column scale to resolve deterministic band positions,
    /// then renders each subplot using its prepared child measurement.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Facet column subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::{Mark, ZeroDCoord};
    use datafusion::prelude::{SessionContext, col};

    #[test]
    fn facet_band_render_ops_group_name_prefixes_row_vs_col() {
        let row_ops = FacetBandRenderOps::row();
        let col_ops = FacetBandRenderOps::col();

        assert_eq!(row_ops.group_name(3, false), "facet_row_3");
        assert_eq!(row_ops.group_name(3, true), "facet_row_3_empty");
        assert_eq!(col_ops.group_name(4, false), "facet_col_4");
        assert_eq!(col_ops.group_name(4, true), "facet_col_4_empty");
    }

    #[tokio::test]
    async fn facet_row_subplot_rejects_column_channel() {
        let ctx = SessionContext::new();
        let mut subplot =
            Subplot::<FacetRow>::new(crate::plot::Plot::<ZeroDCoord>::new()).row(col("row"));
        subplot.state_mut().data = subplot
            .state()
            .data
            .clone()
            .with_channel_value(ColumnDimensionConfig::channel_name(), col("col").into());

        let compiled_state = CompiledMarkState::from_mark_state(subplot.state(), None);
        let result =
            <Subplot<FacetRow> as Mark<FacetRow>>::compile(&subplot, compiled_state, &ctx).await;

        assert!(matches!(result, Err(AvengerChartError::InvalidArgument(_))));
    }
}
