use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, band::bandwidth};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::lit,
    scalar::ScalarValue as DfScalarValue,
};
use serde::{Deserialize, Serialize};

use crate::{
    coords::{
        CellDomainInfo, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform,
        CoordinatedLayout, CoordinatedOverflow, OverflowSpaceRequirement, PaddingSpec,
        PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::{
        coordination::{CoordinationGroupKey, FacetAxis},
        guide::{FacetColGuideConfig, FacetRowGuideConfig},
        layout_plan::{
            FacetBandPlan, FacetCellPlan, aggregate_facet_col_overflow,
            compute_padding_from_overflows, effective_edge_indices,
        },
        marks::facet::{FacetMarkRef, facet_mark_ref},
        sharing_policy,
    },
    layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
    marks::CompiledMark,
    plot::compiled::{
        CompiledPlot, ComponentsMeasurement, scale_provider::DynamicScaleProvider,
        scales::build_scale_builder_from_marks,
    },
    render::EvaluationContext,
    scales::{
        ConfiguredScaleWithSpec, ScaleBuilder,
        domain_extent::{DomainBounds, DomainExtent, RadiusPadding},
    },
};

/// Domain extent with associated sharing level.
///
/// Used to track domain extents per-cell along with the sharing level
/// for coordinating domains across cells at the appropriate hierarchy level.
#[derive(Clone, Debug)]
pub struct ChannelDomainExtent {
    /// The domain extent (bounds + optional radius padding)
    pub extent: DomainExtent,
    /// Scale sharing level (0=Free, N=Level(N), 255=Shared)
    pub sharing_level: u8,
}

/// Measurement data for FacetColumn coordinate system.
///
/// This captures the computed padding from overflow measurement and pre-computed
/// subplot measurements. Marks use the stored measurements directly without re-measuring.
pub struct FacetColCoordMeasurement {
    /// Facet column values (one per cell)
    pub cell_values: Vec<ScalarValue>,
    /// Computed padding between cells in pixels (MAX of adjacent overflow combinations)
    pub padding_inner_px: f32,
    /// Outer left total_overflow (first cell's left edge) - used to adjust scale range
    pub outer_left: f32,
    /// Outer right total_overflow (last cell's right edge) - used to adjust scale range
    pub outer_right: f32,
    /// Filtered DataFrames for each cell
    pub data_overrides: Vec<DataFrame>,
    /// ScaleBuilder for shared scales (caches data extents for rebuilding with updated dimensions).
    /// Used with DynamicScaleProvider to correctly compute radius-aware domains.
    pub shared_scale_builder: ScaleBuilder,
    /// Parent facet path (for nested facets). This is the path to reach this facet level.
    /// When constructing cell paths for nested subplots, prepend this to the cell value.
    pub parent_path: Vec<ScalarValue>,
    /// Stable field identity for coordination grouping at this facet level.
    /// This is typically the facet column field name.
    pub coordination_field_identity: String,
    /// Pre-computed subplot measurements (computed with final subplot width after padding_inner_px).
    /// These are used directly by render_from_data to avoid re-measuring.
    pub subplot_measurements: Vec<ComponentsMeasurement>,
    /// Coordinated overflow values aggregated across ALL facets at this nesting level.
    /// Populated by `coordinate_overflow_for_guides()` after measurement.
    pub coordinated_overflow: CoordinatedOverflow,
    /// Reference to compiled subplot for re-measurement after coordination.
    /// Used by `apply_coordinated_overflow` to re-measure with adjusted height.
    pub compiled_subplot: Arc<CompiledPlot>,
    /// Subplot width (bandwidth) for re-measurement.
    pub subplot_width: f32,
    /// Facet depth in the hierarchy (1 = outermost, 2 = nested, etc.)
    /// INVARIANT: facet_depth == full_cell_path.len() for any cell
    /// Used for sharing level comparison: sharing >= facet_depth means global.
    pub facet_depth: u8,
    /// Domain extents per cell, indexed by cell index.
    /// Each cell's map is keyed by channel name.
    /// Extracted during measurement from each cell's filtered data.
    pub cell_domain_extents: Vec<HashMap<String, ChannelDomainExtent>>,
    /// Coordinated domain extents per cell, indexed by cell index.
    /// Populated during coordination phase with unified extents.
    pub coordinated_domain_extents: Vec<HashMap<String, DomainExtent>>,
    /// Tracks which cells are "empty" (have no data due to Level(N) sharing).
    /// Empty cells are created to maintain uniform layout but contain no data.
    /// Indexed by cell index; true means the cell is empty.
    pub empty_cells: Vec<bool>,
    /// Column scale (as ConfiguredScale) BEFORE Pass 2 adjustments.
    /// Used to recompute subplot width when coordinated layout differs from local.
    pub original_column_scale: ConfiguredScale,
    /// Local layout values (pre-coordination).
    pub local_layout: CoordinatedLayout,
    /// Coordinated layout values (post-coordination). None before coordination.
    pub coordinated_layout: Option<CoordinatedLayout>,
}

#[async_trait::async_trait]
impl CoordMeasurement for FacetColCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn child_measurements(&self) -> &[ComponentsMeasurement] {
        &self.subplot_measurements
    }

    fn child_measurements_mut(&mut self) -> &mut [ComponentsMeasurement] {
        &mut self.subplot_measurements
    }

    fn local_overflow(&self) -> Option<CoordinatedOverflow> {
        let (guide, total) =
            aggregate_facet_col_overflow(&self.subplot_measurements, &self.empty_cells)?;

        Some(CoordinatedOverflow { guide, total })
    }

    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        Some(&self.coordinated_overflow)
    }

    fn set_coordinated_overflow(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_overflow = overflow;
    }

    fn local_layout(&self) -> Option<CoordinatedLayout> {
        Some(self.local_layout.clone())
    }

    fn set_coordinated_layout(&mut self, layout: CoordinatedLayout) {
        self.coordinated_layout = Some(layout);
    }

    fn padding_inner_px(&self) -> Option<f32> {
        if self.padding_inner_px > 0.0 {
            Some(self.padding_inner_px)
        } else {
            None
        }
    }

    fn apply_scale_adjustments(&self, scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        // Apply padding_inner_px, outer edge adjustments, and domain override to the column scale.
        // The outer_left/outer_right values represent legend space at the outer edges
        // of the first/last cells. We reduce the scale range width to account for this space.
        //
        // The total available width is reduced by BOTH outer_left and outer_right.
        // The scale start remains at 0 (or the original start), but the end is reduced
        // so that cells fit within the remaining space after legends are accounted for.
        //
        // IMPORTANT: For Level(N) sharing, we also override the column scale domain
        // to use cell_values from the facet tree (which includes empty cells for uniform layout).
        // Without this, the scale domain would only contain values from the filtered data,
        // not the Level(N)-aware enumerated values.
        if let Some(column_scale) = scales.get_mut("column") {
            let mut updated_config = column_scale.configured().clone();
            let has_empty_cells = self.empty_cells.iter().copied().any(|empty| empty);
            let has_adjacent_non_empty =
                self.empty_cells.windows(2).any(|pair| !pair[0] && !pair[1]);
            let needs_zero_padding_override = has_empty_cells && !has_adjacent_non_empty;

            // Override domain with Level(N)-aware cell_values from facet tree.
            // This ensures the scale domain matches the enumerated cells (including empty ones).
            if !self.cell_values.is_empty() {
                if let Ok(domain_array) =
                    ScalarValue::iter_to_array(self.cell_values.iter().cloned())
                {
                    updated_config = updated_config.with_domain(domain_array);
                }
            }

            // Use coordinated values if available, otherwise use local values.
            //
            // NOTE: We do NOT set band_n on the column scale here. The band_n option
            // is only used during measurement (apply_coordinated_overflow) to compute
            // the correct subplot_width. For rendering, the column scale should position
            // its actual domain cells evenly across the full allocated width. The parent
            // FacetCol already allocated the correct width using band_n, so the child's
            // cells should fill that width naturally. Setting band_n here would compress
            // cells into a fraction of the width, breaking guide bracket alignment.
            if let Some(layout) = &self.coordinated_layout {
                if layout.padding_inner_px > 0.0 || needs_zero_padding_override {
                    updated_config =
                        updated_config.with_option("padding_inner_px", layout.padding_inner_px);
                }
                if layout.outer_left > 0.0 || layout.outer_right > 0.0 {
                    if let Ok((range_start, range_end)) =
                        updated_config.config.numeric_interval_range()
                    {
                        let new_end = range_end - layout.outer_left - layout.outer_right;
                        updated_config = updated_config.with_range_interval((range_start, new_end));
                    }
                }
            } else {
                // No coordination — use local values (existing behavior)
                if self.padding_inner_px > 0.0 || needs_zero_padding_override {
                    updated_config =
                        updated_config.with_option("padding_inner_px", self.padding_inner_px);
                }
                if self.outer_left > 0.0 || self.outer_right > 0.0 {
                    if let Ok((range_start, range_end)) =
                        updated_config.config.numeric_interval_range()
                    {
                        let new_end = range_end - self.outer_left - self.outer_right;
                        updated_config = updated_config.with_range_interval((range_start, new_end));
                    }
                }
            }

            *column_scale =
                ConfiguredScaleWithSpec::new(column_scale.spec().clone(), updated_config);
        }
    }

    fn update_child_dimensions(&mut self, new_height: f32) {
        // Update subplot measurements to use the correct height from second-pass layout.
        // This is needed when legends (or other elements) change the available plot area
        // after the initial measurement pass.
        for measurement in &mut self.subplot_measurements {
            if (measurement.plot_area_height - new_height).abs() > 0.1 {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol update_child_dimensions: height {:.1} -> {:.1}",
                        measurement.plot_area_height, new_height
                    );
                }
                measurement.plot_area_height = new_height;
            }
        }
    }

    async fn apply_coordinated_overflow(
        &mut self,
        eval_ctx: &EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        // Compute legend-adjusted height from coordinated overflow
        let coordinated = &self.coordinated_overflow;
        let legend_top = (coordinated.total.top - coordinated.guide.top).max(0.0);
        let legend_bottom = (coordinated.total.bottom - coordinated.guide.bottom).max(0.0);

        let has_legend_overflow = legend_top > 0.0 || legend_bottom > 0.0;

        // Check if we have coordinated domain extents to apply
        let has_coordinated_extents = self
            .coordinated_domain_extents
            .iter()
            .any(|extents| !extents.is_empty());

        // Check if coordinated layout differs from local layout
        let has_coordinated_layout =
            self.coordinated_layout
                .as_ref()
                .map_or(false, |coordinated| {
                    coordinated.n != self.local_layout.n
                        || (coordinated.padding_inner_px - self.local_layout.padding_inner_px).abs()
                            > 0.01
                        || (coordinated.outer_left - self.local_layout.outer_left).abs() > 0.01
                        || (coordinated.outer_right - self.local_layout.outer_right).abs() > 0.01
                });

        // If coordinated layout changed, recompute subplot_width using band_n.
        // This only updates the width/padding fields — it does NOT trigger re-measurement.
        // Re-measurement would rebuild scales from scratch, changing y-domains.
        if has_coordinated_layout {
            let layout = self.coordinated_layout.as_ref().unwrap();
            let mut scale = self.original_column_scale.clone();

            // Set band_n for coordinated cell count
            scale = scale.with_option("band_n", layout.n as i32);

            // Apply coordinated padding_inner_px
            scale = scale.with_option("padding_inner_px", layout.padding_inner_px);

            // Apply coordinated outer_left + outer_right to range
            if layout.outer_left > 0.0 || layout.outer_right > 0.0 {
                if let Ok((range_start, range_end)) = scale.config.numeric_interval_range() {
                    let new_end = range_end - layout.outer_left - layout.outer_right;
                    scale = scale.with_range_interval((range_start, new_end));
                }
            }

            // Recompute subplot_width from coordinated scale
            let new_subplot_width = bandwidth(&scale.config).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to get coordinated bandwidth: {}",
                    e
                ))
            })?;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol apply_coordinated_overflow: layout coordination: width {:.1} -> {:.1}, n {} -> {}, padding {:.1} -> {:.1}",
                    self.subplot_width,
                    new_subplot_width,
                    self.local_layout.n,
                    layout.n,
                    self.local_layout.padding_inner_px,
                    layout.padding_inner_px
                );
            }

            self.subplot_width = new_subplot_width;
            self.padding_inner_px = layout.padding_inner_px;
            self.outer_left = layout.outer_left;
            self.outer_right = layout.outer_right;
        }

        // Only do full re-measurement for legend overflow or coordinated domain extents.
        // Layout coordination (above) only adjusts subplot_width without re-measuring,
        // because re-measurement rebuilds scales from scratch and can change y-domains.
        if !has_legend_overflow && !has_coordinated_extents {
            return Ok(());
        }

        // Get original height from first measurement (all should be same)
        let original_height = self
            .subplot_measurements
            .first()
            .map(|m| m.plot_area_height)
            .unwrap_or(0.0);

        let adjusted_height = if has_legend_overflow {
            (original_height - legend_top - legend_bottom).max(1.0)
        } else {
            original_height
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol apply_coordinated_overflow: height {:.1} -> {:.1} (legend_top={:.1}, legend_bottom={:.1}, has_coordinated_extents={})",
                original_height,
                adjusted_height,
                legend_top,
                legend_bottom,
                has_coordinated_extents
            );
        }

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(eval_ctx.params.clone());
            eval_ctx.with_params(params)
        };

        let mut new_measurements = Vec::with_capacity(self.subplot_measurements.len());

        for (idx, (value, data_override)) in self
            .cell_values
            .iter()
            .zip(self.data_overrides.iter())
            .enumerate()
        {
            // Build full cell path from parent_path + current cell value
            let mut cell_path: Vec<ScalarValue> = self.parent_path.clone();
            cell_path.push(value.clone());
            let is_empty = self.empty_cells.get(idx).copied().unwrap_or(false);
            let cell = FacetCellPlan {
                value: value.clone(),
                full_path: cell_path,
                exists_in_tree: !is_empty,
                is_empty,
                filtered_df: data_override.clone(),
            };

            // Create a scale builder for this cell with coordinated domain extents applied
            let mut cell_scale_builder = self.shared_scale_builder.clone();

            // Apply coordinated domain extents for this cell if present
            if let Some(cell_extents) = self.coordinated_domain_extents.get(idx) {
                if !cell_extents.is_empty() {
                    cell_scale_builder.extend_with_domain_extents(cell_extents);

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetCol apply_coordinated_overflow: cell[{}] applying {} coordinated domain extents",
                            idx,
                            cell_extents.len()
                        );
                    }
                }
            }

            // Build layout spec with fixed plot area (subplot dimensions)
            let subplot_layout_spec =
                fixed_plot_area_layout_spec(self.subplot_width, adjusted_height);
            let measurement = measure_facet_cell(
                &cell,
                &self.compiled_subplot,
                &subplot_eval_ctx,
                &subplot_layout_spec,
                FacetCellMeasurementMode::ExplicitBuilder {
                    scale_builder: &cell_scale_builder,
                },
            )
            .await?;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol apply_coordinated_overflow: re-measured cell[{}]={:?} at width={:.1} height={:.1} empty={}",
                    idx, value, self.subplot_width, adjusted_height, is_empty
                );
            }

            new_measurements.push(measurement);
        }

        // Replace measurements with re-measured ones
        self.subplot_measurements = new_measurements;

        Ok(())
    }

    fn collect_cell_domain_extents(&self, collector: &mut Vec<CellDomainInfo>) {
        // Collect extents from each cell
        for (cell_idx, cell_extents) in self.cell_domain_extents.iter().enumerate() {
            // Build full cell path: parent_path + cell_value
            let mut full_cell_path = self.parent_path.clone();
            if let Some(cell_value) = self.cell_values.get(cell_idx) {
                full_cell_path.push(cell_value.clone());
            }

            for (channel, annotated) in cell_extents {
                collector.push(CellDomainInfo {
                    full_cell_path: full_cell_path.clone(),
                    channel: channel.clone(),
                    sharing_level: annotated.sharing_level,
                    facet_depth: self.facet_depth,
                    extent: annotated.extent.clone(),
                });
            }
        }
    }

    fn distribute_cell_domain_extents(
        &mut self,
        unified: &HashMap<(String, Vec<ScalarValue>), DomainExtent>,
    ) {
        // Initialize coordinated extents for each cell
        self.coordinated_domain_extents = vec![HashMap::new(); self.cell_values.len()];

        // Distribute to each cell
        for (cell_idx, cell_extents) in self.cell_domain_extents.iter().enumerate() {
            // Build full cell path
            let mut full_cell_path = self.parent_path.clone();
            if let Some(cell_value) = self.cell_values.get(cell_idx) {
                full_cell_path.push(cell_value.clone());
            }

            for (channel, annotated) in cell_extents {
                // Skip Level(0)/Free - no coordination needed, cell uses its own extents
                // This avoids triggering unnecessary re-measurement
                if annotated.sharing_level == 0 {
                    continue;
                }

                let ancestor_key = compute_ancestor_key(
                    &full_cell_path,
                    annotated.sharing_level,
                    self.facet_depth,
                );

                if let Some(unified_extent) = unified.get(&(channel.clone(), ancestor_key)) {
                    self.coordinated_domain_extents[cell_idx]
                        .insert(channel.clone(), unified_extent.clone());
                }
            }
        }
    }

    fn channel_sharing_level(&self, channel: &str) -> u8 {
        // Look up sharing level from the first cell's domain extents
        // All cells should have the same sharing level for a given channel
        if let Some(first_cell_extents) = self.cell_domain_extents.first() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol channel_sharing_level: channel={}, available={:?}",
                    channel,
                    first_cell_extents.keys().collect::<Vec<_>>()
                );
            }
            if let Some(annotated) = first_cell_extents.get(channel) {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol channel_sharing_level: channel={} -> {}",
                        channel, annotated.sharing_level
                    );
                }
                return annotated.sharing_level;
            }
        }
        // Default to Shared (255) for backward compatibility
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol channel_sharing_level: channel={} -> 255 (default)",
                channel
            );
        }
        255
    }

    fn coordinated_subplot_width(&self) -> Option<f32> {
        // Return the coordinated subplot width if coordination has been applied.
        // After apply_coordinated_overflow, self.subplot_width reflects the coordinated value.
        if self.subplot_width > 0.0 {
            Some(self.subplot_width)
        } else {
            None
        }
    }

    fn set_parent_bandwidth(&mut self, bandwidth: f32) {
        // Update original_column_scale range to use the coordinated parent bandwidth.
        // This ensures all FacetCol nodes at the same depth compute the same subplot
        // width from bandwidth(), regardless of which branch they belong to.
        if bandwidth > 0.0 {
            self.original_column_scale = self
                .original_column_scale
                .clone()
                .with_range_interval((0.0, bandwidth));

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetCol set_parent_bandwidth: updated original_column_scale range to (0, {:.1})",
                    bandwidth
                );
            }
        }
    }

    fn coordination_group_key(&self, depth: usize) -> Option<CoordinationGroupKey> {
        Some(CoordinationGroupKey::new(
            depth,
            FacetAxis::Column,
            self.coordination_field_identity.clone(),
        ))
    }
}

