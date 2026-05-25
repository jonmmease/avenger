use crate::chart_core::{MarkRuntimeContext, ScaleSharing, ScaleTypePreference};
use crate::coords::{CoordinateSystemCore, CoordinateSystemTransformCore, FacetAxis};
use crate::error::AvengerChartError;
use crate::facet::coord::{FacetBandCoordMeasurement, FacetColumn, FacetRow};
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::empty_cell_policy::FacetEmptyCellPolicy;
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::facet::ownership_policy::{
    cell_requires_invalid_path_axis_fallback_hidden, has_holes_from_cells,
    resolve_facet_ownership_policy,
};
use crate::facet::placement::{FacetBandPlacement, facet_child_frame_placement_from_band};
use crate::layout::Size2D;
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkCore, CompiledMarkState,
    CompiledSubplotPayload, Subplot, SubplotContainerCoordinateSystem, SubplotDataSource,
};
use crate::plot::CompiledPlot;
use crate::render::{EvaluationContext, RenderContext};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::prelude::SessionContext;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::{future::Future, pin::Pin, sync::Arc};
use tracing::trace;

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
    compiled_subplot: &Arc<CompiledPlot>,
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
    compiled_subplot: &Arc<CompiledPlot>,
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

        let components = Box::pin(compiled_subplot.build_plot_components(
            &cell_eval_ctx,
            &cell.measurement,
            Some(&cell.data_override),
            true,
            &cell.plan.full_path,
        ))
        .await?;

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
    compiled_subplot: &Arc<CompiledPlot>,
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

impl Subplot<FacetRow> {
    /// Set the faceting channel for rows.
    pub fn row<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(RowDimensionConfig::channel_name(), value.into())
    }

    /// Configure row with facet options (e.g., title, slot sharing).
    pub fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        let config = s.config_mut();
        config.facet_row_title = cfg.title;
        config.facet_row_slot_sharing = cfg.slot_sharing;
        config.facet_row_position = cfg.position;
        config.facet_row_empty_cell_policy = cfg.empty_cell_policy;
        s
    }
}

impl Subplot<FacetColumn> {
    /// Set the faceting channel for columns.
    pub fn column<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(ColumnDimensionConfig::channel_name(), value.into())
    }

    /// Configure column with facet options (e.g., title, slot sharing, position).
    pub fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.column(value);
        let cfg = f(FacetColChannelConfig::default());
        let config = s.config_mut();
        config.facet_col_title = cfg.title;
        config.facet_col_slot_sharing = cfg.slot_sharing;
        config.facet_col_position = cfg.position;
        config.facet_col_empty_cell_policy = cfg.empty_cell_policy;
        s
    }

    /// Configure column with facet options.
    pub fn column_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        self.col_with(value, f)
    }
}

fn facet_title_for_channel(
    explicit_title: &Option<String>,
    channel_name: &str,
    compiled_state: &CompiledMarkState,
    session_context: &SessionContext,
) -> Option<String> {
    match explicit_title {
        Some(title) if title.is_empty() => None,
        Some(title) => Some(title.clone()),
        None => compiled_state
            .data
            .channels()
            .get(channel_name)
            .and_then(|cv| cv.expr(session_context))
            .map(|expr| crate::channel::value::expr_to_string(&expr)),
    }
}

async fn compile_facet_subplot_child<OuterC: CoordinateSystemCore>(
    subplot: &Subplot<OuterC>,
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

    subplot.compile_child_plot(session_context).await
}

fn validate_no_channel(
    subplot: &Subplot<impl CoordinateSystemCore>,
    channel_name: &'static str,
    outer_label: &str,
) -> Result<(), AvengerChartError> {
    if subplot
        .data_context_ref()
        .channels()
        .contains_key(channel_name)
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{outer_label} subplots do not support channel `{channel_name}`"
        )));
    }
    Ok(())
}

/// Compiled subplot mark specialized for the FacetRow outer coordinate system.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRowSubplot {
    pub(crate) payload: CompiledSubplotPayload,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
}

impl CompiledFacetRowSubplot {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        self.payload.compiled_subplot()
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
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
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
        subplot: &Subplot<Self>,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        validate_no_channel(subplot, ColumnDimensionConfig::channel_name(), "FacetRow")?;
        let compiled_subplot = compile_facet_subplot_child(subplot, session_context).await?;
        let channel_name = RowDimensionConfig::channel_name();
        let facet_title = facet_title_for_channel(
            &subplot.config().facet_row_title,
            channel_name,
            &compiled_state,
            session_context,
        );

        Ok(Arc::new(CompiledFacetRowSubplot {
            payload: CompiledSubplotPayload::new(
                compiled_state,
                compiled_subplot,
                subplot.config().label.clone(),
                subplot.config().key.clone(),
                SubplotDataSource::InheritParent,
            ),
            facet_title,
            facet_slot_sharing: subplot.config().facet_row_slot_sharing,
            facet_position: subplot.config().facet_row_position.clone(),
            facet_empty_cell_policy: subplot
                .config()
                .facet_row_empty_cell_policy
                .unwrap_or_default(),
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

    fn data_context(&self) -> &crate::marks::CompiledDataContext {
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
            crate::marks::default_scale_type_for_data_type(data_type)
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
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
}

impl CompiledFacetColumnSubplot {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        self.payload.compiled_subplot()
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
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
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
        subplot: &Subplot<Self>,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        validate_no_channel(subplot, RowDimensionConfig::channel_name(), "FacetColumn")?;
        let compiled_subplot = compile_facet_subplot_child(subplot, session_context).await?;
        let channel_name = ColumnDimensionConfig::channel_name();
        let facet_title = facet_title_for_channel(
            &subplot.config().facet_col_title,
            channel_name,
            &compiled_state,
            session_context,
        );

        Ok(Arc::new(CompiledFacetColumnSubplot {
            payload: CompiledSubplotPayload::new(
                compiled_state,
                compiled_subplot,
                subplot.config().label.clone(),
                subplot.config().key.clone(),
                SubplotDataSource::InheritParent,
            ),
            facet_title,
            facet_slot_sharing: subplot.config().facet_col_slot_sharing,
            facet_position: subplot.config().facet_col_position.clone(),
            facet_empty_cell_policy: subplot
                .config()
                .facet_col_empty_cell_policy
                .unwrap_or_default(),
        }))
    }
}

/// Typed view over compiled facet subplot marks.
pub enum FacetSubplotRef<'a> {
    Row(&'a CompiledFacetRowSubplot),
    Col(&'a CompiledFacetColumnSubplot),
}

impl<'a> FacetSubplotRef<'a> {
    pub fn compiled_subplot(self) -> &'a Arc<CompiledPlot> {
        match self {
            Self::Row(mark) => mark.compiled_subplot(),
            Self::Col(mark) => mark.compiled_subplot(),
        }
    }

    pub fn facet_slot_sharing(self) -> Option<ScaleSharing> {
        match self {
            Self::Row(mark) => mark.facet_slot_sharing(),
            Self::Col(mark) => mark.facet_slot_sharing(),
        }
    }

    pub fn facet_empty_cell_policy(self) -> FacetEmptyCellPolicy {
        match self {
            Self::Row(mark) => mark.facet_empty_cell_policy(),
            Self::Col(mark) => mark.facet_empty_cell_policy(),
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

    fn data_context(&self) -> &crate::marks::CompiledDataContext {
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
            crate::marks::default_scale_type_for_data_type(data_type)
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
    /// then renders each subplot using its cached measurement.
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
    use crate::{marks::Mark, zerod::ZeroDCoord};
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
