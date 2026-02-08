use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, band::bandwidth};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::lit,
    scalar::ScalarValue as DfScalarValue,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::{
    coords::{
        CellDomainInfo, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform,
        CoordinatedLayout, CoordinatedOverflow, OverflowSpaceRequirement, PaddingSpec,
        PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::{
        coord_row::compute_band_layout,
        coordination::CoordinationGroupKey,
        guide::FacetColGuideConfig,
        layout_plan::{
            FacetBandPlan, FacetCellPlan, compute_padding_from_overflows, effective_edge_indices,
        },
        marks::facet::{FacetMarkRef, facet_mark_ref},
        path_math, sharing_policy,
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

pub use crate::facet::coord_row::FacetRow;

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

/// Runtime state for a single enumerated facet cell.
#[derive(Debug)]
pub(crate) struct FacetCellRuntime {
    /// Canonical plan metadata (value, path, emptiness, filter intent).
    pub(crate) plan: FacetCellPlan,
    /// Per-cell dataframe override used by render and coordinated re-measure paths.
    pub(crate) data_override: DataFrame,
    /// Final subplot measurement used for rendering.
    pub(crate) measurement: ComponentsMeasurement,
    /// Per-channel local domain extents extracted after final geometry is known.
    pub(crate) local_domain_extents: HashMap<String, ChannelDomainExtent>,
    /// Coordinated per-channel extents distributed during coordination.
    pub(crate) coordinated_domain_extents: HashMap<String, DomainExtent>,
}

/// Measurement data for FacetColumn coordinate system.
///
/// This captures the computed padding from overflow measurement and pre-computed
/// subplot measurements. Marks use the stored measurements directly without re-measuring.
pub struct FacetColCoordMeasurement {
    /// Enumerated cell runtime state.
    pub(crate) cells: Vec<FacetCellRuntime>,
    /// ScaleBuilder for shared scales (caches data extents for rebuilding with updated dimensions).
    /// Used with DynamicScaleProvider to correctly compute radius-aware domains.
    pub shared_scale_builder: ScaleBuilder,
    /// Stable field identity for coordination grouping at this facet level.
    /// This is typically the facet column field name.
    pub coordination_field_identity: String,
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
    /// Column scale (as ConfiguredScale) BEFORE Pass 2 adjustments.
    /// Used to recompute subplot width when coordinated layout differs from local.
    pub original_column_scale: ConfiguredScale,
    /// Local layout values (pre-coordination).
    pub local_layout: CoordinatedLayout,
    /// Coordinated layout values (post-coordination). None before coordination.
    pub coordinated_layout: Option<CoordinatedLayout>,
}

impl FacetColCoordMeasurement {
    fn active_layout(&self) -> &CoordinatedLayout {
        self.coordinated_layout
            .as_ref()
            .unwrap_or(&self.local_layout)
    }

    pub fn cell_values(&self) -> impl Iterator<Item = &ScalarValue> {
        self.cells.iter().map(|cell| &cell.plan.value)
    }

    pub fn child_measurements_iter(&self) -> impl Iterator<Item = &ComponentsMeasurement> {
        self.cells.iter().map(|cell| &cell.measurement)
    }

    pub fn child_measurements_iter_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut ComponentsMeasurement> {
        self.cells.iter_mut().map(|cell| &mut cell.measurement)
    }

    pub fn local_overflow_value(&self) -> Option<CoordinatedOverflow> {
        let cell_count = self.cells.len();
        if cell_count == 0 {
            return None;
        }

        let first_idx = self
            .cells
            .iter()
            .position(|cell| !cell.plan.is_empty)
            .unwrap_or(0);
        let last_idx = self
            .cells
            .iter()
            .rposition(|cell| !cell.plan.is_empty)
            .unwrap_or(cell_count.saturating_sub(1));

        let first_layout = &self.cells[first_idx].measurement.layout;
        let last_layout = &self.cells[last_idx].measurement.layout;

        let guide = OverflowSpaceRequirement {
            top: self
                .cells
                .iter()
                .map(|cell| cell.measurement.layout.overflow.top)
                .fold(0.0f32, f32::max),
            bottom: self
                .cells
                .iter()
                .map(|cell| cell.measurement.layout.overflow.bottom)
                .fold(0.0f32, f32::max),
            left: first_layout.overflow.left,
            right: last_layout.overflow.right,
        };

        let total = OverflowSpaceRequirement {
            top: self
                .cells
                .iter()
                .map(|cell| cell.measurement.layout.total_overflow.top)
                .fold(0.0f32, f32::max),
            bottom: self
                .cells
                .iter()
                .map(|cell| cell.measurement.layout.total_overflow.bottom)
                .fold(0.0f32, f32::max),
            left: first_layout.total_overflow.left,
            right: last_layout.total_overflow.right,
        };

        Some(CoordinatedOverflow { guide, total })
    }

    pub fn local_layout_value(&self) -> CoordinatedLayout {
        self.local_layout.clone()
    }

    pub fn set_coordinated_overflow_value(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_overflow = overflow;
    }

    pub fn set_coordinated_layout_value(&mut self, layout: CoordinatedLayout) {
        self.coordinated_layout = Some(layout);
    }

    pub fn coordinated_subplot_width_value(&self) -> Option<f32> {
        if self.subplot_width > 0.0 {
            Some(self.subplot_width)
        } else {
            None
        }
    }

    pub fn set_parent_bandwidth_value(&mut self, bandwidth: f32) {
        if bandwidth > 0.0 {
            self.original_column_scale = self
                .original_column_scale
                .clone()
                .with_range_interval((0.0, bandwidth));

            debug!(
                bandwidth,
                "FacetCol set_parent_bandwidth updated original column scale range"
            );
        }
    }
}

impl CoordMeasurement for FacetColCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        Some(&self.coordinated_overflow)
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
            let has_empty_cells = self.cells.iter().any(|cell| cell.plan.is_empty);
            let has_adjacent_non_empty = self
                .cells
                .windows(2)
                .any(|pair| !pair[0].plan.is_empty && !pair[1].plan.is_empty);
            let needs_zero_padding_override = has_empty_cells && !has_adjacent_non_empty;
            let cell_values: Vec<ScalarValue> = self.cell_values().cloned().collect();
            let domain_override = if cell_values.is_empty() {
                None
            } else {
                Some(cell_values.as_slice())
            };

            // Always start from the original measured column scale so repeated
            // coordination passes re-apply the same layout adjustments
            // deterministically without shrinking the range multiple times.
            let base_scale = self.original_column_scale.clone();

            // NOTE: We do NOT set band_n on the column scale here. The band_n option
            // is only used during measurement (apply_coordinated_overflow) to compute
            // the correct subplot_width. For rendering, the column scale should position
            // its actual domain cells evenly across the full allocated width.
            let updated_config = apply_facet_col_scale_layout(
                &base_scale,
                self.active_layout(),
                domain_override,
                None,
                false,
                needs_zero_padding_override,
            );

            *column_scale =
                ConfiguredScaleWithSpec::new(column_scale.spec().clone(), updated_config);
        }
    }
}