/// Compute ancestor key for grouping domain extents.
///
/// Uses FULL CELL PATH (not just parent_path).
/// Level(N) means "remove last N components from full_cell_path".
///
/// # Arguments
/// * `full_cell_path` - Complete path to cell: parent_path + [cell_value]
/// * `sharing_level` - The scale sharing level (0=Free, N=Level(N), 255=Shared)
/// * `facet_depth` - 1-based depth in facet hierarchy (MUST equal full_cell_path.len())
///
/// # Examples
///
/// At facet_depth=3 with full_cell_path=["GP", "P", "Cell"]:
/// - Level(0): ["GP", "P", "Cell"] (no sharing, per-cell)
/// - Level(1): ["GP", "P"] (share with siblings, remove last 1)
/// - Level(2): ["GP"] (share with cousins, remove last 2)
/// - Level(3+): [] (global, remove all)
pub fn compute_ancestor_key(
    full_cell_path: &[ScalarValue],
    sharing_level: u8,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    sharing_policy::domain_group_key(full_cell_path, sharing_level, facet_depth)
}

/// Scale selection strategy for measuring a facet cell.
enum FacetCellMeasurementMode<'a> {
    /// Use nested sharing rules (Free/Level(N)/Shared) for nested facet cells.
    NestedSharing {
        nested_col_sharing: Option<u8>,
        nested_depth: u8,
        scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
        shared_scale_builder: &'a ScaleBuilder,
        facet_tree: &'a crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        data_df: &'a DataFrame,
        eval_ctx: &'a EvaluationContext,
    },
    /// Use an explicit scale builder (e.g., coordination re-measure path).
    ExplicitBuilder { scale_builder: &'a ScaleBuilder },
}

