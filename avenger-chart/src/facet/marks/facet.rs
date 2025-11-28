use crate::channel::config_traits::ScaleSharing;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::facet::context::FacetContext;
use crate::facet::coord::{FacetColumn, FacetGrid, FacetRow};
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::marks::facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
use crate::facet::subplot_iterator::{SubplotIteration, SubplotIterator};
use crate::marks::{
    ChannelDescriptor, ChannelValue, CompiledMark, CompiledMarkState, Mark, MarkState,
};
use crate::plot::{CompiledPlot, Plot};
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::collections::{HashMap, HashSet};
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
                        "FacetRow subplot_dims: band_height={:.3} -> rounded={:.3}",
                        band_height, rounded
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

        Ok(Arc::new(CompiledFacetGrid {
            state: compiled_state,
            compiled_subplot,
            row_title: self.facet_row_title.clone(),
            col_title: self.facet_col_title.clone(),
            facet_spacing: self.facet_spacing,
        }))
    }
}

// ============================================================================
// Helper functions for GridFacet two-pass layout
// ============================================================================

/// Measure overflow for all cells in the grid from SubplotRects
///
/// Returns two overflow grids:
/// - `guide_only_grid`: Axis-only overflow for cross-subplot alignment (unified overflow)
/// - `total_overflow_grid`: Full overflow including legends for spacing calculations
#[allow(clippy::too_many_arguments)]
async fn measure_grid_overflow(
    rects: &[crate::coords::SubplotRect],
    compiled_subplot: &crate::plot::CompiledPlot,
    scale_grouping: &crate::facet::scale_grouping::ScaleGrouping,
    row_expr: &datafusion::logical_expr::Expr,
    col_expr: &datafusion::logical_expr::Expr,
    df: &datafusion::prelude::DataFrame,
    session_context: &datafusion::prelude::SessionContext,
    row_iterations: &[crate::facet::subplot_iterator::SubplotIteration],
    col_iterations: &[crate::facet::subplot_iterator::SubplotIteration],
    num_rows: usize,
    num_cols: usize,
) -> Result<
    (
        Vec<Vec<crate::guide::OverflowSpaceRequirement>>, // guide_only (for alignment)
        Vec<Vec<crate::guide::OverflowSpaceRequirement>>, // total (for spacing)
        HashSet<crate::legend::LegendPosition>,           // legend positions used
    ),
    AvengerChartError,
> {
    use datafusion::logical_expr::lit;

    let mut guide_only_grid =
        vec![vec![crate::guide::OverflowSpaceRequirement::default(); num_cols]; num_rows];
    let mut total_overflow_grid =
        vec![vec![crate::guide::OverflowSpaceRequirement::default(); num_cols]; num_rows];
    let mut all_legend_positions: HashSet<crate::legend::LegendPosition> = HashSet::new();

    // Note: rects are guaranteed to be in row-major order by FacetGrid::transform()
    // (see avenger-chart/src/facet/coord.rs:453-469 nested loop structure).
    // We use row_index/col_index fields for explicit cell lookup rather than relying on iteration order.
    for rect in rects {
        let row_idx = rect.row_index.expect("SubplotRect missing row_index");
        let col_idx = rect.col_index.expect("SubplotRect missing col_index");
        let row_value = &rect.value;
        let col_value = rect
            .col_value
            .as_ref()
            .expect("SubplotRect missing col_value");

        // Filter data for this cell (both row AND column match)
        let filter_df = df
            .clone()
            .filter(row_expr.clone().eq(lit(row_value.clone())))?
            .filter(col_expr.clone().eq(lit(col_value.clone())))?;

        // Get cell-specific params by merging row and col FacetContexts
        let cell_params = merge_grid_facet_contexts(
            &row_iterations[row_idx],
            &col_iterations[col_idx],
            num_rows,
            num_cols,
        );

        // Build scales for this subplot position using ScaleGrouping with cell-specific params
        let inner_scales = scale_grouping
            .build_scales_for_position(
                compiled_subplot,
                row_idx,
                col_idx,
                rect.width,
                rect.height,
                session_context,
                &cell_params,
            )
            .await?;

        // Measure both guide-only overflow (for alignment) and total overflow (for spacing)
        let (guide_only_overflow, total_overflow, _legend_info, legend_positions) =
            compiled_subplot
                .measure_with_scales(
                    rect.width,
                    rect.height,
                    session_context,
                    &cell_params,
                    &inner_scales,
                    Some(&filter_df),
                )
                .await?;

        guide_only_grid[row_idx][col_idx] = guide_only_overflow;
        total_overflow_grid[row_idx][col_idx] = total_overflow;
        all_legend_positions.extend(legend_positions);
    }

    Ok((guide_only_grid, total_overflow_grid, all_legend_positions))
}