/// Canonical facet-column scale rewrite helper shared by measurement and render paths.
fn apply_facet_col_scale_layout(
    base: &ConfiguredScale,
    layout: &CoordinatedLayout,
    domain_override: Option<&[ScalarValue]>,
    band_n_override: Option<usize>,
    set_padding_always: bool,
    allow_zero_padding_override: bool,
) -> ConfiguredScale {
    let mut updated = base.clone();

    if let Some(domain_values) = domain_override {
        if !domain_values.is_empty() {
            if let Ok(domain_array) = ScalarValue::iter_to_array(domain_values.iter().cloned()) {
                updated = updated.with_domain(domain_array);
            }
        }
    }

    if let Some(band_n) = band_n_override {
        updated = updated.with_option("band_n", band_n as i32);
    }

    if layout.padding_inner_px > 0.0 || set_padding_always || allow_zero_padding_override {
        updated = updated.with_option("padding_inner_px", layout.padding_inner_px);
    }

    if (layout.outer_left > 0.0 || layout.outer_right > 0.0)
        && let Ok((range_start, range_end)) = updated.config.numeric_interval_range()
    {
        let new_end = range_end - layout.outer_left - layout.outer_right;
        updated = updated.with_range_interval((range_start, new_end));
    }

    updated
}

fn has_coordinated_layout_change(
    local_layout: &CoordinatedLayout,
    coordinated_layout: Option<&CoordinatedLayout>,
) -> bool {
    coordinated_layout.is_some_and(|coordinated| {
        coordinated.n != local_layout.n
            || (coordinated.padding_inner_px - local_layout.padding_inner_px).abs() > 0.01
            || (coordinated.outer_left - local_layout.outer_left).abs() > 0.01
            || (coordinated.outer_right - local_layout.outer_right).abs() > 0.01
    })
}

fn should_remeasure_cells(has_legend_overflow: bool, has_coordinated_extents: bool) -> bool {
    has_legend_overflow || has_coordinated_extents
}

#[cfg(test)]
fn layout_from_measurement_or_local(
    local_layout: &CoordinatedLayout,
    coordinated_layout: Option<&CoordinatedLayout>,
) -> CoordinatedLayout {
    coordinated_layout
        .cloned()
        .unwrap_or_else(|| local_layout.clone())
}

impl FacetColCoordMeasurement {
    pub fn coordination_group_key_for_depth(&self, depth: usize) -> CoordinationGroupKey {
        CoordinationGroupKey::new(depth, format!("col:{}", self.coordination_field_identity))
    }

    pub fn collect_cell_domain_infos(&self, collector: &mut Vec<CellDomainInfo>) {
        for cell in &self.cells {
            for (channel, annotated) in &cell.local_domain_extents {
                collector.push(CellDomainInfo {
                    full_cell_path: cell.plan.full_path.clone(),
                    channel: channel.clone(),
                    sharing_level: annotated.sharing_level,
                    facet_depth: self.facet_depth,
                    extent: annotated.extent.clone(),
                });
            }
        }
    }

    pub fn distribute_coordinated_domain_extents(
        &mut self,
        unified: &HashMap<(String, Vec<ScalarValue>), DomainExtent>,
    ) {
        for cell in &mut self.cells {
            cell.coordinated_domain_extents.clear();

            for (channel, annotated) in &cell.local_domain_extents {
                if annotated.sharing_level == 0 {
                    continue;
                }

                let ancestor_key = sharing_policy::domain_group_key(
                    &cell.plan.full_path,
                    annotated.sharing_level,
                    self.facet_depth,
                );

                if let Some(unified_extent) = unified.get(&(channel.clone(), ancestor_key)) {
                    cell.coordinated_domain_extents
                        .insert(channel.clone(), unified_extent.clone());
                }
            }
        }
    }

