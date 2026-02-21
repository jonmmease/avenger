use crate::channel::config_traits::ScaleSharing;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::band_positions::BandPositionIterator;
use crate::facet::coord::{FacetColumn, FacetRow};
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::empty_cell_policy::FacetEmptyCellPolicy;
use crate::facet::layout_slabs::LayoutSlabs;
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::trace;

fn facet_cell_main_axis_start_offset(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
) -> (f32, f32) {
    let slabs = LayoutSlabs::from_coordinated(&facet_measurement.coordinated_overflow);
    match facet_measurement.axis {
        crate::coords::FacetAxis::Column => (0.0, slabs.legend.top),
        crate::coords::FacetAxis::Row => (slabs.legend.left, 0.0),
    }
}

/// Facet mark for FacetRow or FacetCol outer coordinate system.
/// Renders a provided inner plot for each band value in the facet channel.
#[derive(Clone)]
pub struct Facet<InnerC: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) subplot: Option<Plot<InnerC>>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    pub(crate) facet_row_scale_sharing: Option<ScaleSharing>,
    pub(crate) facet_col_scale_sharing: Option<ScaleSharing>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

impl<InnerC: CoordinateSystem> Facet<InnerC> {
    pub fn new() -> Self {
        Self {
            state: MarkState {
                data: crate::marks::DataContext::default(),
                facet_strategy: crate::marks::FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot: None,
            facet_row_title: None,
            facet_col_title: None,
            facet_spacing: None,
            facet_row_scale_sharing: None,
            facet_col_scale_sharing: None,
            facet_row_position: None,
            facet_col_position: None,
            facet_row_empty_cell_policy: None,
            facet_col_empty_cell_policy: None,
        }
    }

    /// Set explicit data for this mark
    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.state.data = crate::marks::DataContext::new(dataframe);
        self
    }

    /// Set zindex
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    /// Set the faceting channel for rows
    pub fn row<V: Into<ChannelValue>>(self, value: V) -> Self {
        let mut s = self;
        s.state.data = s
            .state
            .data
            .with_channel_value(RowDimensionConfig::channel_name(), value.into());
        s
    }

    /// Configure row with facet options (e.g., title, spacing, scale_sharing)
    pub fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        s.facet_row_title = cfg.title;
        s.facet_spacing = cfg.spacing;
        s.facet_row_scale_sharing = cfg.scale_sharing;
        s.facet_row_position = cfg.position;
        s.facet_row_empty_cell_policy = cfg.empty_cell_policy;
        s
    }

    /// Set the faceting channel for columns
    pub fn column<V: Into<ChannelValue>>(self, value: V) -> Self {
        let mut s = self;
        s.state.data = s
            .state
            .data
            .with_channel_value(ColumnDimensionConfig::channel_name(), value.into());
        s
    }

    /// Configure col with facet options (e.g., title, spacing, scale_sharing, position)
    pub fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.column(value);
        let cfg = f(FacetColChannelConfig::default());
        s.facet_col_title = cfg.title;
        s.facet_spacing = cfg.spacing;
        s.facet_col_scale_sharing = cfg.scale_sharing;
        s.facet_col_position = cfg.position;
        s.facet_col_empty_cell_policy = cfg.empty_cell_policy;
        s
    }

    /// Provide a subplot Plot<InnerC>
    pub fn subplot(mut self, plot: Plot<InnerC>) -> Self {
        self.subplot = Some(plot);
        self
    }
}

/// Compiled facet mark specialized for FacetRow outer coords
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    pub(crate) facet_scale_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
}

impl CompiledFacetRow {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }
    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }
    pub fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    pub fn facet_spacing(&self) -> Option<f32> {
        self.facet_spacing
    }
    pub fn facet_scale_sharing(&self) -> Option<ScaleSharing> {
        self.facet_scale_sharing
    }
    pub fn facet_position(&self) -> Option<&str> {
        self.facet_position.as_deref()
    }
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
    }
}

#[async_trait::async_trait]
impl<InnerC: CoordinateSystem + Clone> Mark<FacetRow> for Facet<InnerC> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let compiled_subplot: Arc<CompiledPlot> = {
            let plot_ref = self
                .subplot
                .as_ref()
                .ok_or_else(|| AvengerChartError::InternalError("Facet subplot not set".into()))?;

