use crate::channel::config_traits::ScaleSharing;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::coord::{FacetColumn, FacetRow};
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
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
    pub(crate) facet_row_scale_sharing: Option<ScaleSharing>,
    pub(crate) facet_col_scale_sharing: Option<ScaleSharing>,
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

    /// Configure col with facet options (e.g., title, spacing, scale_sharing)
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

        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_row_title.clone(),
            facet_spacing: self.facet_spacing,
            facet_scale_sharing: self.facet_row_scale_sharing,
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

    /// Evaluate faceted row layout using a two-pass rendering algorithm
    ///
    /// Delegates to the generic `evaluate_facet` helper with row-specific orientation closures.
    async fn evaluate_from_data(
        &self,
        data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::{batch_to_dataframe, evaluate_facet};

        // Convert RecordBatch to DataFrame if provided (for nested facets)
        let data_override = if let Some(batch) = data {
            Some(batch_to_dataframe(batch, &context.session_context)?)
        } else {
            None
        };

        evaluate_facet::<RowDimensionConfig>(
            coord.as_ref(),
            &self.compiled_subplot,
            &self.state,
            data_override.as_ref(),
            self.facet_title.clone(),
            self.facet_spacing,
            context,
            // Row: height varies with band size, width is fixed
            // Round bandwidth to integer for pixel-aligned subplot dimensions
            |band_height: f32, ctx: &RenderContext| {
                let rounded = band_height.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRow subplot_dims: band_height={:.3} -> rounded={:.3} ctx.plot_width={:.3}",
                        band_height, rounded, ctx.plot_width
                    );
                }
                (ctx.plot_width, rounded)
            },
            // Row: translate vertically
            // Round positions to integers for pixel alignment
            |y_pos: f32| {
                let rounded = y_pos.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRow group_origin: y_pos={:.3} -> rounded={:.3}",
                        y_pos, rounded
                    );
                }
                [0.0, rounded]
            },
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

        Ok(Arc::new(CompiledFacetCol {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_col_title.clone(),
            facet_spacing: self.facet_spacing,
            facet_scale_sharing: self.facet_col_scale_sharing,
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

    /// Evaluate faceted column layout using a two-pass rendering algorithm
    ///
    /// Delegates to the generic `evaluate_facet` helper with column-specific orientation closures.
    async fn evaluate_from_data(
        &self,
        data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::{batch_to_dataframe, evaluate_facet};

        // Convert RecordBatch to DataFrame if provided (for nested facets)
        let data_override = if let Some(batch) = data {
            Some(batch_to_dataframe(batch, &context.session_context)?)
        } else {
            None
        };

        evaluate_facet::<ColumnDimensionConfig>(
            coord.as_ref(),
            &self.compiled_subplot,
            &self.state,
            data_override.as_ref(),
            self.facet_title.clone(),
            self.facet_spacing,
            context,
            // Column: width varies with band size, height is fixed
            // Round bandwidth to integer for pixel-aligned subplot dimensions
            |band_width: f32, ctx: &RenderContext| {
                let rounded = band_width.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol subplot_dims: band_width={:.3} -> rounded={:.3}",
                        band_width, rounded
                    );
                }
                (rounded, ctx.plot_height)
            },
            // Column: translate horizontally
            // Round positions to integers for pixel alignment
            |x_pos: f32| {
                let rounded = x_pos.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol group_origin: x_pos={:.3} -> rounded={:.3}",
                        x_pos, rounded
                    );
                }
                [rounded, 0.0]
            },
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
        if channel == ColumnDimensionConfig::channel_name() && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            // Initial padding_inner_px of 0 - will be dynamically measured and rebuilt
            // during evaluate_from_data based on actual subplot overflow
            options.insert("padding_inner_px".to_string(), lit(0.0f32));
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
    use crate::cartesian::axis::AxisPosition;
    use crate::cartesian::guide::CartesianGuide;
    use crate::guide::CompiledGuide;

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
                AxisPosition::Left => 0.0,  // Push to left, empty space at right
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