    pub async fn apply_coordinated_overflow(
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
            .cells
            .iter()
            .any(|cell| !cell.coordinated_domain_extents.is_empty());

        // Check if coordinated layout differs from local layout
        let has_coordinated_layout =
            has_coordinated_layout_change(&self.local_layout, self.coordinated_layout.as_ref());

        // If coordinated layout changed, recompute subplot_width using band_n.
        // This only updates the width/padding fields — it does NOT trigger re-measurement.
        // Re-measurement would rebuild scales from scratch, changing y-domains.
        if has_coordinated_layout {
            let layout = self.coordinated_layout.as_ref().unwrap();
            let scale = apply_facet_col_scale_layout(
                &self.original_column_scale,
                layout,
                None,
                Some(layout.n),
                true,
                false,
            );

            // Recompute subplot_width from coordinated scale
            let new_subplot_width = bandwidth(&scale.config).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to get coordinated bandwidth: {}",
                    e
                ))
            })?;

            debug!(
                old_width = self.subplot_width,
                new_width = new_subplot_width,
                local_n = self.local_layout.n,
                coordinated_n = layout.n,
                local_padding = self.local_layout.padding_inner_px,
                coordinated_padding = layout.padding_inner_px,
                "FacetCol apply_coordinated_overflow layout coordination"
            );

            self.subplot_width = new_subplot_width;
        }

        // Only do full re-measurement for legend overflow or coordinated domain extents.
        // Layout coordination (above) only adjusts subplot_width without re-measuring,
        // because re-measurement rebuilds scales from scratch and can change y-domains.
        if !should_remeasure_cells(has_legend_overflow, has_coordinated_extents) {
            return Ok(());
        }

        // Get original height from first measurement (all should be same)
        let original_height = self
            .cells
            .first()
            .map(|cell| cell.measurement.plot_area_height)
            .unwrap_or(0.0);

        let adjusted_height = if has_legend_overflow {
            (original_height - legend_top - legend_bottom).max(1.0)
        } else {
            original_height
        };

        debug!(
            original_height,
            adjusted_height,
            legend_top,
            legend_bottom,
            has_coordinated_extents,
            "FacetCol apply_coordinated_overflow height adjustment"
        );

        // Create subplot EvaluationContext with merged params
        let subplot_eval_ctx = {
            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(eval_ctx.params.clone());
            eval_ctx.with_params(params)
        };

        let compiled_subplot = self.compiled_subplot.clone();
        let shared_scale_builder = self.shared_scale_builder.clone();
        for (idx, cell) in self.cells.iter_mut().enumerate() {
            let subplot_layout_spec =
                fixed_plot_area_layout_spec(self.subplot_width, adjusted_height);
            let mut builder = shared_scale_builder.clone();
            if !cell.coordinated_domain_extents.is_empty() {
                builder.extend_with_domain_extents(&cell.coordinated_domain_extents);
            }

            let mode = FacetCellMeasurementMode::ExplicitBuilder {
                scale_builder: &builder,
            };
            let MeasuredFacetCell { measurement, .. } = measure_facet_cell(
                &cell.plan,
                &cell.data_override,
                &compiled_subplot,
                &subplot_eval_ctx,
                &subplot_layout_spec,
                mode,
            )
            .await?;

            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                width = self.subplot_width,
                height = adjusted_height,
                is_empty = cell.plan.is_empty,
                "FacetCol apply_coordinated_overflow re-measured cell"
            );
            cell.measurement = measurement;
        }
        Ok(())
    }
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

struct MeasuredFacetCell {
    measurement: ComponentsMeasurement,
    cell_scale_builder: Option<ScaleBuilder>,
}

struct FacetCellDraft {
    plan: FacetCellPlan,
    data_override: DataFrame,
    measurement: Option<ComponentsMeasurement>,
    local_domain_extents: HashMap<String, ChannelDomainExtent>,
}

enum NestedScalePlan<'a> {
    Shared,
    PerCellBuilder,
    AncestorCachedBuilder {
        cached_builder: &'a ScaleBuilder,
        ancestor_filtered_df: DataFrame,
    },
    EmptySharedNoData,
}

