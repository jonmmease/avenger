use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::coord::{FacetCol, FacetRow};
use crate::facet::dimension_config::{ColDimensionConfig, FacetDimensionConfig, RowDimensionConfig};
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Facet mark for FacetRow or FacetCol outer coordinate system.
/// Renders a provided inner plot for each band value in the facet channel.
#[derive(Clone)]
pub struct Facet<InnerC: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) subplot: Option<Plot<InnerC>>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
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
        s.state.data = s.state.data.with_channel_value(RowDimensionConfig::channel_name(), value.into());
        s
    }

    /// Configure row with facet options (e.g., title, spacing)
    pub fn row_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetRowChannelConfig) -> FacetRowChannelConfig,
    {
        let mut s = self.row(value);
        let cfg = f(FacetRowChannelConfig::default());
        s.facet_row_title = cfg.title;
        s.facet_spacing = cfg.spacing;
        s
    }

    /// Set the faceting channel for columns
    pub fn col<V: Into<ChannelValue>>(self, value: V) -> Self {
        let mut s = self;
        s.state.data = s.state.data.with_channel_value(ColDimensionConfig::channel_name(), value.into());
        s
    }

    /// Configure col with facet options (e.g., title, spacing)
    pub fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.col(value);
        let cfg = f(FacetColChannelConfig::default());
        s.facet_col_title = cfg.title;
        s.facet_spacing = cfg.spacing;
        s
    }

    /// Provide a subplot Plot<InnerC>
    pub fn subplot(mut self, plot: Plot<InnerC>) -> Self {
        self.subplot = Some(plot);
        self
    }
}

/// Compiled facet mark specialized for FacetRow outer coords
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
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
            let plot_clone = plot_ref.clone();
            // Plot<InnerC> implements Clone; explicitly call Clone::clone
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };
        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_row_title.clone(),
            facet_spacing: self.facet_spacing,
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

    /// Evaluate faceted row layout using a two-pass rendering algorithm
    ///
    /// Delegates to the generic `evaluate_facet` helper with row-specific orientation closures.
    async fn evaluate_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::evaluate_facet;

        evaluate_facet::<RowDimensionConfig>(
            &self.compiled_subplot,
            &self.state,
            self.facet_title.clone(),
            self.facet_spacing,
            context,
            // Row: height varies with band size, width is fixed
            |band_height, ctx| (ctx.plot_width, band_height),
            // Row: translate vertically
            |y_pos| [0.0, y_pos],
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
        if channel == RowDimensionConfig::channel_name() && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Initial padding_inner_px of 0 - will be dynamically measured and rebuilt
            // during evaluate_from_data based on actual subplot overflow
            options.insert("padding_inner_px".to_string(), lit(0.0f32));
        }

        options
    }
}

// ============================================================================
// FacetCol Implementation
// ============================================================================

/// Compiled facet mark specialized for FacetCol outer coords
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetCol {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
}

#[async_trait::async_trait]
impl<InnerC: CoordinateSystem + Clone> Mark<FacetCol> for Facet<InnerC> {
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
            let plot_clone = plot_ref.clone();
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };
        Ok(Arc::new(CompiledFacetCol {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_col_title.clone(),
            facet_spacing: self.facet_spacing,
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
            name: ColDimensionConfig::channel_name(),
            required: true,
            default_value: None,
            allow_column_ref: true,
        }]
    }

    /// Evaluate faceted column layout using a two-pass rendering algorithm
    ///
    /// Delegates to the generic `evaluate_facet` helper with column-specific orientation closures.
    async fn evaluate_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::evaluate_facet;

        evaluate_facet::<ColDimensionConfig>(
            &self.compiled_subplot,
            &self.state,
            self.facet_title.clone(),
            self.facet_spacing,
            context,
            // Column: width varies with band size, height is fixed
            |band_width, ctx| (band_width, ctx.plot_height),
            // Column: translate horizontally
            |x_pos| [x_pos, 0.0],
        )
        .await
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == ColDimensionConfig::channel_name() {
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
        if channel == ColDimensionConfig::channel_name() && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Initial padding_inner_px of 0 - will be dynamically measured and rebuilt
            // during evaluate_from_data based on actual subplot overflow
            options.insert("padding_inner_px".to_string(), lit(0.0f32));
        }

        options
    }
}

