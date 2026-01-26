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

/// Trait for accessing common fields from compiled facet marks.
/// This enables generic code to work with both CompiledFacetRow and CompiledFacetCol.
pub trait CompiledFacetSource {
    /// Get the compiled subplot
    fn compiled_subplot(&self) -> &Arc<CompiledPlot>;
    /// Get the compiled mark state
    fn compiled_state(&self) -> &CompiledMarkState;
    /// Get the optional facet title
    fn facet_title(&self) -> Option<&str>;
    /// Get the optional facet spacing
    fn facet_spacing(&self) -> Option<f32>;
    /// Get the optional facet scale sharing mode
    fn facet_scale_sharing(&self) -> Option<ScaleSharing>;
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

impl CompiledFacetSource for CompiledFacetRow {
    fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }
    fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }
    fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    fn facet_spacing(&self) -> Option<f32> {
        self.facet_spacing
    }
    fn facet_scale_sharing(&self) -> Option<ScaleSharing> {
        self.facet_scale_sharing
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

    /// Measure faceted row layout - STUBBED
    ///
    /// Currently returns empty results. Full facet evaluation will be rebuilt
    /// on the EvaluatedFacetTree abstraction.
    async fn measure_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Box<dyn crate::marks::MarkMeasurement>, AvengerChartError> {
        // STUBBED: Return empty measurement
        // TODO: Implement proper facet measurement using FacetRowMeasurement
        Ok(Box::new(crate::marks::EmptyMarkMeasurement))
    }

    /// Render faceted row layout - STUBBED
    ///
    /// Currently returns empty results. Full facet evaluation will be rebuilt
    /// on the EvaluatedFacetTree abstraction.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
        _measurement: &dyn crate::marks::MarkMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // STUBBED: Return empty scene marks
        // TODO: Implement proper facet rendering using cached measurement data
        Ok(Vec::new())
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
}