/// Planning data prepared once before running FacetCol measurement passes.
struct FacetColMeasurePlan {
    cell_values: Vec<ScalarValue>,
    cells: Vec<FacetCellDraft>,
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

#[derive(Default)]
struct OverflowProbeSummary {
    cell_overflows: Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>,
    max_child_padding: f32,
}

fn empty_facet_col_measurement(
    facet_path: &[ScalarValue],
    compiled_subplot: &Arc<CompiledPlot>,
    column_scale: &ConfiguredScaleWithSpec,
) -> Box<dyn CoordMeasurement> {
    Box::new(FacetColCoordMeasurement {
        cells: Vec::new(),
        shared_scale_builder: ScaleBuilder::default(),
        coordinated_overflow: CoordinatedOverflow::default(),
        compiled_subplot: compiled_subplot.clone(),
        subplot_width: 0.0,
        facet_depth: facet_path.len() as u8 + 1,
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
) -> Result<FacetCellPlan, AvengerChartError> {
    let mut path = facet_path.to_vec();
    path.push(value.clone());

    let exists = facet_tree.cell_exists(&path);
    let is_empty = !exists;
    let filter_predicate = if is_empty {
        Some(lit(false))
    } else {
        facet_tree.cell_predicate(&path, 0)
    };

    Ok(FacetCellPlan {
        value: value.clone(),
        full_path: path,
        is_empty,
        filter_predicate,
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

fn resolve_nested_scale_plan<'a>(
    cell: &FacetCellPlan,
    nested_col_sharing: Option<u8>,
    nested_depth: u8,
    scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    data_df: &DataFrame,
) -> Result<NestedScalePlan<'a>, AvengerChartError> {
    if cell.is_empty {
        return Ok(NestedScalePlan::EmptySharedNoData);
    }

    match nested_col_sharing {
        Some(0) => Ok(NestedScalePlan::PerCellBuilder),
        Some(sharing_level) if sharing_level < nested_depth => {
            let ancestor_key = path_math::nested_measurement_ancestor_key(
                &cell.full_path,
                sharing_level,
                nested_depth,
            );
            let cached_builder = scale_builder_cache.get(&ancestor_key).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing cached scale builder for ancestor key {:?}",
                    ancestor_key
                ))
            })?;

            let ancestor_filtered_df = if let Some(pred) = facet_tree.path_predicate(&ancestor_key)
            {
                data_df.clone().filter(pred).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to filter data for ancestor key {:?}: {}",
                        ancestor_key, e
                    ))
                })?
            } else {
                data_df.clone()
            };

            Ok(NestedScalePlan::AncestorCachedBuilder {
                cached_builder,
                ancestor_filtered_df,
            })
        }
        _ => Ok(NestedScalePlan::Shared),
    }
}

async fn execute_measurement_from_plan(
    plan: NestedScalePlan<'_>,
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    subplot_layout_spec: &EvaluatedLayoutSpec,
    shared_scale_builder: &ScaleBuilder,
    eval_ctx: &EvaluationContext,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    let shared_scale_provider = DynamicScaleProvider {
        builder: shared_scale_builder,
        plot: compiled_subplot,
    };

    match plan {
        NestedScalePlan::EmptySharedNoData => compiled_subplot
            .measure_plot_components(
                subplot_eval_ctx,
                subplot_layout_spec,
                &shared_scale_provider,
                None,
                &cell.full_path,
            )
            .await
            .map(|measurement| MeasuredFacetCell {
                measurement,
                cell_scale_builder: None,
            }),
        NestedScalePlan::PerCellBuilder => {
            let cell_scale_builder = build_scale_builder_from_marks(
                &compiled_subplot.marks,
                &compiled_subplot.scale_specs,
                &compiled_subplot.coord_transform,
                &compiled_subplot.data,
                Some(data_override.clone()),
                &eval_ctx.session_context,
                &eval_ctx.params,
                compiled_subplot.get_theme().as_ref(),
            )
            .await?;
            let cell_scale_provider = DynamicScaleProvider {
                builder: &cell_scale_builder,
                plot: compiled_subplot,
            };

            let measurement = compiled_subplot
                .measure_plot_components(
                    subplot_eval_ctx,
                    subplot_layout_spec,
                    &cell_scale_provider,
                    Some(data_override),
                    &cell.full_path,
                )
                .await?;

            Ok(MeasuredFacetCell {
                measurement,
                cell_scale_builder: Some(cell_scale_builder),
            })
        }
        NestedScalePlan::AncestorCachedBuilder {
            cached_builder,
            ancestor_filtered_df,
        } => {
            let cached_scale_provider = DynamicScaleProvider {
                builder: cached_builder,
                plot: compiled_subplot,
            };
            let measurement = compiled_subplot
                .measure_plot_components(
                    subplot_eval_ctx,
                    subplot_layout_spec,
                    &cached_scale_provider,
                    Some(&ancestor_filtered_df),
                    &cell.full_path,
                )
                .await?;

            Ok(MeasuredFacetCell {
                measurement,
                cell_scale_builder: None,
            })
        }
        NestedScalePlan::Shared => {
            let measurement = compiled_subplot
                .measure_plot_components(
                    subplot_eval_ctx,
                    subplot_layout_spec,
                    &shared_scale_provider,
                    Some(data_override),
                    &cell.full_path,
                )
                .await?;

            Ok(MeasuredFacetCell {
                measurement,
                cell_scale_builder: None,
            })
        }
    }
}

/// Measure a single facet cell using a selected scale strategy.
///
/// This is the shared measurement engine used by pass 1, pass 2, and coordinated
/// re-measurement to keep cell measurement behavior consistent.
async fn measure_facet_cell(
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    subplot_layout_spec: &EvaluatedLayoutSpec,
    mode: FacetCellMeasurementMode<'_>,
) -> Result<MeasuredFacetCell, AvengerChartError> {
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
            let plan = resolve_nested_scale_plan(
                cell,
                nested_col_sharing,
                nested_depth,
                scale_builder_cache,
                facet_tree,
                data_df,
            )?;
            execute_measurement_from_plan(
                plan,
                cell,
                data_override,
                compiled_subplot,
                subplot_eval_ctx,
                subplot_layout_spec,
                shared_scale_builder,
                eval_ctx,
            )
            .await
        }
        FacetCellMeasurementMode::ExplicitBuilder { scale_builder } => {
            let scale_provider = DynamicScaleProvider {
                builder: scale_builder,
                plot: compiled_subplot,
            };
            let data_arg = if cell.is_empty {
                None
            } else {
                Some(data_override)
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
                .map(|measurement| MeasuredFacetCell {
                    measurement,
                    cell_scale_builder: None,
                })
        }
    }
}