/// Planning data prepared once before running FacetCol measurement passes.
struct FacetColMeasurePlan {
    cell_values: Vec<ScalarValue>,
    cells: Vec<FacetCellPlan>,
    nested_col_sharing: Option<u8>,
    nested_depth: u8,
    scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder>,
    shared_scale_builder: ScaleBuilder,
}

/// Shared nested-facet measurement context used by pass 1 and pass 2.
struct FacetColNestedMeasureContext<'a> {
    nested_col_sharing: Option<u8>,
    nested_depth: u8,
    scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    shared_scale_builder: &'a ScaleBuilder,
    facet_tree: &'a crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    data_df: &'a DataFrame,
    eval_ctx: &'a EvaluationContext,
}

/// Output of pass 1 (overflow/domain extraction pass).
struct FacetColPass1Result {
    data_overrides: Vec<DataFrame>,
    cell_overflows: Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>,
    cell_domain_extents: Vec<HashMap<String, ChannelDomainExtent>>,
    empty_cells: Vec<bool>,
    max_child_padding: f32,
}

fn empty_facet_col_measurement(
    facet_path: &[ScalarValue],
    compiled_subplot: &Arc<CompiledPlot>,
    column_scale: &ConfiguredScaleWithSpec,
) -> Box<dyn CoordMeasurement> {
    Box::new(FacetColCoordMeasurement {
        cell_values: Vec::new(),
        padding_inner_px: 0.0,
        outer_left: 0.0,
        outer_right: 0.0,
        data_overrides: Vec::new(),
        shared_scale_builder: ScaleBuilder::default(),
        parent_path: facet_path.to_vec(),
        subplot_measurements: Vec::new(),
        coordinated_overflow: CoordinatedOverflow::default(),
        compiled_subplot: compiled_subplot.clone(),
        subplot_width: 0.0,
        facet_depth: facet_path.len() as u8 + 1,
        cell_domain_extents: Vec::new(),
        coordinated_domain_extents: Vec::new(),
        empty_cells: Vec::new(),
        original_column_scale: column_scale.configured().clone(),
        local_layout: CoordinatedLayout::default(),
        coordinated_layout: None,
        coordination_field_identity: "column".to_string(),
    })
}

