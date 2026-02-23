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
        CoordinatedLayout, CoordinatedOverflow, FacetAxis, OverflowSpaceRequirement, PaddingSpec,
        PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::{
        band_ir::{
            FacetBandIrParityReport, FacetBandPhase3Ir, FacetBandPhase4Ir, FacetBandPhase4Sidecars,
            FacetBandPhase5Ir, FacetBandPhase6Ir, FacetBandPhase6Sidecars, OverflowProbeSummary,
            assert_ir_legacy_parity,
        },
        coord_row::compute_band_layout,
        coordination::CoordinationGroupKey,
        coordination_remeasure::{
            FacetCoordRemeasurePlan, FacetCoordRemeasureRequest, derive_facet_coord_remeasure_plan,
            run_facet_coord_remeasure, run_facet_coord_remeasure_with_plan,
        },
        empty_cell_policy::FacetEmptyCellPolicy,
        guide::FacetColGuideConfig,
        layout_plan::{
            FacetBandPlan, FacetCellEmptyKind, FacetCellPlan, compute_padding_from_overflows,
            effective_edge_indices,
        },
        layout_slabs::LayoutSlabs,
        marks::facet::{FacetMarkRef, facet_mark_ref},
        ownership_policy::{has_holes_from_cells, resolve_facet_ownership_policy},
        padding_policy, path_math,
        scale_precompute::{
            FacetScaleNodeArtifacts, FacetScaleNodeKey, build_node_artifacts, canonicalize_path,
            ensure_subtree_precomputed,
        },
        sharing_level::SharingLevel,
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

pub use crate::facet::coord_row::FacetRow;

/// Domain extent with associated sharing level.
///
/// Used to track domain extents per-cell along with the sharing level
/// for coordinating domains across cells at the appropriate hierarchy level.
#[derive(Clone, Debug)]
pub(crate) struct ChannelDomainExtent {
    /// The domain extent (bounds + optional radius padding)
    pub extent: DomainExtent,
    /// Scale sharing level (0=Free, N=Level(N), 255=Shared)
    pub(crate) sharing_level: SharingLevel,
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
pub struct FacetBandCoordMeasurement {
    /// Facet axis orientation for this measurement.
    pub axis: FacetAxis,
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
    pub subplot_cross_size: f32,
    /// Facet depth in the hierarchy (1 = outermost, 2 = nested, etc.)
    /// INVARIANT: facet_depth == full_cell_path.len() for any cell
    /// Used for sharing level comparison: sharing >= facet_depth means global.
    pub facet_depth: u8,
    /// Column scale (as ConfiguredScale) BEFORE Pass 2 adjustments.
    /// Used to recompute subplot width when coordinated layout differs from local.
    pub original_band_scale: ConfiguredScale,
    /// Local layout values (pre-coordination).
    pub local_layout: CoordinatedLayout,
    /// Coordinated layout values (post-coordination). None before coordination.
    pub coordinated_layout: Option<CoordinatedLayout>,
    /// Channel sharing levels observed in non-empty child local extents.
    ///
    /// Data-empty cells may have no local domain extents; this map preserves
    /// per-channel sharing metadata so those cells can still inherit coordinated
    /// shared domains.
    pub(crate) channel_sharing_levels: HashMap<String, SharingLevel>,
    /// Effective policy used when deciding whether empty cells are renderable.
    pub empty_cell_policy: FacetEmptyCellPolicy,
}

#[derive(Debug, Clone)]
pub(crate) struct FacetBandCoordApplyPlan {
    pub(crate) axis: FacetAxis,
    pub(crate) legend_start: f32,
    pub(crate) legend_end: f32,
    pub(crate) legend_slab_applied: f32,
    pub(crate) has_legend_overflow: bool,
    pub(crate) has_coordinated_extents: bool,
    pub(crate) has_coordinated_layout: bool,
    pub(crate) remeasure_required: bool,
    pub(crate) has_holes: bool,
    pub(crate) axis_owner_ignore_empty_cells: bool,
    pub(crate) original_main_size: f32,
    pub(crate) adjusted_main_size: f32,
    pub(crate) legend_main_axis_shrink: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FacetBandCoordApplyOutcome {
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) remeasure_triggered: bool,
    pub(crate) remeasured_cell_count: usize,
    pub(crate) remeasured_non_empty_cell_count: usize,
    pub(crate) remeasured_with_coordinated_extents_count: usize,
}

impl FacetBandCoordMeasurement {
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

        let renderable_indices: Vec<usize> = self
            .cells
            .iter()
            .enumerate()
            .filter_map(|(idx, cell)| {
                renderable_for_empty_policy(self.empty_cell_policy, !cell.plan.has_data_rows)
                    .then_some(idx)
            })
            .collect();
        if renderable_indices.is_empty() {
            return Some(CoordinatedOverflow::default());
        }

        let first_idx = *renderable_indices.first().unwrap_or(&0);
        let last_idx = *renderable_indices
            .last()
            .unwrap_or(&cell_count.saturating_sub(1));

        let first_layout = &self.cells[first_idx].measurement.layout;
        let last_layout = &self.cells[last_idx].measurement.layout;

        let max_guide_top = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.overflow.top)
            .fold(0.0f32, f32::max);
        let max_guide_bottom = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.overflow.bottom)
            .fold(0.0f32, f32::max);
        let max_guide_left = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.overflow.left)
            .fold(0.0f32, f32::max);
        let max_guide_right = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.overflow.right)
            .fold(0.0f32, f32::max);

        let max_total_top = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.total_overflow.top)
            .fold(0.0f32, f32::max);
        let max_total_bottom = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.total_overflow.bottom)
            .fold(0.0f32, f32::max);
        let max_total_left = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.total_overflow.left)
            .fold(0.0f32, f32::max);
        let max_total_right = renderable_indices
            .iter()
            .map(|idx| &self.cells[*idx])
            .map(|cell| cell.measurement.layout.total_overflow.right)
            .fold(0.0f32, f32::max);

        let guide = match self.axis {
            FacetAxis::Column => OverflowSpaceRequirement {
                top: max_guide_top,
                bottom: max_guide_bottom,
                left: first_layout.overflow.left,
                right: last_layout.overflow.right,
            },
            FacetAxis::Row => OverflowSpaceRequirement {
                top: first_layout.overflow.top,
                bottom: last_layout.overflow.bottom,
                left: max_guide_left,
                right: max_guide_right,
            },
        };

        let total = match self.axis {
            FacetAxis::Column => OverflowSpaceRequirement {
                top: max_total_top,
                bottom: max_total_bottom,
                left: first_layout.total_overflow.left,
                right: last_layout.total_overflow.right,
            },
            FacetAxis::Row => OverflowSpaceRequirement {
                top: first_layout.total_overflow.top,
                bottom: last_layout.total_overflow.bottom,
                left: max_total_left,
                right: max_total_right,
            },
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
        let layout = coordinated_layout_preserving_outer_edges(&self.local_layout, layout);
        self.coordinated_layout = Some(layout);
    }

    pub fn coordinated_subplot_cross_size(&self) -> Option<f32> {
        if self.subplot_cross_size > 0.0 {
            Some(self.subplot_cross_size)
        } else {
            None
        }
    }

    pub fn set_parent_bandwidth_value(&mut self, bandwidth: f32) {
        if bandwidth > 0.0 {
            self.original_band_scale = self
                .original_band_scale
                .clone()
                .with_range_interval((0.0, bandwidth));

            debug!(
                bandwidth,
                "FacetCol set_parent_bandwidth updated original column scale range"
            );
        }
    }
}