            // Validate that subplot doesn't have its own data
            if plot_ref.data.is_some() {
                return Err(AvengerChartError::InvalidArgument(
                    "Nested facet plots should not have their own data attached. \
                     Data flows from the parent facet to child plots. \
                     Remove the .data() call from the inner Plot."
                        .to_string(),
                ));
            }

            let plot_clone = plot_ref.clone();
            // Plot<InnerC> implements Clone; explicitly call Clone::clone
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };

        // Derive facet title: explicit title > derived from row name > None (if explicitly disabled)
        let facet_title = match &self.facet_row_title {
            Some(title) if title.is_empty() => None, // Explicitly disabled with empty string
            Some(title) => Some(title.clone()),      // Explicit title
            None => {
                // Derive from row expression
                let channel_name = RowDimensionConfig::channel_name();
                compiled_state
                    .data
                    .channels()
                    .get(channel_name)
                    .and_then(|cv| cv.expr(session_context))
                    .map(|expr| crate::channel::value::expr_to_string(&expr))
            }
        };

        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot,
            facet_title,
            facet_spacing: self.facet_spacing,
            facet_scale_sharing: self.facet_row_scale_sharing,
            facet_position: self.facet_row_position.clone(),
            facet_empty_cell_policy: self.facet_row_empty_cell_policy.unwrap_or_default(),
        }))
    }
}