/// Build a single facet cell context from value and parent path.
fn build_facet_cell(
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    value: &ScalarValue,
    data_df: &DataFrame,
) -> Result<FacetCellPlan, AvengerChartError> {
    let mut path = facet_path.to_vec();
    path.push(value.clone());

    let exists = facet_tree.cell_exists(&path);
    let is_empty = !exists;
    let predicate = facet_tree.cell_predicate(&path, 0);

    // Empty cells use an explicit false predicate so they preserve schema but
    // contain no rows (instead of leaking unrelated data from unfiltered input).
    let filtered_df = if is_empty {
        data_df.clone().filter(lit(false)).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to create empty DataFrame for column {:?}: {}",
                value, e
            ))
        })?
    } else if let Some(pred) = predicate {
        data_df.clone().filter(pred).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to filter data for column {:?}: {}",
                value, e
            ))
        })?
    } else {
        data_df.clone()
    };

    Ok(FacetCellPlan {
        value: value.clone(),
        full_path: path,
        exists_in_tree: exists,
        is_empty,
        filtered_df,
    })
}

/// Build a fixed-plot-area layout spec for subplot measurement.
fn fixed_plot_area_layout_spec(width: f32, height: f32) -> EvaluatedLayoutSpec {
    EvaluatedLayoutSpec {
        canvas: EvaluatedSizeMode::Auto,
        plot_area: EvaluatedSizeMode::Fixed { width, height },
        margins: EvaluatedMargins {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
    }
}

/// Measure a single facet cell using a selected scale strategy.
///
/// This is the shared measurement engine used by pass 1, pass 2, and coordinated
/// re-measurement to keep cell measurement behavior consistent.
async fn measure_facet_cell(
    cell: &FacetCellPlan,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    subplot_layout_spec: &EvaluatedLayoutSpec,
    mode: FacetCellMeasurementMode<'_>,
) -> Result<ComponentsMeasurement, AvengerChartError> {
    match mode {
        FacetCellMeasurementMode::NestedSharing {
            nested_col_sharing,
            nested_depth,
            scale_builder_cache,
            shared_scale_builder,
            facet_tree,
            data_df,
            eval_ctx,
        } => {
            let shared_scale_provider = DynamicScaleProvider {
                builder: shared_scale_builder,
                plot: compiled_subplot,
            };

            if cell.is_empty {
                return compiled_subplot
                    .measure_plot_components(
                        subplot_eval_ctx,
                        subplot_layout_spec,
                        &shared_scale_provider,
                        None,
                        &cell.full_path,
                    )
                    .await;
            }

            match nested_col_sharing {
                Some(0) => {
                    let cell_scale_builder = build_scale_builder_from_marks(
                        &compiled_subplot.marks,
                        &compiled_subplot.scale_specs,
                        &compiled_subplot.coord_transform,
                        &compiled_subplot.data,
                        Some(cell.filtered_df.clone()),
                        &eval_ctx.session_context,
                        &eval_ctx.params,
                        compiled_subplot.get_theme().as_ref(),
                    )
                    .await?;

                    let cell_scale_provider = DynamicScaleProvider {
                        builder: &cell_scale_builder,
                        plot: compiled_subplot,
                    };

                    compiled_subplot
                        .measure_plot_components(
                            subplot_eval_ctx,
                            subplot_layout_spec,
                            &cell_scale_provider,
                            Some(&cell.filtered_df),
                            &cell.full_path,
                        )
                        .await
                }
                Some(sharing_level) if sharing_level < nested_depth => {
                    // Nested sharing is defined one level deeper than `cell.full_path`,
                    // so convert Level(N) at nested depth to a truncation on the
                    // current cell path by removing `N - 1` components.
                    let ancestor_key = compute_ancestor_key(
                        &cell.full_path,
                        sharing_level.saturating_sub(1),
                        cell.full_path.len() as u8,
                    );

                    let cached_builder =
                        scale_builder_cache.get(&ancestor_key).ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "Missing cached scale builder for ancestor key {:?}",
                                ancestor_key
                            ))
                        })?;

                    let cached_scale_provider = DynamicScaleProvider {
                        builder: cached_builder,
                        plot: compiled_subplot,
                    };

                    let ancestor_filtered_df =
                        if let Some(pred) = facet_tree.path_predicate(&ancestor_key) {
                            data_df.clone().filter(pred).map_err(|e| {
                                AvengerChartError::InternalError(format!(
                                    "Failed to filter data for ancestor key {:?}: {}",
                                    ancestor_key, e
                                ))
                            })?
                        } else {
                            data_df.clone()
                        };

                    compiled_subplot
                        .measure_plot_components(
                            subplot_eval_ctx,
                            subplot_layout_spec,
                            &cached_scale_provider,
                            Some(&ancestor_filtered_df),
                            &cell.full_path,
                        )
                        .await
                }
                _ => {
                    compiled_subplot
                        .measure_plot_components(
                            subplot_eval_ctx,
                            subplot_layout_spec,
                            &shared_scale_provider,
                            Some(&cell.filtered_df),
                            &cell.full_path,
                        )
                        .await
                }
            }
        }
        FacetCellMeasurementMode::ExplicitBuilder { scale_builder } => {
            let scale_provider = DynamicScaleProvider {
                builder: scale_builder,
                plot: compiled_subplot,
            };
            let data_arg = if cell.is_empty {
                None
            } else {
                Some(&cell.filtered_df)
            };
            compiled_subplot
                .measure_plot_components(
                    subplot_eval_ctx,
                    subplot_layout_spec,
                    &scale_provider,
                    data_arg,
                    &cell.full_path,
                )
                .await
        }
    }
}