async fn measure_nested_cell(
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    subplot_width: f32,
    plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_width, plot_height);
    let mode = FacetCellMeasurementMode::NestedSharing {
        nested_col_sharing: nested_ctx.nested_col_sharing,
        nested_depth: nested_ctx.nested_depth,
        scale_builder_cache: nested_ctx.scale_builder_cache,
        shared_scale_builder: nested_ctx.shared_scale_builder,
        facet_tree: nested_ctx.facet_tree,
        data_df: nested_ctx.data_df,
        eval_ctx: nested_ctx.eval_ctx,
    };
    measure_facet_cell(
        cell,
        data_override,
        compiled_subplot,
        subplot_eval_ctx,
        &subplot_layout_spec,
        mode,
    )
    .await
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

    if !scale_builder_cache.is_empty() {
        debug!(
            scale_builder_count = scale_builder_cache.len(),
            sharing_level = nested_col_sharing.unwrap_or(255),
            "FacetCol built cached scale builders"
        );
    }

    // Build canonical per-cell contexts once and reuse across pass 1/pass 2.
    let cell_plans: Vec<FacetCellPlan> = cell_values
        .iter()
        .map(|value| build_facet_cell(facet_tree, facet_path, value))
        .collect::<Result<_, _>>()?;
    let cells: Vec<FacetCellDraft> = cell_plans
        .into_iter()
        .map(|plan| {
            let data_override = if let Some(predicate) = plan.filter_predicate.clone() {
                data_df.clone().filter(predicate).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to filter data for facet cell {:?}: {}",
                        plan.value, e
                    ))
                })?
            } else {
                data_df.clone()
            };

            Ok(FacetCellDraft {
                plan,
                data_override,
                measurement: None,
                local_domain_extents: HashMap::new(),
            })
        })
        .collect::<Result<_, AvengerChartError>>()?;

    Ok(FacetColMeasurePlan {
        cell_values,
        cells,
        nested_col_sharing,
        nested_depth,
        scale_builder_cache,
        shared_scale_builder,
    })
}

async fn build_extent_builder_for_cell(
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<ScaleBuilder, AvengerChartError> {
    build_scale_builder_from_marks(
        &compiled_subplot.marks,
        &compiled_subplot.scale_specs,
        &compiled_subplot.coord_transform,
        &compiled_subplot.data,
        Some(data_override.clone()),
        &nested_ctx.eval_ctx.session_context,
        &nested_ctx.eval_ctx.params,
        compiled_subplot.get_theme().as_ref(),
    )
    .await
}

fn annotate_domain_extents(
    raw_extents: HashMap<String, DomainExtent>,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> HashMap<String, ChannelDomainExtent> {
    raw_extents
        .into_iter()
        .map(|(channel, extent)| {
            let sharing_level = nested_ctx.facet_tree.channel_sharing_level_or(&channel, 0);
            (
                channel,
                ChannelDomainExtent {
                    extent,
                    sharing_level,
                },
            )
        })
        .collect()
}

async fn measure_cells_overflow_probe(
    cells: &[FacetCellDraft],
    subplot_width: f32,
    plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<OverflowProbeSummary, AvengerChartError> {
    let mut summary = OverflowProbeSummary::default();
    summary.cell_overflows.reserve(cells.len());

    for (idx, cell) in cells.iter().enumerate() {
        if cell.plan.is_empty {
            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                "FacetCol overflow probe empty cell"
            );
        }

        let MeasuredFacetCell { measurement, .. } = measure_nested_cell(
            &cell.plan,
            &cell.data_override,
            subplot_width,
            plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_ctx,
        )
        .await?;

        let guide_overflow = measurement.layout.overflow.clone();
        let total_overflow = measurement.layout.total_overflow.clone();

        if let Some(child_facet_col) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetColCoordMeasurement>()
        {
            summary.max_child_padding = summary
                .max_child_padding
                .max(child_facet_col.active_layout().padding_inner_px);
        }

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            guide_top = guide_overflow.top,
            guide_bottom = guide_overflow.bottom,
            guide_left = guide_overflow.left,
            guide_right = guide_overflow.right,
            total_top = total_overflow.top,
            total_bottom = total_overflow.bottom,
            total_left = total_overflow.left,
            total_right = total_overflow.right,
            "FacetCol overflow probe result"
        );

        summary
            .cell_overflows
            .push((guide_overflow, total_overflow));
    }

    Ok(summary)
}

async fn measure_cells_final_and_extents(
    cells: &mut [FacetCellDraft],
    subplot_width: f32,
    plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetColNestedMeasureContext<'_>,
) -> Result<(), AvengerChartError> {
    for (idx, cell) in cells.iter_mut().enumerate() {
        if cell.plan.is_empty {
            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                "FacetCol final measure empty cell"
            );
        }

        let MeasuredFacetCell {
            measurement,
            cell_scale_builder,
        } = measure_nested_cell(
            &cell.plan,
            &cell.data_override,
            subplot_width,
            plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_ctx,
        )
        .await?;

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            subplot_width,
            is_empty = cell.plan.is_empty,
            "FacetCol final measure result"
        );

        let local_extents = if !cell.plan.is_empty {
            let extent_builder = if let Some(builder) = cell_scale_builder {
                builder
            } else {
                build_extent_builder_for_cell(&cell.data_override, compiled_subplot, nested_ctx)
                    .await?
            };
            annotate_domain_extents(
                extent_builder.extract_domain_extents(&["x", "y", "x2", "y2"]),
                nested_ctx,
            )
        } else {
            HashMap::new()
        };

        cell.measurement = Some(measurement);
        cell.local_domain_extents = local_extents;
    }

    Ok(())
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

        let ancestor_key = path_math::nested_measurement_ancestor_key(
            &full_path,
            sharing_level,
            full_path.len() as u8 + 1,
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
pub struct FacetColumn;

struct FacetColMeasurePipeline<'a> {
    scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    plot_height: f32,
    eval_ctx: &'a EvaluationContext,
    data: Option<&'a DataFrame>,
    compiled_marks: &'a [Arc<dyn CompiledMark>],
    facet_path: &'a [ScalarValue],
}