/// Calculate row padding from 2D overflow grid
///
/// For each row boundary (i, i+1), computes the maximum gap needed:
/// `gap[i] = max(bottom[row_i][col] + top[row_i+1][col])` across all columns
///
/// This ensures consistent row spacing that accounts for overflow from all columns.
pub(crate) fn calculate_row_padding(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> f32 {
    let num_rows = overflow_grid.len();
    if num_rows <= 1 {
        return 0.0;
    }

    let mut max_gap: f32 = 0.0;
    for i in 0..(num_rows - 1) {
        // Get max bottom overflow for row i across all columns
        let max_bottom = overflow_grid[i]
            .iter()
            .map(|o| o.bottom)
            .fold(0.0f32, f32::max);

        // Get max top overflow for row i+1 across all columns
        let max_top = overflow_grid[i + 1]
            .iter()
            .map(|o| o.top)
            .fold(0.0f32, f32::max);

        max_gap = max_gap.max(max_bottom + max_top);
    }

    max_gap.ceil()
}

/// Calculate column padding from 2D overflow grid
///
/// For each column boundary (j, j+1), computes the maximum gap needed:
/// `gap[j] = max(right[row][col_j] + left[row][col_j+1])` across all rows
///
/// This ensures consistent column spacing that accounts for overflow from all rows.
pub(crate) fn calculate_col_padding(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> f32 {
    let num_cols = overflow_grid.first().map(|row| row.len()).unwrap_or(0);
    if num_cols <= 1 {
        return 0.0;
    }

    let mut max_gap: f32 = 0.0;
    for j in 0..(num_cols - 1) {
        // Get max right overflow for col j across all rows
        let max_right = overflow_grid
            .iter()
            .map(|row| row[j].right)
            .fold(0.0f32, f32::max);

        // Get max left overflow for col j+1 across all rows
        let max_left = overflow_grid
            .iter()
            .map(|row| row[j + 1].left)
            .fold(0.0f32, f32::max);

        max_gap = max_gap.max(max_right + max_left);
    }

    max_gap.ceil()
}

/// Calculate both row and column padding from 2D overflow grid
fn calculate_grid_padding(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> (f32, f32) {
    let row_padding = calculate_row_padding(overflow_grid);
    let col_padding = calculate_col_padding(overflow_grid);
    (row_padding, col_padding)
}

/// Compute global maximum overflow across all cells in the grid
///
/// Returns a single OverflowSpaceRequirement where each side has the maximum
/// value from all cells. This ensures all subplots use identical overflow
/// sizes, causing legends to naturally align across the grid.
fn compute_global_max_overflow(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> crate::guide::OverflowSpaceRequirement {
    let mut max_overflow = crate::guide::OverflowSpaceRequirement::default();
    for row in overflow_grid {
        for cell_overflow in row {
            max_overflow.top = max_overflow.top.max(cell_overflow.top);
            max_overflow.bottom = max_overflow.bottom.max(cell_overflow.bottom);
            max_overflow.left = max_overflow.left.max(cell_overflow.left);
            max_overflow.right = max_overflow.right.max(cell_overflow.right);
        }
    }
    max_overflow
}

/// Extract 1D row overflow vector from 2D overflow grid
/// Takes max overflow in each direction across all columns in each row
fn extract_row_overflow(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> Vec<crate::guide::OverflowSpaceRequirement> {
    overflow_grid
        .iter()
        .map(|row_overflows| {
            // Take max overflow in each direction across all columns in this row
            row_overflows.iter().fold(
                crate::guide::OverflowSpaceRequirement::default(),
                |acc, o| crate::guide::OverflowSpaceRequirement {
                    top: acc.top.max(o.top),
                    bottom: acc.bottom.max(o.bottom),
                    left: acc.left.max(o.left),
                    right: acc.right.max(o.right),
                },
            )
        })
        .collect()
}

/// Extract 1D column overflow vector from 2D overflow grid
/// Takes max overflow in each direction across all rows in each column
fn extract_col_overflow(
    overflow_grid: &[Vec<crate::guide::OverflowSpaceRequirement>],
) -> Vec<crate::guide::OverflowSpaceRequirement> {
    let num_cols = overflow_grid.first().map(|row| row.len()).unwrap_or(0);

    // For each column, combine overflow from all rows
    (0..num_cols)
        .map(|col_idx| {
            overflow_grid.iter().fold(
                crate::guide::OverflowSpaceRequirement::default(),
                |acc, row| {
                    let o = &row[col_idx];
                    crate::guide::OverflowSpaceRequirement {
                        top: acc.top.max(o.top),
                        bottom: acc.bottom.max(o.bottom),
                        left: acc.left.max(o.left),
                        right: acc.right.max(o.right),
                    }
                },
            )
        })
        .collect()
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

    fn wants_full_data_batch(&self) -> bool {
        true // Facets need full data for nested filtering
    }

    /// Evaluate grid facet layout (2D row×col grid)
    async fn evaluate_from_data(
        &self,
        data: Option<&datafusion::arrow::record_batch::RecordBatch>,
        _scalars: &datafusion::arrow::record_batch::RecordBatch,
        context: &RenderContext,
        coord: Box<dyn crate::coords::CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::batch_to_dataframe;
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Convert RecordBatch to DataFrame if provided (for nested facets)
        let data_override = if let Some(batch) = data {
            Some(batch_to_dataframe(batch, &context.session_context)?)
        } else {
            None
        };

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

        // Get row and col expressions for render-time key extraction
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

        // Determine data source: parent override OR compiled data
        let ctx = &context.session_context;
        let df = if let Some(override_df) = data_override.as_ref() {
            // Nested facet: use filtered data from parent
            override_df.clone()
        } else {
            // Top-level facet: use compiled data
            self.state.data.dataframe_with_context(ctx).ok_or_else(|| {
                AvengerChartError::InternalError("GridFacet could not access data".into())
            })?
        };

        // Extract facet keys at render time from actual data
        use crate::facet::keys::FacetKeyExtractor;
        let mut row_domain_vals = FacetKeyExtractor::extract_keys(&df, &row_expr).await?;
        let mut col_domain_vals = FacetKeyExtractor::extract_keys(&df, &col_expr).await?;

        // Sort domain values to ensure deterministic facet ordering
        row_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        col_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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

        // ========== PASS 1: MEASUREMENT PHASE ==========
        // Extract initial positions from scales (or use fallback for degenerate cases)
        let initial_row_positions: Vec<f32> = if let Some(row_scale) = row_scale_opt {
            row_scale
                .configured()
                .scale_scalars_to_numeric(&row_domain_vals)?
        } else {
            // Fallback: single value at origin for degenerate case
            vec![0.0; row_domain_vals.len()]
        };

        let initial_col_positions: Vec<f32> = if let Some(col_scale) = col_scale_opt {
            col_scale
                .configured()
                .scale_scalars_to_numeric(&col_domain_vals)?
        } else {
            // Fallback: single value at origin for degenerate case
            vec![0.0; col_domain_vals.len()]
        };

        // Critical: Verify lengths match
        assert_eq!(
            initial_row_positions.len(),
            row_domain_vals.len(),
            "Row positions length must match row domain values"
        );
        assert_eq!(
            initial_col_positions.len(),
            col_domain_vals.len(),
            "Col positions length must match col domain values"
        );

        // Build position_channels and position_values for coord.transform()
        use std::collections::HashMap;
        let mut position_channels_pass1 = HashMap::new();
        position_channels_pass1.insert(
            "row",
            avenger_common::value::ScalarOrArray::new_array(initial_row_positions),
        );
        position_channels_pass1.insert(
            "column",
            avenger_common::value::ScalarOrArray::new_array(initial_col_positions),
        );

        let mut position_values_pass1 = HashMap::new();
        position_values_pass1.insert("row", row_domain_vals.clone());
        position_values_pass1.insert("column", col_domain_vals.clone());

        // Call coord.transform() to get initial geometry
        let initial_geometry = coord.transform(
            &position_channels_pass1,
            Some(&position_values_pass1),
            context.plot_width,
            context.plot_height,
        )?;

        let initial_rects = initial_geometry
            .as_any()
            .downcast_ref::<crate::coords::SubplotGeometry>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Expected SubplotGeometry from GridFacet coord transform".into(),
                )
            })?
            .rects
            .as_slice();

        // Collect SubplotIterations for cell-specific FacetContext
        // These will be reused in Pass 2 to ensure measurement and rendering use identical context
        let row_iterations: Vec<_> = SubplotIterator::<RowDimensionConfig>::new(
            row_domain_vals.clone(),
            context.params.clone(),
            scale_sharing_by_channel.clone(),
        )
        .collect();

        let col_iterations: Vec<_> = SubplotIterator::<ColumnDimensionConfig>::new(
            col_domain_vals.clone(),
            context.params.clone(),
            scale_sharing_by_channel.clone(),
        )
        .collect();

        // Measure overflow for all grid cells using the helper function
        // Returns:
        // - guide_only_grid: For alignment (axis overflow only, no legends)
        // - total_overflow_grid: For spacing (includes legend dimensions)
        // - legend_positions: Set of legend positions used across all subplots
        let (guide_only_grid, total_overflow_grid, legend_positions) = measure_grid_overflow(
            initial_rects,
            &self.compiled_subplot,
            &scale_grouping,
            &row_expr,
            &col_expr,
            &df,
            &context.session_context,
            &row_iterations,
            &col_iterations,
            num_rows,
            num_cols,
        )
        .await?;

        // Compute global max overflow across all cells for legend alignment
        // Use GUIDE-ONLY overflow so all subplots have identical axis overflow,
        // causing legends to naturally align relative to the overflow region.
        let global_max_overflow = compute_global_max_overflow(&guide_only_grid);

        // ========== BETWEEN PASSES: CALCULATE PADDING ==========
        // Calculate padding from TOTAL overflow measurements (includes legends)
        // This ensures enough space between subplots for legends to fit.
        let (row_padding_px, col_padding_px) = calculate_grid_padding(&total_overflow_grid);

        // Get configured facet_spacing
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

        // Zero padding for single-dimension axes to avoid shrinking
        let row_padding_px = if num_rows == 1 {
            0.0
        } else {
            row_padding_px + spacing
        };
        let col_padding_px = if num_cols == 1 {
            0.0
        } else {
            col_padding_px + spacing
        };

        // Extract overflow vectors for coord update (use total overflow for proper spacing)
        let row_overflow = extract_row_overflow(&total_overflow_grid);
        let col_overflow = extract_col_overflow(&total_overflow_grid);

        // Debug: log measured spacing and gaps
        tracing::debug!(
            num_rows = num_rows,
            num_cols = num_cols,
            spacing = spacing,
            row_padding_px = row_padding_px,
            col_padding_px = col_padding_px,
            "GridFacet: computed facet spacing (including theme spacing)"
        );

        // Update coord with measured padding
        let updated_coord = coord.with_measured_padding(&crate::coords::PaddingSpec::Grid {
            row_padding_px,
            col_padding_px,
            row_overflow: row_overflow.clone(),
            col_overflow: col_overflow.clone(),
        });

        // ========== REBUILD SCALES WITH PADDING ==========
        // Rebuild row and col scales with measured spacing
        let mut updated_scales = context.scales.clone();

        if row_padding_px > 0.0 && row_scale_opt.is_some() {
            use avenger_scales::scalar::Scalar;
            let row_scale = row_scale_opt.unwrap();
            let mut new_config = row_scale.configured().config.clone();
            new_config.options.insert(
                "padding_inner_px".to_string(),
                Scalar::from_f32(row_padding_px),
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
                padding_inner_px = row_padding_px,
                "GridFacet: updated row scale padding_inner_px"
            );
        }

        if col_padding_px > 0.0 && col_scale_opt.is_some() {
            use avenger_scales::scalar::Scalar;
            let col_scale = col_scale_opt.unwrap();
            let mut new_config = col_scale.configured().config.clone();
            new_config.options.insert(
                "padding_inner_px".to_string(),
                Scalar::from_f32(col_padding_px),
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
                padding_inner_px = col_padding_px,
                "GridFacet: updated col scale padding_inner_px"
            );
        }

        // ========== PASS 2: FINAL RENDERING PHASE ==========
        // Extract updated positions from rebuilt scales
        let updated_row_positions: Vec<f32> = if let Some(row_scale) = updated_scales.get("row") {
            row_scale
                .configured()
                .scale_scalars_to_numeric(&row_domain_vals)?
        } else {
            vec![0.0; row_domain_vals.len()]
        };

        let updated_col_positions: Vec<f32> = if let Some(col_scale) = updated_scales.get("column")
        {
            col_scale
                .configured()
                .scale_scalars_to_numeric(&col_domain_vals)?
        } else {
            vec![0.0; col_domain_vals.len()]
        };

        // Build position_channels for Pass 2
        let mut final_position_channels = HashMap::new();
        final_position_channels.insert(
            "row",
            avenger_common::value::ScalarOrArray::new_array(updated_row_positions),
        );
        final_position_channels.insert(
            "column",
            avenger_common::value::ScalarOrArray::new_array(updated_col_positions),
        );

        let mut final_position_values = HashMap::new();
        final_position_values.insert("row", row_domain_vals.clone());
        final_position_values.insert("column", col_domain_vals.clone());

        // Get final geometry from updated coord
        let final_geometry = updated_coord.transform(
            &final_position_channels,
            Some(&final_position_values),
            context.plot_width,
            context.plot_height,
        )?;

        let final_rects = final_geometry
            .as_any()
            .downcast_ref::<crate::coords::SubplotGeometry>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Expected SubplotGeometry from GridFacet coord transform".into(),
                )
            })?
            .rects
            .as_slice();

        // Verify counts match
        assert_eq!(
            final_rects.len(),
            num_rows * num_cols,
            "Final rects count must equal num_rows * num_cols"
        );

        // Reuse iteration vectors from Pass 1 to ensure identical FacetContext
        // These were already built before measure_grid_overflow and contain cell-specific params
        assert_eq!(
            row_iterations.len(),
            num_rows,
            "Row iteration count mismatch"
        );
        assert_eq!(
            col_iterations.len(),
            num_cols,
            "Col iteration count mismatch"
        );

        let mut marks: Vec<SceneMark> = Vec::new();

        // Render each grid cell using index-based lookup
        for rect in final_rects {
            let row_idx = rect.row_index.expect("SubplotRect missing row_index");
            let col_idx = rect.col_index.expect("SubplotRect missing col_index");

            // Index-based lookup - no reliance on ordering!
            let row_iteration = &row_iterations[row_idx];
            let col_iteration = &col_iterations[col_idx];

            // Double-check indices match (can be debug_assert in production)
            assert_eq!(row_iteration.index, row_idx, "Row iteration index mismatch");
            assert_eq!(col_iteration.index, col_idx, "Col iteration index mismatch");

            // Merge row and col contexts into unified GridFacet context
            let mut merged_params =
                merge_grid_facet_contexts(row_iteration, col_iteration, num_rows, num_cols);

            // Build coordinated_spacing for legend alignment
            // Uses global max overflow only for sides with legends
            {
                use crate::facet::coordination::FacetCoordinationContext;
                use crate::legend::LegendPosition;

                let mut coordinated_spacing = std::collections::HashMap::new();
                if legend_positions.contains(&LegendPosition::Right) {
                    coordinated_spacing
                        .insert("legend_right".to_string(), global_max_overflow.right);
                }
                if legend_positions.contains(&LegendPosition::Left) {
                    coordinated_spacing.insert("legend_left".to_string(), global_max_overflow.left);
                }
                if legend_positions.contains(&LegendPosition::Top) {
                    coordinated_spacing.insert("legend_top".to_string(), global_max_overflow.top);
                }
                if legend_positions.contains(&LegendPosition::Bottom) {
                    coordinated_spacing
                        .insert("legend_bottom".to_string(), global_max_overflow.bottom);
                }

                // Create or update coordination context with coordinated_spacing
                let coord_ctx = FacetCoordinationContext::from_params(&merged_params)
                    .unwrap_or_default()
                    .with_coordinated_spacing(coordinated_spacing);
                merged_params.extend(coord_ctx.to_params());
            }

            // Filter to rows matching both row AND col values
            let filter_df = df
                .clone()
                .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
                .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

            // Check if this cell has any data
            let cell_count = filter_df.clone().count().await?;
            let cell_is_empty = cell_count == 0;

            // Build scales for this subplot position using ScaleGrouping
            let inner_scales = scale_grouping
                .build_scales_for_position(
                    &self.compiled_subplot,
                    row_idx,
                    col_idx,
                    rect.width,
                    rect.height,
                    &context.session_context,
                    &merged_params,
                )
                .await?;

            // CRITICAL CHANGE: Use rect.x, rect.y from coord.transform() instead of manual calculation
            let x_offset = rect.x;
            let y_offset = rect.y;

            // Render subplot using build_plot_components with Render mode
            let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
                scales: inner_scales.clone(),
            };

            let components = self
                .compiled_subplot
                .build_plot_components(
                    rect.width,
                    rect.height,
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
            if !components.debug_marks.is_empty() {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    // Use total overflow measurement for debug visualization
                    let overflow_dbg = &total_overflow_grid[row_idx][col_idx];

                    eprintln!(
                        "GRID SUBPLOT r={} c={}: x={:.3} y={:.3} w={:.3} h={:.3} overflowT={:.3} overflowB={:.3} overflowL={:.3} overflowR={:.3}",
                        row_idx,
                        col_idx,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
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

        // Return marks and scale updates so guides receive the updated scales with spacing.
        // Also return overflow info so the outer guide can position facet labels past legends.
        Ok((
            marks,
            crate::layout::LayoutUpdates::new(
                updated_scales,
                Some(row_overflow),
                Some(col_overflow),
            ),
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