async fn build_facet_col_measure_plan(
    cell_values: Vec<ScalarValue>,
    facet_path: &[ScalarValue],
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<FacetColMeasurePlan, AvengerChartError> {
    // Build ScaleBuilder from FULL data (caches data extents for shared domain computation).
    let shared_scale_builder = build_scale_builder_from_marks(
        &compiled_subplot.marks,
        &compiled_subplot.scale_specs,
        &compiled_subplot.coord_transform,
        &compiled_subplot.data,
        Some(data_df.clone()),
        &eval_ctx.session_context,
        &eval_ctx.params,
        compiled_subplot.get_theme().as_ref(),
    )
    .await?;

    // Get nested FacetCol's sharing level for scale builder selection.
    let nested_depth = (facet_path.len() + 2) as u8; // +1 for current, +1 for nested
    let nested_col_sharing: Option<u8> = compiled_subplot.marks.iter().find_map(|m| {
        facet_mark_ref(m.as_ref())
            .and_then(|facet| facet.facet_scale_sharing())
            .map(|s| s.to_level())
    });

    // Build scale builder cache for intermediate sharing levels (0 < level < depth).
    let scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder> =
        if let Some(sharing_level) = nested_col_sharing {
            if sharing_level > 0 && sharing_level < nested_depth {
                build_ancestor_group_scale_builders(
                    &cell_values,
                    sharing_level,
                    facet_path,
                    facet_tree,
                    data_df,
                    compiled_subplot,
                    eval_ctx,
                )
                .await?
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && !scale_builder_cache.is_empty() {
        eprintln!(
            "FacetCol: built {} scale builders for sharing level {}",
            scale_builder_cache.len(),
            nested_col_sharing.unwrap_or(255)
        );
    }

    // Build canonical per-cell contexts once and reuse across pass 1/pass 2.
    let cells: Vec<FacetCellPlan> = cell_values
        .iter()
        .map(|value| build_facet_cell(facet_tree, facet_path, value, data_df))
        .collect::<Result<_, _>>()?;

    Ok(FacetColMeasurePlan {
        cell_values,
        cells,
        nested_col_sharing,
        nested_depth,
        scale_builder_cache,
        shared_scale_builder,
    })
}

async fn run_facet_col_measure_pass1(
    cells: &[FacetCellPlan],
    subplot_width: f32,
    plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<FacetColPass1Result, AvengerChartError> {
    let mut data_overrides = Vec::with_capacity(cells.len());
    let mut cell_overflows = Vec::with_capacity(cells.len());
    let mut cell_domain_extents = Vec::with_capacity(cells.len());
    let mut empty_cells = Vec::with_capacity(cells.len());
    let mut max_child_padding: f32 = 0.0;

    for (idx, cell) in cells.iter().enumerate() {
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && cell.is_empty {
            eprintln!(
                "FacetCol pass1: cell[{}]={:?} is empty (path not in tree)",
                idx, cell.value
            );
        }
        empty_cells.push(cell.is_empty);

        let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_width, plot_height);
        let measurement = measure_facet_cell(
            cell,
            compiled_subplot,
            subplot_eval_ctx,
            &subplot_layout_spec,
            FacetCellMeasurementMode::NestedSharing {
                nested_col_sharing: nested_ctx.nested_col_sharing,
                nested_depth: nested_ctx.nested_depth,
                scale_builder_cache: nested_ctx.scale_builder_cache,
                shared_scale_builder: nested_ctx.shared_scale_builder,
                facet_tree: nested_ctx.facet_tree,
                data_df: nested_ctx.data_df,
                eval_ctx: nested_ctx.eval_ctx,
            },
        )
        .await?;

        let guide_overflow = measurement.layout.overflow.clone();
        let total_overflow = measurement.layout.total_overflow.clone();

        if let Some(child_padding) = measurement.coord_measurement.padding_inner_px() {
            max_child_padding = max_child_padding.max(child_padding);
        }

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure pass 1: cell[{}]={:?} guide_overflow={{top={:.1}, bottom={:.1}, left={:.1}, right={:.1}}} total_overflow={{top={:.1}, bottom={:.1}, left={:.1}, right={:.1}}}",
                idx,
                cell.value,
                guide_overflow.top,
                guide_overflow.bottom,
                guide_overflow.left,
                guide_overflow.right,
                total_overflow.top,
                total_overflow.bottom,
                total_overflow.left,
                total_overflow.right
            );
        }

        // Extract per-cell domain extents for level-aware coordination.
        // Skip for empty cells (no data to extract domains from).
        let annotated_extents: HashMap<String, ChannelDomainExtent> = if cell.exists_in_tree {
            let cell_scale_builder = build_scale_builder_from_marks(
                &compiled_subplot.marks,
                &compiled_subplot.scale_specs,
                &compiled_subplot.coord_transform,
                &compiled_subplot.data,
                Some(cell.filtered_df.clone()),
                &nested_ctx.eval_ctx.session_context,
                &nested_ctx.eval_ctx.params,
                compiled_subplot.get_theme().as_ref(),
            )
            .await?;

            let raw_extents = cell_scale_builder.extract_domain_extents(&["x", "y", "x2", "y2"]);

            raw_extents
                .into_iter()
                .map(|(channel, extent)| {
                    let sharing_level = nested_ctx.facet_tree.channel_sharing_level_or(&channel, 0); // Free
                    (
                        channel,
                        ChannelDomainExtent {
                            extent,
                            sharing_level,
                        },
                    )
                })
                .collect()
        } else {
            HashMap::new()
        };

        cell_domain_extents.push(annotated_extents);
        data_overrides.push(cell.filtered_df.clone());
        cell_overflows.push((guide_overflow, total_overflow));
    }

    Ok(FacetColPass1Result {
        data_overrides,
        cell_overflows,
        cell_domain_extents,
        empty_cells,
        max_child_padding,
    })
}

async fn run_facet_col_measure_pass2(
    cells: &[FacetCellPlan],
    data_overrides: &[DataFrame],
    subplot_width: f32,
    plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<(Vec<ComponentsMeasurement>, Vec<bool>), AvengerChartError> {
    let mut subplot_measurements = Vec::with_capacity(cells.len());
    let mut empty_cells = Vec::with_capacity(cells.len());

    for (idx, (cell, filtered_df)) in cells.iter().zip(data_overrides.iter()).enumerate() {
        let pass2_cell = FacetCellPlan {
            value: cell.value.clone(),
            full_path: cell.full_path.clone(),
            exists_in_tree: cell.exists_in_tree,
            is_empty: cell.is_empty,
            filtered_df: filtered_df.clone(),
        };
        let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_width, plot_height);
        let measurement = measure_facet_cell(
            &pass2_cell,
            compiled_subplot,
            subplot_eval_ctx,
            &subplot_layout_spec,
            FacetCellMeasurementMode::NestedSharing {
                nested_col_sharing: nested_ctx.nested_col_sharing,
                nested_depth: nested_ctx.nested_depth,
                scale_builder_cache: nested_ctx.scale_builder_cache,
                shared_scale_builder: nested_ctx.shared_scale_builder,
                facet_tree: nested_ctx.facet_tree,
                data_df: nested_ctx.data_df,
                eval_ctx: nested_ctx.eval_ctx,
            },
        )
        .await?;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure pass 2: cell[{}]={:?} measured at width={:.1} path_exists={}",
                idx, cell.value, subplot_width, cell.exists_in_tree
            );
        }

        subplot_measurements.push(measurement);
        empty_cells.push(cell.is_empty);
    }

    Ok((subplot_measurements, empty_cells))
}

