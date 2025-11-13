use crate::channel::config_traits::ScaleSharing;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::context::FacetContext;
use crate::facet::coord::{FacetColumn, FacetGrid, FacetRow};
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::keys::FacetKeyExtractor;
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::facet::subplot_iterator::{SubplotIteration, SubplotIterator};
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use crate::serialization::SerializableScalar;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{FromInto, serde_as};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Helper functions for serializing/deserializing Arc<Mutex<Option<...>>>
pub(crate) fn serialize_cached_overflow<S>(
    value: &Arc<std::sync::Mutex<Option<crate::guide::OverflowSpaceRequirement>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let guard = value.lock().unwrap();
    guard.serialize(serializer)
}

pub(crate) fn deserialize_cached_overflow<'de, D>(
    deserializer: D,
) -> Result<Arc<std::sync::Mutex<Option<crate::guide::OverflowSpaceRequirement>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::deserialize(deserializer)?;
    Ok(Arc::new(std::sync::Mutex::new(value)))
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
        s.state.data = s
            .state
            .data
            .with_channel_value(RowDimensionConfig::channel_name(), value.into());
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
    pub fn column<V: Into<ChannelValue>>(self, value: V) -> Self {
        let mut s = self;
        s.state.data = s
            .state
            .data
            .with_channel_value(ColumnDimensionConfig::channel_name(), value.into());
        s
    }

    /// Configure col with facet options (e.g., title, spacing)
    pub fn col_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(FacetColChannelConfig) -> FacetColChannelConfig,
    {
        let mut s = self.column(value);
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
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    #[serde_as(as = "Vec<FromInto<SerializableScalar>>")]
    pub(crate) distinct_keys: Vec<ScalarValue>,
    /// Cached Pass 1 maximum overflow across all subplots
    /// Set during evaluation for use by guide
    #[serde(
        serialize_with = "serialize_cached_overflow",
        deserialize_with = "deserialize_cached_overflow"
    )]
    pub(crate) cached_edge_overflow:
        std::sync::Arc<std::sync::Mutex<Option<crate::guide::OverflowSpaceRequirement>>>,
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
        let distinct_keys = {
            let df = compiled_state
                .data
                .dataframe_with_context(session_context)
                .ok_or_else(|| AvengerChartError::InternalError("FacetRow requires data".into()))?;
            let expr = compiled_state
                .data
                .channels()
                .get(RowDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(session_context))
                .ok_or_else(|| {
                    AvengerChartError::InternalError("Facet 'row' channel not found".into())
                })?;
            FacetKeyExtractor::extract_keys(&df, &expr).await?
        };

        Ok(Arc::new(CompiledFacetRow {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_row_title.clone(),
            facet_spacing: self.facet_spacing,
            distinct_keys,
            cached_edge_overflow: std::sync::Arc::new(std::sync::Mutex::new(None)),
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
            if self.distinct_keys.is_empty() {
                None
            } else {
                Some(&self.distinct_keys)
            },
            // Row: height varies with band size, width is fixed
            // Round bandwidth to integer for pixel-aligned subplot dimensions
            |band_height, ctx| {
                let rounded = band_height.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRow subplot_dims: band_height={:.3} -> rounded={:.3}",
                        band_height, rounded
                    );
                }
                (ctx.plot_width, rounded)
            },
            // Row: translate vertically
            // Round positions to integers for pixel alignment
            |y_pos| {
                let rounded = y_pos.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRow group_origin: y_pos={:.3} -> rounded={:.3}",
                        y_pos, rounded
                    );
                }
                [0.0, rounded]
            },
            &self.cached_edge_overflow,
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
    #[serde_as(as = "Vec<FromInto<SerializableScalar>>")]
    pub(crate) distinct_keys: Vec<ScalarValue>,
    /// Cached Pass 1 maximum overflow across all subplots
    /// Set during evaluation for use by guide
    #[serde(
        serialize_with = "serialize_cached_overflow",
        deserialize_with = "deserialize_cached_overflow"
    )]
    pub(crate) cached_edge_overflow:
        std::sync::Arc<std::sync::Mutex<Option<crate::guide::OverflowSpaceRequirement>>>,
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
            let plot_clone = plot_ref.clone();
            let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
            Arc::new(plot_owned.compile(session_context).await?)
        };
        let distinct_keys = {
            let df = compiled_state
                .data
                .dataframe_with_context(session_context)
                .ok_or_else(|| {
                    AvengerChartError::InternalError("FacetColumn requires data".into())
                })?;
            let expr = compiled_state
                .data
                .channels()
                .get(ColumnDimensionConfig::channel_name())
                .and_then(|cv| cv.expr(session_context))
                .ok_or_else(|| {
                    AvengerChartError::InternalError("Facet 'column' channel not found".into())
                })?;
            FacetKeyExtractor::extract_keys(&df, &expr).await?
        };

        Ok(Arc::new(CompiledFacetCol {
            state: compiled_state,
            compiled_subplot,
            facet_title: self.facet_col_title.clone(),
            facet_spacing: self.facet_spacing,
            distinct_keys,
            cached_edge_overflow: std::sync::Arc::new(std::sync::Mutex::new(None)),
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
            name: ColumnDimensionConfig::channel_name(),
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

        evaluate_facet::<ColumnDimensionConfig>(
            &self.compiled_subplot,
            &self.state,
            self.facet_title.clone(),
            self.facet_spacing,
            context,
            if self.distinct_keys.is_empty() {
                None
            } else {
                Some(&self.distinct_keys)
            },
            // Column: width varies with band size, height is fixed
            // Round bandwidth to integer for pixel-aligned subplot dimensions
            |band_width, ctx| {
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
            |x_pos| {
                let rounded = x_pos.round();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol group_origin: x_pos={:.3} -> rounded={:.3}",
                        x_pos, rounded
                    );
                }
                [rounded, 0.0]
            },
            &self.cached_edge_overflow,
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
// GridFacet Implementation
// ============================================================================

/// Merge row and column SubplotIteration contexts into a unified GridFacet context
///
/// This helper takes FacetContext information from both row and column subplot iterations
/// and combines them into a single FacetContext suitable for GridFacet subplots.
///
/// The merged context contains:
/// - `position`: (row_iteration.index, col_iteration.index)
/// - `grid_dimensions`: (num_rows, num_cols)
/// - `unified_channels`: {"x", "y"} (both axes unified in GridFacet)
/// - `scale_sharing`: from either iteration (should be identical)
pub fn merge_grid_facet_contexts(
    row_iteration: &SubplotIteration,
    col_iteration: &SubplotIteration,
    num_rows: usize,
    num_cols: usize,
) -> IndexMap<String, ScalarValue> {
    let mut params = row_iteration.params.clone();

    // Extract existing contexts (if any)
    let row_ctx = FacetContext::from_params(&row_iteration.params);
    let col_ctx = FacetContext::from_params(&col_iteration.params);

    // Get scale_sharing from either context (should be identical)
    let scale_sharing = row_ctx
        .as_ref()
        .map(|c| c.scale_sharing.clone())
        .or_else(|| col_ctx.as_ref().map(|c| c.scale_sharing.clone()))
        .unwrap_or_default();

    // Build unified GridFacet context
    let mut unified_channels = HashSet::new();
    unified_channels.insert("x".to_string());
    unified_channels.insert("y".to_string());

    let grid_ctx = FacetContext {
        position: (row_iteration.index, col_iteration.index),
        grid_dimensions: (num_rows, num_cols),
        unified_channels,
        scale_sharing,
    };

    // Merge into params
    params.extend(grid_ctx.to_params());
    params
}

/// Compiled facet mark specialized for GridFacet outer coords (2D row×col grid)
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetGrid {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) row_title: Option<String>,
    pub(crate) col_title: Option<String>,
    #[serde_as(as = "Vec<FromInto<SerializableScalar>>")]
    pub(crate) row_keys: Vec<ScalarValue>,
    #[serde_as(as = "Vec<FromInto<SerializableScalar>>")]
    pub(crate) col_keys: Vec<ScalarValue>,
    pub(crate) facet_spacing: Option<f32>,
}

#[async_trait::async_trait]
impl<InnerC: CoordinateSystem + Clone> Mark<FacetGrid> for Facet<InnerC> {
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
        let data_df = compiled_state
            .data
            .dataframe_with_context(session_context)
            .ok_or_else(|| AvengerChartError::InternalError("FacetGrid requires data".into()))?;
        let row_expr = compiled_state
            .data
            .channels()
            .get(RowDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(session_context))
            .ok_or_else(|| {
                AvengerChartError::InternalError("FacetGrid 'row' channel not found".into())
            })?;
        let col_expr = compiled_state
            .data
            .channels()
            .get(ColumnDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(session_context))
            .ok_or_else(|| {
                AvengerChartError::InternalError("FacetGrid 'column' channel not found".into())
            })?;
        let row_keys = {
            let df = data_df.clone();
            FacetKeyExtractor::extract_keys(&df, &row_expr).await?
        };
        let col_keys = FacetKeyExtractor::extract_keys(&data_df, &col_expr).await?;

        Ok(Arc::new(CompiledFacetGrid {
            state: compiled_state,
            compiled_subplot,
            row_title: self.facet_row_title.clone(),
            col_title: self.facet_col_title.clone(),
            row_keys,
            col_keys,
            facet_spacing: self.facet_spacing,
        }))
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledFacetGrid {
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
        "facet_grid"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "row",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "column",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    /// Evaluate grid facet layout (2D row×col grid)
    async fn evaluate_from_data(
        &self,
        _data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Validate required channels with helpful error messages
        let _row_channel = self.state.data.channels().get("row").ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridFacet requires .row() channel expression.\n\
                 Example: Facet::new().row(col(\"Species\")).col(col(\"Year\")).subplot(...)"
                    .to_string(),
            )
        })?;

        let _col_channel = self.state.data.channels().get("column").ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridFacet requires .col() channel expression.\n\
                 Example: Facet::new().row(col(\"Species\")).col(col(\"Year\")).subplot(...)"
                    .to_string(),
            )
        })?;

        // Get row and col scales (may not exist for degenerate single-value cases)
        let row_scale_opt = context.scales.get("row");
        let col_scale_opt = context.scales.get("column");

        // Handle degenerate cases where a scale doesn't exist (single unique value in that dimension)
        let row_domain_vals = if let Some(row_scale) = row_scale_opt {
            match row_scale.domain_values()? {
                crate::scales::extensions::DomainValues::Discrete(vals) => vals,
                _ => {
                    return Err(AvengerChartError::InternalError(
                        "GridFacet requires discrete row scale".into(),
                    ));
                }
            }
        } else {
            self.row_keys.clone()
        };

        let col_domain_vals = if let Some(col_scale) = col_scale_opt {
            match col_scale.domain_values()? {
                crate::scales::extensions::DomainValues::Discrete(vals) => vals,
                _ => {
                    return Err(AvengerChartError::InternalError(
                        "GridFacet requires discrete col scale".into(),
                    ));
                }
            }
        } else {
            self.col_keys.clone()
        };

        // Get row and col expressions
        let row_expr = self
            .state
            .data
            .channels()
            .get("row")
            .and_then(|cv| cv.expr(&context.session_context))
            .ok_or_else(|| {
                AvengerChartError::InternalError("GridFacet 'row' channel not found".into())
            })?;
        let col_expr = self
            .state
            .data
            .channels()
            .get("column")
            .and_then(|cv| cv.expr(&context.session_context))
            .ok_or_else(|| {
                AvengerChartError::InternalError("GridFacet 'col' channel not found".into())
            })?;

        // Get DataFrame
        let df = self
            .state
            .data
            .dataframe_with_context(&context.session_context)
            .ok_or_else(|| {
                AvengerChartError::InternalError("GridFacet could not access data".into())
            })?;

        // Compute scale sharing for ALL channels used by marks (not just coord channels)
        // This ensures non-coordinate channels like fill, color, size also get per-subplot scales
        let mut scale_sharing_by_channel = std::collections::HashMap::new();

        // Collect all unique channel names from all marks
        let mut all_channels = std::collections::HashSet::new();
        for m in &self.compiled_subplot.marks {
            for ch in m.data_context().channels().keys() {
                all_channels.insert(ch.as_str());
            }
        }

        // Determine sharing mode for each channel
        for ch in all_channels {
            let mut mode = ScaleSharing::Free;
            for m in &self.compiled_subplot.marks {
                if let Some(cv) = m.data_context().channels().get(ch) {
                    if let Some(share_mode) = cv.get_share_mode() {
                        // Upgrade to more restrictive sharing
                        mode = match (mode, share_mode) {
                            (ScaleSharing::Free, new_mode) => new_mode,
                            (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                            (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                            (existing, _) => existing,
                        };
                    }
                }
            }
            scale_sharing_by_channel.insert(ch.to_string(), mode);
        }
        // Note: No normalization needed for GridFacet (has both rows and columns)

        // ====================================================================================
        // PASS 1: Measure overflow for all grid cells to calculate required subplot spacing
        // ====================================================================================

        let (num_rows, num_cols) = (row_domain_vals.len(), col_domain_vals.len());

        // Initial band dimensions (will be recalculated after measuring required spacing)
        let mut band_w = context.plot_width / num_cols.max(1) as f32;
        let mut band_h = context.plot_height / num_rows.max(1) as f32;

        // Build ScaleGrouping once for reuse across both passes
        use crate::facet::scale_grouping::ScaleGrouping;
        let scale_grouping = ScaleGrouping::build(
            &self.compiled_subplot,
            &scale_sharing_by_channel,
            &row_domain_vals,
            &col_domain_vals,
            &df,
            &row_expr,
            &col_expr,
            &context.session_context,
            &context.params,
        )
        .await?;

        // Create a 2D array to store overflow measurements for each grid cell
        // overflow_grid[row_idx][col_idx] = OverflowSpaceRequirement
        let mut overflow_grid: Vec<Vec<crate::guide::OverflowSpaceRequirement>> =
            vec![vec![crate::guide::OverflowSpaceRequirement::default(); num_cols]; num_rows];

        // Measure overflow for each grid cell using nested SubplotIterators
        use crate::facet::dimension_config::{ColumnDimensionConfig, RowDimensionConfig};

        let row_iter_pass1 = SubplotIterator::<RowDimensionConfig>::new(
            row_domain_vals.clone(),
            context.params.clone(),
            scale_sharing_by_channel.clone(),
        );

        for row_iteration in row_iter_pass1 {
            let col_iter_pass1 = SubplotIterator::<ColumnDimensionConfig>::new(
                col_domain_vals.clone(),
                context.params.clone(),
                scale_sharing_by_channel.clone(),
            );

            for col_iteration in col_iter_pass1 {
                // Merge row and col contexts into unified GridFacet context
                let merged_params =
                    merge_grid_facet_contexts(&row_iteration, &col_iteration, num_rows, num_cols);

                // Filter to rows matching both row AND col values
                let filter_df = df
                    .clone()
                    .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
                    .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

                // Build scales for this subplot position using ScaleGrouping
                let inner_scales = scale_grouping
                    .build_scales_for_position(
                        &self.compiled_subplot,
                        row_iteration.index,
                        col_iteration.index,
                        band_w,
                        band_h,
                        &context.session_context,
                        &merged_params,
                    )
                    .await?;

                // Measure guide AND legend overflow for this cell using evaluate_in_canvas with Measure mode
                let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
                    scales: inner_scales.clone(),
                };

                let components = self
                    .compiled_subplot
                    .build_plot_components(
                        band_w,
                        band_h,
                        &context.session_context,
                        &merged_params,
                        &scale_provider,
                        crate::plot::compiled::EvaluationMode::Measure,
                        Some(&filter_df),
                        true, // Plot area mode: both dimensions are already plot area size
                    )
                    .await?;

                // Extract overflow from the returned components
                let overflow = components.overflow.unwrap_or_default();
                overflow_grid[row_iteration.index][col_iteration.index] = overflow;
            }
        }

        // Calculate required vertical spacing (between rows)
        let mut max_vertical_gap = 0.0_f32;
        for row_idx in 0..num_rows.saturating_sub(1) {
            for col_idx in 0..num_cols {
                let gap = RowDimensionConfig::calculate_adjacent_overflow(
                    &overflow_grid[row_idx][col_idx],
                    &overflow_grid[row_idx + 1][col_idx],
                );
                max_vertical_gap = max_vertical_gap.max(gap);
            }
        }

        // Calculate required horizontal spacing (between columns)
        let mut max_horizontal_gap = 0.0_f32;
        for row_idx in 0..num_rows {
            for col_idx in 0..num_cols.saturating_sub(1) {
                let gap = ColumnDimensionConfig::calculate_adjacent_overflow(
                    &overflow_grid[row_idx][col_idx],
                    &overflow_grid[row_idx][col_idx + 1],
                );
                max_horizontal_gap = max_horizontal_gap.max(gap);
            }
        }

        // Add configured facet_spacing to both dimensions
        const DEFAULT_FACET_SPACING: f32 = 3.0;
        let spacing = self.facet_spacing.unwrap_or_else(|| {
            let facet_ctx = context
                .theme
                .facet_context_with_params(context.params.clone());
            context
                .theme
                .query(&facet_ctx, "spacing")
                .and_then(|v| v.as_number())
                .map(|n| n as f32)
                .unwrap_or(DEFAULT_FACET_SPACING)
        });

        max_vertical_gap += spacing;
        max_horizontal_gap += spacing;

        // Debug: log measured spacing and gaps
        tracing::debug!(
            num_rows = num_rows,
            num_cols = num_cols,
            spacing = spacing,
            max_vertical_gap = max_vertical_gap,
            max_horizontal_gap = max_horizontal_gap,
            "GridFacet: computed facet spacing (including theme spacing)"
        );

        // Rebuild row and col scales with measured spacing
        let mut updated_scales = context.scales.clone();

        if max_vertical_gap > 0.0 && row_scale_opt.is_some() {
            use avenger_scales::scalar::Scalar;
            let row_scale = row_scale_opt.unwrap();
            let mut new_config = row_scale.configured().config.clone();
            new_config.options.insert(
                "padding_inner_px".to_string(),
                Scalar::from_f32(max_vertical_gap),
            );
            let new_configured = avenger_scales::scales::ConfiguredScale {
                scale_impl: row_scale.configured().scale_impl.clone(),
                config: new_config,
            };
            updated_scales.insert(
                "row".to_string(),
                crate::scales::ConfiguredScaleWithSpec::new(
                    row_scale.spec().clone(),
                    new_configured,
                ),
            );
            tracing::debug!(
                padding_inner_px = max_vertical_gap,
                "GridFacet: updated row scale padding_inner_px"
            );
        }

        if max_horizontal_gap > 0.0 && col_scale_opt.is_some() {
            use avenger_scales::scalar::Scalar;
            let col_scale = col_scale_opt.unwrap();
            let mut new_config = col_scale.configured().config.clone();
            new_config.options.insert(
                "padding_inner_px".to_string(),
                Scalar::from_f32(max_horizontal_gap),
            );
            let new_configured = avenger_scales::scales::ConfiguredScale {
                scale_impl: col_scale.configured().scale_impl.clone(),
                config: new_config,
            };
            updated_scales.insert(
                "column".to_string(),
                crate::scales::ConfiguredScaleWithSpec::new(
                    col_scale.spec().clone(),
                    new_configured,
                ),
            );
            tracing::debug!(
                padding_inner_px = max_horizontal_gap,
                "GridFacet: updated col scale padding_inner_px"
            );
        }

        // Recalculate band dimensions using updated scales with spacing
        // After setting padding_inner_px, the band scale automatically reduces bandwidth
        // to make room for gaps. Extract this new bandwidth.
        if let Some(row_scale) = updated_scales.get("row") {
            if let Ok(band_iter) =
                crate::facet::band_positions::BandPositionIterator::from_scale(row_scale)
            {
                let positions: Vec<_> = band_iter.collect();
                if let Some(first) = positions.first() {
                    band_h = first.bandwidth;
                }
            }
        }

        if let Some(col_scale) = updated_scales.get("column") {
            if let Ok(band_iter) =
                crate::facet::band_positions::BandPositionIterator::from_scale(col_scale)
            {
                let positions: Vec<_> = band_iter.collect();
                if let Some(first) = positions.first() {
                    band_w = first.bandwidth;
                }
            }
        }

        // Rebuild base scales with new band dimensions
        // Note: ScaleGrouping will rebuild scales with new band dimensions in Pass 2
        // No need to rebuild base_scales here since we use build_scales_for_position()

        // ====================================================================================
        // PASS 2: Render all grid cells using measured spacing
        // ====================================================================================

        // Get band positions from updated scales for proper positioning with spacing
        use crate::facet::band_positions::BandPositionIterator;
        let row_band_iter_pass2 = if let Some(row_scale) = updated_scales.get("row") {
            Some(BandPositionIterator::from_scale(row_scale)?)
        } else {
            None
        };
        let col_band_iter_pass2 = if let Some(col_scale) = updated_scales.get("column") {
            Some(BandPositionIterator::from_scale(col_scale)?)
        } else {
            None
        };

        let mut marks: Vec<SceneMark> = Vec::new();

        // Create nested SubplotIterators for Pass 2 rendering
        let row_iter_pass2 = SubplotIterator::<RowDimensionConfig>::new(
            row_domain_vals.clone(),
            context.params.clone(),
            scale_sharing_by_channel.clone(),
        );

        // Zip row iterator with row band positions (or use fallback positions)
        let row_iter_with_bands = if let Some(band_iter) = row_band_iter_pass2 {
            row_iter_pass2.zip(band_iter).collect::<Vec<_>>()
        } else {
            // Fallback: use index-based positions without band scale
            row_iter_pass2
                .enumerate()
                .map(|(idx, iter)| {
                    let band_pos = crate::facet::band_positions::BandPosition::new(
                        iter.facet_value.clone(),
                        idx as f32 * band_h,
                        band_h,
                    );
                    (iter, band_pos)
                })
                .collect()
        };

        // Render each grid cell using nested loops
        for (row_iteration, row_band_pos) in row_iter_with_bands {
            let col_iter_pass2 = SubplotIterator::<ColumnDimensionConfig>::new(
                col_domain_vals.clone(),
                context.params.clone(),
                scale_sharing_by_channel.clone(),
            );

            // Zip col iterator with col band positions (or use fallback positions)
            let col_iter_with_bands = if let Some(_band_iter) = col_band_iter_pass2.as_ref() {
                // Need to recreate the band iterator for each row since iterators aren't Clone
                let col_scale = updated_scales.get("column").unwrap();
                let band_iter = BandPositionIterator::from_scale(col_scale)?;
                col_iter_pass2.zip(band_iter).collect::<Vec<_>>()
            } else {
                col_iter_pass2
                    .enumerate()
                    .map(|(idx, iter)| {
                        let band_pos = crate::facet::band_positions::BandPosition::new(
                            iter.facet_value.clone(),
                            idx as f32 * band_w,
                            band_w,
                        );
                        (iter, band_pos)
                    })
                    .collect()
            };

            for (col_iteration, col_band_pos) in col_iter_with_bands {
                // Merge row and col contexts into unified GridFacet context
                let merged_params =
                    merge_grid_facet_contexts(&row_iteration, &col_iteration, num_rows, num_cols);

                // Filter to rows matching both row AND col values
                let filter_df = df
                    .clone()
                    .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
                    .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

                // Check if this cell has any data
                let cell_count = filter_df.clone().count().await?;
                let cell_is_empty = cell_count == 0;

                // Build scales for this subplot position using ScaleGrouping (Pass 2 with final band dimensions)
                let inner_scales = scale_grouping
                    .build_scales_for_position(
                        &self.compiled_subplot,
                        row_iteration.index,
                        col_iteration.index,
                        band_w,
                        band_h,
                        &context.session_context,
                        &merged_params,
                    )
                    .await?;

                // Calculate subplot position from band positions
                let x_offset = col_band_pos.start();
                let y_offset = row_band_pos.start();

                // Render subplot using evaluate_in_canvas with Render mode
                // This creates all marks including legends, titles, and subtitles
                // For empty cells, use the filtered dataframe which will result in no data marks
                let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
                    scales: inner_scales.clone(),
                };

                let components = self
                    .compiled_subplot
                    .build_plot_components(
                        band_w,
                        band_h,
                        &context.session_context,
                        &merged_params,
                        &scale_provider,
                        crate::plot::compiled::EvaluationMode::Render,
                        Some(&filter_df),
                        true, // Plot area mode: both dimensions are already plot area size
                    )
                    .await?;

                // Wrap data marks in a clipped group translated to grid cell position
                use avenger_scenegraph::marks::group::SceneGroup;
                if !components.data_marks.is_empty() {
                    let data_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.data_marks,
                        clip: components.clip,
                        zindex: Some(0),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(data_group));
                }

                // Wrap guide marks (axes) in a non-clipped translated group
                if !components.guide_marks.is_empty() {
                    let guide_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.guide_marks,
                        clip: avenger_scenegraph::marks::group::Clip::None,
                        zindex: Some(1),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(guide_group));
                }

                // Wrap legend marks in a non-clipped translated group
                // Skip legends for empty cells to avoid showing misleading legend entries
                if !cell_is_empty && !components.legend_marks.is_empty() {
                    let legend_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.legend_marks,
                        clip: avenger_scenegraph::marks::group::Clip::None,
                        zindex: Some(2),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(legend_group));
                }

                // Wrap title marks in a non-clipped translated group
                if !components.title_marks.is_empty() {
                    let title_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.title_marks,
                        clip: avenger_scenegraph::marks::group::Clip::None,
                        zindex: Some(3),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(title_group));
                }

                // Wrap subtitle marks in a non-clipped translated group
                if !components.subtitle_marks.is_empty() {
                    let subtitle_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.subtitle_marks,
                        clip: avenger_scenegraph::marks::group::Clip::None,
                        zindex: Some(4),
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(subtitle_group));
                }

                // Wrap debug marks in a non-clipped translated group
                // Debug marks are in absolute canvas coordinates relative to the subplot,
                // so we need to translate them to the correct grid cell position
                if !components.debug_marks.is_empty() {
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        // Use Pass 1 overflow measurement for debug visualization
                        // This matches the overflow used to compute the final band scale padding
                        let overflow_dbg = &overflow_grid[row_iteration.index][col_iteration.index];

                        // Position within plot-area coordinates (before outer plot translation)
                        let row_band_start = row_band_pos.start();
                        let _row_band_end = row_band_pos.end();
                        let col_band_start = col_band_pos.start();
                        let _col_band_end = col_band_pos.end();

                        eprintln!(
                            "GRID SUBPLOT r={} c={}: row_start={:.3} h={:.3} col_start={:.3} w={:.3} overflowT={:.3} overflowB={:.3} overflowL={:.3} overflowR={:.3}",
                            row_iteration.index,
                            col_iteration.index,
                            row_band_start,
                            band_h,
                            col_band_start,
                            band_w,
                            overflow_dbg.top,
                            overflow_dbg.bottom,
                            overflow_dbg.left,
                            overflow_dbg.right
                        );
                    }
                    let debug_group = SceneGroup {
                        origin: [x_offset, y_offset].into(),
                        marks: components.debug_marks,
                        clip: avenger_scenegraph::marks::group::Clip::None,
                        zindex: Some(100), // High z-index to ensure debug marks render on top
                        ..Default::default()
                    };
                    marks.push(SceneMark::Group(debug_group));
                }
            }
        }

        // Return marks and scale updates so guides receive the updated scales with spacing
        Ok((
            marks,
            Box::new(crate::layout::ScaleUpdates::new(updated_scales)),
        ))
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        if channel == "row" || channel == "column" {
            // Use band scale for grid faceting channels
            Some(Box::new(crate::scales::spec::Band::default()))
        } else {
            None
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

        // Configure band scale padding for grid facet channels
        if (channel == "row" || channel == "column") && scale_impl.scale_type() == "band" {
            options.insert("padding_outer".to_string(), lit(0.0f32));
            options.insert("padding_inner_px".to_string(), lit(0.0f32));
        }

        options
    }
}
