use crate::channel::config_traits::ScaleSharing;
use crate::coords::{CoordinateSystem, FacetAxis};
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
use crate::render::context::FacetRuntimeSizingMode;
use crate::render::{EvaluationContext, RenderContext};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::trace;

#[path = "facet_canvas_fit.rs"]
mod facet_canvas_fit;
#[path = "facet_fixed_subplot.rs"]
mod facet_fixed_subplot;

fn facet_cell_main_axis_start_offset(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
) -> (f32, f32) {
    let slabs = LayoutSlabs::from_coordinated(&facet_measurement.coordinated_overflow);
    match facet_measurement.axis {
        crate::coords::FacetAxis::Column => (0.0, slabs.legend.top),
        crate::coords::FacetAxis::Row => (slabs.legend.left, 0.0),
    }
}

#[derive(Clone, Copy, Debug)]
struct FacetBandRenderOps {
    axis: FacetAxis,
    scale_name: &'static str,
    label: &'static str,
    group_prefix: &'static str,
}

impl FacetBandRenderOps {
    fn row() -> Self {
        Self {
            axis: FacetAxis::Row,
            scale_name: "row",
            label: "FacetRow",
            group_prefix: "facet_row_",
        }
    }

    fn col() -> Self {
        Self {
            axis: FacetAxis::Column,
            scale_name: "column",
            label: "FacetCol",
            group_prefix: "facet_col_",
        }
    }

    fn subplot_origin(self, position: f32, origin_offset_x: f32, origin_offset_y: f32) -> [f32; 2] {
        match self.axis {
            FacetAxis::Column => [position + origin_offset_x, origin_offset_y],
            FacetAxis::Row => [origin_offset_x, position + origin_offset_y],
        }
    }

    fn group_name(self, idx: usize, is_empty: bool) -> String {
        if is_empty {
            format!("{}{idx}_empty", self.group_prefix)
        } else {
            format!("{}{idx}", self.group_prefix)
        }
    }

    fn resolve_positions(
        self,
        configured: &ConfiguredScale,
        cell_values: &[ScalarValue],
    ) -> Result<Vec<f32>, AvengerChartError> {
        match self.axis {
            FacetAxis::Column => resolve_facet_col_positions(configured, cell_values),
            FacetAxis::Row => resolve_facet_row_positions(configured, cell_values),
        }
    }