/// Build scale builders for each ancestor group based on sharing level.
///
/// For Level(N) sharing where 0 < N < depth, cells are grouped by ancestor key
/// (computed by removing the last N path components). Each group shares a single
/// scale builder built from the union of data in all cells of that group.
#[allow(clippy::too_many_arguments)]
async fn build_ancestor_group_scale_builders(
    cell_values: &[ScalarValue],
    sharing_level: u8,
    parent_path: &[ScalarValue],
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    eval_ctx: &EvaluationContext,
) -> Result<HashMap<Vec<ScalarValue>, ScaleBuilder>, AvengerChartError> {
    let mut cache = HashMap::new();

    let mut groups: HashMap<Vec<ScalarValue>, Vec<ScalarValue>> = HashMap::new();
    for value in cell_values {
        let mut full_path = parent_path.to_vec();
        full_path.push(value.clone());

        // Nested sharing is defined one level deeper than `full_path`, so convert
        // Level(N) at nested depth to truncation on current cell path by removing
        // `N - 1` components.
        let ancestor_key = compute_ancestor_key(
            &full_path,
            sharing_level.saturating_sub(1),
            full_path.len() as u8,
        );

        groups.entry(ancestor_key).or_default().push(value.clone());
    }

    // Build one scale builder per group
    for (ancestor_key, _group_values) in groups {
        // Get combined filter for all cells in group
        let group_predicate = facet_tree.path_predicate(&ancestor_key);
        let filtered_df = if let Some(pred) = group_predicate {
            data_df.clone().filter(pred).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to filter data for ancestor key {:?}: {}",
                    ancestor_key, e
                ))
            })?
        } else {
            data_df.clone()
        };

        let scale_builder = build_scale_builder_from_marks(
            &compiled_subplot.marks,
            &compiled_subplot.scale_specs,
            &compiled_subplot.coord_transform,
            &compiled_subplot.data,
            Some(filtered_df),
            &eval_ctx.session_context,
            &eval_ctx.params,
            compiled_subplot.get_theme().as_ref(),
        )
        .await?;

        cache.insert(ancestor_key, scale_builder);
    }

    Ok(cache)
}

/// Union two domain extents.
///
/// Combines the bounds of two extents to form a single extent that covers both.
/// For radius padding, takes the maximum of each direction.
pub fn union_domain_extents(a: &DomainExtent, b: &DomainExtent) -> DomainExtent {
    match (&a.bounds, &b.bounds) {
        (
            DomainBounds::Numeric {
                min: a_min,
                max: a_max,
            },
            DomainBounds::Numeric {
                min: b_min,
                max: b_max,
            },
        ) => DomainExtent {
            bounds: DomainBounds::Numeric {
                min: a_min.min(*b_min),
                max: a_max.max(*b_max),
            },
            radius: union_radius_padding(&a.radius, &b.radius),
        },
        (
            DomainBounds::Temporal {
                min: a_min,
                max: a_max,
            },
            DomainBounds::Temporal {
                min: b_min,
                max: b_max,
            },
        ) => DomainExtent {
            bounds: DomainBounds::Temporal {
                min: (*a_min).min(*b_min),
                max: (*a_max).max(*b_max),
            },
            radius: union_radius_padding(&a.radius, &b.radius),
        },
        (DomainBounds::Discrete(a_vals), DomainBounds::Discrete(b_vals)) => {
            let mut combined = a_vals.clone();
            for val in b_vals {
                if !combined.contains(val) {
                    combined.push(val.clone());
                }
            }
            DomainExtent {
                bounds: DomainBounds::Discrete(combined),
                radius: None,
            }
        }
        _ => a.clone(), // Type mismatch: keep first
    }
}

/// Union two optional radius padding values.
///
/// Takes the maximum of each direction (max_lower, max_upper).
fn union_radius_padding(
    a: &Option<RadiusPadding>,
    b: &Option<RadiusPadding>,
) -> Option<RadiusPadding> {
    match (a, b) {
        (Some(a), Some(b)) => Some(RadiusPadding {
            max_lower: a.max_lower.max(b.max_lower),
            max_upper: a.max_upper.max(b.max_upper),
        }),
        (Some(r), None) | (None, Some(r)) => Some(r.clone()),
        (None, None) => None,
    }
}