impl CompiledFacetSource for CompiledFacetCol {
    fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }
    fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }
    fn facet_title(&self) -> Option<&str> {
        self.facet_title.as_deref()
    }
    fn facet_spacing(&self) -> Option<f32> {
        self.facet_spacing
    }
    fn facet_scale_sharing(&self) -> Option<ScaleSharing> {
        self.facet_scale_sharing
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

    /// Measure faceted column layout
    ///
    /// For each column value:
    /// 1. Filter the data to that column's subset
    /// 2. Measure the subplot with filtered data
    /// 3. Cache the measurement for the render pass
    async fn measure_from_data(
        &self,
        data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<Box<dyn crate::marks::MarkMeasurement>, AvengerChartError> {
        use avenger_scales::scales::band::bandwidth;
        use crate::facet::marks::facet_evaluation::batch_to_dataframe;
        use crate::plot::compiled::scale_provider::PrebuiltScaleProvider;
        use datafusion::common::ScalarValue;

        // Get the column scale for layout calculations
        let column_scale = context
            .scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        // Get bandwidth (subplot width) from the band scale
        let subplot_width = bandwidth(&column_scale.configured().config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        // DEBUG: Print scale config info
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            use avenger_scales::scales::band::step;
            let step_val = step(&column_scale.configured().config).unwrap_or(-1.0);
            let (range_min, range_max) = column_scale.configured().config.numeric_interval_range().unwrap_or((0.0, 0.0));
            eprintln!(
                "FacetCol measure: range=[{:.1}, {:.1}] bandwidth={:.1} step={:.1}",
                range_min, range_max, subplot_width, step_val
            );
        }

        // Get column values from the facet tree
        let facet_tree = &context.facet_tree;
        let root = facet_tree.root().ok_or_else(|| {
            AvengerChartError::InternalError("No facet tree root found".into())
        })?;

        // Collect column values
        let column_values: Vec<ScalarValue> = root.values().cloned().collect();

        if column_values.is_empty() {
            return Ok(Box::new(crate::marks::FacetColMeasurement {
                column_values: Vec::new(),
                column_positions: Vec::new(),
                subplot_width,
                subplot_height: context.plot_height,
                subplot_measurements: Vec::new(),
                data_overrides: Vec::new(),
            }));
        }

        // Get positions from the band scale
        let domain_array = ScalarValue::iter_to_array(column_values.iter().cloned()).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to create domain array: {}", e))
        })?;
        let positions = column_scale
            .configured()
            .scale_impl
            .scale_to_numeric(&column_scale.configured().config, &domain_array)
            .map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to scale positions: {}", e))
            })?;
        let column_positions: Vec<f32> = positions.as_vec(column_values.len(), None);

        // Convert RecordBatch to DataFrame for filtering
        let data_batch = data.ok_or_else(|| {
            AvengerChartError::InternalError("Facet mark requires data".into())
        })?;
        let base_df = batch_to_dataframe(data_batch, &context.session_context)?;

        // For each column value, filter data and measure subplot
        let mut subplot_measurements = Vec::with_capacity(column_values.len());
        let mut data_overrides = Vec::with_capacity(column_values.len());

        // For shared scales: build scales once from the FULL data, then reuse for all subplots
        // Use the facet's full (unfiltered) data to derive scale domains
        let shared_scales = self
            .compiled_subplot
            .build_scales_for_dataframe(
                &base_df,
                subplot_width,
                context.plot_height,
                &context.session_context,
                &context.params,
            )
            .await?;

        // Now use PrebuiltScaleProvider with the shared scales for all subplots
        let scale_provider = PrebuiltScaleProvider { scales: shared_scales };

        for (idx, value) in column_values.iter().enumerate() {
            // Get filter predicate for this column value
            // sharing_level=0 means use Free (full filter)
            let predicate = facet_tree.cell_predicate(&[value.clone()], 0);

            // Filter the data
            let filtered_df = if let Some(pred) = predicate {
                base_df.clone().filter(pred).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to filter data for column {:?}: {}",
                        value, e
                    ))
                })?
            } else {
                base_df.clone()
            };

            // Measure the subplot with filtered data
            let measurement = self
                .compiled_subplot
                .measure_plot_components(
                    subplot_width,
                    context.plot_height,
                    &context.session_context,
                    &context.params,
                    &scale_provider,
                    Some(&filtered_df),
                    true, // dimensions_are_plot_area
                    context.facet_tree.clone(),
                )
                .await?;

            subplot_measurements.push(measurement);
            data_overrides.push(filtered_df);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol: column[{}]={:?} position={:.1} width={:.1}",
                    idx, value, column_positions[idx], subplot_width
                );
            }
        }

        Ok(Box::new(crate::marks::FacetColMeasurement {
            column_values,
            column_positions,
            subplot_width,
            subplot_height: context.plot_height,
            subplot_measurements,
            data_overrides,
        }))
    }

    /// Render faceted column layout
    ///
    /// Uses cached measurements from measure_from_data to render each subplot
    /// and position them using SceneGroups.
    async fn render_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
        measurement: &dyn crate::marks::MarkMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_scenegraph::marks::group::{Clip, SceneGroup};

        // Downcast measurement to FacetColMeasurement
        let facet_measurement = measurement
            .as_any()
            .downcast_ref::<crate::marks::FacetColMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Expected FacetColMeasurement in render_from_data".into(),
                )
            })?;

        let mut scene_marks = Vec::with_capacity(facet_measurement.column_values.len());

        // Render each subplot using its cached measurement
        for (idx, (measurement, data_override)) in facet_measurement
            .subplot_measurements
            .iter()
            .zip(facet_measurement.data_overrides.iter())
            .enumerate()
        {
            let position = facet_measurement.column_positions[idx];

            // Build subplot components using cached measurement
            // Pass the cell position for axis visibility decisions
            let facet_position = [idx];
            let components = self
                .compiled_subplot
                .build_plot_components(
                    &context.session_context,
                    measurement,
                    Some(data_override),
                    true, // dimensions_are_plot_area
                    context.facet_tree.clone(),
                    Some(&facet_position),
                )
                .await?;

            // Collect all marks from the subplot
            let mut subplot_marks = Vec::new();
            subplot_marks.extend(components.guide_marks);
            subplot_marks.extend(components.data_marks);
            subplot_marks.extend(components.legend_marks);
            subplot_marks.extend(components.title_marks);
            subplot_marks.extend(components.subtitle_marks);

            // Create SceneGroup with origin for positioning
            let subplot_group = SceneGroup {
                name: format!("facet_col_{}", idx),
                origin: [position, 0.0],
                clip: Clip::None, // Let subplot handle its own clipping
                marks: subplot_marks,
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
            };

            scene_marks.push(SceneMark::Group(subplot_group));

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol render: column[{}] at x={:.1}",
                    idx, position
                );
            }
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