    fn trace_main_axis_start_offset(self, origin_offset_x: f32, origin_offset_y: f32) {
        trace!(
            legend_start_left = origin_offset_x,
            legend_start_top = origin_offset_y,
            "{} render main-axis start slab offset",
            self.label
        );
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

async fn render_facet_band_common(
    ops: FacetBandRenderOps,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let mode = context.eval.facet_runtime_sizing_mode();
    match mode {
        FacetRuntimeSizingMode::CanvasFit => {
            facet_canvas_fit::render_facet_band_canvas_fit(
                ops,
                compiled_subplot,
                facet_empty_cell_policy,
                context,
            )
            .await
        }
        FacetRuntimeSizingMode::FixedSubplot { .. } => {
            facet_fixed_subplot::render_facet_band_fixed_subplot(
                ops,
                compiled_subplot,
                facet_empty_cell_policy,
                context,
            )
            .await
        }
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
    pub(crate) facet_row_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_col_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

impl<InnerC: CoordinateSystem> Default for Facet<InnerC> {
    fn default() -> Self {
        Self::new()
    }
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
            facet_row_slot_sharing: None,
            facet_col_slot_sharing: None,
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

    /// Configure row with facet options (e.g., title, slot sharing)
    pub fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        s.facet_row_title = cfg.title;
        s.facet_row_slot_sharing = cfg.slot_sharing;
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

    /// Configure col with facet options (e.g., title, slot sharing, position)
    pub fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.column(value);
        let cfg = f(FacetColChannelConfig::default());
        s.facet_col_title = cfg.title;
        s.facet_col_slot_sharing = cfg.slot_sharing;
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

/// Compiled facet mark specialized for the FacetRow outer coordinate system.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
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
    pub fn facet_slot_sharing(&self) -> Option<ScaleSharing> {
        self.facet_slot_sharing
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
            facet_slot_sharing: self.facet_row_slot_sharing,
            facet_position: self.facet_row_position.clone(),
            facet_empty_cell_policy: self.facet_row_empty_cell_policy.unwrap_or_default(),
        }))
    }
}

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
    /// Uses the coordinate-system measurement from RenderContext (computed by FacetRow)
    /// and the adjusted row scale to resolve deterministic band positions.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        render_facet_band_common(
            FacetBandRenderOps::row(),
            &self.compiled_subplot,
            self.facet_empty_cell_policy,
            context,
        )
        .await
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == RowDimensionConfig::channel_name() {
            // Use band scale for row faceting regardless of domain type (categorical input expected)
            Some(Box::new(crate::scales::spec::Band))
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

/// Compiled facet mark specialized for the FacetCol outer coordinate system.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetCol {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_slot_sharing: Option<ScaleSharing>,
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
    pub fn facet_slot_sharing(&self) -> Option<ScaleSharing> {
        self.facet_slot_sharing
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
            facet_slot_sharing: self.facet_col_slot_sharing,
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

    /// Render faceted column layout
    ///
    /// Uses the coordinate-system measurement from RenderContext (computed by FacetColumn)
    /// and the already-adjusted column scale to resolve deterministic band positions,
    /// then renders each subplot using its cached measurement.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        render_facet_band_common(
            FacetBandRenderOps::col(),
            &self.compiled_subplot,
            self.facet_empty_cell_policy,
            context,
        )
        .await
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == ColumnDimensionConfig::channel_name() {
            // Use band scale for column faceting regardless of domain type (categorical input expected)
            Some(Box::new(crate::scales::spec::Band))
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

fn resolve_facet_band_positions(
    configured: &avenger_scales::scales::ConfiguredScale,
    cell_values: &[ScalarValue],
    axis_label: &str,
) -> Result<Vec<f32>, AvengerChartError> {
    let bands: Vec<_> = BandPositionIterator::from_configured_scale(configured)?.collect();
    if bands.len() != cell_values.len() {
        return Err(AvengerChartError::InternalError(format!(
            "{axis_label} render: band positions length {} did not match facet cell count {}",
            bands.len(),
            cell_values.len()
        )));
    }

    if !bands
        .iter()
        .zip(cell_values.iter())
        .all(|(band, value)| band.value == *value)
    {
        return Err(AvengerChartError::InternalError(format!(
            "{axis_label} render: band position order did not align with facet cell order"
        )));
    }
    Ok(bands.into_iter().map(|band| band.start()).collect())
}

fn resolve_facet_col_positions(
    configured: &avenger_scales::scales::ConfiguredScale,
    cell_values: &[ScalarValue],
) -> Result<Vec<f32>, AvengerChartError> {
    resolve_facet_band_positions(configured, cell_values, "FacetCol")
}

fn resolve_facet_row_positions(
    configured: &avenger_scales::scales::ConfiguredScale,
    cell_values: &[ScalarValue],
) -> Result<Vec<f32>, AvengerChartError> {
    resolve_facet_band_positions(configured, cell_values, "FacetRow")
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scales::scales::band::BandScale;

    fn make_band_scale(range: (f32, f32)) -> avenger_scales::scales::ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ])
        .unwrap();
        BandScale::configured(domain, range)
    }

    #[test]
    fn resolve_facet_band_positions_returns_aligned_band_positions() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ];

        let positions = resolve_facet_band_positions(&scale, &cell_values, "FacetCol").unwrap();
        assert_eq!(positions.len(), 2);
        assert!(positions[0] < positions[1]);
    }

    #[test]
    fn resolve_facet_band_positions_errors_on_count_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
            ScalarValue::Utf8(Some("c".to_string())),
        ];

        let err = resolve_facet_band_positions(&scale, &cell_values, "FacetCol").unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band positions length"));
        assert!(message.contains("facet cell count"));
    }

    #[test]
    fn resolve_facet_band_positions_errors_on_order_mismatch() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("b".to_string())),
            ScalarValue::Utf8(Some("a".to_string())),
        ];

        let err = resolve_facet_band_positions(&scale, &cell_values, "FacetCol").unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("band position order"));
        assert!(message.contains("facet cell order"));
    }

    #[test]
    fn resolve_facet_row_and_col_wrappers_delegate_to_shared_resolver() {
        let scale = make_band_scale((0.0, 100.0));
        let cell_values = vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ];
        let col = resolve_facet_col_positions(&scale, &cell_values).unwrap();
        let row = resolve_facet_row_positions(&scale, &cell_values).unwrap();
        assert_eq!(col, row);
    }

    #[test]
    fn facet_band_render_ops_origin_mapping_row_vs_col() {
        let row_ops = FacetBandRenderOps::row();
        let col_ops = FacetBandRenderOps::col();
        let position = 25.0;
        let origin_offset_x = 10.0;
        let origin_offset_y = 5.0;

        assert_eq!(
            row_ops.subplot_origin(position, origin_offset_x, origin_offset_y),
            [10.0, 30.0]
        );
        assert_eq!(
            col_ops.subplot_origin(position, origin_offset_x, origin_offset_y),
            [35.0, 5.0]
        );
    }

    #[test]
    fn facet_band_render_ops_group_name_prefixes_row_vs_col() {
        let row_ops = FacetBandRenderOps::row();
        let col_ops = FacetBandRenderOps::col();

        assert_eq!(row_ops.group_name(3, false), "facet_row_3");
        assert_eq!(row_ops.group_name(3, true), "facet_row_3_empty");
        assert_eq!(col_ops.group_name(4, false), "facet_col_4");
        assert_eq!(col_ops.group_name(4, true), "facet_col_4_empty");
    }
}