/// Row faceting coordinate system
///
/// Facets data along the row dimension, creating a vertical stack of subplots.
/// Each subplot represents one unique value from the `row` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetRow>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .row(col("species"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetRow {
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl CoordinateSystem for FacetRow {
    type Guide = FacetRowGuideConfig;

    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

fn compute_band_layout(positions: &[f32], extent: f32) -> (Vec<f32>, f32) {
    if positions.is_empty() {
        return (Vec::new(), 0.0);
    }

    let mut sorted = positions.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    // Spacing is owned by the band scale configuration (padding_inner[_px]).
    // Derive cell bandwidth directly from the positioned starts and total extent.
    let inferred_bandwidth = if sorted.len() > 1 {
        let first = sorted.first().copied().unwrap_or(0.0);
        let last = sorted.last().copied().unwrap_or(first);
        (extent - (last - first)).max(0.0)
    } else {
        extent.max(0.0)
    };

    // Fallback to min adjacent distance if inference is invalid.
    let fallback_bandwidth = sorted
        .windows(2)
        .filter_map(|pair| {
            let gap = (pair[1] - pair[0]).abs();
            if gap.is_finite() && gap > 0.0 {
                Some(gap)
            } else {
                None
            }
        })
        .fold(f32::INFINITY, f32::min);

    let bandwidth = if inferred_bandwidth.is_finite() && inferred_bandwidth > 0.0 {
        inferred_bandwidth
    } else if fallback_bandwidth.is_finite() && fallback_bandwidth > 0.0 {
        fallback_bandwidth
    } else {
        extent.max(0.0)
    };

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "compute_band_layout: positions={:?} inferred_bandwidth={:.3} fallback_bandwidth={:.3} bandwidth={:.3}",
            positions, inferred_bandwidth, fallback_bandwidth, bandwidth
        );
    }

    // Input positions are already starts (not centers), so use them directly
    let starts: Vec<f32> = positions.to_vec();

    (starts, bandwidth)
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(&self, spec: &PaddingSpec) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            PaddingSpec::Single {
                padding_px: _,
                overflow,
            } => {
                let mut updated = self.clone();
                // Band spacing is controlled directly by scale padding options.
                // Keep transform-time padding disabled to avoid double-accounting.
                updated.padding_px = None;
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let row_positions = position_channels.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' channel for FacetRow transform".into())
        })?;

        let count = row_positions.len();
        if count == 0 {
            return Ok(Box::new(SubplotGeometry::default()));
        }

        let centers = row_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_height);

        if starts.is_empty() {
            return Ok(Box::new(SubplotGeometry::default()));
        }

        // Extract actual row values from position_values (if provided)
        let row_values = position_values
            .and_then(|pv| pv.get("row"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = row_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                SubplotRect::new(value, 0.0, start, plot_width, bandwidth)
            })
            .collect();

        Ok(Box::new(SubplotGeometry::new(rects)))
    }

    fn default_range(
        &self,
        channel: &str,
        _plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "row" => Some((0.0, plot_area_height)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, DfScalarValue> {
        let mut options = HashMap::new();
        if channel == "row" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at top/bottom
            // Set inner padding to 0.1 for default spacing between facets
            options.insert(
                "padding_inner".to_string(),
                DfScalarValue::Float64(Some(0.1)),
            );
            options.insert(
                "padding_outer".to_string(),
                DfScalarValue::Float64(Some(0.0)),
            );
            options.insert("round".to_string(), DfScalarValue::Boolean(Some(true)));
        }
        options
    }
}