impl CoordMeasurement for FacetBandCoordMeasurement {
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
        // Apply facet band layout adjustments at render-time:
        // - domain override from cell_values
        // - coordinated padding/outer edges
        // - band_n override when coordinated layout expects more slots than the local domain
        //
        // The band_n override preserves equal subplot sizing for ragged nested layouts by
        // reserving trailing empty slots in branches with fewer local values.
        if let Some(band_scale) = scales.get_mut(self.axis.scale_name()) {
            let has_hole_cells = self.cells.iter().any(|cell| {
                !cell.plan.has_data_rows
                    && !renderable_for_empty_policy(
                        self.empty_cell_policy,
                        !cell.plan.has_data_rows,
                    )
            });
            let has_adjacent_non_empty = self.cells.windows(2).any(|pair| {
                renderable_for_empty_policy(self.empty_cell_policy, !pair[0].plan.has_data_rows)
                    && renderable_for_empty_policy(
                        self.empty_cell_policy,
                        !pair[1].plan.has_data_rows,
                    )
            });
            let needs_zero_padding_override = has_hole_cells && !has_adjacent_non_empty;
            let cell_values: Vec<ScalarValue> = self.cell_values().cloned().collect();
            let domain_override = if cell_values.is_empty() {
                None
            } else {
                Some(cell_values.as_slice())
            };
            let layout = self.active_layout();
            let band_n_override = if layout.n > cell_values.len() {
                Some(layout.n)
            } else {
                None
            };

            // Always start from the original measured column scale so repeated
            // coordination passes re-apply the same layout adjustments
            // deterministically without shrinking the range multiple times.
            let base_scale = self.original_band_scale.clone();

            let updated_config = apply_facet_band_scale_layout(
                self.axis,
                &base_scale,
                layout,
                domain_override,
                band_n_override,
                ScaleLayoutRewriteMode::RenderPass {
                    allow_zero_padding_override: needs_zero_padding_override,
                    side_specific_outer_edges: true,
                },
            );

            *band_scale = ConfiguredScaleWithSpec::new(band_scale.spec().clone(), updated_config);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ScaleLayoutRewriteMode {
    MeasurementPass {
        side_specific_outer_edges: bool,
    },
    RenderPass {
        allow_zero_padding_override: bool,
        side_specific_outer_edges: bool,
    },
    RemeasurePass {
        side_specific_outer_edges: bool,
    },
}

impl ScaleLayoutRewriteMode {
    #[inline]
    fn set_padding_always(self) -> bool {
        matches!(
            self,
            ScaleLayoutRewriteMode::MeasurementPass { .. }
                | ScaleLayoutRewriteMode::RemeasurePass { .. }
        )
    }

    #[inline]
    fn allow_zero_padding_override(self) -> bool {
        match self {
            ScaleLayoutRewriteMode::RenderPass {
                allow_zero_padding_override,
                ..
            } => allow_zero_padding_override,
            _ => false,
        }
    }

    #[inline]
    fn side_specific_outer_edges(self) -> bool {
        match self {
            ScaleLayoutRewriteMode::MeasurementPass {
                side_specific_outer_edges,
            }
            | ScaleLayoutRewriteMode::RenderPass {
                side_specific_outer_edges,
                ..
            }
            | ScaleLayoutRewriteMode::RemeasurePass {
                side_specific_outer_edges,
            } => side_specific_outer_edges,
        }
    }
}

/// Canonical facet-band scale rewrite helper shared by measurement and render paths.
fn apply_facet_band_scale_layout(
    _axis: FacetAxis,
    base: &ConfiguredScale,
    layout: &CoordinatedLayout,
    domain_override: Option<&[ScalarValue]>,
    band_n_override: Option<usize>,
    mode: ScaleLayoutRewriteMode,
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

    if layout.padding_inner_px > 0.0
        || mode.set_padding_always()
        || mode.allow_zero_padding_override()
    {
        updated = updated.with_option("padding_inner_px", layout.padding_inner_px);
    }

    if (layout.outer_start > 0.0 || layout.outer_end > 0.0)
        && let Ok((range_start, range_end)) = updated.config.numeric_interval_range()
    {
        let new_start = if mode.side_specific_outer_edges() {
            range_start + layout.outer_start
        } else {
            range_start
        };
        let mut new_end = if mode.side_specific_outer_edges() {
            range_end - layout.outer_end
        } else {
            range_end - layout.outer_start - layout.outer_end
        };

        if new_end <= new_start {
            new_end = new_start + 1.0;
        }

        updated = updated.with_range_interval((new_start, new_end));
    }

    updated
}

fn coordinated_layout_preserving_outer_edges(
    local_layout: &CoordinatedLayout,
    mut coordinated_layout: CoordinatedLayout,
) -> CoordinatedLayout {
    coordinated_layout.outer_start = local_layout.outer_start;
    coordinated_layout.outer_end = local_layout.outer_end;
    coordinated_layout
}

fn has_coordinated_layout_change(
    local_layout: &CoordinatedLayout,
    coordinated_layout: Option<&CoordinatedLayout>,
) -> bool {
    coordinated_layout.is_some_and(|coordinated| {
        coordinated.n != local_layout.n
            || (coordinated.padding_inner_px - local_layout.padding_inner_px).abs() > 0.01
            || (coordinated.outer_start - local_layout.outer_start).abs() > 0.01
            || (coordinated.outer_end - local_layout.outer_end).abs() > 0.01
    })
}

fn should_remeasure_cells(has_legend_overflow: bool, has_coordinated_extents: bool) -> bool {
    has_legend_overflow || has_coordinated_extents
}

fn legend_axis_overflow(axis: FacetAxis, coordinated: &CoordinatedOverflow) -> (f32, f32) {
    let slabs = LayoutSlabs::from_coordinated(coordinated);
    match axis {
        FacetAxis::Column => slabs.legend_vertical(),
        FacetAxis::Row => slabs.legend_horizontal(),
    }
}

fn adjusted_size_for_legend_overflow(
    original_size: f32,
    legend_start: f32,
    legend_end: f32,
) -> f32 {
    (original_size - legend_start - legend_end).max(1.0)
}

fn collect_channel_sharing_levels(cells: &[FacetCellDraft]) -> HashMap<String, SharingLevel> {
    let mut sharing_levels = HashMap::new();
    for cell in cells {
        for (channel, annotated) in &cell.local_domain_extents {
            sharing_levels
                .entry(channel.clone())
                .or_insert(annotated.sharing_level);
        }
    }
    sharing_levels
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

impl FacetBandCoordMeasurement {
    pub fn coordination_group_key_for_depth(&self, depth: usize) -> CoordinationGroupKey {
        CoordinationGroupKey::new(
            depth,
            format!(
                "{}:{}",
                self.axis.coordination_key_prefix(),
                self.coordination_field_identity
            ),
        )
    }

    pub fn collect_cell_domain_infos(&self, collector: &mut Vec<CellDomainInfo>) {
        for cell in &self.cells {
            for (channel, annotated) in &cell.local_domain_extents {
                collector.push(CellDomainInfo {
                    full_cell_path: cell.plan.full_path.clone(),
                    channel: channel.clone(),
                    sharing_level: annotated.sharing_level.raw(),
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

            // Data-empty cells can have no local domain extents even when channel
            // sharing is enabled. Fill coordinated extents from saved sharing levels
            // so owner cells without local data still render shared domains.
            for (channel, sharing_level) in &self.channel_sharing_levels {
                if *sharing_level == 0 || cell.coordinated_domain_extents.contains_key(channel) {
                    continue;
                }

                let ancestor_key = sharing_policy::domain_group_key(
                    &cell.plan.full_path,
                    *sharing_level,
                    self.facet_depth,
                );

                if let Some(unified_extent) = unified.get(&(channel.clone(), ancestor_key)) {
                    cell.coordinated_domain_extents
                        .insert(channel.clone(), unified_extent.clone());
                    trace!(
                        axis = ?self.axis,
                        facet_depth = self.facet_depth,
                        channel = %channel,
                        sharing_level = sharing_level.raw(),
                        cell_path = ?cell.plan.full_path,
                        "FacetBand distributed coordinated extents via empty-cell fallback"
                    );
                }
            }
        }
    }

    pub(crate) fn derive_coordinated_apply_plan(&self) -> FacetBandCoordApplyPlan {
        let (legend_start, legend_end) =
            legend_axis_overflow(self.axis, &self.coordinated_overflow);
        let legend_slab_applied = legend_start + legend_end;
        let has_legend_overflow = legend_start > 0.0 || legend_end > 0.0;
        let has_coordinated_extents = self
            .cells
            .iter()
            .any(|cell| !cell.coordinated_domain_extents.is_empty());
        let has_coordinated_layout =
            has_coordinated_layout_change(&self.local_layout, self.coordinated_layout.as_ref());
        let remeasure_required =
            should_remeasure_cells(has_legend_overflow, has_coordinated_extents);
        let policy = resolve_facet_ownership_policy(
            self.empty_cell_policy,
            has_holes_from_cells(self.cells.iter().map(|cell| cell.plan.is_empty)),
        );
        let original_main_size = self
            .cells
            .first()
            .map(|cell| match self.axis {
                FacetAxis::Column => cell.measurement.plot_area_height,
                FacetAxis::Row => cell.measurement.plot_area_width,
            })
            .unwrap_or(0.0);
        let adjusted_main_size =
            adjusted_size_for_legend_overflow(original_main_size, legend_start, legend_end);
        let legend_main_axis_shrink = (original_main_size - adjusted_main_size).max(0.0);

        FacetBandCoordApplyPlan {
            axis: self.axis,
            legend_start,
            legend_end,
            legend_slab_applied,
            has_legend_overflow,
            has_coordinated_extents,
            has_coordinated_layout,
            remeasure_required,
            has_holes: policy.has_holes,
            axis_owner_ignore_empty_cells: policy.axis_owner_ignore_empty_cells,
            original_main_size,
            adjusted_main_size,
            legend_main_axis_shrink,
        }
    }

    fn apply_coordinated_layout_cross_size(&mut self) -> Result<(), AvengerChartError> {
        let Some(layout) = self.coordinated_layout.as_ref() else {
            debug!(
                axis = ?self.axis,
                facet_depth = self.facet_depth,
                coordination_field = %self.coordination_field_identity,
                "FacetBand apply_coordinated_overflow missing coordinated layout; skipping layout rewrite"
            );
            return Ok(());
        };
        let cell_values: Vec<ScalarValue> = self.cell_values().cloned().collect();
        let domain_override = if cell_values.is_empty() {
            None
        } else {
            Some(cell_values.as_slice())
        };
        let scale = apply_facet_band_scale_layout(
            self.axis,
            &self.original_band_scale,
            layout,
            domain_override,
            Some(layout.n),
            ScaleLayoutRewriteMode::RemeasurePass {
                side_specific_outer_edges: true,
            },
        );

        let new_subplot_cross_size = bandwidth(&scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get coordinated bandwidth: {}", e))
        })?;

        debug!(
            old_width = self.subplot_cross_size,
            new_width = new_subplot_cross_size,
            local_n = self.local_layout.n,
            coordinated_n = layout.n,
            local_padding = self.local_layout.padding_inner_px,
            coordinated_padding = layout.padding_inner_px,
            local_outer_start = self.local_layout.outer_start,
            local_outer_end = self.local_layout.outer_end,
            coordinated_outer_start = layout.outer_start,
            coordinated_outer_end = layout.outer_end,
            "FacetCol apply_coordinated_overflow layout coordination"
        );

        self.subplot_cross_size = new_subplot_cross_size;
        Ok(())
    }

    fn build_coordinated_remeasure_plan(
        &self,
        plan: &FacetBandCoordApplyPlan,
    ) -> FacetCoordRemeasurePlan {
        derive_facet_coord_remeasure_plan(&self.cells, plan, self.subplot_cross_size)
    }

    fn build_coordinated_remeasure_request(
        &self,
        plan: &FacetBandCoordApplyPlan,
    ) -> FacetCoordRemeasureRequest {
        FacetCoordRemeasureRequest {
            axis: self.axis,
            subplot_cross_size: self.subplot_cross_size,
            adjusted_main_size: plan.adjusted_main_size,
            axis_owner_ignore_empty_cells: plan.axis_owner_ignore_empty_cells,
        }
    }

    pub(crate) async fn apply_coordinated_overflow_with_plan_and_remeasure_plan(
        &mut self,
        eval_ctx: &EvaluationContext,
        plan: &FacetBandCoordApplyPlan,
        derived_remeasure_plan: Option<&FacetCoordRemeasurePlan>,
    ) -> Result<FacetBandCoordApplyOutcome, AvengerChartError> {
        let fallback_remeasure_plan;
        let effective_remeasure_plan = if plan.remeasure_required {
            let plan_ref = match derived_remeasure_plan {
                Some(plan_ref) => plan_ref,
                None => {
                    fallback_remeasure_plan = self.build_coordinated_remeasure_plan(plan);
                    &fallback_remeasure_plan
                }
            };
            debug_assert_eq!(plan_ref.request.axis, self.axis);
            debug_assert_eq!(
                plan_ref.request.axis_owner_ignore_empty_cells,
                plan.axis_owner_ignore_empty_cells
            );
            Some(plan_ref)
        } else {
            None
        };

        self.apply_coordinated_overflow_with_plan_impl(eval_ctx, plan, effective_remeasure_plan)
            .await
    }

    pub(crate) async fn apply_coordinated_overflow_with_plan(
        &mut self,
        eval_ctx: &EvaluationContext,
        plan: &FacetBandCoordApplyPlan,
    ) -> Result<FacetBandCoordApplyOutcome, AvengerChartError> {
        self.apply_coordinated_overflow_with_plan_and_remeasure_plan(eval_ctx, plan, None)
            .await
    }

    async fn apply_coordinated_overflow_with_plan_impl(
        &mut self,
        eval_ctx: &EvaluationContext,
        plan: &FacetBandCoordApplyPlan,
        remeasure_plan: Option<&FacetCoordRemeasurePlan>,
    ) -> Result<FacetBandCoordApplyOutcome, AvengerChartError> {
        debug_assert_eq!(
            plan.axis, self.axis,
            "coordinated apply plan axis must match measurement axis"
        );

        let subplot_cross_size_before = self.subplot_cross_size;
        let legend_slabs = LayoutSlabs::from_coordinated(&self.coordinated_overflow);

        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            coordination_field = %self.coordination_field_identity,
            legend_start = plan.legend_start,
            legend_end = plan.legend_end,
            legend_top = legend_slabs.legend.top,
            legend_right = legend_slabs.legend.right,
            legend_bottom = legend_slabs.legend.bottom,
            legend_left = legend_slabs.legend.left,
            "FacetBand apply_coordinated_overflow legend slab sides"
        );

        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            coordination_field = %self.coordination_field_identity,
            legend_start = plan.legend_start,
            legend_end = plan.legend_end,
            legend_slab_applied = plan.legend_slab_applied,
            has_legend_overflow = plan.has_legend_overflow,
            has_coordinated_extents = plan.has_coordinated_extents,
            has_coordinated_layout = plan.has_coordinated_layout,
            has_holes = plan.has_holes,
            axis_owner_ignore_empty_cells = plan.axis_owner_ignore_empty_cells,
            coordinated_guide_top = self.coordinated_overflow.guide.top,
            coordinated_guide_right = self.coordinated_overflow.guide.right,
            coordinated_guide_bottom = self.coordinated_overflow.guide.bottom,
            coordinated_guide_left = self.coordinated_overflow.guide.left,
            coordinated_total_top = self.coordinated_overflow.total.top,
            coordinated_total_right = self.coordinated_overflow.total.right,
            coordinated_total_bottom = self.coordinated_overflow.total.bottom,
            coordinated_total_left = self.coordinated_overflow.total.left,
            "FacetBand apply_coordinated_overflow coordinated inputs"
        );

        if plan.has_coordinated_layout {
            self.apply_coordinated_layout_cross_size()?;
        }

        if !plan.remeasure_required {
            return Ok(FacetBandCoordApplyOutcome {
                subplot_cross_size_before,
                subplot_cross_size_after: self.subplot_cross_size,
                remeasure_triggered: false,
                remeasured_cell_count: 0,
                remeasured_non_empty_cell_count: 0,
                remeasured_with_coordinated_extents_count: 0,
            });
        }

        debug!(
            axis = ?self.axis,
            original_main_size = plan.original_main_size,
            adjusted_main_size = plan.adjusted_main_size,
            legend_start = plan.legend_start,
            legend_end = plan.legend_end,
            legend_main_axis_shrink = plan.legend_main_axis_shrink,
            has_coordinated_extents = plan.has_coordinated_extents,
            "FacetBand apply_coordinated_overflow size adjustment"
        );
        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            coordination_field = %self.coordination_field_identity,
            original_main_size = plan.original_main_size,
            adjusted_main_size = plan.adjusted_main_size,
            legend_start = plan.legend_start,
            legend_end = plan.legend_end,
            legend_slab_applied = plan.legend_slab_applied,
            legend_main_axis_shrink = plan.legend_main_axis_shrink,
            "FacetBand apply_coordinated_overflow legend slab application"
        );

        let remeasure_outcome = match remeasure_plan {
            Some(remeasure_plan) => {
                let mut execution_plan = remeasure_plan.clone();
                execution_plan.request.axis = self.axis;
                execution_plan.request.subplot_cross_size = self.subplot_cross_size;
                execution_plan.request.adjusted_main_size = plan.adjusted_main_size;
                execution_plan.request.axis_owner_ignore_empty_cells =
                    plan.axis_owner_ignore_empty_cells;
                run_facet_coord_remeasure_with_plan(
                    &mut self.cells,
                    &self.compiled_subplot,
                    &self.shared_scale_builder,
                    eval_ctx,
                    &execution_plan,
                )
                .await?
            }
            None => {
                let request = self.build_coordinated_remeasure_request(plan);
                run_facet_coord_remeasure(
                    &mut self.cells,
                    &self.compiled_subplot,
                    &self.shared_scale_builder,
                    eval_ctx,
                    &request,
                )
                .await?
            }
        };
        let remeasured_cell_count = remeasure_outcome.cell_outcomes.len();
        for (idx, cell_outcome) in remeasure_outcome.cell_outcomes.iter().enumerate() {
            debug_assert_eq!(
                cell_outcome.cell_index, idx,
                "coordination remeasure cell outcome ordering must match cell ordering"
            );
        }
        let remeasured_non_empty_cell_count = remeasure_outcome
            .cell_outcomes
            .iter()
            .filter(|outcome| outcome.has_data_rows)
            .count();
        let remeasured_with_coordinated_extents_count = remeasure_outcome
            .cell_outcomes
            .iter()
            .filter(|outcome| outcome.used_coordinated_extents)
            .count();
        let remeasured_max_plot_area_width = remeasure_outcome
            .cell_outcomes
            .iter()
            .map(|outcome| outcome.plot_area_width)
            .fold(0.0f32, f32::max);
        let remeasured_max_plot_area_height = remeasure_outcome
            .cell_outcomes
            .iter()
            .map(|outcome| outcome.plot_area_height)
            .fold(0.0f32, f32::max);
        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            remeasured_cell_count,
            remeasured_non_empty_cell_count,
            remeasured_with_coordinated_extents_count,
            remeasured_max_plot_area_width,
            remeasured_max_plot_area_height,
            "FacetBand apply_coordinated_overflow remeasure outcome summary"
        );

        Ok(FacetBandCoordApplyOutcome {
            subplot_cross_size_before,
            subplot_cross_size_after: self.subplot_cross_size,
            remeasure_triggered: true,
            remeasured_cell_count,
            remeasured_non_empty_cell_count,
            remeasured_with_coordinated_extents_count,
        })
    }

    pub async fn apply_coordinated_overflow(
        &mut self,
        eval_ctx: &EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        let plan = self.derive_coordinated_apply_plan();
        let _ = self
            .apply_coordinated_overflow_with_plan(eval_ctx, &plan)
            .await?;
        Ok(())
    }
}

/// Scale selection strategy for measuring a facet cell.
enum FacetCellMeasurementMode<'a> {
    /// Use nested sharing rules (Free/Level(N)/Shared) for nested facet cells.
    NestedSharing {
        nested_sharing_level: Option<SharingLevel>,
        nested_depth: u8,
        ancestor_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
        per_cell_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
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
    PerCellCachedBuilder {
        cached_builder: &'a ScaleBuilder,
    },
    PerCellBuilderFallback,
    AncestorCachedBuilder {
        cached_builder: &'a ScaleBuilder,
        ancestor_filtered_df: DataFrame,
    },
    EmptySharedNoData,
}

/// Planning data prepared once before running FacetCol measurement passes.
struct FacetBandMeasurePlan {
    cell_values: Vec<ScalarValue>,
    cells: Vec<FacetCellDraft>,
    scale_artifacts: Arc<FacetScaleNodeArtifacts>,
}

/// Shared nested-facet measurement context used by pass 1 and pass 2.
#[derive(Clone)]
pub(crate) struct FacetBandNestedMeasureContext {
    scale_artifacts: Arc<FacetScaleNodeArtifacts>,
    facet_tree: Arc<crate::facet::evaluated_facet_tree::EvaluatedFacetTree>,
    data_df: DataFrame,
    eval_ctx: EvaluationContext,
}

struct LegacyFacetPhase4Prep {
    plan: FacetBandMeasurePlan,
    subplot_eval_ctx: EvaluationContext,
    nested_measure_ctx: FacetBandNestedMeasureContext,
}

struct LegacyFacetPhase6LocalFinalization {
    band_layout_plan: FacetBandPlan,
    final_subplot_main_or_cross_size: f32,
    cells: Vec<FacetCellDraft>,
}

fn derive_padding_inner_px_from_probe(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
) -> f32 {
    let parent_padding = compute_padding_from_overflows(
        axis,
        &pass1
            .cell_overflows
            .iter()
            .map(|(_, total)| total.clone())
            .collect::<Vec<_>>(),
        pass1_renderable_cells,
    );

    padding_policy::derive_parent_padding(parent_padding, pass1.max_child_padding)
}

fn renderable_for_empty_policy(policy: FacetEmptyCellPolicy, is_empty: bool) -> bool {
    if !is_empty {
        return true;
    }
    matches!(policy.effective(), FacetEmptyCellPolicy::EmptySubplot)
}

fn renderable_mask_for_cells(cells: &[FacetCellDraft], policy: FacetEmptyCellPolicy) -> Vec<bool> {
    cells
        .iter()
        .map(|cell| renderable_for_empty_policy(policy, !cell.plan.has_data_rows))
        .collect()
}

fn empty_facet_band_measurement(
    axis: FacetAxis,
    facet_path: &[ScalarValue],
    compiled_subplot: &Arc<CompiledPlot>,
    band_scale: &ConfiguredScaleWithSpec,
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Box<dyn CoordMeasurement> {
    Box::new(FacetBandCoordMeasurement {
        axis,
        cells: Vec::new(),
        shared_scale_builder: ScaleBuilder::default(),
        coordinated_overflow: CoordinatedOverflow::default(),
        compiled_subplot: compiled_subplot.clone(),
        subplot_cross_size: 0.0,
        facet_depth: facet_path.len() as u8 + 1,
        original_band_scale: band_scale.configured().clone(),
        local_layout: CoordinatedLayout::default(),
        coordinated_layout: None,
        coordination_field_identity: axis.scale_name().to_string(),
        channel_sharing_levels: HashMap::new(),
        empty_cell_policy,
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

    let in_domain_slot = facet_tree.cell_exists(&path);
    let has_data_rows = if in_domain_slot {
        facet_tree.cell_has_data(&path)
    } else {
        false
    };
    let is_empty = !has_data_rows;
    let empty_kind = if !in_domain_slot {
        FacetCellEmptyKind::DomainPlaceholder
    } else {
        FacetCellEmptyKind::DataEmpty
    };
    let filter_predicate = if in_domain_slot {
        facet_tree.cell_predicate(&path, 0)
    } else {
        Some(lit(false))
    };

    Ok(FacetCellPlan {
        value: value.clone(),
        full_path: path,
        in_domain_slot,
        has_data_rows,
        empty_kind,
        is_empty,
        filter_predicate,
    })
}

/// Build canonical per-cell facet plans for the current node path.
///
/// This phase is geometry-independent and depends only on the evaluated facet tree
/// semantics plus the enumerated cell values for this node.
fn build_facet_cell_plans(
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    cell_values: &[ScalarValue],
) -> Result<Vec<FacetCellPlan>, AvengerChartError> {
    cell_values
        .iter()
        .map(|value| build_facet_cell(facet_tree, facet_path, value))
        .collect::<Result<_, _>>()
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
    nested_sharing_level: Option<SharingLevel>,
    nested_depth: u8,
    ancestor_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    per_cell_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    data_df: &DataFrame,
) -> Result<NestedScalePlan<'a>, AvengerChartError> {
    if !cell.in_domain_slot {
        return Ok(NestedScalePlan::EmptySharedNoData);
    }

    match nested_sharing_level {
        Some(sharing_level) if sharing_level.is_free() => {
            let canonical_full_path = canonicalize_path(&cell.full_path);
            if let Some(cached_builder) = per_cell_scale_builder_cache
                .get(&canonical_full_path)
                .or_else(|| per_cell_scale_builder_cache.get(&cell.full_path))
            {
                Ok(NestedScalePlan::PerCellCachedBuilder { cached_builder })
            } else {
                debug!(
                    full_path = ?cell.full_path,
                    "Facet nested measurement missing per-cell cached builder; falling back to on-demand build"
                );
                Ok(NestedScalePlan::PerCellBuilderFallback)
            }
        }
        Some(sharing_level) if sharing_level < nested_depth => {
            let ancestor_key = path_math::nested_measurement_ancestor_key(
                &cell.full_path,
                sharing_level,
                nested_depth,
            );
            let canonical_ancestor_key = canonicalize_path(&ancestor_key);
            let cached_builder = ancestor_scale_builder_cache
                .get(&canonical_ancestor_key)
                .or_else(|| ancestor_scale_builder_cache.get(&ancestor_key))
                .ok_or_else(|| {
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
        NestedScalePlan::PerCellCachedBuilder { cached_builder } => {
            let cached_scale_provider = DynamicScaleProvider {
                builder: cached_builder,
                plot: compiled_subplot,
            };
            let measurement = compiled_subplot
                .measure_plot_components(
                    subplot_eval_ctx,
                    subplot_layout_spec,
                    &cached_scale_provider,
                    Some(data_override),
                    &cell.full_path,
                )
                .await?;

            Ok(MeasuredFacetCell {
                measurement,
                cell_scale_builder: Some(cached_builder.clone()),
            })
        }
        NestedScalePlan::PerCellBuilderFallback => {
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
            nested_sharing_level,
            nested_depth,
            ancestor_scale_builder_cache,
            per_cell_scale_builder_cache,
            shared_scale_builder,
            facet_tree,
            data_df,
            eval_ctx,
        } => {
            let plan = resolve_nested_scale_plan(
                cell,
                nested_sharing_level,
                nested_depth,
                ancestor_scale_builder_cache,
                per_cell_scale_builder_cache,
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
            let data_arg = if cell.in_domain_slot {
                Some(data_override)
            } else {
                None
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

pub(crate) async fn measure_facet_cell_with_explicit_builder(
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    subplot_layout_spec: &EvaluatedLayoutSpec,
    scale_builder: &ScaleBuilder,
) -> Result<ComponentsMeasurement, AvengerChartError> {
    let mode = FacetCellMeasurementMode::ExplicitBuilder { scale_builder };
    let MeasuredFacetCell { measurement, .. } = measure_facet_cell(
        cell,
        data_override,
        compiled_subplot,
        subplot_eval_ctx,
        subplot_layout_spec,
        mode,
    )
    .await?;
    Ok(measurement)
}

async fn measure_nested_cell(
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_plot_width, subplot_plot_height);
    let mode = FacetCellMeasurementMode::NestedSharing {
        nested_sharing_level: nested_ctx.scale_artifacts.nested_col_sharing,
        nested_depth: nested_ctx.scale_artifacts.nested_depth,
        ancestor_scale_builder_cache: &nested_ctx.scale_artifacts.ancestor_scale_builder_cache,
        per_cell_scale_builder_cache: &nested_ctx.scale_artifacts.per_cell_scale_builder_cache,
        shared_scale_builder: &nested_ctx.scale_artifacts.shared_scale_builder,
        facet_tree: nested_ctx.facet_tree.as_ref(),
        data_df: &nested_ctx.data_df,
        eval_ctx: &nested_ctx.eval_ctx,
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

async fn build_facet_band_measure_plan(
    cell_plans: Vec<FacetCellPlan>,
    facet_path: &[ScalarValue],
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<FacetBandMeasurePlan, AvengerChartError> {
    let cell_values: Vec<ScalarValue> = cell_plans.iter().map(|plan| plan.value.clone()).collect();

    let node_key = FacetScaleNodeKey::new(compiled_subplot, facet_path);
    let scale_artifacts = if let Some(artifacts) = eval_ctx
        .facet_scale_precompute_store()
        .get_node_artifacts(&node_key)
    {
        artifacts
    } else {
        debug!(
            facet_path = ?facet_path,
            "Facet scale node artifacts missing from precompute store; building fallback artifacts on demand"
        );
        let artifacts = Arc::new(
            build_node_artifacts(
                &cell_values,
                facet_path,
                data_df,
                compiled_subplot,
                facet_tree,
                eval_ctx,
            )
            .await?,
        );
        eval_ctx
            .facet_scale_precompute_store()
            .insert_node_artifacts(node_key, artifacts.clone());
        artifacts
    };

    if !scale_artifacts.ancestor_scale_builder_cache.is_empty() {
        debug!(
            scale_builder_count = scale_artifacts.ancestor_scale_builder_cache.len(),
            sharing_level = scale_artifacts
                .nested_col_sharing
                .unwrap_or(SharingLevel::GLOBAL)
                .raw(),
            "FacetBand using precomputed ancestor cached scale builders"
        );
    }
    if !scale_artifacts.per_cell_scale_builder_cache.is_empty() {
        debug!(
            per_cell_builder_count = scale_artifacts.per_cell_scale_builder_cache.len(),
            "FacetBand using precomputed per-cell scale builders"
        );
    }

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

    Ok(FacetBandMeasurePlan {
        cell_values,
        cells,
        scale_artifacts,
    })
}

async fn prepare_phase4_measurement_inputs_legacy(
    cell_plans: Vec<FacetCellPlan>,
    facet_path: &[ScalarValue],
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    empty_cell_policy: FacetEmptyCellPolicy,
    eval_ctx: &EvaluationContext,
) -> Result<LegacyFacetPhase4Prep, AvengerChartError> {
    let plan = build_facet_band_measure_plan(
        cell_plans,
        facet_path,
        data_df,
        compiled_subplot,
        &eval_ctx.facet_tree,
        eval_ctx,
    )
    .await?;

    let policy = resolve_facet_ownership_policy(
        empty_cell_policy,
        has_holes_from_cells(plan.cells.iter().map(|cell| cell.plan.is_empty)),
    );

    let subplot_eval_ctx = {
        let mut params = compiled_subplot.get_default_params().clone();
        params.extend(eval_ctx.params.clone());
        let eval_ctx = eval_ctx.with_params(params);
        eval_ctx.with_axis_owner_ignore_empty_cells(policy.axis_owner_ignore_empty_cells)
    };

    let nested_measure_ctx = FacetBandNestedMeasureContext {
        scale_artifacts: plan.scale_artifacts.clone(),
        facet_tree: eval_ctx.facet_tree.clone(),
        data_df: data_df.clone(),
        eval_ctx: eval_ctx.clone(),
    };

    Ok(LegacyFacetPhase4Prep {
        plan,
        subplot_eval_ctx,
        nested_measure_ctx,
    })
}

fn phase4_cells_as_drafts(
    phase4: &FacetBandPhase4Ir,
    sidecars: &FacetBandPhase4Sidecars,
) -> Vec<FacetCellDraft> {
    assert_eq!(
        phase4.phase3.cells.len(),
        sidecars.data_overrides.len(),
        "FacetBand phase-4 IR/sidecar invariant violated: mismatched cell/data_override lengths"
    );

    phase4
        .phase3
        .cells
        .iter()
        .zip(sidecars.data_overrides.iter())
        .map(|(cell, data_override)| FacetCellDraft {
            plan: FacetCellPlan::from(cell),
            data_override: data_override.clone(),
            measurement: None,
            local_domain_extents: HashMap::new(),
        })
        .collect()
}

async fn build_phase4_ir_and_sidecars(
    phase3: FacetBandPhase3Ir,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    band_scale: &ConfiguredScaleWithSpec,
    initial_subplot_band_size: f32,
    eval_ctx: &EvaluationContext,
) -> Result<(FacetBandPhase4Ir, FacetBandPhase4Sidecars), AvengerChartError> {
    let cell_plans: Vec<FacetCellPlan> = phase3.cells.iter().map(FacetCellPlan::from).collect();
    let legacy_phase4 = prepare_phase4_measurement_inputs_legacy(
        cell_plans,
        &phase3.node_id.facet_path,
        data_df,
        compiled_subplot,
        phase3.empty_cell_policy,
        eval_ctx,
    )
    .await?;

    let renderable_mask =
        renderable_mask_for_cells(&legacy_phase4.plan.cells, phase3.empty_cell_policy);
    let data_overrides = legacy_phase4
        .plan
        .cells
        .iter()
        .map(|cell| cell.data_override.clone())
        .collect::<Vec<_>>();
    let scale_artifacts = legacy_phase4.plan.scale_artifacts.clone();

    let phase4 = FacetBandPhase4Ir {
        phase3: phase3.clone(),
        renderable_mask,
        scale_artifacts_key: FacetScaleNodeKey::new(compiled_subplot, &phase3.node_id.facet_path),
    };
    let sidecars = FacetBandPhase4Sidecars {
        data_overrides,
        subplot_eval_ctx: legacy_phase4.subplot_eval_ctx,
        nested_measure_ctx: legacy_phase4.nested_measure_ctx,
        compiled_subplot: compiled_subplot.clone(),
        original_band_scale: band_scale.configured().clone(),
        initial_subplot_band_size,
        scale_artifacts,
    };
    Ok((phase4, sidecars))
}

async fn run_phase5_overflow_probe_ir(
    phase4: &FacetBandPhase4Ir,
    sidecars: &FacetBandPhase4Sidecars,
    subplot_plot_width: f32,
    subplot_plot_height: f32,
) -> Result<FacetBandPhase5Ir, AvengerChartError> {
    assert_eq!(
        phase4.renderable_mask.len(),
        phase4.phase3.cells.len(),
        "FacetBand phase-4 IR invariant violated: renderable mask length mismatch"
    );
    let expected_key = FacetScaleNodeKey::new(
        &sidecars.compiled_subplot,
        &phase4.phase3.node_id.facet_path,
    );
    assert_eq!(
        phase4.scale_artifacts_key, expected_key,
        "FacetBand phase-4 IR invariant violated: scale artifacts key mismatch"
    );

    let cells = phase4_cells_as_drafts(phase4, sidecars);
    let overflow_probe_summary = run_phase5_overflow_probe_legacy(
        &cells,
        subplot_plot_width,
        subplot_plot_height,
        &sidecars.compiled_subplot,
        &sidecars.subplot_eval_ctx,
        &sidecars.nested_measure_ctx,
        phase4.phase3.empty_cell_policy,
    )
    .await?;

    Ok(FacetBandPhase5Ir {
        phase4: phase4.clone(),
        overflow_probe_summary,
    })
}

async fn build_extent_builder_for_cell(
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    nested_ctx: &FacetBandNestedMeasureContext,
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
    nested_ctx: &FacetBandNestedMeasureContext,
) -> HashMap<String, ChannelDomainExtent> {
    raw_extents
        .into_iter()
        .map(|(channel, extent)| {
            let sharing_level = nested_ctx.facet_tree.channel_sharing_level_typed(&channel);
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
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Result<OverflowProbeSummary, AvengerChartError> {
    let mut summary = OverflowProbeSummary::default();
    summary.cell_overflows.reserve(cells.len());

    for (idx, cell) in cells.iter().enumerate() {
        if !cell.plan.has_data_rows {
            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                "FacetBand overflow probe empty cell"
            );
        }

        let cell_eval_ctx = if !cell.plan.has_data_rows
            && matches!(cell.plan.empty_kind, FacetCellEmptyKind::DomainPlaceholder)
            && matches!(
                empty_cell_policy.effective(),
                FacetEmptyCellPolicy::EmptySubplot
            ) {
            subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
        } else {
            subplot_eval_ctx.clone()
        };

        let MeasuredFacetCell { measurement, .. } = measure_nested_cell(
            &cell.plan,
            &cell.data_override,
            subplot_plot_width,
            subplot_plot_height,
            compiled_subplot,
            &cell_eval_ctx,
            nested_ctx,
        )
        .await?;

        let guide_overflow = measurement.layout.overflow.clone();
        let total_overflow = measurement.layout.total_overflow.clone();

        if let Some(child_facet_col) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
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
            "FacetBand overflow probe result"
        );

        summary
            .cell_overflows
            .push((guide_overflow, total_overflow));
    }

    Ok(summary)
}

async fn run_phase5_overflow_probe_legacy(
    cells: &[FacetCellDraft],
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Result<OverflowProbeSummary, AvengerChartError> {
    let overflow_probe_summary = measure_cells_overflow_probe(
        cells,
        subplot_plot_width,
        subplot_plot_height,
        compiled_subplot,
        subplot_eval_ctx,
        nested_ctx,
        empty_cell_policy,
    )
    .await?;

    Ok(overflow_probe_summary)
}

async fn measure_cells_final_and_extents(
    cells: &mut [FacetCellDraft],
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Result<(), AvengerChartError> {
    for (idx, cell) in cells.iter_mut().enumerate() {
        if !cell.plan.has_data_rows {
            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                "FacetBand final measure empty cell"
            );
        }

        let cell_eval_ctx = if !cell.plan.has_data_rows
            && matches!(cell.plan.empty_kind, FacetCellEmptyKind::DomainPlaceholder)
            && matches!(
                empty_cell_policy.effective(),
                FacetEmptyCellPolicy::EmptySubplot
            ) {
            subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
        } else {
            subplot_eval_ctx.clone()
        };

        let MeasuredFacetCell {
            measurement,
            cell_scale_builder,
        } = measure_nested_cell(
            &cell.plan,
            &cell.data_override,
            subplot_plot_width,
            subplot_plot_height,
            compiled_subplot,
            &cell_eval_ctx,
            nested_ctx,
        )
        .await?;

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            subplot_plot_width,
            subplot_plot_height,
            is_empty = !cell.plan.has_data_rows,
            "FacetBand final measure result"
        );

        let local_extents = if cell.plan.has_data_rows {
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

#[derive(Clone, Copy, Debug)]
struct FacetAxisOps {
    axis: FacetAxis,
    scale_name: &'static str,
    facet_label: &'static str,
    missing_scale_err: &'static str,
    missing_mark_err: &'static str,
}

impl FacetAxisOps {
    fn for_axis(axis: FacetAxis) -> Self {
        match axis {
            FacetAxis::Column => Self {
                axis,
                scale_name: "column",
                facet_label: "FacetColumn",
                missing_scale_err: "No column scale found",
                missing_mark_err: "FacetColumn coord requires a CompiledFacetCol mark",
            },
            FacetAxis::Row => Self {
                axis,
                scale_name: "row",
                facet_label: "FacetRow",
                missing_scale_err: "No row scale found",
                missing_mark_err: "FacetRow coord requires a CompiledFacetRow mark",
            },
        }
    }

    fn matches_mark(self, facet_mark: &FacetMarkRef<'_>) -> bool {
        matches!(
            (self.axis, facet_mark),
            (FacetAxis::Column, FacetMarkRef::Col(_)) | (FacetAxis::Row, FacetMarkRef::Row(_))
        )
    }

    fn enumeration_axis(self) -> FacetAxis {
        self.axis
    }

    fn derive_outer_edges(
        self,
        pass1: &OverflowProbeSummary,
        first_edge_idx: usize,
        last_edge_idx: usize,
    ) -> (f32, f32) {
        match self.axis {
            FacetAxis::Column => {
                let outer_start = pass1
                    .cell_overflows
                    .get(first_edge_idx)
                    .map(|(guide, total)| (total.left - guide.left).max(0.0))
                    .unwrap_or(0.0);
                let outer_end = pass1
                    .cell_overflows
                    .get(last_edge_idx)
                    .map(|(guide, total)| (total.right - guide.right).max(0.0))
                    .unwrap_or(0.0);
                (outer_start, outer_end)
            }
            FacetAxis::Row => {
                let outer_start = pass1
                    .cell_overflows
                    .get(first_edge_idx)
                    .map(|(guide, total)| (total.top - guide.top).max(0.0))
                    .unwrap_or(0.0);
                let outer_end = pass1
                    .cell_overflows
                    .get(last_edge_idx)
                    .map(|(guide, total)| (total.bottom - guide.bottom).max(0.0))
                    .unwrap_or(0.0);
                (outer_start, outer_end)
            }
        }
    }

    fn measure_dims(self, plot_other_axis: f32, subplot_band_size: f32) -> (f32, f32) {
        match self.axis {
            FacetAxis::Column => (subplot_band_size, plot_other_axis),
            FacetAxis::Row => (plot_other_axis, subplot_band_size),
        }
    }
}

struct FacetBandMeasurePipeline<'a> {
    axis_ops: FacetAxisOps,
    scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    plot_other_axis_size: f32,
    eval_ctx: &'a EvaluationContext,
    data: Option<&'a DataFrame>,
    compiled_marks: &'a [Arc<dyn CompiledMark>],
    facet_path: &'a [ScalarValue],
}

struct FacetBandResolvedNode<'a> {
    compiled_subplot: &'a Arc<CompiledPlot>,
    band_scale: &'a ConfiguredScaleWithSpec,
    subplot_band_size: f32,
    current_sharing_level: SharingLevel,
    coordination_field_identity: String,
    empty_cell_policy: FacetEmptyCellPolicy,
}

enum ResolveBandNodeOutcome<'a> {
    Empty(Box<dyn CoordMeasurement>),
    Ready(FacetBandResolvedNode<'a>),
}

impl<'a> FacetBandMeasurePipeline<'a> {
    fn new(
        axis_ops: FacetAxisOps,
        scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
        plot_other_axis_size: f32,
        eval_ctx: &'a EvaluationContext,
        data: Option<&'a DataFrame>,
        compiled_marks: &'a [Arc<dyn CompiledMark>],
        facet_path: &'a [ScalarValue],
    ) -> Self {
        Self {
            axis_ops,
            scales,
            plot_other_axis_size,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        }
    }

    async fn run(&self) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Stage 1: resolve this facet node and enumerate values for the current path.
        let resolved = match self.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Empty(measurement) => return Ok(measurement),
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
        };
        let cell_values = self.enumerate_cell_values(&resolved);

        debug!(
            axis = ?self.axis_ops.axis,
            facet_path = ?self.facet_path,
            cell_values = ?cell_values,
            "FacetBand enumerated cell values"
        );

        if cell_values.is_empty() {
            return Ok(empty_facet_band_measurement(
                self.axis_ops.enumeration_axis(),
                self.facet_path,
                resolved.compiled_subplot,
                resolved.band_scale,
                resolved.empty_cell_policy,
            ));
        }

        let data_df = self.data.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "{} measure requires data",
                self.axis_ops.facet_label
            ))
        })?;

        // Stage 2: precompute subtree scale builders used by nested sharing measurement.
        ensure_subtree_precomputed(
            self.compiled_marks,
            self.facet_path,
            data_df,
            &self.eval_ctx.facet_tree,
            self.eval_ctx,
        )
        .await?;

        // Stage 3: build geometry-independent per-cell semantic IR for this node.
        let phase3 = self.build_phase3_ir(&resolved, &cell_values)?;

        // Stage 4: geometry-independent IR + sidecars (filtered data + nested context).
        let (phase4, phase4_sidecars) = build_phase4_ir_and_sidecars(
            phase3.clone(),
            data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            self.eval_ctx,
        )
        .await?;

        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, resolved.subplot_band_size);

        // Stage 5: Phase 5 overflow probe IR (non-mutating, estimated slot size).
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &phase4_sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;

        // Stage 6: Phase 6 local layout finalization IR.
        let (phase6, phase6_sidecars) = self
            .run_phase6_local_layout_finalization_ir(&phase5, &phase4_sidecars)
            .await?;

        let phase6 =
            self.build_coord_measurement_from_ir(phase6, phase6_sidecars, &phase4_sidecars);

        #[cfg(any(test, debug_assertions))]
        {
            let legacy_measurement = self
                .run_legacy_pipeline_from_phase3(&resolved, data_df, phase3)
                .await?;
            if let (Some(ir), Some(legacy)) = (
                phase6.as_any().downcast_ref::<FacetBandCoordMeasurement>(),
                legacy_measurement
                    .as_any()
                    .downcast_ref::<FacetBandCoordMeasurement>(),
            ) {
                let report: FacetBandIrParityReport = assert_ir_legacy_parity(ir, legacy);
                trace!(
                    axis = ?self.axis_ops.axis,
                    compared_cells = report.compared_cells,
                    "FacetBand IR parity with legacy pipeline"
                );
            }
        }

        // Stage 7: package runtime cell state for coordination/rendering.
        Ok(phase6)
    }

    fn resolve_node_or_empty(&self) -> Result<ResolveBandNodeOutcome<'a>, AvengerChartError> {
        let facet_mark = self
            .compiled_marks
            .iter()
            .find_map(|m| {
                let mark = facet_mark_ref(m.as_ref())?;
                self.axis_ops.matches_mark(&mark).then_some(mark)
            })
            .ok_or_else(|| {
                AvengerChartError::InternalError(self.axis_ops.missing_mark_err.to_string())
            })?;

        let (compiled_subplot, current_sharing_level, empty_cell_policy) = match facet_mark {
            FacetMarkRef::Col(mark) => (
                mark.compiled_subplot(),
                mark.facet_scale_sharing()
                    .map(SharingLevel::from)
                    .unwrap_or(SharingLevel::FREE),
                mark.facet_empty_cell_policy(),
            ),
            FacetMarkRef::Row(mark) => (
                mark.compiled_subplot(),
                mark.facet_scale_sharing()
                    .map(SharingLevel::from)
                    .unwrap_or(SharingLevel::FREE),
                mark.facet_empty_cell_policy(),
            ),
        };

        let band_scale = self.scales.get(self.axis_ops.scale_name).ok_or_else(|| {
            AvengerChartError::InternalError(self.axis_ops.missing_scale_err.to_string())
        })?;

        let subplot_band_size = bandwidth(&band_scale.configured().config).map_err(|e| {
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
                axis = ?self.axis_ops.axis,
                facet_path = ?self.facet_path,
                "FacetBand path not present in tree; returning empty measurement"
            );
            return Ok(ResolveBandNodeOutcome::Empty(empty_facet_band_measurement(
                self.axis_ops.enumeration_axis(),
                self.facet_path,
                compiled_subplot,
                band_scale,
                empty_cell_policy,
            )));
        };

        Ok(ResolveBandNodeOutcome::Ready(FacetBandResolvedNode {
            compiled_subplot,
            band_scale,
            subplot_band_size,
            current_sharing_level,
            coordination_field_identity: current_node.field.clone(),
            empty_cell_policy,
        }))
    }

    fn enumerate_cell_values(&self, resolved: &FacetBandResolvedNode<'_>) -> Vec<ScalarValue> {
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
            .enumerate_values_for_facet(self.facet_path, resolved.current_sharing_level.raw())
            .unwrap_or_else(fallback)
    }

    fn build_cell_plans_legacy(
        &self,
        cell_values: &[ScalarValue],
    ) -> Result<Vec<FacetCellPlan>, AvengerChartError> {
        build_facet_cell_plans(&self.eval_ctx.facet_tree, self.facet_path, cell_values)
    }

    fn build_phase3_ir(
        &self,
        resolved: &FacetBandResolvedNode<'_>,
        cell_values: &[ScalarValue],
    ) -> Result<FacetBandPhase3Ir, AvengerChartError> {
        FacetBandPhase3Ir::from_tree_and_values(
            self.axis_ops.axis,
            self.facet_path,
            self.facet_path.len() as u8 + 1,
            resolved.coordination_field_identity.clone(),
            resolved.empty_cell_policy,
            &self.eval_ctx.facet_tree,
            cell_values,
        )
    }

    fn derive_layout_plan(
        &self,
        cells: &[FacetCellDraft],
        cell_values: &[ScalarValue],
        pass1: &OverflowProbeSummary,
        empty_cell_policy: FacetEmptyCellPolicy,
    ) -> FacetBandPlan {
        let pass1_renderable_cells = renderable_mask_for_cells(cells, empty_cell_policy);

        let padding_inner_px =
            derive_padding_inner_px_from_probe(self.axis_ops.axis, pass1, &pass1_renderable_cells);

        let (first_edge_idx, last_edge_idx) =
            effective_edge_indices(&pass1_renderable_cells, pass1.cell_overflows.len())
                .unwrap_or((0, 0));
        let (outer_start, outer_end) =
            self.axis_ops
                .derive_outer_edges(pass1, first_edge_idx, last_edge_idx);

        debug!(
            axis = ?self.axis_ops.axis,
            padding_inner_px,
            outer_start,
            outer_end,
            cell_count = cell_values.len(),
            first_edge_idx,
            last_edge_idx,
            "FacetBand derived local layout"
        );

        FacetBandPlan {
            padding_inner_px,
            outer_start,
            outer_end,
            n: cell_values.len(),
        }
    }

    fn build_pass2_scale(
        &self,
        band_scale: &ConfiguredScale,
        band_plan: &FacetBandPlan,
        cell_values: &[ScalarValue],
        initial_subplot_band_size: f32,
    ) -> Result<f32, AvengerChartError> {
        let pass2_layout = CoordinatedLayout {
            padding_inner_px: band_plan.padding_inner_px,
            outer_start: band_plan.outer_start,
            outer_end: band_plan.outer_end,
            n: band_plan.n,
        };

        let updated_band_scale = apply_facet_band_scale_layout(
            self.axis_ops.axis,
            band_scale,
            &pass2_layout,
            Some(cell_values),
            None,
            ScaleLayoutRewriteMode::MeasurementPass {
                side_specific_outer_edges: true,
            },
        );

        let final_subplot_band_size = bandwidth(&updated_band_scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to get final bandwidth: {}", e))
        })?;

        debug!(
            axis = ?self.axis_ops.axis,
            initial_subplot_band_size,
            final_subplot_band_size,
            "FacetBand pass 2 scale bandwidth"
        );

        Ok(final_subplot_band_size)
    }

    async fn run_phase6_local_layout_finalization_legacy(
        &self,
        mut plan: FacetBandMeasurePlan,
        phase5_summary: &OverflowProbeSummary,
        initial_subplot_band_size: f32,
        compiled_subplot: &Arc<CompiledPlot>,
        subplot_eval_ctx: &EvaluationContext,
        nested_measure_ctx: &FacetBandNestedMeasureContext,
        band_scale: &ConfiguredScale,
        empty_cell_policy: FacetEmptyCellPolicy,
    ) -> Result<LegacyFacetPhase6LocalFinalization, AvengerChartError> {
        let band_layout_plan = self.derive_layout_plan(
            &plan.cells,
            &plan.cell_values,
            phase5_summary,
            empty_cell_policy,
        );
        let final_subplot_band_size = self.build_pass2_scale(
            band_scale,
            &band_layout_plan,
            &plan.cell_values,
            initial_subplot_band_size,
        )?;
        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, final_subplot_band_size);

        measure_cells_final_and_extents(
            &mut plan.cells,
            subplot_plot_width,
            subplot_plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_measure_ctx,
            empty_cell_policy,
        )
        .await?;

        Ok(LegacyFacetPhase6LocalFinalization {
            band_layout_plan,
            final_subplot_main_or_cross_size: final_subplot_band_size,
            cells: plan.cells,
        })
    }

    async fn run_phase6_local_layout_finalization_ir(
        &self,
        phase5: &FacetBandPhase5Ir,
        phase4_sidecars: &FacetBandPhase4Sidecars,
    ) -> Result<(FacetBandPhase6Ir, FacetBandPhase6Sidecars), AvengerChartError> {
        let plan = FacetBandMeasurePlan {
            cell_values: phase5.phase4.phase3.cell_values.clone(),
            cells: phase4_cells_as_drafts(&phase5.phase4, phase4_sidecars),
            scale_artifacts: phase4_sidecars.scale_artifacts.clone(),
        };
        let legacy_phase6 = self
            .run_phase6_local_layout_finalization_legacy(
                plan,
                &phase5.overflow_probe_summary,
                phase4_sidecars.initial_subplot_band_size,
                &phase4_sidecars.compiled_subplot,
                &phase4_sidecars.subplot_eval_ctx,
                &phase4_sidecars.nested_measure_ctx,
                &phase4_sidecars.original_band_scale,
                phase5.phase4.phase3.empty_cell_policy,
            )
            .await?;

        let channel_sharing_levels = collect_channel_sharing_levels(&legacy_phase6.cells);
        let mut measurements = Vec::with_capacity(legacy_phase6.cells.len());
        let mut local_domain_extents = Vec::with_capacity(legacy_phase6.cells.len());
        for mut cell in legacy_phase6.cells {
            measurements.push(cell.measurement.take().expect(
                "FacetBand IR invariant violated: missing final cell measurement in phase-6 sidecars",
            ));
            local_domain_extents.push(cell.local_domain_extents);
        }

        let phase6 = FacetBandPhase6Ir {
            phase5: phase5.clone(),
            band_layout_plan: legacy_phase6.band_layout_plan,
            final_subplot_cross_size: legacy_phase6.final_subplot_main_or_cross_size,
            channel_sharing_levels,
        };
        let sidecars = FacetBandPhase6Sidecars {
            measurements,
            local_domain_extents,
        };
        Ok((phase6, sidecars))
    }

    fn build_coord_measurement_from_ir(
        &self,
        phase6: FacetBandPhase6Ir,
        phase6_sidecars: FacetBandPhase6Sidecars,
        phase4_sidecars: &FacetBandPhase4Sidecars,
    ) -> Box<dyn CoordMeasurement> {
        let FacetBandPhase6Ir {
            phase5,
            band_layout_plan,
            final_subplot_cross_size,
            channel_sharing_levels,
        } = phase6;
        let FacetBandPhase5Ir { phase4, .. } = phase5;
        let FacetBandPhase4Ir { phase3, .. } = phase4;
        let crate::facet::band_ir::FacetBandPhase3Ir {
            facet_depth,
            coordination_field_identity,
            empty_cell_policy,
            cells,
            ..
        } = phase3;

        assert_eq!(
            cells.len(),
            phase4_sidecars.data_overrides.len(),
            "FacetBand IR/sidecar invariant violated: phase-3 cell count must equal phase-4 data overrides"
        );
        assert_eq!(
            cells.len(),
            phase6_sidecars.measurements.len(),
            "FacetBand IR/sidecar invariant violated: phase-3 cell count must equal phase-6 measurements"
        );
        assert_eq!(
            cells.len(),
            phase6_sidecars.local_domain_extents.len(),
            "FacetBand IR/sidecar invariant violated: phase-3 cell count must equal phase-6 local extents"
        );

        let cell_runtimes: Vec<FacetCellRuntime> = cells
            .into_iter()
            .zip(phase4_sidecars.data_overrides.iter().cloned())
            .zip(
                phase6_sidecars
                    .measurements
                    .into_iter()
                    .zip(phase6_sidecars.local_domain_extents.into_iter()),
            )
            .map(
                |((cell, data_override), (measurement, local_domain_extents))| FacetCellRuntime {
                    data_override,
                    plan: FacetCellPlan::from(&cell),
                    measurement,
                    local_domain_extents,
                    coordinated_domain_extents: HashMap::new(),
                },
            )
            .collect();

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_layout_plan.padding_inner_px,
            outer_start: band_layout_plan.outer_start,
            outer_end: band_layout_plan.outer_end,
            n: band_layout_plan.n,
        };

        Box::new(FacetBandCoordMeasurement {
            axis: self.axis_ops.axis,
            cells: cell_runtimes,
            shared_scale_builder: phase4_sidecars.scale_artifacts.shared_scale_builder.clone(),
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: phase4_sidecars.compiled_subplot.clone(),
            subplot_cross_size: final_subplot_cross_size,
            facet_depth,
            original_band_scale: phase4_sidecars.original_band_scale.clone(),
            local_layout,
            coordinated_layout: None,
            coordination_field_identity,
            channel_sharing_levels,
            empty_cell_policy,
        })
    }

    fn build_coord_measurement_legacy(
        &self,
        band_plan: FacetBandPlan,
        cells: Vec<FacetCellDraft>,
        shared_scale_builder: ScaleBuilder,
        compiled_subplot: &Arc<CompiledPlot>,
        final_subplot_band_size: f32,
        original_band_scale: ConfiguredScale,
        coordination_field_identity: String,
        empty_cell_policy: FacetEmptyCellPolicy,
    ) -> Box<dyn CoordMeasurement> {
        let channel_sharing_levels = collect_channel_sharing_levels(&cells);
        let cell_runtimes: Vec<FacetCellRuntime> = cells
            .into_iter()
            .map(|cell| FacetCellRuntime {
                data_override: cell.data_override,
                plan: cell.plan,
                measurement: cell.measurement.expect(
                    "FacetBand internal invariant violated: missing final cell measurement",
                ),
                local_domain_extents: cell.local_domain_extents,
                coordinated_domain_extents: HashMap::new(),
            })
            .collect();

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_plan.padding_inner_px,
            outer_start: band_plan.outer_start,
            outer_end: band_plan.outer_end,
            n: band_plan.n,
        };

        Box::new(FacetBandCoordMeasurement {
            axis: self.axis_ops.axis,
            cells: cell_runtimes,
            shared_scale_builder,
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: compiled_subplot.clone(),
            subplot_cross_size: final_subplot_band_size,
            facet_depth: self.facet_path.len() as u8 + 1,
            original_band_scale,
            local_layout,
            coordinated_layout: None,
            coordination_field_identity,
            channel_sharing_levels,
            empty_cell_policy,
        })
    }

    async fn run_legacy_pipeline_from_phase3(
        &self,
        resolved: &FacetBandResolvedNode<'_>,
        data_df: &DataFrame,
        phase3: FacetBandPhase3Ir,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        let cell_plans = self.build_cell_plans_legacy(&phase3.cell_values)?;
        let legacy_phase4 = prepare_phase4_measurement_inputs_legacy(
            cell_plans,
            self.facet_path,
            data_df,
            resolved.compiled_subplot,
            resolved.empty_cell_policy,
            self.eval_ctx,
        )
        .await?;

        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, resolved.subplot_band_size);
        let phase5_summary = run_phase5_overflow_probe_legacy(
            &legacy_phase4.plan.cells,
            subplot_plot_width,
            subplot_plot_height,
            resolved.compiled_subplot,
            &legacy_phase4.subplot_eval_ctx,
            &legacy_phase4.nested_measure_ctx,
            resolved.empty_cell_policy,
        )
        .await?;

        let shared_scale_builder = legacy_phase4
            .plan
            .scale_artifacts
            .shared_scale_builder
            .clone();
        let legacy_phase6 = self
            .run_phase6_local_layout_finalization_legacy(
                legacy_phase4.plan,
                &phase5_summary,
                resolved.subplot_band_size,
                resolved.compiled_subplot,
                &legacy_phase4.subplot_eval_ctx,
                &legacy_phase4.nested_measure_ctx,
                resolved.band_scale.configured(),
                resolved.empty_cell_policy,
            )
            .await?;

        Ok(self.build_coord_measurement_legacy(
            legacy_phase6.band_layout_plan,
            legacy_phase6.cells,
            shared_scale_builder,
            resolved.compiled_subplot,
            legacy_phase6.final_subplot_main_or_cross_size,
            resolved.band_scale.configured().clone(),
            phase3.coordination_field_identity,
            resolved.empty_cell_policy,
        ))
    }
}

pub(crate) async fn measure_facet_row(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    FacetBandMeasurePipeline::new(
        FacetAxisOps::for_axis(FacetAxis::Row),
        scales,
        plot_width,
        eval_ctx,
        data,
        compiled_marks,
        facet_path,
    )
    .run()
    .await
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
        FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
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
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::guide::FacetDirection;
    use crate::prelude::*;
    use crate::theme::Theme;
    use avenger_scales::scales::band::BandScale;
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::sync::Arc;

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

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn sample_tree_for_cell_plan_tests() -> crate::facet::evaluated_facet_tree::EvaluatedFacetTree {
        let child = PartitionNode::leaf_with_observed(
            FacetDirection::Column,
            255,
            "team".to_string(),
            None,
            vec![s("A"), s("B")],
            vec![s("A")],
        );
        let mut children = IndexMap::new();
        children.insert(s("Group"), Box::new(child));
        let root =
            PartitionNode::branch(FacetDirection::Row, 255, "dept".to_string(), None, children);
        crate::facet::evaluated_facet_tree::EvaluatedFacetTree::new(Some(root))
    }

    struct FacetBandPipelineFixture {
        scales: HashMap<String, ConfiguredScaleWithSpec>,
        eval_ctx: EvaluationContext,
        data_df: DataFrame,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        plot_other_axis_size: f32,
    }

    async fn build_facet_band_pipeline_fixture(
        axis: FacetAxis,
    ) -> Result<FacetBandPipelineFixture, AvengerChartError> {
        let session = SessionContext::new();
        let data_df = session
            .sql(
                "SELECT * FROM (VALUES \
                 ('A', 'R1', 1.0, 10.0), \
                 ('B', 'R2', 2.0, 20.0), \
                 ('A', 'R2', 3.0, 30.0) \
                 ) AS t(col_group, row_group, x, y)",
            )
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        let inner_subplot = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill("#4682b4")
                .size(42.0),
        );

        let compiled_plot = match axis {
            FacetAxis::Column => {
                Plot::<FacetColumn>::new()
                    .data(data_df.clone())
                    .mark(
                        Facet::new()
                            .col_with(col("col_group"), |c| c)
                            .subplot(inner_subplot.clone()),
                    )
                    .compile(&session)
                    .await?
            }
            FacetAxis::Row => {
                Plot::<FacetRow>::new()
                    .data(data_df.clone())
                    .mark(
                        Facet::new()
                            .row_with(col("row_group"), |c| c)
                            .subplot(inner_subplot),
                    )
                    .compile(&session)
                    .await?
            }
        };

        let facet_tree =
            Arc::new(EvaluatedFacetTree::from_compiled_plot(&compiled_plot, &session).await?);
        let eval_ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(session.clone()),
            IndexMap::new(),
            facet_tree,
        );

        let scale_builder = build_scale_builder_from_marks(
            &compiled_plot.marks,
            &compiled_plot.scale_specs,
            &compiled_plot.coord_transform,
            &compiled_plot.data,
            None,
            &session,
            &eval_ctx.params,
            compiled_plot.get_theme().as_ref(),
        )
        .await?;

        let plot_width = 320.0;
        let plot_height = 240.0;
        let scales = compiled_plot
            .build_scales_from_builder(
                &scale_builder,
                plot_width,
                plot_height,
                &session,
                &eval_ctx.params,
            )
            .await?;

        let plot_other_axis_size = match axis {
            FacetAxis::Column => plot_height,
            FacetAxis::Row => plot_width,
        };

        Ok(FacetBandPipelineFixture {
            scales,
            eval_ctx,
            data_df,
            compiled_marks: compiled_plot.marks.clone(),
            plot_other_axis_size,
        })
    }