struct FacetColResolvedNode<'a> {
    compiled_subplot: &'a Arc<CompiledPlot>,
    column_scale: &'a ConfiguredScaleWithSpec,
    subplot_width: f32,
    current_sharing_level: u8,
    coordination_field_identity: String,
}

enum ResolveNodeOutcome<'a> {
    Empty(Box<dyn CoordMeasurement>),
    Ready(FacetColResolvedNode<'a>),
}

impl<'a> FacetColMeasurePipeline<'a> {
    fn new(
        scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
        plot_height: f32,
        eval_ctx: &'a EvaluationContext,
        data: Option<&'a DataFrame>,
        compiled_marks: &'a [Arc<dyn CompiledMark>],
        facet_path: &'a [ScalarValue],
    ) -> Self {
        Self {
            scales,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        }
    }

    async fn run(&self) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        let resolved = match self.resolve_node_or_empty()? {
            ResolveNodeOutcome::Empty(measurement) => return Ok(measurement),
            ResolveNodeOutcome::Ready(resolved) => resolved,
        };
        let cell_values = self.enumerate_cell_values(&resolved);

        debug!(
            facet_path = ?self.facet_path,
            cell_values = ?cell_values,
            "FacetCol enumerated cell values"
        );

        if cell_values.is_empty() {
            return Ok(empty_facet_col_measurement(
                self.facet_path,
                resolved.compiled_subplot,
                resolved.column_scale,
            ));
        }

        let data_df = self.data.ok_or_else(|| {
            AvengerChartError::InternalError("FacetColumn measure requires data".into())
        })?;

        let mut plan = self
            .build_measure_plan(cell_values, data_df, resolved.compiled_subplot)
            .await?;

        let subplot_eval_ctx = {
            let mut params = resolved.compiled_subplot.get_default_params().clone();
            params.extend(self.eval_ctx.params.clone());
            self.eval_ctx.with_params(params)
        };

        let nested_measure_ctx = FacetColNestedMeasureContext {
            nested_col_sharing: plan.nested_col_sharing,
            nested_depth: plan.nested_depth,
            scale_builder_cache: &plan.scale_builder_cache,
            shared_scale_builder: &plan.shared_scale_builder,
            facet_tree: &self.eval_ctx.facet_tree,
            data_df,
            eval_ctx: self.eval_ctx,
        };

        let pass1 = self
            .probe_overflow(
                &plan.cells,
                resolved.subplot_width,
                resolved.compiled_subplot,
                &subplot_eval_ctx,
                &nested_measure_ctx,
            )
            .await?;

        let band_plan = self.derive_layout_plan(&plan.cells, &plan.cell_values, &pass1);
        let final_subplot_width = self.build_pass2_scale(
            resolved.column_scale,
            &band_plan,
            &plan.cell_values,
            resolved.subplot_width,
        )?;

        self.measure_final_cells(
            &mut plan.cells,
            final_subplot_width,
            resolved.compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
        )
        .await?;