/// Column faceting coordinate system
///
/// Facets data along the column dimension, creating a horizontal row of subplots.
/// Each subplot represents one unique value from the `column` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetColumn>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .column(col("year"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetColumn {
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl CoordinateSystem for FacetColumn {
    type Guide = FacetColGuideConfig;

    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for FacetColumn {
    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(&self, spec: &PaddingSpec) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            PaddingSpec::Single {
                padding_px: _,
                overflow,
            } => {
                let mut updated = self.clone();
                // Band spacing is controlled directly by scale padding options.
                // Keep transform-time padding disabled to avoid double-accounting.
                updated.padding_px = None;
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
        }
    }

    async fn measure(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        _plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Find the FacetCol mark to access its subplot
        let facet_mark = compiled_marks
            .iter()
            .find_map(|m| match facet_mark_ref(m.as_ref()) {
                Some(FacetMarkRef::Col(facet_col)) => Some(facet_col),
                _ => None,
            })
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "FacetColumn coord requires a CompiledFacetCol mark".into(),
                )
            })?;

        let compiled_subplot = facet_mark.compiled_subplot();

        // Get the column scale for layout calculations
        let column_scale = scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        // Get bandwidth (subplot width) from the band scale
        let subplot_width = bandwidth(&column_scale.configured().config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        // EARLY CHECK: Get cell values from the facet tree, navigating to the correct node
        // for nested facets. We do this BEFORE requiring data so that empty cell contexts
        // (where parent passed None for data) can be detected and handled.
        let facet_tree = &eval_ctx.facet_tree;
        let current_node = if facet_path.is_empty() {
            facet_tree.root()
        } else {
            facet_tree.node_at_path(facet_path)
        };

        // If the node doesn't exist, this is an "empty cell" context from a parent facet
        // with Level(N) sharing. Return an empty measurement - the parent handles layout.
        let current_node = match current_node {
            Some(node) => node,
            None => {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetCol: path {:?} not in tree (empty cell context), returning empty measurement",
                        facet_path
                    );
                }
                return Ok(empty_facet_col_measurement(
                    facet_path,
                    compiled_subplot,
                    column_scale,
                ));
            }
        };
        let coordination_field_identity = current_node.field.clone();

        // Now we can require data (path exists, so this is not an empty cell context)
        let data_df = data.ok_or_else(|| {
            AvengerChartError::InternalError("FacetColumn measure requires data".into())
        })?;

        // Get this facet's sharing level for cell enumeration
        // Level(0) = Free (show only values in current subtree)
        // Level(N > 0) = show values from ancestor level N (may include empty cells)
        // Default is Shared (255) = show all values globally
        let current_sharing_level: u8 = facet_mark
            .facet_scale_sharing
            .map(|s| s.to_level())
            .unwrap_or(255);

        let cell_values = facet_tree
            .enumerate_values_for_facet(facet_path, current_sharing_level)
            .unwrap_or_else(|| current_node.values().cloned().collect());

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol: facet_path={:?} cell_values={:?}",
                facet_path, cell_values
            );
        }

        if cell_values.is_empty() {
            return Ok(empty_facet_col_measurement(
                facet_path,
                compiled_subplot,
                column_scale,
            ));
        }

        let plan = build_facet_col_measure_plan(
            cell_values,
            facet_path,
            data_df,
            compiled_subplot,
            facet_tree,
            eval_ctx,
        )
        .await?;
        let FacetColMeasurePlan {
            cell_values,
            cells,
            nested_col_sharing,
            nested_depth,
            scale_builder_cache,
            shared_scale_builder,
        } = plan;

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = compiled_subplot.get_default_params().clone();
            params.extend(eval_ctx.params.clone());
            eval_ctx.with_params(params)
        };

        let nested_measure_ctx = FacetColNestedMeasureContext {
            nested_col_sharing,
            nested_depth,
            scale_builder_cache: &scale_builder_cache,
            shared_scale_builder: &shared_scale_builder,
            facet_tree,
            data_df,
            eval_ctx,
        };
        let pass1 = run_facet_col_measure_pass1(
            &cells,
            subplot_width,
            plot_height,
            compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
        )
        .await?;
        let FacetColPass1Result {
            data_overrides,
            cell_overflows,
            cell_domain_extents,
            empty_cells: pass1_empty_cells,
            max_child_padding,
        } = pass1;

        // Compute padding_inner_px from adjacent total overflow combinations
        // This ensures gaps between cells accommodate both axes and legends
        // Also take max with child FacetCol padding to maintain consistent spacing in nested structures
        let padding_inner_px = compute_padding_from_overflows(
            &cell_overflows
                .iter()
                .map(|(_, total)| total.clone())
                .collect::<Vec<_>>(),
            &pass1_empty_cells,
        )
        .max(max_child_padding);

        // Compute outer edge legend-only overflows for scale range adjustment.
        // We only adjust for LEGEND overflow, not guide overflow, because:
        // - Guide overflow (axes, tick labels) is already handled by each subplot's internal layout
        // - Legend overflow extends beyond the subplot, requiring the facet to allocate extra space
        let (first_edge_idx, last_edge_idx) =
            effective_edge_indices(&pass1_empty_cells, cell_overflows.len()).unwrap_or((0, 0));
        let outer_left = cell_overflows
            .get(first_edge_idx)
            .map(|(guide, total)| (total.left - guide.left).max(0.0))
            .unwrap_or(0.0);
        let outer_right = cell_overflows
            .get(last_edge_idx)
            .map(|(guide, total)| (total.right - guide.right).max(0.0))
            .unwrap_or(0.0);

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure: padding_inner_px={:.1}, outer_left={:.1}, outer_right={:.1} from {} cells (edge_idx={}..={})",
                padding_inner_px,
                outer_left,
                outer_right,
                cell_values.len(),
                first_edge_idx,
                last_edge_idx
            );
        }

        // Capture original column scale BEFORE Pass 2 adjustments
        let original_column_scale = column_scale.configured().clone();

        // === PASS 2: Rebuild column scale with padding_inner_px, range adjustment, AND domain ===
        // This ensures measurements are computed with the final subplot width that accounts for:
        // 1. Inner padding between cells (padding_inner_px)
        // 2. Outer edge legend space (outer_left + outer_right reduce the range)
        // 3. Level(N)-aware cell_values domain (for correct facet labels)
        //
        // We must apply these adjustments here to match what apply_scale_adjustments() does during render.
        // Without this, cells would be measured at a larger width than they're rendered at,
        // and FacetColGuide would show labels from filtered data instead of enumerated cell_values.
        let mut updated_column_scale = column_scale
            .configured()
            .clone()
            .with_option("padding_inner_px", padding_inner_px);

        // Override domain with Level(N)-aware cell_values from facet tree.
        // This ensures FacetColGuide shows labels for all enumerated cells (including empty ones).
        if !cell_values.is_empty() {
            if let Ok(domain_array) = ScalarValue::iter_to_array(cell_values.iter().cloned()) {
                updated_column_scale = updated_column_scale.with_domain(domain_array);
            }
        }

        // Also reduce range for outer legend space (same logic as apply_scale_adjustments)
        // Both outer_left and outer_right reduce the total available width
        if outer_left > 0.0 || outer_right > 0.0 {
            if let Ok((range_start, range_end)) =
                updated_column_scale.config.numeric_interval_range()
            {
                // Keep start unchanged, reduce end by both outer edges
                let new_end = range_end - outer_left - outer_right;
                updated_column_scale =
                    updated_column_scale.with_range_interval((range_start, new_end));
            }
        }

        let final_subplot_width = bandwidth(&updated_column_scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get final bandwidth: {}", e))
        })?;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetCol coord measure pass 2: initial_width={:.1} -> final_width={:.1}",
                subplot_width, final_subplot_width
            );
        }

        let band_plan = FacetBandPlan {
            cells,
            padding_inner_px,
            outer_left,
            outer_right,
            n: cell_values.len(),
        };
        let local_layout = CoordinatedLayout {
            padding_inner_px: band_plan.padding_inner_px,
            outer_left: band_plan.outer_left,
            outer_right: band_plan.outer_right,
            n: band_plan.n,
        };

        let (subplot_measurements, empty_cells) = run_facet_col_measure_pass2(
            &band_plan.cells,
            &data_overrides,
            final_subplot_width,
            plot_height,
            compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
        )
        .await?;

        Ok(Box::new(FacetColCoordMeasurement {
            cell_values,
            padding_inner_px: band_plan.padding_inner_px,
            outer_left: band_plan.outer_left,
            outer_right: band_plan.outer_right,
            data_overrides,
            shared_scale_builder,
            parent_path: facet_path.to_vec(),
            subplot_measurements,
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: compiled_subplot.clone(),
            subplot_width: final_subplot_width,
            facet_depth: facet_path.len() as u8 + 1,
            cell_domain_extents,
            coordinated_domain_extents: Vec::new(),
            empty_cells,
            original_column_scale,
            local_layout,
            coordinated_layout: None,
            coordination_field_identity,
        }))
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let column_positions = position_channels.get("column").ok_or_else(|| {
            AvengerChartError::InternalError(
                "Missing 'column' channel for FacetColumn transform".into(),
            )
        })?;

        let count = column_positions.len();
        if count == 0 {
            return Ok(Box::new(SubplotGeometry::default()));
        }

        let centers = column_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_width);

        if starts.is_empty() {
            return Ok(Box::new(SubplotGeometry::default()));
        }

        // Extract actual column values from position_values (if provided)
        let column_values = position_values
            .and_then(|pv| pv.get("column"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = column_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                SubplotRect::new(value, start, 0.0, bandwidth, plot_height)
            })
            .collect();

        Ok(Box::new(SubplotGeometry::new(rects)))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        _plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "column" => Some((0.0, plot_area_width)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, DfScalarValue> {
        let mut options = HashMap::new();
        if channel == "column" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at left/right
            // Set inner padding to 0.1 for default spacing between facets
            options.insert(
                "padding_inner".to_string(),
                DfScalarValue::Float64(Some(0.1)),
            );
            options.insert(
                "padding_outer".to_string(),
                DfScalarValue::Float64(Some(0.0)),
            );
            options.insert("round".to_string(), DfScalarValue::Boolean(Some(true)));
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_row_constructs_struct() {
        let coord = FacetRow {
            padding_px: Some(5.0),
            overflow_by_facet: Some(vec![OverflowSpaceRequirement {
                top: 1.0,
                bottom: 2.0,
                left: 3.0,
                right: 4.0,
            }]),
        };
        assert_eq!(coord.padding_px, Some(5.0));
        assert!(coord.overflow_by_facet.is_some());
    }

    #[test]
    fn compute_band_layout_derives_bandwidth_from_scale_positions() {
        let positions = vec![0.0, 100.0, 200.0];
        let (starts, bandwidth) = compute_band_layout(&positions, 260.0);
        assert_eq!(starts, positions);
        assert!((bandwidth - 60.0).abs() < 0.01);
    }

    #[test]
    fn compute_band_layout_falls_back_when_inference_invalid() {
        let positions = vec![10.0, 20.0, 30.0];
        let (_starts, bandwidth) = compute_band_layout(&positions, 0.0);
        assert!((bandwidth - 10.0).abs() < 0.01);
    }
}