    async fn build_phase_measurement_fixture()
    -> Result<(Arc<CompiledPlot>, LegacyFacetPhase4Prep), AvengerChartError> {
        let session = SessionContext::new();
        let data_df = session
            .sql(
                "SELECT * FROM (VALUES \
                 ('Group', 'A', 1.0, 10.0), \
                 ('Group', 'A', 2.0, 20.0) \
                 ) AS t(dept, team, x, y)",
            )
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        let subplot_plot = Plot::<Cartesian>::new().data(data_df.clone()).mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill("#4682b4")
                .size(42.0),
        );
        let compiled_subplot = Arc::new(subplot_plot.compile(&session).await?);

        let facet_tree = Arc::new(sample_tree_for_cell_plan_tests());
        let eval_ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(session.clone()),
            IndexMap::new(),
            facet_tree.clone(),
        );

        let facet_path = vec![s("Group")];
        let cell_plans =
            build_facet_cell_plans(&facet_tree, &facet_path, &[s("A"), s("B"), s("C")])?;
        let phase4 = prepare_phase4_measurement_inputs_legacy(
            cell_plans,
            &facet_path,
            &data_df,
            &compiled_subplot,
            FacetEmptyCellPolicy::Hole,
            &eval_ctx,
        )
        .await?;

        Ok((compiled_subplot, phase4))
    }

    async fn build_coord_measurement_for_apply_plan_tests(
        axis: FacetAxis,
    ) -> Result<(Box<dyn CoordMeasurement>, EvaluationContext), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(axis).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(axis),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let measurement = pipeline.run().await?;
        Ok((measurement, fixture.eval_ctx))
    }

    #[test]
    fn layout_from_measurement_or_local_prefers_coordinated() {
        let local = CoordinatedLayout {
            padding_inner_px: 4.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 2,
        };
        let coordinated = CoordinatedLayout {
            padding_inner_px: 10.0,
            outer_start: 5.0,
            outer_end: 6.0,
            n: 4,
        };
        let selected = layout_from_measurement_or_local(&local, Some(&coordinated));
        assert_eq!(selected.padding_inner_px, coordinated.padding_inner_px);
        assert_eq!(selected.outer_start, coordinated.outer_start);
        assert_eq!(selected.outer_end, coordinated.outer_end);
        assert_eq!(selected.n, coordinated.n);

        let fallback = layout_from_measurement_or_local(&local, None);
        assert_eq!(fallback.padding_inner_px, local.padding_inner_px);
        assert_eq!(fallback.outer_start, local.outer_start);
        assert_eq!(fallback.outer_end, local.outer_end);
        assert_eq!(fallback.n, local.n);
    }

    #[test]
    fn apply_facet_band_scale_layout_applies_domain_padding_and_range() {
        let base = make_band_scale((0.0, 300.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 12.0,
            outer_start: 10.0,
            outer_end: 20.0,
            n: 3,
        };
        let domain_override = vec![
            ScalarValue::Utf8(Some("x".to_string())),
            ScalarValue::Utf8(Some("y".to_string())),
            ScalarValue::Utf8(Some("z".to_string())),
        ];

        let updated = apply_facet_band_scale_layout(
            FacetAxis::Column,
            &base,
            &layout,
            Some(domain_override.as_slice()),
            None,
            ScaleLayoutRewriteMode::MeasurementPass {
                side_specific_outer_edges: true,
            },
        );

        let (start, end) = updated.config.numeric_interval_range().unwrap();
        assert_eq!(start, 10.0);
        assert_eq!(end, 280.0);
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
    fn set_coordinated_layout_preserves_local_outer_edges() {
        let local = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_start: 11.0,
            outer_end: 12.0,
            n: 2,
        };
        let coordinated = CoordinatedLayout {
            padding_inner_px: 18.0,
            outer_start: 91.0,
            outer_end: 92.0,
            n: 4,
        };

        let merged = coordinated_layout_preserving_outer_edges(&local, coordinated);
        assert_eq!(merged.padding_inner_px, 18.0);
        assert_eq!(merged.n, 4);
        assert_eq!(merged.outer_start, local.outer_start);
        assert_eq!(merged.outer_end, local.outer_end);
    }

    #[test]
    fn apply_facet_band_scale_layout_zero_padding_override_is_optional() {
        let base = make_band_scale((0.0, 200.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            outer_start: 0.0,
            outer_end: 0.0,
            n: 2,
        };

        let no_override = apply_facet_band_scale_layout(
            FacetAxis::Column,
            &base,
            &layout,
            None,
            None,
            ScaleLayoutRewriteMode::RenderPass {
                allow_zero_padding_override: false,
                side_specific_outer_edges: true,
            },
        );
        assert!(!no_override.config.options.contains_key("padding_inner_px"));

        let with_override = apply_facet_band_scale_layout(
            FacetAxis::Column,
            &base,
            &layout,
            None,
            None,
            ScaleLayoutRewriteMode::RenderPass {
                allow_zero_padding_override: true,
                side_specific_outer_edges: true,
            },
        );
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
    fn apply_facet_band_scale_layout_reserves_start_and_end_edges_independently() {
        let base = make_band_scale((0.0, 300.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 12.0,
            outer_start: 10.0,
            outer_end: 20.0,
            n: 3,
        };

        let updated = apply_facet_band_scale_layout(
            FacetAxis::Column,
            &base,
            &layout,
            None,
            None,
            ScaleLayoutRewriteMode::MeasurementPass {
                side_specific_outer_edges: true,
            },
        );
        let (start, end) = updated.config.numeric_interval_range().unwrap();
        assert_eq!(start, 10.0);
        assert_eq!(end, 280.0);
    }

    #[test]
    fn has_coordinated_layout_change_detects_any_dimension_shift() {
        let local = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 3,
        };
        let same = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 3,
        };
        let changed = CoordinatedLayout {
            padding_inner_px: 6.0,
            outer_start: 1.0,
            outer_end: 2.0,
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

    #[tokio::test]
    async fn derive_coordinated_apply_plan_layout_only_no_remeasure()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let mut coordinated = facet_band.local_layout.clone();
        coordinated.n = coordinated.n.saturating_add(1);
        coordinated.padding_inner_px += 7.0;
        facet_band.set_coordinated_layout_value(coordinated);

        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(plan.has_coordinated_layout);
        assert!(!plan.has_legend_overflow);
        assert!(!plan.has_coordinated_extents);
        assert!(!plan.remeasure_required);
        assert!((plan.adjusted_main_size - plan.original_main_size).abs() <= 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn derive_coordinated_apply_plan_legend_overflow_requires_remeasure()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.coordinated_overflow.total.top = 12.0;

        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(plan.has_legend_overflow);
        assert!(plan.remeasure_required);
        assert!(plan.adjusted_main_size < plan.original_main_size);
        Ok(())
    }

    #[tokio::test]
    async fn derive_coordinated_apply_plan_coordinated_extents_requires_remeasure()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.cells[0]
            .coordinated_domain_extents
            .insert("x".to_string(), DomainExtent::numeric(0.0, 99.0));

        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(plan.has_coordinated_extents);
        assert!(plan.remeasure_required);
        Ok(())
    }

    #[tokio::test]
    async fn derive_coordinated_apply_plan_axis_owner_ignore_empty_cells_matches_holes()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.empty_cell_policy = FacetEmptyCellPolicy::Auto;

        for cell in facet_band.cells.iter_mut() {
            cell.plan.is_empty = false;
        }

        let no_holes_plan = facet_band.derive_coordinated_apply_plan();
        assert!(!no_holes_plan.has_holes);
        assert!(!no_holes_plan.axis_owner_ignore_empty_cells);

        facet_band.cells[0].plan.is_empty = true;
        let with_holes_plan = facet_band.derive_coordinated_apply_plan();
        assert!(with_holes_plan.has_holes);
        assert!(with_holes_plan.axis_owner_ignore_empty_cells);
        Ok(())
    }

    #[tokio::test]
    async fn apply_coordinated_overflow_with_plan_layout_only_updates_cross_size()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let mut coordinated = facet_band.local_layout.clone();
        coordinated.n = coordinated.n.saturating_add(3);
        coordinated.padding_inner_px += 9.0;
        facet_band.set_coordinated_layout_value(coordinated);

        let before_cross_size = facet_band.subplot_cross_size;
        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(!plan.remeasure_required);
        let outcome = facet_band
            .apply_coordinated_overflow_with_plan(&eval_ctx, &plan)
            .await?;

        assert!((outcome.subplot_cross_size_before - before_cross_size).abs() <= 0.01);
        assert!(!outcome.remeasure_triggered);
        assert_eq!(outcome.remeasured_cell_count, 0);
        assert_eq!(outcome.remeasured_non_empty_cell_count, 0);
        assert_eq!(outcome.remeasured_with_coordinated_extents_count, 0);
        assert!((facet_band.subplot_cross_size - before_cross_size).abs() > 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn apply_coordinated_overflow_with_plan_remeasures_cells_when_required()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.coordinated_overflow.total.top = 18.0;
        let expected_cells = facet_band.cells.len();
        let first_before = facet_band
            .cells
            .first()
            .expect("expected at least one facet cell")
            .measurement
            .plot_area_height;
        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(plan.remeasure_required);

        let outcome = facet_band
            .apply_coordinated_overflow_with_plan(&eval_ctx, &plan)
            .await?;
        let first_after = facet_band
            .cells
            .first()
            .expect("expected at least one facet cell")
            .measurement
            .plot_area_height;

        assert!(outcome.remeasure_triggered);
        assert_eq!(outcome.remeasured_cell_count, expected_cells);
        assert!(outcome.remeasured_non_empty_cell_count > 0);
        assert!(
            outcome.remeasured_non_empty_cell_count <= outcome.remeasured_cell_count,
            "non-empty count must be bounded by total remeasured cells"
        );
        assert!(
            outcome.remeasured_with_coordinated_extents_count <= outcome.remeasured_cell_count,
            "coordinated-extents count must be bounded by total remeasured cells"
        );
        assert!(first_after < first_before);
        Ok(())
    }

    #[tokio::test]
    async fn apply_coordinated_overflow_with_explicit_remeasure_plan_matches_intent_counts()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.coordinated_overflow.total.top = 18.0;
        facet_band.cells[0]
            .coordinated_domain_extents
            .insert("x".to_string(), DomainExtent::numeric(0.0, 99.0));

        let plan = facet_band.derive_coordinated_apply_plan();
        assert!(plan.remeasure_required);
        let explicit_remeasure_plan =
            crate::facet::coordination_remeasure::derive_facet_coord_remeasure_plan(
                &facet_band.cells,
                &plan,
                facet_band.subplot_cross_size,
            );
        let expected_cell_count = explicit_remeasure_plan.cell_intents.len();
        let expected_non_empty_count = explicit_remeasure_plan
            .cell_intents
            .iter()
            .filter(|intent| intent.has_data_rows)
            .count();
        let expected_with_extents_count = explicit_remeasure_plan
            .cell_intents
            .iter()
            .filter(|intent| intent.use_coordinated_extents)
            .count();

        let outcome = facet_band
            .apply_coordinated_overflow_with_plan_and_remeasure_plan(
                &eval_ctx,
                &plan,
                Some(&explicit_remeasure_plan),
            )
            .await?;

        assert!(outcome.remeasure_triggered);
        assert_eq!(outcome.remeasured_cell_count, expected_cell_count);
        assert_eq!(
            outcome.remeasured_non_empty_cell_count,
            expected_non_empty_count
        );
        assert_eq!(
            outcome.remeasured_with_coordinated_extents_count,
            expected_with_extents_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn coord_remeasure_updates_all_cells_and_returns_per_cell_outcomes()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let request = crate::facet::coordination_remeasure::FacetCoordRemeasureRequest {
            axis: FacetAxis::Column,
            subplot_cross_size: facet_band.subplot_cross_size,
            adjusted_main_size: facet_band
                .cells
                .first()
                .map(|cell| cell.measurement.plot_area_height)
                .unwrap_or(120.0),
            axis_owner_ignore_empty_cells: false,
        };

        let outcome = crate::facet::coordination_remeasure::run_facet_coord_remeasure(
            &mut facet_band.cells,
            &facet_band.compiled_subplot,
            &facet_band.shared_scale_builder,
            &eval_ctx,
            &request,
        )
        .await?;

        assert_eq!(outcome.cell_outcomes.len(), facet_band.cells.len());
        for (idx, cell_outcome) in outcome.cell_outcomes.iter().enumerate() {
            assert_eq!(cell_outcome.cell_index, idx);
            assert!(cell_outcome.plot_area_width > 0.0);
            assert!(cell_outcome.plot_area_height > 0.0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn coord_remeasure_marks_cells_with_coordinated_extents_usage()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_apply_plan_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.cells[0]
            .coordinated_domain_extents
            .insert("x".to_string(), DomainExtent::numeric(0.0, 10.0));

        let request = crate::facet::coordination_remeasure::FacetCoordRemeasureRequest {
            axis: FacetAxis::Column,
            subplot_cross_size: facet_band.subplot_cross_size,
            adjusted_main_size: facet_band
                .cells
                .first()
                .map(|cell| cell.measurement.plot_area_height)
                .unwrap_or(120.0),
            axis_owner_ignore_empty_cells: false,
        };

        let outcome = crate::facet::coordination_remeasure::run_facet_coord_remeasure(
            &mut facet_band.cells,
            &facet_band.compiled_subplot,
            &facet_band.shared_scale_builder,
            &eval_ctx,
            &request,
        )
        .await?;

        let used_count = outcome
            .cell_outcomes
            .iter()
            .filter(|cell_outcome| cell_outcome.used_coordinated_extents)
            .count();
        assert_eq!(used_count, 1);
        assert!(outcome.cell_outcomes[0].used_coordinated_extents);
        Ok(())
    }

    #[test]
    fn adjusted_size_for_top_legend_overflow_reduces_height() {
        assert_eq!(adjusted_size_for_legend_overflow(291.0, 48.0, 0.0), 243.0);
    }

    #[test]
    fn adjusted_size_for_bottom_legend_overflow_reduces_height() {
        assert_eq!(adjusted_size_for_legend_overflow(291.0, 0.0, 48.0), 243.0);
    }

    #[test]
    fn adjusted_size_for_bottom_legend_overflow_clamps_to_minimum() {
        assert_eq!(adjusted_size_for_legend_overflow(20.0, 0.0, 48.0), 1.0);
    }

    #[test]
    fn adjusted_size_for_top_and_bottom_legend_overflow_reduces_height() {
        assert_eq!(adjusted_size_for_legend_overflow(291.0, 20.0, 28.0), 243.0);
    }

    #[test]
    fn facet_axis_ops_measure_dims_column_and_row() {
        let column_ops = FacetAxisOps::for_axis(FacetAxis::Column);
        assert_eq!(column_ops.measure_dims(240.0, 80.0), (80.0, 240.0));

        let row_ops = FacetAxisOps::for_axis(FacetAxis::Row);
        assert_eq!(row_ops.measure_dims(320.0, 60.0), (320.0, 60.0));
    }

    #[test]
    fn facet_axis_ops_outer_edge_extraction_column_vs_row() {
        let probe = OverflowProbeSummary {
            cell_overflows: vec![
                (
                    OverflowSpaceRequirement {
                        top: 1.0,
                        right: 2.0,
                        bottom: 3.0,
                        left: 4.0,
                    },
                    OverflowSpaceRequirement {
                        top: 5.0,
                        right: 6.0,
                        bottom: 7.0,
                        left: 8.0,
                    },
                ),
                (
                    OverflowSpaceRequirement {
                        top: 9.0,
                        right: 10.0,
                        bottom: 11.0,
                        left: 12.0,
                    },
                    OverflowSpaceRequirement {
                        top: 13.0,
                        right: 14.0,
                        bottom: 15.0,
                        left: 16.0,
                    },
                ),
            ],
            max_child_padding: 0.0,
        };

        let (col_start, col_end) =
            FacetAxisOps::for_axis(FacetAxis::Column).derive_outer_edges(&probe, 0, 1);
        assert_eq!(col_start, 4.0);
        assert_eq!(col_end, 4.0);

        let (row_start, row_end) =
            FacetAxisOps::for_axis(FacetAxis::Row).derive_outer_edges(&probe, 0, 1);
        assert_eq!(row_start, 4.0);
        assert_eq!(row_end, 4.0);
    }

    #[test]
    fn derive_layout_plan_does_not_depend_on_child_padding() {
        let pass1 = OverflowProbeSummary {
            cell_overflows: vec![
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement {
                        right: 7.0,
                        ..Default::default()
                    },
                ),
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement {
                        left: 5.0,
                        ..Default::default()
                    },
                ),
            ],
            max_child_padding: 50.0,
        };
        let renderable_cells = vec![true, true];

        assert_eq!(
            derive_padding_inner_px_from_probe(FacetAxis::Column, &pass1, &renderable_cells),
            12.0
        );
    }

    #[test]
    fn derive_layout_plan_propagates_moderate_child_padding() {
        let pass1 = OverflowProbeSummary {
            cell_overflows: vec![
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement {
                        right: 7.0,
                        ..Default::default()
                    },
                ),
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement {
                        left: 5.0,
                        ..Default::default()
                    },
                ),
            ],
            max_child_padding: 18.0,
        };
        let renderable_cells = vec![true, true];

        assert_eq!(
            derive_padding_inner_px_from_probe(FacetAxis::Column, &pass1, &renderable_cells),
            18.0
        );
    }

    #[test]
    fn renderable_for_empty_policy_respects_hole_vs_empty_subplot() {
        assert!(renderable_for_empty_policy(
            FacetEmptyCellPolicy::Hole,
            false
        ));
        assert!(!renderable_for_empty_policy(
            FacetEmptyCellPolicy::Hole,
            true
        ));
        assert!(renderable_for_empty_policy(
            FacetEmptyCellPolicy::EmptySubplot,
            true
        ));
        assert!(!renderable_for_empty_policy(
            FacetEmptyCellPolicy::Auto,
            true
        ));
    }

    #[test]
    fn build_facet_cell_distinguishes_domain_placeholder_and_data_empty() {
        let tree = sample_tree_for_cell_plan_tests();

        let data_empty = build_facet_cell(&tree, &[s("Group")], &s("B")).unwrap();
        assert!(data_empty.in_domain_slot);
        assert!(!data_empty.has_data_rows);
        assert!(data_empty.is_empty);
        assert_eq!(data_empty.empty_kind, FacetCellEmptyKind::DataEmpty);

        let placeholder = build_facet_cell(&tree, &[s("Group")], &s("C")).unwrap();
        assert!(!placeholder.in_domain_slot);
        assert!(!placeholder.has_data_rows);
        assert!(placeholder.is_empty);
        assert_eq!(
            placeholder.empty_kind,
            FacetCellEmptyKind::DomainPlaceholder
        );
    }

    #[test]
    fn build_facet_cell_plans_preserves_enumeration_order() {
        let tree = sample_tree_for_cell_plan_tests();
        let cell_values = vec![s("B"), s("C"), s("A")];
        let plans = build_facet_cell_plans(&tree, &[s("Group")], &cell_values).unwrap();
        let planned_values: Vec<ScalarValue> = plans.into_iter().map(|p| p.value).collect();
        assert_eq!(planned_values, cell_values);
    }

    #[test]
    fn build_facet_cell_plans_preserves_empty_kind_classification() {
        let tree = sample_tree_for_cell_plan_tests();
        let plans = build_facet_cell_plans(&tree, &[s("Group")], &[s("B"), s("C")]).unwrap();

        let data_empty = &plans[0];
        assert!(data_empty.in_domain_slot);
        assert!(!data_empty.has_data_rows);
        assert_eq!(data_empty.empty_kind, FacetCellEmptyKind::DataEmpty);

        let placeholder = &plans[1];
        assert!(!placeholder.in_domain_slot);
        assert!(!placeholder.has_data_rows);
        assert_eq!(
            placeholder.empty_kind,
            FacetCellEmptyKind::DomainPlaceholder
        );
    }

    #[test]
    fn build_facet_cell_plans_preserves_predicate_parity_for_slots() {
        let tree = sample_tree_for_cell_plan_tests();
        let plans = build_facet_cell_plans(&tree, &[s("Group")], &[s("A"), s("C")]).unwrap();

        let in_domain_from_helper = &plans[0];
        let in_domain_direct = build_facet_cell(&tree, &[s("Group")], &s("A")).unwrap();
        assert_eq!(
            in_domain_from_helper.filter_predicate,
            in_domain_direct.filter_predicate
        );

        let out_of_domain_from_helper = &plans[1];
        assert_eq!(out_of_domain_from_helper.filter_predicate, Some(lit(false)));
    }

    #[tokio::test]
    async fn phase5_overflow_probe_does_not_mutate_cell_drafts() -> Result<(), AvengerChartError> {
        let (compiled_subplot, phase4) = build_phase_measurement_fixture().await?;

        let before: Vec<(bool, usize)> = phase4
            .plan
            .cells
            .iter()
            .map(|cell| (cell.measurement.is_none(), cell.local_domain_extents.len()))
            .collect();

        let phase5 = run_phase5_overflow_probe_legacy(
            &phase4.plan.cells,
            140.0,
            140.0,
            &compiled_subplot,
            &phase4.subplot_eval_ctx,
            &phase4.nested_measure_ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;

        assert_eq!(phase5.cell_overflows.len(), 3);

        let after: Vec<(bool, usize)> = phase4
            .plan
            .cells
            .iter()
            .map(|cell| (cell.measurement.is_none(), cell.local_domain_extents.len()))
            .collect();
        assert_eq!(before, after);

        Ok(())
    }

    #[tokio::test]
    async fn phase6_local_layout_finalization_populates_measurements_and_extents()
    -> Result<(), AvengerChartError> {
        let (compiled_subplot, phase4) = build_phase_measurement_fixture().await?;
        let LegacyFacetPhase4Prep {
            mut plan,
            subplot_eval_ctx,
            nested_measure_ctx,
        } = phase4;

        measure_cells_final_and_extents(
            &mut plan.cells,
            140.0,
            140.0,
            &compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;

        assert!(plan.cells.iter().all(|cell| cell.measurement.is_some()));
        assert!(
            plan.cells
                .iter()
                .filter(|cell| cell.plan.has_data_rows)
                .all(|cell| !cell.local_domain_extents.is_empty())
        );

        Ok(())
    }

    #[tokio::test]
    async fn phase4_ir_sidecars_align_with_cells() -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => {
                panic!("expected ready facet node for phase4 IR test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3.clone(),
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;

        assert_eq!(phase4.phase3.cells.len(), sidecars.data_overrides.len());
        assert_eq!(phase4.phase3.cells.len(), phase4.renderable_mask.len());
        assert_eq!(phase4.phase3.cell_values, cell_values);
        Ok(())
    }

    #[tokio::test]
    async fn phase5_ir_probe_is_non_mutating() -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => {
                panic!("expected ready facet node for phase5 IR test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3,
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;

        let before_rows = sidecars.data_overrides.len();
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;

        assert_eq!(before_rows, sidecars.data_overrides.len());
        assert_eq!(
            phase5.overflow_probe_summary.cell_overflows.len(),
            phase4.phase3.cells.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase6_ir_finalization_produces_layout_and_measurements()
    -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => {
                panic!("expected ready facet node for phase6 IR test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3,
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;
        let (phase6, phase6_sidecars) = pipeline
            .run_phase6_local_layout_finalization_ir(&phase5, &sidecars)
            .await?;

        assert_eq!(phase6.band_layout_plan.n, phase4.phase3.cells.len());
        assert_eq!(
            phase6_sidecars.measurements.len(),
            phase4.phase3.cells.len()
        );
        assert_eq!(
            phase6_sidecars.local_domain_extents.len(),
            phase4.phase3.cells.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn ir_to_coord_measurement_preserves_axis_and_layout_fields()
    -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => {
                panic!("expected ready facet node for coord-measurement IR test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3,
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;
        let (phase6, phase6_sidecars) = pipeline
            .run_phase6_local_layout_finalization_ir(&phase5, &sidecars)
            .await?;
        let expected_n = phase6.band_layout_plan.n;
        let expected_size = phase6.final_subplot_cross_size;
        let measurement =
            pipeline.build_coord_measurement_from_ir(phase6, phase6_sidecars, &sidecars);
        let facet_band = measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");

        assert_eq!(facet_band.axis, FacetAxis::Column);
        assert_eq!(facet_band.local_layout.n, expected_n);
        assert!((facet_band.subplot_cross_size - expected_size).abs() <= 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn ir_phase3_parity_with_legacy_cell_plan_builder() -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => panic!("expected ready facet node"),
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let legacy_plans = pipeline.build_cell_plans_legacy(&cell_values)?;

        let ir_plans: Vec<FacetCellPlan> = phase3.cells.iter().map(FacetCellPlan::from).collect();
        assert_eq!(ir_plans.len(), legacy_plans.len());
        for (ir, legacy) in ir_plans.iter().zip(legacy_plans.iter()) {
            assert_eq!(ir.value, legacy.value);
            assert_eq!(ir.full_path, legacy.full_path);
            assert_eq!(ir.in_domain_slot, legacy.in_domain_slot);
            assert_eq!(ir.has_data_rows, legacy.has_data_rows);
            assert_eq!(ir.empty_kind, legacy.empty_kind);
            assert_eq!(ir.filter_predicate, legacy.filter_predicate);
        }
        Ok(())
    }

    #[tokio::test]
    async fn ir_phase5_parity_with_legacy_probe_summary() -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => panic!("expected ready facet node"),
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3,
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;
        let legacy_cells = phase4_cells_as_drafts(&phase4, &sidecars);
        let legacy_summary = run_phase5_overflow_probe_legacy(
            &legacy_cells,
            subplot_plot_width,
            subplot_plot_height,
            resolved.compiled_subplot,
            &sidecars.subplot_eval_ctx,
            &sidecars.nested_measure_ctx,
            phase4.phase3.empty_cell_policy,
        )
        .await?;

        assert_eq!(
            phase5.overflow_probe_summary.cell_overflows.len(),
            legacy_summary.cell_overflows.len()
        );
        for (ir, legacy) in phase5
            .overflow_probe_summary
            .cell_overflows
            .iter()
            .zip(legacy_summary.cell_overflows.iter())
        {
            assert!((ir.0.top - legacy.0.top).abs() <= 0.01);
            assert!((ir.0.right - legacy.0.right).abs() <= 0.01);
            assert!((ir.0.bottom - legacy.0.bottom).abs() <= 0.01);
            assert!((ir.0.left - legacy.0.left).abs() <= 0.01);
            assert!((ir.1.top - legacy.1.top).abs() <= 0.01);
            assert!((ir.1.right - legacy.1.right).abs() <= 0.01);
            assert!((ir.1.bottom - legacy.1.bottom).abs() <= 0.01);
            assert!((ir.1.left - legacy.1.left).abs() <= 0.01);
        }
        Ok(())
    }

    #[tokio::test]
    async fn ir_phase6_parity_with_legacy_local_layout_and_cell_measurements()
    -> Result<(), AvengerChartError> {
        let fixture = build_facet_band_pipeline_fixture(FacetAxis::Column).await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let resolved = match pipeline.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
            ResolveBandNodeOutcome::Empty(_) => panic!("expected ready facet node"),
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let phase3 = pipeline.build_phase3_ir(&resolved, &cell_values)?;
        let (phase4, sidecars) = build_phase4_ir_and_sidecars(
            phase3.clone(),
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let phase5 = run_phase5_overflow_probe_ir(
            &phase4,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
        )
        .await?;
        let (phase6, phase6_sidecars) = pipeline
            .run_phase6_local_layout_finalization_ir(&phase5, &sidecars)
            .await?;
        let ir_measurement =
            pipeline.build_coord_measurement_from_ir(phase6, phase6_sidecars, &sidecars);
        let legacy_measurement = pipeline
            .run_legacy_pipeline_from_phase3(&resolved, &fixture.data_df, phase3)
            .await?;

        let ir_band = ir_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("expected IR facet measurement");
        let legacy_band = legacy_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("expected legacy facet measurement");
        let report = assert_ir_legacy_parity(ir_band, legacy_band);
        assert_eq!(report.compared_cells, ir_band.cells.len());
        Ok(())
    }

    #[tokio::test]
    async fn facet_band_pipeline_resolve_node_returns_empty_for_invalid_path()
    -> Result<(), AvengerChartError> {
        for axis in [FacetAxis::Column, FacetAxis::Row] {
            let fixture = build_facet_band_pipeline_fixture(axis).await?;
            let invalid_path = vec![ScalarValue::Utf8(Some("missing".to_string()))];
            let pipeline = FacetBandMeasurePipeline::new(
                FacetAxisOps::for_axis(axis),
                &fixture.scales,
                fixture.plot_other_axis_size,
                &fixture.eval_ctx,
                Some(&fixture.data_df),
                &fixture.compiled_marks,
                &invalid_path,
            );

            let outcome = pipeline.resolve_node_or_empty()?;
            assert!(matches!(outcome, ResolveBandNodeOutcome::Empty(_)));
        }

        Ok(())
    }

    #[tokio::test]
    async fn facet_band_pipeline_packages_measurement_with_correct_axis()
    -> Result<(), AvengerChartError> {
        for axis in [FacetAxis::Column, FacetAxis::Row] {
            let fixture = build_facet_band_pipeline_fixture(axis).await?;
            let facet_path: Vec<ScalarValue> = Vec::new();
            let pipeline = FacetBandMeasurePipeline::new(
                FacetAxisOps::for_axis(axis),
                &fixture.scales,
                fixture.plot_other_axis_size,
                &fixture.eval_ctx,
                Some(&fixture.data_df),
                &fixture.compiled_marks,
                &facet_path,
            );

            let measurement = pipeline.run().await?;
            let facet_band = measurement
                .as_any()
                .downcast_ref::<FacetBandCoordMeasurement>()
                .expect("expected FacetBandCoordMeasurement");
            assert_eq!(facet_band.axis, axis);
            assert!(!facet_band.cells.is_empty());
            assert_eq!(
                facet_band.original_band_scale.scale_impl.scale_type(),
                "band"
            );
        }

        Ok(())
    }
}