        Ok(self.build_coord_measurement(
            band_plan,
            plan.cells,
            plan.shared_scale_builder,
            resolved.compiled_subplot,
            final_subplot_width,
            resolved.column_scale,
            resolved.coordination_field_identity,
        ))
    }

    fn resolve_node_or_empty(&self) -> Result<ResolveNodeOutcome<'a>, AvengerChartError> {
        let facet_mark = self
            .compiled_marks
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

        let column_scale = self
            .scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        let subplot_width = bandwidth(&column_scale.configured().config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
        })?;

        let facet_tree = &self.eval_ctx.facet_tree;
        let current_node = if self.facet_path.is_empty() {
            facet_tree.root()
        } else {
            facet_tree.node_at_path(self.facet_path)
        };

        let Some(current_node) = current_node else {
            debug!(
                facet_path = ?self.facet_path,
                "FacetCol path not present in tree; returning empty measurement"
            );
            return Ok(ResolveNodeOutcome::Empty(empty_facet_col_measurement(
                self.facet_path,
                compiled_subplot,
                column_scale,
            )));
        };

        let current_sharing_level: u8 = facet_mark
            .facet_scale_sharing()
            .map(|s| s.to_level())
            .unwrap_or(255);

        Ok(ResolveNodeOutcome::Ready(FacetColResolvedNode {
            compiled_subplot,
            column_scale,
            subplot_width,
            current_sharing_level,
            coordination_field_identity: current_node.field.clone(),
        }))
    }

    fn enumerate_cell_values(&self, resolved: &FacetColResolvedNode<'_>) -> Vec<ScalarValue> {
        let facet_tree = &self.eval_ctx.facet_tree;
        let current_node = if self.facet_path.is_empty() {
            facet_tree.root()
        } else {
            facet_tree.node_at_path(self.facet_path)
        };

        let fallback = || {
            current_node
                .map(|node| node.values().cloned().collect())
                .unwrap_or_default()
        };

        facet_tree
            .enumerate_values_for_facet(self.facet_path, resolved.current_sharing_level)
            .unwrap_or_else(fallback)
    }

    async fn build_measure_plan(
        &self,
        cell_values: Vec<ScalarValue>,
        data_df: &DataFrame,
        compiled_subplot: &Arc<CompiledPlot>,
    ) -> Result<FacetColMeasurePlan, AvengerChartError> {
        build_facet_col_measure_plan(
            cell_values,
            self.facet_path,
            data_df,
            compiled_subplot,
            &self.eval_ctx.facet_tree,
            self.eval_ctx,
        )
        .await
    }

    async fn probe_overflow(
        &self,
        cells: &[FacetCellDraft],
        subplot_width: f32,
        compiled_subplot: &Arc<CompiledPlot>,
        subplot_eval_ctx: &EvaluationContext,
        nested_measure_ctx: &FacetColNestedMeasureContext<'_>,
    ) -> Result<OverflowProbeSummary, AvengerChartError> {
        measure_cells_overflow_probe(
            cells,
            subplot_width,
            self.plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_measure_ctx,
        )
        .await
    }

    fn derive_layout_plan(
        &self,
        cells: &[FacetCellDraft],
        cell_values: &[ScalarValue],
        pass1: &OverflowProbeSummary,
    ) -> FacetBandPlan {
        let pass1_empty_cells: Vec<bool> = cells.iter().map(|cell| cell.plan.is_empty).collect();

        let padding_inner_px = compute_padding_from_overflows(
            &pass1
                .cell_overflows
                .iter()
                .map(|(_, total)| total.clone())
                .collect::<Vec<_>>(),
            &pass1_empty_cells,
        )
        .max(pass1.max_child_padding);

        let (first_edge_idx, last_edge_idx) =
            effective_edge_indices(&pass1_empty_cells, pass1.cell_overflows.len())
                .unwrap_or((0, 0));
        let outer_left = pass1
            .cell_overflows
            .get(first_edge_idx)
            .map(|(guide, total)| (total.left - guide.left).max(0.0))
            .unwrap_or(0.0);
        let outer_right = pass1
            .cell_overflows
            .get(last_edge_idx)
            .map(|(guide, total)| (total.right - guide.right).max(0.0))
            .unwrap_or(0.0);

        debug!(
            padding_inner_px,
            outer_left,
            outer_right,
            cell_count = cell_values.len(),
            first_edge_idx,
            last_edge_idx,
            "FacetCol derived local layout"
        );

        FacetBandPlan {
            padding_inner_px,
            outer_left,
            outer_right,
            n: cell_values.len(),
        }
    }

    fn build_pass2_scale(
        &self,
        column_scale: &ConfiguredScaleWithSpec,
        band_plan: &FacetBandPlan,
        cell_values: &[ScalarValue],
        initial_subplot_width: f32,
    ) -> Result<f32, AvengerChartError> {
        let pass2_layout = CoordinatedLayout {
            padding_inner_px: band_plan.padding_inner_px,
            outer_left: band_plan.outer_left,
            outer_right: band_plan.outer_right,
            n: band_plan.n,
        };

        let updated_column_scale = apply_facet_col_scale_layout(
            column_scale.configured(),
            &pass2_layout,
            Some(cell_values),
            None,
            true,
            false,
        );

        let final_subplot_width = bandwidth(&updated_column_scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get final bandwidth: {}", e))
        })?;

        debug!(
            initial_width = initial_subplot_width,
            final_width = final_subplot_width,
            "FacetCol pass 2 scale bandwidth"
        );

        Ok(final_subplot_width)
    }

    async fn measure_final_cells(
        &self,
        cells: &mut [FacetCellDraft],
        final_subplot_width: f32,
        compiled_subplot: &Arc<CompiledPlot>,
        subplot_eval_ctx: &EvaluationContext,
        nested_measure_ctx: &FacetColNestedMeasureContext<'_>,
    ) -> Result<(), AvengerChartError> {
        measure_cells_final_and_extents(
            cells,
            final_subplot_width,
            self.plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_measure_ctx,
        )
        .await
    }

    fn build_coord_measurement(
        &self,
        band_plan: FacetBandPlan,
        cells: Vec<FacetCellDraft>,
        shared_scale_builder: ScaleBuilder,
        compiled_subplot: &Arc<CompiledPlot>,
        final_subplot_width: f32,
        column_scale: &ConfiguredScaleWithSpec,
        coordination_field_identity: String,
    ) -> Box<dyn CoordMeasurement> {
        let cell_runtimes: Vec<FacetCellRuntime> = cells
            .into_iter()
            .map(|cell| FacetCellRuntime {
                data_override: cell.data_override,
                plan: cell.plan,
                measurement: cell
                    .measurement
                    .expect("FacetCol internal invariant violated: missing final cell measurement"),
                local_domain_extents: cell.local_domain_extents,
                coordinated_domain_extents: HashMap::new(),
            })
            .collect();

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_plan.padding_inner_px,
            outer_left: band_plan.outer_left,
            outer_right: band_plan.outer_right,
            n: band_plan.n,
        };

        Box::new(FacetColCoordMeasurement {
            cells: cell_runtimes,
            shared_scale_builder,
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: compiled_subplot.clone(),
            subplot_width: final_subplot_width,
            facet_depth: self.facet_path.len() as u8 + 1,
            original_column_scale: column_scale.configured().clone(),
            local_layout,
            coordinated_layout: None,
            coordination_field_identity,
        })
    }
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

    fn with_measured_padding(&self, _spec: &PaddingSpec) -> Box<dyn CoordinateSystemTransform> {
        // Facet spacing is encoded in the column/row band scale options.
        Box::new(self.clone())
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
        FacetColMeasurePipeline::new(
            scales,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        )
        .run()
        .await
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
    use avenger_scales::scales::band::BandScale;

    fn make_band_scale(range: (f32, f32)) -> ConfiguredScale {
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
    fn layout_from_measurement_or_local_prefers_coordinated() {
        let local = CoordinatedLayout {
            padding_inner_px: 4.0,
            outer_left: 1.0,
            outer_right: 2.0,
            n: 2,
        };
        let coordinated = CoordinatedLayout {
            padding_inner_px: 10.0,
            outer_left: 5.0,
            outer_right: 6.0,
            n: 4,
        };
        let selected = layout_from_measurement_or_local(&local, Some(&coordinated));
        assert_eq!(selected.padding_inner_px, coordinated.padding_inner_px);
        assert_eq!(selected.outer_left, coordinated.outer_left);
        assert_eq!(selected.outer_right, coordinated.outer_right);
        assert_eq!(selected.n, coordinated.n);

        let fallback = layout_from_measurement_or_local(&local, None);
        assert_eq!(fallback.padding_inner_px, local.padding_inner_px);
        assert_eq!(fallback.outer_left, local.outer_left);
        assert_eq!(fallback.outer_right, local.outer_right);
        assert_eq!(fallback.n, local.n);
    }

    #[test]
    fn apply_facet_col_scale_layout_applies_domain_padding_and_range() {
        let base = make_band_scale((0.0, 300.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 12.0,
            outer_left: 10.0,
            outer_right: 20.0,
            n: 3,
        };
        let domain_override = vec![
            ScalarValue::Utf8(Some("x".to_string())),
            ScalarValue::Utf8(Some("y".to_string())),
            ScalarValue::Utf8(Some("z".to_string())),
        ];

        let updated = apply_facet_col_scale_layout(
            &base,
            &layout,
            Some(domain_override.as_slice()),
            None,
            true,
            false,
        );

        let (start, end) = updated.config.numeric_interval_range().unwrap();
        assert_eq!(start, 0.0);
        assert_eq!(end, 270.0);
        assert_eq!(
            updated
                .config
                .options
                .get("padding_inner_px")
                .unwrap()
                .as_f32()
                .unwrap(),
            12.0
        );
        assert_eq!(updated.config.domain.len(), 3);
    }

    #[test]
    fn apply_facet_col_scale_layout_zero_padding_override_is_optional() {
        let base = make_band_scale((0.0, 200.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            outer_left: 0.0,
            outer_right: 0.0,
            n: 2,
        };

        let no_override = apply_facet_col_scale_layout(&base, &layout, None, None, false, false);
        assert!(!no_override.config.options.contains_key("padding_inner_px"));

        let with_override = apply_facet_col_scale_layout(&base, &layout, None, None, false, true);
        assert!(
            with_override
                .config
                .options
                .contains_key("padding_inner_px")
        );
        assert_eq!(
            with_override
                .config
                .options
                .get("padding_inner_px")
                .unwrap()
                .as_f32()
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn has_coordinated_layout_change_detects_any_dimension_shift() {
        let local = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_left: 1.0,
            outer_right: 2.0,
            n: 3,
        };
        let same = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_left: 1.0,
            outer_right: 2.0,
            n: 3,
        };
        let changed = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_left: 1.0,
            outer_right: 2.0,
            n: 4,
        };

        assert!(!has_coordinated_layout_change(&local, None));
        assert!(!has_coordinated_layout_change(&local, Some(&same)));
        assert!(has_coordinated_layout_change(&local, Some(&changed)));
    }

    #[test]
    fn should_remeasure_cells_only_for_legend_or_extents() {
        assert!(!should_remeasure_cells(false, false));
        assert!(should_remeasure_cells(true, false));
        assert!(should_remeasure_cells(false, true));
        assert!(should_remeasure_cells(true, true));
    }
}