// No helper methods needed for CompiledFacetRow - moved to facet_evaluation module

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetRow {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::CompiledDataContext {
        &self.state.data
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

    /// Render faceted row layout
    ///
    /// Uses coord_measurement from RenderContext (computed by FacetRow coord system)
    /// and the adjusted row scale to resolve deterministic band positions.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::facet::coord::FacetBandCoordMeasurement;
        use avenger_scales::scales::band::bandwidth;
        use avenger_scenegraph::marks::group::SceneGroup;

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

        let row_scale = context
            .scales()
            .get("row")
            .ok_or_else(|| AvengerChartError::InternalError("No row scale found".into()))?;

        let cell_values: Vec<ScalarValue> = facet_measurement.cell_values().cloned().collect();
        let configured = row_scale.configured();
        let cell_positions = resolve_facet_row_positions(configured, &cell_values)?;

        let subplot_height = bandwidth(&configured.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        let subplot_eval_ctx = {
            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(context.eval.params.clone());
            context.eval.with_params(params)
        };
        let (origin_offset_x, origin_offset_y) =
            facet_cell_main_axis_start_offset(facet_measurement);

        trace!(
            legend_start_left = origin_offset_x,
            legend_start_top = origin_offset_y,
            "FacetRow render main-axis start slab offset"
        );
        let effective_empty_policy = self.facet_empty_cell_policy.effective();

        let mut scene_marks = Vec::with_capacity(facet_measurement.cells.len());

        for (idx, cell) in facet_measurement.cells.iter().enumerate() {
            let position = cell_positions[idx];
            let subplot_origin = [origin_offset_x, position + origin_offset_y];
            let is_empty_cell = cell.plan.is_empty;

            if is_empty_cell && matches!(effective_empty_policy, FacetEmptyCellPolicy::Hole) {
                let empty_group = SceneGroup {
                    name: format!("facet_row_{}_empty", idx),
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

            let cell_eval_ctx = if is_empty_cell && !cell.plan.in_domain_slot {
                subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
            } else {
                subplot_eval_ctx.clone()
            };

            let components = self
                .compiled_subplot
                .build_plot_components(
                    &cell_eval_ctx,
                    &cell.measurement,
                    Some(&cell.data_override),
                    true,
                    &cell.plan.full_path,
                )
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

            let subplot_group = SceneGroup {
                name: format!("facet_row_{}", idx),
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

            trace!(
                cell_index = idx,
                origin_x = subplot_origin[0],
                origin_y = subplot_origin[1],
                y = position,
                height = subplot_height,
                "FacetRow render position"
            );
        }

        Ok(scene_marks)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == RowDimensionConfig::channel_name() {
            // Use band scale for row faceting regardless of domain type (categorical input expected)
            Some(Box::new(crate::scales::spec::Band::default()))
        } else {
            crate::marks::default_scale_for_data_type(data_type)
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

// ============================================================================
// FacetCol Implementation
// ============================================================================

/// Compiled facet mark specialized for FacetCol outer coords
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetCol {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    pub(crate) facet_scale_sharing: Option<ScaleSharing>,
    pub(crate) facet_position: Option<String>,
    #[serde(default)]
    pub(crate) facet_empty_cell_policy: FacetEmptyCellPolicy,
}

impl CompiledFacetCol {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }
    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }
    pub fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    pub fn facet_spacing(&self) -> Option<f32> {
        self.facet_spacing
    }
    pub fn facet_scale_sharing(&self) -> Option<ScaleSharing> {
        self.facet_scale_sharing
    }
    pub fn facet_position(&self) -> Option<&str> {
        self.facet_position.as_deref()
    }
    pub fn facet_empty_cell_policy(&self) -> FacetEmptyCellPolicy {
        self.facet_empty_cell_policy
    }
}

/// Typed view over compiled facet marks.
pub enum FacetMarkRef<'a> {
    Row(&'a CompiledFacetRow),
    Col(&'a CompiledFacetCol),
}

impl<'a> FacetMarkRef<'a> {
    pub fn compiled_subplot(self) -> &'a Arc<CompiledPlot> {
        match self {
            Self::Row(mark) => mark.compiled_subplot(),
            Self::Col(mark) => mark.compiled_subplot(),
        }
    }

    pub fn facet_scale_sharing(self) -> Option<ScaleSharing> {
        match self {
            Self::Row(mark) => mark.facet_scale_sharing(),
            Self::Col(mark) => mark.facet_scale_sharing(),
        }
    }

    pub fn facet_empty_cell_policy(self) -> FacetEmptyCellPolicy {
        match self {
            Self::Row(mark) => mark.facet_empty_cell_policy(),
            Self::Col(mark) => mark.facet_empty_cell_policy(),
        }
    }
}

/// Downcast a compiled mark into a typed facet mark reference.
pub fn facet_mark_ref(mark: &dyn CompiledMark) -> Option<FacetMarkRef<'_>> {
    match mark.mark_type() {
        "facet_col" => mark
            .as_any()
            .downcast_ref::<CompiledFacetCol>()
            .map(FacetMarkRef::Col),
        "facet_row" => mark
            .as_any()
            .downcast_ref::<CompiledFacetRow>()
            .map(FacetMarkRef::Row),
        _ => None,
    }
}

#[async_trait::async_trait]
impl<InnerC: CoordinateSystem + Clone> Mark<FacetColumn> for Facet<InnerC> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let compiled_subplot: Arc<CompiledPlot> = {
            let plot_ref = self
                .subplot
                .as_ref()
                .ok_or_else(|| AvengerChartError::InternalError("Facet subplot not set".into()))?;

            // Validate that subplot doesn't have its own data
            if plot_ref.data.is_some() {
                return Err(AvengerChartError::InvalidArgument(
                    "Nested facet plots should not have their own data attached. \
                     Data flows from the parent facet to child plots. \
                     Remove the .data() call from the inner Plot."
                        .to_string(),
                ));
            }

            let plot_clone = plot_ref.clone();
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };

        // Derive facet title: explicit title > derived from column name > None (if explicitly disabled)
        let facet_title = match &self.facet_col_title {
            Some(title) if title.is_empty() => None, // Explicitly disabled with empty string
            Some(title) => Some(title.clone()),      // Explicit title
            None => {
                // Derive from column expression
                let channel_name = ColumnDimensionConfig::channel_name();
                compiled_state
                    .data
                    .channels()
                    .get(channel_name)
                    .and_then(|cv| cv.expr(session_context))
                    .map(|expr| crate::channel::value::expr_to_string(&expr))
            }
        };

        Ok(Arc::new(CompiledFacetCol {
            state: compiled_state,
            compiled_subplot,
            facet_title,
            facet_spacing: self.facet_spacing,
            facet_scale_sharing: self.facet_col_scale_sharing,
            facet_position: self.facet_col_position.clone(),
            facet_empty_cell_policy: self.facet_col_empty_cell_policy.unwrap_or_default(),
        }))
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetCol {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &crate::marks::CompiledDataContext {
        &self.state.data
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

    /// Measure faceted column layout
    ///
    /// Render faceted column layout
    ///
    /// Uses coord_measurement from RenderContext (computed by FacetColumn coord system)
    /// and the already-adjusted column scale to resolve deterministic band positions,
    /// then renders each subplot using its cached measurement.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::facet::coord::FacetBandCoordMeasurement;
        use avenger_scales::scales::band::bandwidth;
        use avenger_scenegraph::marks::group::SceneGroup;

        // Get coord_measurement from context and downcast to FacetBandCoordMeasurement
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

        // Get the column scale (already has padding_inner_px from ComponentsMeasurement)
        let column_scale = context
            .scales()
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        // Compute positions from the scale
        let cell_values: Vec<ScalarValue> = facet_measurement.cell_values().cloned().collect();
        let configured = column_scale.configured();
        let cell_positions = resolve_facet_col_positions(configured, &cell_values)?;

        // Get subplot width from scale (used for debug logging)
        let subplot_width = bandwidth(&configured.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(context.eval.params.clone());
            context.eval.with_params(params)
        };
        let (origin_offset_x, origin_offset_y) =
            facet_cell_main_axis_start_offset(facet_measurement);

        trace!(
            legend_start_left = origin_offset_x,
            legend_start_top = origin_offset_y,
            "FacetCol render main-axis start slab offset"
        );
        let effective_empty_policy = self.facet_empty_cell_policy.effective();

        let mut scene_marks = Vec::with_capacity(facet_measurement.cells.len());

        // Render each subplot using pre-computed measurements from coord.measure()
        // Note: Measurements have already been adjusted for legend overflow by
        // coordinate_overflow_for_guides() before build_plot_components() is called.
        for (idx, cell) in facet_measurement.cells.iter().enumerate() {
            let position = cell_positions[idx];
            let subplot_origin = [position + origin_offset_x, origin_offset_y];
            let is_empty_cell = cell.plan.is_empty;

            // For empty cells (created by Level(N) sharing for uniform layout),
            // render an empty group at the correct position to preserve layout.
            // Don't call build_plot_components which would trigger guide rendering
            // with potentially invalid scale domains (causing NaN/Inf errors).
            if is_empty_cell && matches!(effective_empty_policy, FacetEmptyCellPolicy::Hole) {
                let empty_group = SceneGroup {
                    name: format!("facet_col_{}_empty", idx),
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

            let cell_eval_ctx = if is_empty_cell && !cell.plan.in_domain_slot {
                subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
            } else {
                subplot_eval_ctx.clone()
            };

            // Build plot components using pre-computed measurement
            let components = self
                .compiled_subplot
                .build_plot_components(
                    &cell_eval_ctx,
                    &cell.measurement,
                    Some(&cell.data_override),
                    true, // dimensions_are_plot_area
                    &cell.plan.full_path,
                )
                .await?;

            // Mirror top-level plot composition: only data marks are clipped to plot area.
            // Guides/legends/titles must render outside the plot clip region.
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

            let subplot_group = SceneGroup {
                name: format!("facet_col_{}", idx),
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

            trace!(
                cell_index = idx,
                origin_x = subplot_origin[0],
                origin_y = subplot_origin[1],
                x = position,
                width = subplot_width,
                "FacetCol render position"
            );
        }

        Ok(scene_marks)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == ColumnDimensionConfig::channel_name() {
            // Use band scale for column faceting regardless of domain type (categorical input expected)
            Some(Box::new(crate::scales::spec::Band::default()))
        } else {
            crate::marks::default_scale_for_data_type(data_type)
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

fn resolve_facet_col_positions(
    configured: &avenger_scales::scales::ConfiguredScale,
    cell_values: &[ScalarValue],
) -> Result<Vec<f32>, AvengerChartError> {
    let bands: Vec<_> = BandPositionIterator::from_configured_scale(configured)?.collect();
    if bands.len() != cell_values.len() {
        return Err(AvengerChartError::InternalError(format!(
            "FacetCol render: band positions length {} did not match facet cell count {}",
            bands.len(),
            cell_values.len()
        )));
    }

    if !bands
        .iter()
        .zip(cell_values.iter())
        .all(|(band, value)| band.value == *value)
    {
        return Err(AvengerChartError::InternalError(
            "FacetCol render: band position order did not align with facet cell order".into(),
        ));
    }
    Ok(bands.into_iter().map(|band| band.start()).collect())
}

fn resolve_facet_row_positions(
    configured: &avenger_scales::scales::ConfiguredScale,
    cell_values: &[ScalarValue],
) -> Result<Vec<f32>, AvengerChartError> {
    let bands: Vec<_> = BandPositionIterator::from_configured_scale(configured)?.collect();
    if bands.len() != cell_values.len() {
        return Err(AvengerChartError::InternalError(format!(
            "FacetRow render: band positions length {} did not match facet cell count {}",
            bands.len(),
            cell_values.len()
        )));
    }

    if !bands
        .iter()
        .zip(cell_values.iter())
        .all(|(band, value)| band.value == *value)
    {
        return Err(AvengerChartError::InternalError(
            "FacetRow render: band position order did not align with facet cell order".into(),
        ));
    }

    Ok(bands.into_iter().map(|band| band.start()).collect())
}

// ============================================================================
// Axis-Aligned Empty Space Helper
// ============================================================================

/// Determine the band scale alignment value based on subplot axis position.
///
/// When faceting creates cells with different numbers of subplots (e.g., in nested facets
/// with Free scaling), empty space is allocated. This function determines where to place
/// that empty space relative to the axis labels:
///
/// - X-axis at Bottom → align=1.0 (push subplots to bottom, empty space at top)
/// - X-axis at Top → align=0.0 (push subplots to top, empty space at bottom)
/// - Y-axis at Left → align=0.0 (push subplots to left, empty space at right)
/// - Y-axis at Right → align=1.0 (push subplots to right, empty space at left)
///
/// This ensures subplots are visually aligned with their axis labels.
///
/// # Arguments
/// * `is_row_facet` - true for FacetRow (vertical stacking), false for FacetCol (horizontal)
/// * `compiled_subplot` - The compiled subplot to query axis position from
///
/// # Returns
/// Band scale align value: 0.0 (start) or 1.0 (end)
pub fn determine_facet_band_align(is_row_facet: bool, compiled_subplot: &CompiledPlot) -> f32 {
    use crate::{
        cartesian::{axis::AxisPosition, guide::CartesianGuide},
        guide::CompiledGuide,
    };

    // Try to get the axis position from the compiled guide
    if let Some(guide) = compiled_subplot.compiled_guide.as_ref() {
        // For row facets, check x-axis position (determines vertical alignment)
        // For column facets, check y-axis position (determines horizontal alignment)
        let channel = if is_row_facet { "x" } else { "y" };

        if let Some(position) = guide.axis_position(channel) {
            return match position {
                // Row faceting: x-axis position determines vertical alignment
                AxisPosition::Bottom => 1.0, // Push to bottom, empty space at top
                AxisPosition::Top => 0.0,    // Push to top, empty space at bottom
                // Column faceting: y-axis position determines horizontal alignment
                AxisPosition::Left => 0.0, // Push to left, empty space at right
                AxisPosition::Right => 1.0, // Push to right, empty space at left
            };
        }

        // If guide exists but doesn't provide axis_position, try downcasting to CartesianGuide
        // to access the axis_position method directly
        if let Some(cartesian) = guide.as_any().downcast_ref::<CartesianGuide>() {
            if let Some(position) = cartesian.axis_position(channel) {
                return match position {
                    AxisPosition::Bottom => 1.0,
                    AxisPosition::Top => 0.0,
                    AxisPosition::Left => 0.0,
                    AxisPosition::Right => 1.0,
                };
            }
        }
    }

    // Default: 0.0 (start alignment) for backward compatibility
    // This is the original hardcoded behavior
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scales::scales::band::BandScale;

    fn make_band_scale(range: (f32, f32)) -> avenger_scales::scales::ConfiguredScale {
        let domain = ScalarValue::iter_to_array(
            vec![
                ScalarValue::Utf8(Some("a".to_string())),
                ScalarValue::Utf8(Some("b".to_string())),
            ]
            .into_iter(),
        )
        .unwrap();
        BandScale::configured(domain, range)
    }

    #[test]
    fn resolve_facet_col_positions_returns_aligned_band_positions() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ];

        let positions = resolve_facet_col_positions(&scale, &cell_values).unwrap();
        assert_eq!(positions.len(), 2);
        assert!(positions[0] < positions[1]);
    }

    #[test]
    fn resolve_facet_col_positions_errors_on_count_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
            ScalarValue::Utf8(Some("c".to_string())),
        ];

        let err = resolve_facet_col_positions(&scale, &cell_values).unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band positions length"));
        assert!(message.contains("facet cell count"));
    }

    #[test]
    fn resolve_facet_col_positions_errors_on_order_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("b".to_string())),
            ScalarValue::Utf8(Some("a".to_string())),
        ];

        let err = resolve_facet_col_positions(&scale, &cell_values).unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band position order"));
        assert!(message.contains("facet cell order"));
    }
}
