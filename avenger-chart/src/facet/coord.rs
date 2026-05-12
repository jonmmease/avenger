//! Facet band synthesis pipeline (FacetColumn and FacetRow).
//!
//! This module implements local per-node evaluation using AG-style semantics:
//! - synthesized attributes: per-cell semantics, overflow probes, local layout, measured cells;
//! - inherited attributes: coordinated values applied later by `coordination.rs`.
//!
//! Reference terminology:
//! - JastAdd concept overview: https://jastadd.cs.lth.se/web/documentation/concept-overview.php
//! - JastAdd reference manual: https://jastadd.cs.lth.se/web/documentation/reference-manual.php
//! - Knuth attribute grammars: https://doi.org/10.1007/BF01692511
//!
use std::{
    any::Any,
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, band::bandwidth};
#[cfg(test)]
use datafusion::logical_expr::lit;
use datafusion::{common::ScalarValue, dataframe::DataFrame, scalar::ScalarValue as DfScalarValue};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::{
    cartesian::axis::AxisPosition,
    coords::{
        CellDomainInfo, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform,
        CoordinatedLayout, CoordinatedOverflow, FacetAxis, OverflowSpaceRequirement, PaddingSpec,
        PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::{
        band_attributes::{
            FacetBandLocalSynthesis, FacetBandMeasuredRuntime, FacetBandOverflowSynthesis,
            FacetBandPreparedRuntime, FacetBandPreparedSynthesis, FacetBandSemantics,
            OverflowProbeSummary,
        },
        coord_row::compute_band_layout,
        coordination::CoordinationGroupKey,
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
        probe_summary::FacetCellProbeSummary,
        scale_precompute::{
            FacetScaleNodeArtifacts, FacetScaleNodeKey, build_node_artifacts, canonicalize_path,
            ensure_subtree_precomputed,
        },
        sharing_level::SharingLevel,
        sharing_policy,
    },
    layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, LayoutBounds},
    legend::LegendPosition,
    marks::CompiledMark,
    plot::compiled::{
        CompiledPlot, ComponentsMeasurement, scale_provider::DynamicScaleProvider,
        scales::build_scale_builder_from_marks,
    },
    render::{
        EvaluationContext, FacetSubtreeCheckpoint, FacetSubtreeSelector, LegendMeasurements,
        context::FacetRuntimeSizingMode,
    },
    scales::{
        ConfiguredScaleWithSpec, PlotAreaRangeEndpoint, ScaleBuilder, ScaleRangeBinding,
        domain_extent::{DomainBounds, DomainExtent, RadiusPadding},
    },
};

#[cfg(test)]
use crate::facet::probe_summary::FacetBandProbeSynthesis;

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

/// Canvas-fit facet band measurement type.
pub type FacetBandCoordMeasurementCanvasFit = FacetBandCoordMeasurement;

/// Fixed-subplot facet band measurement type.
///
/// This wraps the shared facet runtime payload with fixed-mode-only placement data.
pub struct FacetBandCoordMeasurementFixed {
    pub(crate) base: FacetBandCoordMeasurement,
    /// Explicit main-axis positions used by fixed-subplot rendering.
    pub(crate) fixed_main_axis_positions: Vec<f32>,
}

impl Deref for FacetBandCoordMeasurementFixed {
    type Target = FacetBandCoordMeasurement;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for FacetBandCoordMeasurementFixed {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub(crate) fn facet_band_canvas_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&FacetBandCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
}

pub(crate) fn facet_band_fixed_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&FacetBandCoordMeasurementFixed> {
    coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementFixed>()
}

pub(crate) fn facet_band_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&FacetBandCoordMeasurement> {
    facet_band_canvas_ref(coord_measurement)
        .or_else(|| facet_band_fixed_ref(coord_measurement).map(|fixed| &fixed.base))
}

pub(crate) fn facet_band_canvas_mut(
    coord_measurement: &mut dyn CoordMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
}

pub(crate) fn facet_band_fixed_mut(
    coord_measurement: &mut dyn CoordMeasurement,
) -> Option<&mut FacetBandCoordMeasurementFixed> {
    coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurementFixed>()
}

/// Probe-only facet band measurement used during phase-5 synthesized aggregation.
///
/// This carries enough local information for guide overflow measurement and band-scale
/// layout rewrites without materializing child `ComponentsMeasurement` trees.
#[derive(Debug, Clone)]
pub(crate) struct FacetBandProbeMeasurement {
    pub(crate) axis: FacetAxis,
    pub(crate) cell_values: Vec<ScalarValue>,
    pub(crate) cell_has_data_rows: Vec<bool>,
    pub(crate) local_overflow: CoordinatedOverflow,
    pub(crate) original_band_scale: ConfiguredScale,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) fixed_plot_area_lock: bool,
}

impl FacetBandProbeMeasurement {
    pub(crate) fn cell_values(&self) -> impl Iterator<Item = &ScalarValue> {
        self.cell_values.iter()
    }

    pub(crate) fn local_overflow_value(&self) -> CoordinatedOverflow {
        self.local_overflow.clone()
    }
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
    pub(crate) remeasure_skipped_cell_count: usize,
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

    pub(crate) fn apply_cross_axis_coordinated_side_slabs_to_cells(&mut self) {
        let coordinated = self.coordinated_overflow.clone();
        let sides = match self.axis {
            FacetAxis::Column => [AxisPosition::Top, AxisPosition::Bottom],
            FacetAxis::Row => [AxisPosition::Left, AxisPosition::Right],
        };

        for cell in &mut self.cells {
            for side in sides {
                apply_measurement_side_slab(
                    &mut cell.measurement,
                    side,
                    overflow_side_value(&coordinated.guide, side),
                    overflow_side_value(&coordinated.total, side),
                );
            }
        }
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

    pub(crate) fn retarget_parent_plot_area_no_remeasure(
        &mut self,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
        eval_ctx: &EvaluationContext,
        new_plot_area_width: f32,
        new_plot_area_height: f32,
    ) -> Result<(), AvengerChartError> {
        let parent_main_size = match self.axis {
            FacetAxis::Column => new_plot_area_width,
            FacetAxis::Row => new_plot_area_height,
        }
        .max(1.0);
        self.original_band_scale = self
            .original_band_scale
            .clone()
            .with_range_interval((0.0, parent_main_size));
        self.apply_scale_adjustments(scales);

        let band_scale = scales.get(self.axis.scale_name()).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "FacetBand retarget missing {} scale",
                self.axis.scale_name()
            ))
        })?;
        let new_subplot_cross_size = bandwidth(&band_scale.configured().config).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to get retargeted facet bandwidth: {}",
                e
            ))
        })?;
        self.subplot_cross_size = new_subplot_cross_size;

        let (legend_start, legend_end) =
            legend_axis_overflow(self.axis, &self.coordinated_overflow);
        let (target_plot_area_width, target_plot_area_height) = match self.axis {
            FacetAxis::Column => (
                new_subplot_cross_size,
                adjusted_size_for_legend_overflow(
                    new_plot_area_height.max(1.0),
                    legend_start,
                    legend_end,
                ),
            ),
            FacetAxis::Row => (
                adjusted_size_for_legend_overflow(
                    new_plot_area_width.max(1.0),
                    legend_start,
                    legend_end,
                ),
                new_subplot_cross_size,
            ),
        };
        let compiled_subplot = self.compiled_subplot.clone();
        for cell in &mut self.cells {
            retarget_measurement_plot_area_no_remeasure(
                &mut cell.measurement,
                compiled_subplot.as_ref(),
                eval_ctx,
                &cell.plan.full_path,
                target_plot_area_width,
                target_plot_area_height,
            )?;
        }
        self.apply_cross_axis_coordinated_side_slabs_to_cells();

        Ok(())
    }
}

fn apply_facet_band_scale_adjustment(
    axis: FacetAxis,
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    original_band_scale: &ConfiguredScale,
    active_layout: &CoordinatedLayout,
    cell_values: &[ScalarValue],
    has_hole_cells: bool,
    has_adjacent_non_empty: bool,
) {
    let Some(band_scale) = scales.get_mut(axis.scale_name()) else {
        return;
    };

    let needs_zero_padding_override =
        active_layout.padding_inner_px <= 0.01 || (has_hole_cells && !has_adjacent_non_empty);
    let domain_override = if cell_values.is_empty() {
        None
    } else {
        Some(cell_values)
    };
    let band_n_override = if active_layout.n > cell_values.len() {
        Some(active_layout.n)
    } else {
        None
    };

    let updated_config = apply_facet_band_scale_layout(
        axis,
        original_band_scale,
        active_layout,
        domain_override,
        band_n_override,
        ScaleLayoutRewriteMode::RenderPass {
            allow_zero_padding_override: needs_zero_padding_override,
            side_specific_outer_edges: true,
        },
    );

    band_scale.set_configured(updated_config);
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
        let has_hole_cells = self.cells.iter().any(|cell| {
            !cell.plan.has_data_rows
                && !renderable_for_empty_policy(self.empty_cell_policy, !cell.plan.has_data_rows)
        });
        let has_adjacent_non_empty = self.cells.windows(2).any(|pair| {
            renderable_for_empty_policy(self.empty_cell_policy, !pair[0].plan.has_data_rows)
                && renderable_for_empty_policy(self.empty_cell_policy, !pair[1].plan.has_data_rows)
        });
        let cell_values = self.cell_values().cloned().collect::<Vec<_>>();
        apply_facet_band_scale_adjustment(
            self.axis,
            scales,
            &self.original_band_scale,
            self.active_layout(),
            &cell_values,
            has_hole_cells,
            has_adjacent_non_empty,
        );
    }
}

impl CoordMeasurement for FacetBandCoordMeasurementFixed {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        Some(&self.base.coordinated_overflow)
    }

    fn apply_scale_adjustments(&self, _scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        // Fixed-subplot mode keeps leaf plot area sizing locked, so render-time
        // facet band scale rewrites are intentionally disabled.
    }
}

impl CoordMeasurement for FacetBandProbeMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn apply_scale_adjustments(&self, scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        if self.fixed_plot_area_lock {
            return;
        }

        let has_hole_cells = self.cell_has_data_rows.iter().any(|has_data_rows| {
            !*has_data_rows && !renderable_for_empty_policy(self.empty_cell_policy, true)
        });
        let has_adjacent_non_empty = self.cell_has_data_rows.windows(2).any(|pair| {
            renderable_for_empty_policy(self.empty_cell_policy, !pair[0])
                && renderable_for_empty_policy(self.empty_cell_policy, !pair[1])
        });

        apply_facet_band_scale_adjustment(
            self.axis,
            scales,
            &self.original_band_scale,
            &self.local_layout,
            &self.cell_values,
            has_hole_cells,
            has_adjacent_non_empty,
        );
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

fn is_leaf_subplot(compiled_subplot: &CompiledPlot) -> bool {
    !compiled_subplot
        .marks
        .iter()
        .any(|mark| facet_mark_ref(mark.as_ref()).is_some())
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

    pub(crate) async fn apply_coordinated_overflow_with_plan(
        &mut self,
        eval_ctx: &EvaluationContext,
        plan: &FacetBandCoordApplyPlan,
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
            self.apply_cross_axis_coordinated_side_slabs_to_cells();
            return Ok(FacetBandCoordApplyOutcome {
                subplot_cross_size_before,
                subplot_cross_size_after: self.subplot_cross_size,
                remeasure_triggered: false,
                remeasured_cell_count: 0,
                remeasure_skipped_cell_count: 0,
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

        let (target_plot_area_width, target_plot_area_height) = match self.axis {
            FacetAxis::Column => (self.subplot_cross_size, plan.adjusted_main_size.max(1.0)),
            FacetAxis::Row => (plan.adjusted_main_size.max(1.0), self.subplot_cross_size),
        };
        let compiled_subplot = self.compiled_subplot.clone();
        let shared_scale_builder = self.shared_scale_builder.clone();
        for cell in &mut self.cells {
            retarget_measurement_plot_area_and_domains_no_remeasure(
                &mut cell.measurement,
                compiled_subplot.as_ref(),
                eval_ctx,
                &cell.plan.full_path,
                &shared_scale_builder,
                &cell.coordinated_domain_extents,
                target_plot_area_width,
                target_plot_area_height,
            )
            .await?;
        }
        self.apply_cross_axis_coordinated_side_slabs_to_cells();
        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            cell_count = self.cells.len(),
            target_plot_area_width,
            target_plot_area_height,
            "FacetBand apply_coordinated_overflow retargeted cells without remeasure"
        );

        Ok(FacetBandCoordApplyOutcome {
            subplot_cross_size_before,
            subplot_cross_size_after: self.subplot_cross_size,
            remeasure_triggered: false,
            remeasured_cell_count: 0,
            remeasure_skipped_cell_count: 0,
            remeasured_non_empty_cell_count: 0,
            remeasured_with_coordinated_extents_count: 0,
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
    last_cell_scale_builder: Option<ScaleBuilder>,
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

struct FacetPreparedRuntimeInputs {
    plan: FacetBandMeasurePlan,
    subplot_eval_ctx: EvaluationContext,
    nested_measure_ctx: FacetBandNestedMeasureContext,
}

struct FacetBandOverflowRuntime {
    cells: Vec<FacetCellDraft>,
}

struct FacetLocalLayoutOutcome {
    band_layout_plan: FacetBandPlan,
    final_subplot_main_or_cross_size: f32,
    cells: Vec<FacetCellDraft>,
}

#[derive(Debug, Default, Clone, Copy)]
struct FacetPipelinePerfCounters {
    phase5_leaf_measure_count: usize,
    phase5_non_leaf_probe_aggregate_count: usize,
    phase5_non_leaf_full_measure_count: usize,
    phase6_full_measure_count: usize,
}

pub(crate) fn retarget_scale_ranges_for_plot_area(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) -> usize {
    let mut retarget_count = 0usize;

    for (scale_name, scale_with_spec) in scales.iter_mut() {
        let binding = scale_with_spec.range_binding();
        let Some(((range_start, range_end), (new_range_start, new_range_end))) =
            scale_with_spec.retarget_plot_area_range(new_plot_area_width, new_plot_area_height)
        else {
            continue;
        };

        retarget_count += 1;

        trace!(
            scale = %scale_name,
            ?binding,
            old_range_start = range_start,
            old_range_end = range_end,
            new_range_start,
            new_range_end,
            new_plot_area_width,
            new_plot_area_height,
            "retargeted coordinate-owned scale range after plot resize"
        );
    }

    retarget_count
}

#[derive(Clone, Copy)]
enum LegendMainAxis {
    Horizontal,
    Vertical,
}

fn legend_main_axis_span(bounds: &LayoutBounds, axis: LegendMainAxis) -> f32 {
    match axis {
        LegendMainAxis::Horizontal => bounds.width,
        LegendMainAxis::Vertical => bounds.height,
    }
}

fn set_legend_main_axis_bounds(
    bounds: &mut LayoutBounds,
    axis: LegendMainAxis,
    start: f32,
    span: f32,
) {
    match axis {
        LegendMainAxis::Horizontal => {
            bounds.x = start;
            bounds.width = span;
        }
        LegendMainAxis::Vertical => {
            bounds.y = start;
            bounds.height = span;
        }
    }
}

fn retarget_flexible_legend_group_main_axis(
    legends: &mut indexmap::IndexMap<String, LayoutBounds>,
    legend_measurements: &LegendMeasurements,
    legend_keys: &[String],
    axis: LegendMainAxis,
    origin: f32,
    new_span: f32,
) {
    let mut flexible_count = 0usize;
    let mut old_flexible_total = 0.0f32;
    let mut fixed_total = 0.0f32;

    for key in legend_keys {
        let Some(bounds) = legends.get(key) else {
            continue;
        };
        let span = legend_main_axis_span(bounds, axis).max(0.0);
        let is_flexible = legend_measurements
            .get(key)
            .map(|measurement| measurement.flexible)
            .unwrap_or(false);
        if is_flexible {
            flexible_count += 1;
            old_flexible_total += span;
        } else {
            fixed_total += span;
        }
    }

    if flexible_count == 0 {
        return;
    }

    let flexible_available = (new_span - fixed_total).max(0.0);
    let mut cursor = origin;
    for key in legend_keys {
        let Some(bounds) = legends.get_mut(key) else {
            continue;
        };
        let old_span = legend_main_axis_span(bounds, axis).max(0.0);
        let is_flexible = legend_measurements
            .get(key)
            .map(|measurement| measurement.flexible)
            .unwrap_or(false);
        let span = if is_flexible {
            if old_flexible_total > 0.01 {
                flexible_available * old_span / old_flexible_total
            } else {
                flexible_available / flexible_count as f32
            }
        } else {
            old_span
        };
        set_legend_main_axis_bounds(bounds, axis, cursor, span);
        cursor += span;
    }
}

fn overflow_side_value(overflow: &OverflowSpaceRequirement, side: AxisPosition) -> f32 {
    match side {
        AxisPosition::Top => overflow.top,
        AxisPosition::Right => overflow.right,
        AxisPosition::Bottom => overflow.bottom,
        AxisPosition::Left => overflow.left,
    }
}

fn set_overflow_side_value(
    overflow: &mut OverflowSpaceRequirement,
    side: AxisPosition,
    value: f32,
) {
    let value = value.max(0.0);
    match side {
        AxisPosition::Top => overflow.top = value,
        AxisPosition::Right => overflow.right = value,
        AxisPosition::Bottom => overflow.bottom = value,
        AxisPosition::Left => overflow.left = value,
    }
}

fn legend_position_for_axis_side(side: AxisPosition) -> LegendPosition {
    match side {
        AxisPosition::Top => LegendPosition::Top,
        AxisPosition::Right => LegendPosition::Right,
        AxisPosition::Bottom => LegendPosition::Bottom,
        AxisPosition::Left => LegendPosition::Left,
    }
}

fn legend_cross_axis_extent(layout: &crate::layout::LayoutResult, side: AxisPosition) -> f32 {
    let position = legend_position_for_axis_side(side);
    let Some(keys) = layout.legends_by_position.get(&position) else {
        return 0.0;
    };

    keys.iter()
        .filter_map(|key| layout.legends.get(key))
        .map(|bounds| match side {
            AxisPosition::Left | AxisPosition::Right => bounds.width,
            AxisPosition::Top | AxisPosition::Bottom => bounds.height,
        })
        .fold(0.0, f32::max)
}

fn translate_layout_result(layout: &mut crate::layout::LayoutResult, dx: f32, dy: f32) {
    if dx.abs() <= 0.01 && dy.abs() <= 0.01 {
        return;
    }

    layout.plot_area.x += dx;
    layout.plot_area.y += dy;

    for bounds in layout.guide_overflows.values_mut() {
        bounds.x += dx;
        bounds.y += dy;
    }
    for bounds in layout.legends.values_mut() {
        bounds.x += dx;
        bounds.y += dy;
    }
    if let Some(bounds) = &mut layout.title {
        bounds.x += dx;
        bounds.y += dy;
    }
    if let Some(bounds) = &mut layout.subtitle {
        bounds.x += dx;
        bounds.y += dy;
    }
}

fn guide_overflow_bounds(plot_area: LayoutBounds, side: AxisPosition, guide: f32) -> LayoutBounds {
    let guide = guide.max(0.0);
    match side {
        AxisPosition::Top => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y - guide,
            width: plot_area.width,
            height: guide,
        },
        AxisPosition::Right => LayoutBounds {
            x: plot_area.x + plot_area.width,
            y: plot_area.y,
            width: guide,
            height: plot_area.height,
        },
        AxisPosition::Bottom => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y + plot_area.height,
            width: plot_area.width,
            height: guide,
        },
        AxisPosition::Left => LayoutBounds {
            x: plot_area.x - guide,
            y: plot_area.y,
            width: guide,
            height: plot_area.height,
        },
    }
}

fn set_guide_overflow_geometry(
    layout: &mut crate::layout::LayoutResult,
    side: AxisPosition,
    guide: f32,
) {
    if guide <= 0.01 {
        layout.guide_overflows.remove(&side);
        return;
    }

    layout
        .guide_overflows
        .insert(side, guide_overflow_bounds(layout.plot_area, side, guide));
}

fn anchor_legends_for_side(
    layout: &mut crate::layout::LayoutResult,
    side: AxisPosition,
    guide: f32,
) {
    let position = legend_position_for_axis_side(side);
    let Some(keys) = layout.legends_by_position.get(&position).cloned() else {
        return;
    };

    for key in keys {
        let Some(bounds) = layout.legends.get_mut(&key) else {
            continue;
        };
        match side {
            AxisPosition::Top => {
                bounds.y = layout.plot_area.y - guide.max(0.0) - bounds.height;
            }
            AxisPosition::Right => {
                bounds.x = layout.plot_area.x + layout.plot_area.width + guide.max(0.0);
            }
            AxisPosition::Bottom => {
                bounds.y = layout.plot_area.y + layout.plot_area.height + guide.max(0.0);
            }
            AxisPosition::Left => {
                bounds.x = layout.plot_area.x - guide.max(0.0) - bounds.width;
            }
        }
    }
}

fn apply_layout_side_slab(
    layout: &mut crate::render::LayoutSolution,
    side: AxisPosition,
    guide: f32,
    total: f32,
) {
    let guide = guide
        .max(overflow_side_value(&layout.overflow, side))
        .max(0.0);
    let total = total
        .max(guide)
        .max(guide + legend_cross_axis_extent(&layout.taffy_layout, side));
    let old_total = overflow_side_value(&layout.total_overflow, side)
        .max(overflow_side_value(&layout.overflow, side));
    let delta = total - old_total;

    match side {
        AxisPosition::Left => {
            translate_layout_result(&mut layout.taffy_layout, delta, 0.0);
            layout.canvas_size.0 = (layout.canvas_size.0 + delta).max(1.0);
        }
        AxisPosition::Top => {
            translate_layout_result(&mut layout.taffy_layout, 0.0, delta);
            layout.canvas_size.1 = (layout.canvas_size.1 + delta).max(1.0);
        }
        AxisPosition::Right => {
            layout.canvas_size.0 = (layout.canvas_size.0 + delta).max(1.0);
        }
        AxisPosition::Bottom => {
            layout.canvas_size.1 = (layout.canvas_size.1 + delta).max(1.0);
        }
    }

    set_overflow_side_value(&mut layout.overflow, side, guide);
    set_overflow_side_value(&mut layout.total_overflow, side, total);
    set_guide_overflow_geometry(&mut layout.taffy_layout, side, guide);
    anchor_legends_for_side(&mut layout.taffy_layout, side, guide);
}

fn apply_measurement_side_slab(
    measurement: &mut ComponentsMeasurement,
    side: AxisPosition,
    guide: f32,
    total: f32,
) {
    apply_layout_side_slab(&mut measurement.layout, side, guide, total);
    measurement.canvas_size = measurement.layout.canvas_size;
}

fn retarget_layout_for_resized_plot_area(
    layout: &mut crate::render::LayoutSolution,
    legend_measurements: &LegendMeasurements,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
    _width_delta: f32,
    _height_delta: f32,
) {
    layout.taffy_layout.plot_area.width = new_plot_area_width;
    layout.taffy_layout.plot_area.height = new_plot_area_height;

    for (position, legend_keys) in layout.taffy_layout.legends_by_position.clone() {
        match position {
            LegendPosition::Left | LegendPosition::Right => {
                retarget_flexible_legend_group_main_axis(
                    &mut layout.taffy_layout.legends,
                    legend_measurements,
                    &legend_keys,
                    LegendMainAxis::Vertical,
                    layout.taffy_layout.plot_area.y,
                    new_plot_area_height,
                );
            }
            LegendPosition::Top | LegendPosition::Bottom => {
                retarget_flexible_legend_group_main_axis(
                    &mut layout.taffy_layout.legends,
                    legend_measurements,
                    &legend_keys,
                    LegendMainAxis::Horizontal,
                    layout.taffy_layout.plot_area.x,
                    new_plot_area_width,
                );
            }
        }
    }

    for side in [
        AxisPosition::Top,
        AxisPosition::Right,
        AxisPosition::Bottom,
        AxisPosition::Left,
    ] {
        let guide = overflow_side_value(&layout.overflow, side);
        set_guide_overflow_geometry(&mut layout.taffy_layout, side, guide);
        anchor_legends_for_side(&mut layout.taffy_layout, side, guide);
    }
}

fn update_measurement_plot_area_metadata(
    measurement: &mut ComponentsMeasurement,
    compiled_plot: &CompiledPlot,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    old_plot_area_width: f32,
    old_plot_area_height: f32,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) {
    let width_delta = new_plot_area_width - old_plot_area_width;
    let height_delta = new_plot_area_height - old_plot_area_height;
    measurement.plot_area_width = new_plot_area_width;
    measurement.plot_area_height = new_plot_area_height;
    measurement.canvas_size.0 = (measurement.canvas_size.0 + width_delta).max(1.0);
    measurement.canvas_size.1 = (measurement.canvas_size.1 + height_delta).max(1.0);
    measurement.layout.canvas_size = measurement.canvas_size;
    retarget_layout_for_resized_plot_area(
        &mut measurement.layout,
        &measurement.legend_plan.measurements,
        new_plot_area_width,
        new_plot_area_height,
        width_delta,
        height_delta,
    );
    measurement.params.insert(
        "width".to_string(),
        ScalarValue::Float32(Some(new_plot_area_width)),
    );
    measurement.params.insert(
        "height".to_string(),
        ScalarValue::Float32(Some(new_plot_area_height)),
    );
    measurement.clip = compiled_plot.resolved_clip_region(
        eval_ctx,
        facet_path,
        &measurement.scales,
        new_plot_area_width,
        new_plot_area_height,
    );
}

pub(crate) fn retarget_measurement_plot_area_no_remeasure(
    measurement: &mut ComponentsMeasurement,
    compiled_plot: &CompiledPlot,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) -> Result<(), AvengerChartError> {
    let old_plot_area_width = measurement.plot_area_width;
    let old_plot_area_height = measurement.plot_area_height;
    if (old_plot_area_width - new_plot_area_width).abs() <= 0.01
        && (old_plot_area_height - new_plot_area_height).abs() <= 0.01
    {
        return Ok(());
    }

    retarget_scale_ranges_for_plot_area(
        &mut measurement.scales,
        new_plot_area_width,
        new_plot_area_height,
    );

    if let Some(facet_band) = facet_band_canvas_mut(measurement.coord_measurement.as_mut()) {
        facet_band.retarget_parent_plot_area_no_remeasure(
            &mut measurement.scales,
            eval_ctx,
            new_plot_area_width,
            new_plot_area_height,
        )?;
    } else {
        measurement
            .coord_measurement
            .apply_scale_adjustments(&mut measurement.scales);
    }

    update_measurement_plot_area_metadata(
        measurement,
        compiled_plot,
        eval_ctx,
        facet_path,
        old_plot_area_width,
        old_plot_area_height,
        new_plot_area_width,
        new_plot_area_height,
    );
    measurement.legend_plan.retarget_scales(&measurement.scales);
    Ok(())
}

async fn retarget_measurement_plot_area_and_domains_no_remeasure(
    measurement: &mut ComponentsMeasurement,
    compiled_plot: &CompiledPlot,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    scale_builder: &ScaleBuilder,
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) -> Result<(), AvengerChartError> {
    if coordinated_domain_extents.is_empty() {
        retarget_measurement_plot_area_no_remeasure(
            measurement,
            compiled_plot,
            eval_ctx,
            facet_path,
            new_plot_area_width,
            new_plot_area_height,
        )?;
        return Ok(());
    }

    let old_plot_area_width = measurement.plot_area_width;
    let old_plot_area_height = measurement.plot_area_height;
    let mut builder = scale_builder.clone();
    builder.extend_with_domain_extents(coordinated_domain_extents);

    let mut params = measurement.params.clone();
    params.insert(
        "width".to_string(),
        ScalarValue::Float32(Some(new_plot_area_width)),
    );
    params.insert(
        "height".to_string(),
        ScalarValue::Float32(Some(new_plot_area_height)),
    );

    let mut rebuilt_scales = compiled_plot
        .build_scales_from_builder(
            &builder,
            new_plot_area_width,
            new_plot_area_height,
            eval_ctx.session_context.as_ref(),
            &params,
        )
        .await?;
    if let Some(facet_band) = facet_band_canvas_mut(measurement.coord_measurement.as_mut()) {
        facet_band.retarget_parent_plot_area_no_remeasure(
            &mut rebuilt_scales,
            eval_ctx,
            new_plot_area_width,
            new_plot_area_height,
        )?;
    } else {
        measurement
            .coord_measurement
            .apply_scale_adjustments(&mut rebuilt_scales);
    }
    measurement.scales = rebuilt_scales;

    update_measurement_plot_area_metadata(
        measurement,
        compiled_plot,
        eval_ctx,
        facet_path,
        old_plot_area_width,
        old_plot_area_height,
        new_plot_area_width,
        new_plot_area_height,
    );
    measurement.legend_plan.retarget_scales(&measurement.scales);

    trace!(
        facet_path = ?facet_path,
        coordinated_domain_count = coordinated_domain_extents.len(),
        old_plot_area_width,
        old_plot_area_height,
        new_plot_area_width,
        new_plot_area_height,
        "FacetBand retargeted measurement scales with coordinated domains without remeasure"
    );

    Ok(())
}

fn derive_padding_inner_px_from_probe(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
) -> f32 {
    let required_parent_padding = compute_padding_from_overflows(
        axis,
        &pass1
            .cell_overflows
            .iter()
            .map(|(_, total)| total.clone())
            .collect::<Vec<_>>(),
        pass1_renderable_cells,
    );
    let renderable_count = pass1_renderable_cells
        .iter()
        .filter(|renderable| **renderable)
        .count();
    let parent_padding = if renderable_count > 1 {
        required_parent_padding.max(padding_policy::MIN_SUBPLOT_MAIN_GAP)
    } else {
        required_parent_padding
    };

    padding_policy::derive_parent_padding(parent_padding, pass1.max_child_padding)
}

#[cfg(test)]
pub(crate) fn derive_band_overflow_from_probe(
    axis: FacetAxis,
    renderable_mask: &[bool],
    cell_overflows: &[(OverflowSpaceRequirement, OverflowSpaceRequirement)],
) -> CoordinatedOverflow {
    if cell_overflows.is_empty() {
        return CoordinatedOverflow::default();
    }

    let mut renderable_indices = if renderable_mask.len() == cell_overflows.len() {
        renderable_mask
            .iter()
            .enumerate()
            .filter_map(|(idx, renderable)| renderable.then_some(idx))
            .collect::<Vec<_>>()
    } else {
        (0..cell_overflows.len()).collect::<Vec<_>>()
    };
    if renderable_indices.is_empty() {
        renderable_indices.push(0);
    }

    let first_idx = *renderable_indices
        .first()
        .expect("renderable indices must not be empty");
    let last_idx = *renderable_indices
        .last()
        .expect("renderable indices must not be empty");

    let first_guide = &cell_overflows[first_idx].0;
    let last_guide = &cell_overflows[last_idx].0;
    let first_total = &cell_overflows[first_idx].1;
    let last_total = &cell_overflows[last_idx].1;

    let max_guide_top = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].0.top)
        .fold(0.0, f32::max);
    let max_guide_bottom = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].0.bottom)
        .fold(0.0, f32::max);
    let max_guide_left = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].0.left)
        .fold(0.0, f32::max);
    let max_guide_right = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].0.right)
        .fold(0.0, f32::max);

    let max_total_top = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].1.top)
        .fold(0.0, f32::max);
    let max_total_bottom = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].1.bottom)
        .fold(0.0, f32::max);
    let max_total_left = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].1.left)
        .fold(0.0, f32::max);
    let max_total_right = renderable_indices
        .iter()
        .map(|idx| cell_overflows[*idx].1.right)
        .fold(0.0, f32::max);

    let guide = match axis {
        FacetAxis::Column => OverflowSpaceRequirement {
            top: max_guide_top,
            bottom: max_guide_bottom,
            left: first_guide.left,
            right: last_guide.right,
        },
        FacetAxis::Row => OverflowSpaceRequirement {
            top: first_guide.top,
            bottom: last_guide.bottom,
            left: max_guide_left,
            right: max_guide_right,
        },
    };

    let total = match axis {
        FacetAxis::Column => OverflowSpaceRequirement {
            top: max_total_top,
            bottom: max_total_bottom,
            left: first_total.left,
            right: last_total.right,
        },
        FacetAxis::Row => OverflowSpaceRequirement {
            top: first_total.top,
            bottom: last_total.bottom,
            left: max_total_left,
            right: max_total_right,
        },
    };

    CoordinatedOverflow { guide, total }
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

/// Returns the main-axis extent of a cell for fixed-subplot positioning.
///
/// For leaf cells, this is simply `plot_area_width` (or height for rows).
/// For intermediate cells (containing a facet band), the actual rendered extent
/// includes inter-cell gaps computed by `compute_fixed_main_axis_positions`, which
/// may exceed `plot_area_width`. When fixed main-axis positions are available,
/// we use them to compute the true extent: last_position + last_cell_size.
fn cell_main_plot_size(axis: FacetAxis, measurement: &ComponentsMeasurement) -> f32 {
    // Check if this cell contains a fixed facet band with computed positions
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementFixed>()
    {
        let positions = &facet_band.fixed_main_axis_positions;
        if let Some(&last_pos) = positions.last() {
            if let Some(last_cell) = facet_band.base.cells.last() {
                let last_cell_size = match axis {
                    FacetAxis::Column => last_cell.measurement.plot_area_width,
                    FacetAxis::Row => last_cell.measurement.plot_area_height,
                };
                return (last_pos + last_cell_size).max(0.0);
            }
        }
    }

    // Fallback for leaf cells or cells without fixed positions
    match axis {
        FacetAxis::Column => measurement.plot_area_width,
        FacetAxis::Row => measurement.plot_area_height,
    }
    .max(0.0)
}

fn cell_main_axis_overflow_pair(
    axis: FacetAxis,
    measurement: &ComponentsMeasurement,
) -> (f32, f32) {
    // Fixed-subplot placement uses these per-cell edge paddings to keep adjacent
    // subplot groups separated. Add a small legend-owned breathing room so legends
    // don't visually crowd the next subplot when they sit near branch boundaries.
    const LEGEND_EDGE_BREATHING_ROOM: f32 = 8.0;

    let total = &measurement.layout.total_overflow;
    let guide = &measurement.layout.overflow;
    let (
        legend_left_from_bounds,
        legend_right_from_bounds,
        legend_top_from_bounds,
        legend_bottom_from_bounds,
    ) = legend_bounds_overflow_edges(measurement);
    let (coord_legend_left, coord_legend_right, coord_legend_top, coord_legend_bottom) =
        coordinated_legend_overflow_edges(measurement);

    // Prefer whichever legend estimate is larger:
    // - total-guide delta captures coordinated legend slabs in layout overflow
    // - legend bounds capture per-cell legend boxes directly from taffy placement
    let legend_left = (total.left - guide.left)
        .max(0.0)
        .max(coord_legend_left)
        .max(legend_left_from_bounds);
    let legend_right = (total.right - guide.right)
        .max(0.0)
        .max(coord_legend_right)
        .max(legend_right_from_bounds);
    let legend_top = (total.top - guide.top)
        .max(0.0)
        .max(coord_legend_top)
        .max(legend_top_from_bounds);
    let legend_bottom = (total.bottom - guide.bottom)
        .max(0.0)
        .max(coord_legend_bottom)
        .max(legend_bottom_from_bounds);

    // Also consider the full coordinated overflow from the inner facet band.
    // The taffy `total` may be stale or may not include overflow from nested facet
    // levels (e.g., nested legends or row labels updated during coordination).
    let (coord_total_left, coord_total_right, coord_total_top, coord_total_bottom) =
        coordinated_total_overflow_edges(measurement);

    match axis {
        FacetAxis::Column => {
            let mut left = total.left.max(coord_total_left).max(0.0);
            let mut right = total.right.max(coord_total_right).max(0.0);
            if legend_left > 0.0 {
                left += LEGEND_EDGE_BREATHING_ROOM;
            }
            if legend_right > 0.0 {
                right += LEGEND_EDGE_BREATHING_ROOM;
            }
            (left, right)
        }
        FacetAxis::Row => {
            let mut top = total.top.max(coord_total_top).max(0.0);
            let mut bottom = total.bottom.max(coord_total_bottom).max(0.0);
            if legend_top > 0.0 {
                top += LEGEND_EDGE_BREATHING_ROOM;
            }
            if legend_bottom > 0.0 {
                bottom += LEGEND_EDGE_BREATHING_ROOM;
            }
            (top, bottom)
        }
    }
}

fn legend_bounds_overflow_edges(measurement: &ComponentsMeasurement) -> (f32, f32, f32, f32) {
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    let mut top = 0.0f32;
    let mut bottom = 0.0f32;
    let plot_width = measurement.plot_area_width;
    let plot_height = measurement.plot_area_height;

    for bounds in measurement.layout.taffy_layout.legends.values() {
        left = left.max((-bounds.x).max(0.0));
        right = right.max((bounds.x + bounds.width - plot_width).max(0.0));
        top = top.max((-bounds.y).max(0.0));
        bottom = bottom.max((bounds.y + bounds.height - plot_height).max(0.0));
    }

    (left, right, top, bottom)
}

fn coordinated_legend_overflow_edges(measurement: &ComponentsMeasurement) -> (f32, f32, f32, f32) {
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementFixed>()
    {
        let total = &facet_band.coordinated_overflow.total;
        let guide = &facet_band.coordinated_overflow.guide;
        return (
            (total.left - guide.left).max(0.0),
            (total.right - guide.right).max(0.0),
            (total.top - guide.top).max(0.0),
            (total.bottom - guide.bottom).max(0.0),
        );
    }

    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementCanvasFit>()
    {
        let total = &facet_band.coordinated_overflow.total;
        let guide = &facet_band.coordinated_overflow.guide;
        return (
            (total.left - guide.left).max(0.0),
            (total.right - guide.right).max(0.0),
            (total.top - guide.top).max(0.0),
            (total.bottom - guide.bottom).max(0.0),
        );
    }

    (0.0, 0.0, 0.0, 0.0)
}

/// Returns the full coordinated overflow (guide + legend) from the inner facet band,
/// recursing through all nested levels to capture overflow from deeply nested legends
/// or guides that may not be visible at the immediate inner level.
///
/// For example, in a 3-level col>col>col layout with a legend at the innermost level,
/// the middle level's `coordinated_overflow.total` won't include the innermost legend.
/// This function walks down through nested cells to find the maximum overflow at any depth.
fn coordinated_total_overflow_edges(measurement: &ComponentsMeasurement) -> (f32, f32, f32, f32) {
    // Iterative traversal to avoid stack overflow with deep nesting + async test stacks
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    let mut top = 0.0f32;
    let mut bottom = 0.0f32;

    // Work queue of measurements to inspect
    let mut queue: Vec<&ComponentsMeasurement> = vec![measurement];

    while let Some(m) = queue.pop() {
        let band: Option<&FacetBandCoordMeasurement> = m
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurementFixed>()
            .map(|fb| &fb.base)
            .or_else(|| {
                m.coord_measurement
                    .as_any()
                    .downcast_ref::<FacetBandCoordMeasurementCanvasFit>()
                    .map(|fb| fb as &FacetBandCoordMeasurement)
            });

        if let Some(band) = band {
            let total = &band.coordinated_overflow.total;
            left = left.max(total.left.max(0.0));
            right = right.max(total.right.max(0.0));
            top = top.max(total.top.max(0.0));
            bottom = bottom.max(total.bottom.max(0.0));

            // Enqueue child cells for inspection
            for cell in &band.cells {
                queue.push(&cell.measurement);
            }
        }
    }

    (left, right, top, bottom)
}

pub(crate) fn compute_fixed_main_axis_positions(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    layout: &CoordinatedLayout,
) -> Vec<f32> {
    if cells.is_empty() {
        return Vec::new();
    }

    let mut positions = Vec::with_capacity(cells.len());
    let mut cursor = layout.outer_start.max(0.0);
    let gap = layout
        .padding_inner_px
        .max(padding_policy::MIN_SUBPLOT_MAIN_GAP);

    for (idx, cell) in cells.iter().enumerate() {
        positions.push(cursor);
        cursor += cell_main_plot_size(axis, &cell.measurement);
        if idx + 1 < cells.len() {
            let (_, after_current) = cell_main_axis_overflow_pair(axis, &cell.measurement);
            let (before_next, _) = cell_main_axis_overflow_pair(axis, &cells[idx + 1].measurement);
            cursor += gap.max(after_current + before_next);
        }
    }

    positions
}

fn synthesize_subtree_plot_area_from_leaf_size(
    node: &crate::facet::evaluated_facet_tree::PartitionNode,
    leaf_plot_width: f32,
    leaf_plot_height: f32,
) -> (f32, f32) {
    match &node.content {
        crate::facet::evaluated_facet_tree::PartitionContent::Leaf { values } => {
            let count = values.len().max(1) as f32;
            match node.direction {
                crate::guide::FacetDirection::Column => (leaf_plot_width * count, leaf_plot_height),
                crate::guide::FacetDirection::Row => (leaf_plot_width, leaf_plot_height * count),
            }
        }
        crate::facet::evaluated_facet_tree::PartitionContent::Branch { children } => {
            let mut child_sizes = children.values().map(|child| {
                synthesize_subtree_plot_area_from_leaf_size(
                    child.as_ref(),
                    leaf_plot_width,
                    leaf_plot_height,
                )
            });

            let Some((first_w, first_h)) = child_sizes.next() else {
                return (leaf_plot_width, leaf_plot_height);
            };

            match node.direction {
                crate::guide::FacetDirection::Column => {
                    let mut total_w = first_w;
                    let mut max_h = first_h;
                    for (w, h) in child_sizes {
                        total_w += w;
                        max_h = max_h.max(h);
                    }
                    (total_w, max_h)
                }
                crate::guide::FacetDirection::Row => {
                    let mut max_w = first_w;
                    let mut total_h = first_h;
                    for (w, h) in child_sizes {
                        max_w = max_w.max(w);
                        total_h += h;
                    }
                    (max_w, total_h)
                }
            }
        }
    }
}

fn empty_facet_band_measurement(
    axis: FacetAxis,
    facet_path: &[ScalarValue],
    compiled_subplot: &Arc<CompiledPlot>,
    band_scale: &ConfiguredScaleWithSpec,
    empty_cell_policy: FacetEmptyCellPolicy,
    fixed_mode: bool,
) -> Box<dyn CoordMeasurement> {
    let base = FacetBandCoordMeasurement {
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
    };
    if fixed_mode {
        Box::new(FacetBandCoordMeasurementFixed {
            base,
            fixed_main_axis_positions: Vec::new(),
        })
    } else {
        Box::new(base)
    }
}

/// Build a single facet cell context from value and parent path.
#[cfg(test)]
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
#[cfg(test)]
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
        NestedScalePlan::EmptySharedNoData => Box::pin(compiled_subplot.measure_plot_components(
            subplot_eval_ctx,
            subplot_layout_spec,
            &shared_scale_provider,
            None,
            &cell.full_path,
        ))
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
            let measurement = Box::pin(compiled_subplot.measure_plot_components(
                subplot_eval_ctx,
                subplot_layout_spec,
                &cached_scale_provider,
                Some(data_override),
                &cell.full_path,
            ))
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

            let measurement = Box::pin(compiled_subplot.measure_plot_components(
                subplot_eval_ctx,
                subplot_layout_spec,
                &cell_scale_provider,
                Some(data_override),
                &cell.full_path,
            ))
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
            let measurement = Box::pin(compiled_subplot.measure_plot_components(
                subplot_eval_ctx,
                subplot_layout_spec,
                &cached_scale_provider,
                Some(&ancestor_filtered_df),
                &cell.full_path,
            ))
            .await?;

            Ok(MeasuredFacetCell {
                measurement,
                cell_scale_builder: None,
            })
        }
        NestedScalePlan::Shared => {
            let measurement = Box::pin(compiled_subplot.measure_plot_components(
                subplot_eval_ctx,
                subplot_layout_spec,
                &shared_scale_provider,
                Some(data_override),
                &cell.full_path,
            ))
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
    }
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
    Box::pin(measure_facet_cell(
        cell,
        data_override,
        compiled_subplot,
        subplot_eval_ctx,
        &subplot_layout_spec,
        mode,
    ))
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
                last_cell_scale_builder: None,
            })
        })
        .collect::<Result<_, AvengerChartError>>()?;

    Ok(FacetBandMeasurePlan {
        cell_values,
        cells,
        scale_artifacts,
    })
}

async fn prepare_measurement_inputs(
    cell_plans: Vec<FacetCellPlan>,
    facet_path: &[ScalarValue],
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    empty_cell_policy: FacetEmptyCellPolicy,
    eval_ctx: &EvaluationContext,
) -> Result<FacetPreparedRuntimeInputs, AvengerChartError> {
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

    Ok(FacetPreparedRuntimeInputs {
        plan,
        subplot_eval_ctx,
        nested_measure_ctx,
    })
}

fn prepared_cells_as_drafts(
    prepared_synthesis: &FacetBandPreparedSynthesis,
    sidecars: &FacetBandPreparedRuntime,
) -> Vec<FacetCellDraft> {
    assert_eq!(
        prepared_synthesis.cell_synthesis.cells.len(),
        sidecars.data_overrides.len(),
        "FacetBand prepared-synthesis attributes/sidecar invariant violated: mismatched cell/data_override lengths"
    );

    prepared_synthesis
        .cell_synthesis
        .cells
        .iter()
        .zip(sidecars.data_overrides.iter())
        .map(|(cell, data_override)| FacetCellDraft {
            plan: FacetCellPlan::from(cell),
            data_override: data_override.clone(),
            measurement: None,
            local_domain_extents: HashMap::new(),
            last_cell_scale_builder: None,
        })
        .collect()
}

async fn synthesize_prepared_attributes_and_runtime(
    cell_synthesis: FacetBandSemantics,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    band_scale: &ConfiguredScaleWithSpec,
    initial_subplot_band_size: f32,
    eval_ctx: &EvaluationContext,
) -> Result<(FacetBandPreparedSynthesis, FacetBandPreparedRuntime), AvengerChartError> {
    let cell_plans: Vec<FacetCellPlan> = cell_synthesis
        .cells
        .iter()
        .map(FacetCellPlan::from)
        .collect();
    let measurement_inputs = prepare_measurement_inputs(
        cell_plans,
        &cell_synthesis.node_id.facet_path,
        data_df,
        compiled_subplot,
        cell_synthesis.empty_cell_policy,
        eval_ctx,
    )
    .await?;

    let renderable_mask = renderable_mask_for_cells(
        &measurement_inputs.plan.cells,
        cell_synthesis.empty_cell_policy,
    );
    let data_overrides = measurement_inputs
        .plan
        .cells
        .iter()
        .map(|cell| cell.data_override.clone())
        .collect::<Vec<_>>();
    let scale_artifacts = measurement_inputs.plan.scale_artifacts.clone();

    let prepared_synthesis = FacetBandPreparedSynthesis {
        cell_synthesis: cell_synthesis.clone(),
        renderable_mask,
        scale_artifacts_key: FacetScaleNodeKey::new(
            compiled_subplot,
            &cell_synthesis.node_id.facet_path,
        ),
    };
    let sidecars = FacetBandPreparedRuntime {
        data_overrides,
        subplot_eval_ctx: measurement_inputs.subplot_eval_ctx,
        nested_measure_ctx: measurement_inputs.nested_measure_ctx,
        compiled_subplot: compiled_subplot.clone(),
        original_band_scale: band_scale.configured().clone(),
        initial_subplot_band_size,
        scale_artifacts,
    };
    Ok((prepared_synthesis, sidecars))
}

async fn synthesize_overflow_probe_attributes(
    prepared_synthesis: &FacetBandPreparedSynthesis,
    sidecars: &FacetBandPreparedRuntime,
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    perf_counters: &mut FacetPipelinePerfCounters,
) -> Result<(FacetBandOverflowSynthesis, FacetBandOverflowRuntime), AvengerChartError> {
    assert_eq!(
        prepared_synthesis.renderable_mask.len(),
        prepared_synthesis.cell_synthesis.cells.len(),
        "FacetBand prepared-synthesis attributes invariant violated: renderable mask length mismatch"
    );
    let expected_key = FacetScaleNodeKey::new(
        &sidecars.compiled_subplot,
        &prepared_synthesis.cell_synthesis.node_id.facet_path,
    );
    assert_eq!(
        prepared_synthesis.scale_artifacts_key, expected_key,
        "FacetBand prepared-synthesis attributes invariant violated: scale artifacts key mismatch"
    );

    let mut cells = prepared_cells_as_drafts(prepared_synthesis, sidecars);
    let overflow_probe_summary = synthesize_overflow_probe(
        &mut cells,
        subplot_plot_width,
        subplot_plot_height,
        &sidecars.compiled_subplot,
        &sidecars.subplot_eval_ctx,
        &sidecars.nested_measure_ctx,
        prepared_synthesis.cell_synthesis.empty_cell_policy,
        is_leaf_subplot(&sidecars.compiled_subplot),
        perf_counters,
    )
    .await?;

    Ok((
        FacetBandOverflowSynthesis {
            prepared_synthesis: prepared_synthesis.clone(),
            overflow_probe_summary,
        },
        FacetBandOverflowRuntime { cells },
    ))
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

async fn capture_estimated_overflow_probe_if_requested(
    eval_ctx: &EvaluationContext,
    compiled_subplot: &Arc<CompiledPlot>,
    measurement: &ComponentsMeasurement,
    data_override: &DataFrame,
    full_path: &[ScalarValue],
) -> Result<(), AvengerChartError> {
    let Some(request) = eval_ctx.facet_subtree_snapshot_request() else {
        return Ok(());
    };
    if request.checkpoint != FacetSubtreeCheckpoint::EstimatedOverflowProbe {
        return Ok(());
    }

    let matches_selector = match &request.selector {
        FacetSubtreeSelector::ByFacetPath(path) => path.as_slice() == full_path,
        FacetSubtreeSelector::ByCoordNodePath(path) => {
            path.as_slice() == eval_ctx.facet_coord_node_path()
        }
    };
    if !matches_selector {
        return Ok(());
    }

    let components = compiled_subplot
        .build_plot_components(eval_ctx, measurement, Some(data_override), true, full_path)
        .await?;
    let evaluated = compiled_subplot.components_to_evaluated_plot(eval_ctx, components);
    let _ = eval_ctx.capture_facet_subtree_snapshot(&request, evaluated);
    Ok(())
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
    cells: &mut [FacetCellDraft],
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
    empty_cell_policy: FacetEmptyCellPolicy,
    is_leaf_node: bool,
    perf_counters: &mut FacetPipelinePerfCounters,
) -> Result<OverflowProbeSummary, AvengerChartError> {
    let mut summary = OverflowProbeSummary::default();
    summary.cell_overflows.reserve(cells.len());
    summary.cell_probe_summaries.reserve(cells.len());

    for (idx, cell) in cells.iter_mut().enumerate() {
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
        let cell_eval_ctx = cell_eval_ctx.with_facet_coord_node_path_appended(idx);
        let (probe_plot_width, probe_plot_height) = cell_eval_ctx
            .facet_probe_size_override(&cell.plan.full_path)
            .unwrap_or((subplot_plot_width, subplot_plot_height));

        let cell_summary = if is_leaf_node {
            perf_counters.phase5_leaf_measure_count += 1;
            let measured = Box::pin(measure_nested_cell(
                &cell.plan,
                &cell.data_override,
                probe_plot_width,
                probe_plot_height,
                compiled_subplot,
                &cell_eval_ctx,
                nested_ctx,
            ))
            .await?;
            capture_estimated_overflow_probe_if_requested(
                &cell_eval_ctx,
                compiled_subplot,
                &measured.measurement,
                &cell.data_override,
                &cell.plan.full_path,
            )
            .await?;
            let mut max_child_padding = 0.0f32;
            measured
                .measurement
                .coord_measurement
                .as_any()
                .downcast_ref::<FacetBandCoordMeasurement>()
                .inspect(|child_facet_band| {
                    max_child_padding = child_facet_band.active_layout().padding_inner_px;
                });
            let cell_probe_summary = FacetCellProbeSummary {
                guide_overflow: measured.measurement.layout.overflow.clone(),
                total_overflow: measured.measurement.layout.total_overflow.clone(),
                max_child_padding,
            };
            cell.last_cell_scale_builder = measured.cell_scale_builder;
            cell.measurement = Some(measured.measurement);
            cell_probe_summary
        } else {
            perf_counters.phase5_non_leaf_probe_aggregate_count += 1;
            perf_counters.phase5_non_leaf_full_measure_count += 1;
            let measured = Box::pin(measure_nested_cell(
                &cell.plan,
                &cell.data_override,
                probe_plot_width,
                probe_plot_height,
                compiled_subplot,
                &cell_eval_ctx,
                nested_ctx,
            ))
            .await?;
            capture_estimated_overflow_probe_if_requested(
                &cell_eval_ctx,
                compiled_subplot,
                &measured.measurement,
                &cell.data_override,
                &cell.plan.full_path,
            )
            .await?;
            let mut max_child_padding = 0.0f32;
            measured
                .measurement
                .coord_measurement
                .as_any()
                .downcast_ref::<FacetBandCoordMeasurement>()
                .inspect(|child_facet_band| {
                    max_child_padding = child_facet_band.active_layout().padding_inner_px;
                });
            let cell_probe_summary = FacetCellProbeSummary {
                guide_overflow: measured.measurement.layout.overflow.clone(),
                total_overflow: measured.measurement.layout.total_overflow.clone(),
                max_child_padding,
            };
            cell.last_cell_scale_builder = measured.cell_scale_builder;
            cell.measurement = Some(measured.measurement);
            trace!(cell_index = idx, "FacetBand non-leaf phase-5 full measure");
            cell_probe_summary
        };
        let guide_overflow = cell_summary.guide_overflow.clone();
        let total_overflow = cell_summary.total_overflow.clone();
        summary.max_child_padding = summary
            .max_child_padding
            .max(cell_summary.max_child_padding);

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            probe_plot_width,
            probe_plot_height,
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
        summary.cell_probe_summaries.push(cell_summary);

        if is_leaf_node {
            debug_assert!(cell.measurement.is_some());
        }
    }

    Ok(summary)
}

async fn synthesize_overflow_probe(
    cells: &mut [FacetCellDraft],
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    nested_ctx: &FacetBandNestedMeasureContext,
    empty_cell_policy: FacetEmptyCellPolicy,
    is_leaf_node: bool,
    perf_counters: &mut FacetPipelinePerfCounters,
) -> Result<OverflowProbeSummary, AvengerChartError> {
    let overflow_probe_summary = measure_cells_overflow_probe(
        cells,
        subplot_plot_width,
        subplot_plot_height,
        compiled_subplot,
        subplot_eval_ctx,
        nested_ctx,
        empty_cell_policy,
        is_leaf_node,
        perf_counters,
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
    _perf_counters: &mut FacetPipelinePerfCounters,
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
        let measurement = cell.measurement.as_mut().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "FacetBand phase-6 no-remeasure invariant violated: missing phase-5 measurement for cell {:?}",
                cell.plan.value
            ))
        })?;
        retarget_measurement_plot_area_no_remeasure(
            measurement,
            compiled_subplot,
            &cell_eval_ctx,
            &cell.plan.full_path,
            subplot_plot_width,
            subplot_plot_height,
        )?;

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            subplot_plot_width,
            subplot_plot_height,
            is_empty = !cell.plan.has_data_rows,
            "FacetBand phase-6 finalized cell measurement"
        );

        let local_extents = if cell.plan.has_data_rows {
            let extent_builder = if let Some(builder) = cell.last_cell_scale_builder.clone() {
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
pub(crate) struct FacetAxisOps {
    axis: FacetAxis,
    scale_name: &'static str,
    facet_label: &'static str,
    missing_scale_err: &'static str,
    missing_mark_err: &'static str,
}

impl FacetAxisOps {
    pub(crate) fn for_axis(axis: FacetAxis) -> Self {
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

    fn main_size(self, plot_width: f32, plot_height: f32) -> f32 {
        match self.axis {
            FacetAxis::Column => plot_width,
            FacetAxis::Row => plot_height,
        }
    }
}

pub(crate) struct FacetBandMeasurePipeline<'a> {
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
    pub(crate) fn new(
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

    pub(crate) async fn run(&self) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Step 1: Resolve -- resolve this facet node and enumerate cell values for the current path.
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
                matches!(
                    self.eval_ctx.facet_runtime_sizing_mode(),
                    FacetRuntimeSizingMode::FixedSubplot { .. }
                ),
            ));
        }
        let subplot_band_size = self.resolve_subplot_band_size(&resolved, &cell_values);

        let data_df = self.data.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "{} measure requires data",
                self.axis_ops.facet_label
            ))
        })?;

        // Step 2: Precompute -- precompute subtree scale builders used by nested sharing measurement.
        ensure_subtree_precomputed(
            self.compiled_marks,
            self.facet_path,
            data_df,
            &self.eval_ctx.facet_tree,
            self.eval_ctx,
        )
        .await?;

        // Step 3: Synthesize cell attributes -- build geometry-independent per-cell semantics.
        let cell_synthesis = self.synthesize_cell_attributes(&resolved, &cell_values)?;

        // Step 4: Synthesize prepared attributes -- build synthesis attributes + runtime sidecars.
        let (prepared_synthesis, prepared_runtime) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis.clone(),
            data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            subplot_band_size,
            self.eval_ctx,
        )
        .await?;

        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, subplot_band_size);
        let mut perf_counters = FacetPipelinePerfCounters::default();

        // Step 5: Synthesize overflow probe -- non-mutating probe at estimated slot size.
        let (overflow_synthesis, overflow_runtime) = synthesize_overflow_probe_attributes(
            &prepared_synthesis,
            &prepared_runtime,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;

        // Step 6: Synthesize local layout -- finalize band layout and measure cells.
        let (local_synthesis, measured_runtime) = self
            .synthesize_local_attributes(
                &overflow_synthesis,
                overflow_runtime,
                &prepared_runtime,
                &mut perf_counters,
            )
            .await?;

        let coord_measurement =
            self.assemble_coord_measurement(local_synthesis, measured_runtime, &prepared_runtime);

        debug!(
            axis = ?self.axis_ops.axis,
            facet_path = ?self.facet_path,
            phase5_leaf_measure_count = perf_counters.phase5_leaf_measure_count,
            phase5_non_leaf_probe_aggregate_count = perf_counters.phase5_non_leaf_probe_aggregate_count,
            phase5_non_leaf_full_measure_count = perf_counters.phase5_non_leaf_full_measure_count,
            phase6_full_measure_count = perf_counters.phase6_full_measure_count,
            "FacetBand synthesized measurement counters"
        );
        self.eval_ctx.record_facet_band_measure_run(
            perf_counters.phase5_leaf_measure_count,
            perf_counters.phase5_non_leaf_probe_aggregate_count,
            perf_counters.phase5_non_leaf_full_measure_count,
            perf_counters.phase6_full_measure_count,
        );

        // Step 7: Assemble -- package runtime cell state for coordination and rendering.
        Ok(coord_measurement)
    }

    #[cfg(test)]
    async fn synthesize_band_probe_only(
        &self,
        perf_counters: &mut FacetPipelinePerfCounters,
    ) -> Result<FacetBandProbeSynthesis, AvengerChartError> {
        let resolved = match self.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Empty(_) => {
                return Ok(FacetBandProbeSynthesis {
                    cell_probe_summary: FacetCellProbeSummary {
                        guide_overflow: OverflowSpaceRequirement::default(),
                        total_overflow: OverflowSpaceRequirement::default(),
                        max_child_padding: 0.0,
                    },
                });
            }
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
        };
        let cell_values = self.enumerate_cell_values(&resolved);
        if cell_values.is_empty() {
            return Ok(FacetBandProbeSynthesis {
                cell_probe_summary: FacetCellProbeSummary {
                    guide_overflow: OverflowSpaceRequirement::default(),
                    total_overflow: OverflowSpaceRequirement::default(),
                    max_child_padding: 0.0,
                },
            });
        }
        let subplot_band_size = self.resolve_subplot_band_size(&resolved, &cell_values);

        let data_df = self.data.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "{} measure requires data",
                self.axis_ops.facet_label
            ))
        })?;

        ensure_subtree_precomputed(
            self.compiled_marks,
            self.facet_path,
            data_df,
            &self.eval_ctx.facet_tree,
            self.eval_ctx,
        )
        .await?;

        let cell_synthesis = self.synthesize_cell_attributes(&resolved, &cell_values)?;
        let (prepared_synthesis, prepared_runtime) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis,
            data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            subplot_band_size,
            self.eval_ctx,
        )
        .await?;
        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, subplot_band_size);
        let (overflow_synthesis, overflow_runtime) = synthesize_overflow_probe_attributes(
            &prepared_synthesis,
            &prepared_runtime,
            subplot_plot_width,
            subplot_plot_height,
            perf_counters,
        )
        .await?;

        let plan = FacetBandMeasurePlan {
            cell_values: overflow_synthesis
                .prepared_synthesis
                .cell_synthesis
                .cell_values
                .clone(),
            cells: overflow_runtime.cells,
            scale_artifacts: prepared_runtime.scale_artifacts.clone(),
        };
        let (
            band_layout_plan,
            _final_subplot_band_size,
            final_subplot_plot_width,
            final_subplot_plot_height,
        ) = self.derive_local_layout_from_probe(
            &plan,
            &overflow_synthesis.overflow_probe_summary,
            prepared_runtime.initial_subplot_band_size,
            &prepared_runtime.original_band_scale,
            overflow_synthesis
                .prepared_synthesis
                .cell_synthesis
                .empty_cell_policy,
        )?;

        let mut final_probe_cells =
            prepared_cells_as_drafts(&overflow_synthesis.prepared_synthesis, &prepared_runtime);
        let final_overflow_summary = synthesize_overflow_probe(
            &mut final_probe_cells,
            final_subplot_plot_width,
            final_subplot_plot_height,
            &prepared_runtime.compiled_subplot,
            &prepared_runtime.subplot_eval_ctx,
            &prepared_runtime.nested_measure_ctx,
            overflow_synthesis
                .prepared_synthesis
                .cell_synthesis
                .empty_cell_policy,
            is_leaf_subplot(&prepared_runtime.compiled_subplot),
            perf_counters,
        )
        .await?;

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_layout_plan.padding_inner_px,
            outer_start: band_layout_plan.outer_start,
            outer_end: band_layout_plan.outer_end,
            n: band_layout_plan.n,
        };
        let local_overflow = derive_band_overflow_from_probe(
            self.axis_ops.axis,
            &overflow_synthesis.prepared_synthesis.renderable_mask,
            &final_overflow_summary.cell_overflows,
        );
        let probe_measurement = FacetBandProbeMeasurement {
            axis: self.axis_ops.axis,
            cell_values: plan.cell_values.clone(),
            cell_has_data_rows: plan
                .cells
                .iter()
                .map(|cell| cell.plan.has_data_rows)
                .collect(),
            local_overflow,
            original_band_scale: prepared_runtime.original_band_scale.clone(),
            local_layout,
            empty_cell_policy: overflow_synthesis
                .prepared_synthesis
                .cell_synthesis
                .empty_cell_policy,
            fixed_plot_area_lock: matches!(
                self.eval_ctx.facet_runtime_sizing_mode(),
                FacetRuntimeSizingMode::FixedSubplot { .. }
            ),
        };

        let mut adjusted_scales = self.scales.clone();
        probe_measurement.apply_scale_adjustments(&mut adjusted_scales);
        let guide_overflow =
            if let Some(compiled_guide) = &prepared_runtime.compiled_subplot.compiled_guide {
                let configured_scales = adjusted_scales
                    .iter()
                    .map(|(name, scale)| (name.clone(), scale.configured().clone()))
                    .collect::<HashMap<_, _>>();
                compiled_guide
                    .measure_overflow(
                        &configured_scales,
                        final_subplot_plot_width,
                        final_subplot_plot_height,
                        prepared_runtime.compiled_subplot.get_theme().as_ref(),
                        &self.eval_ctx.params,
                        self.data,
                        self.eval_ctx.session_context.as_ref(),
                        self.eval_ctx.facet_tree.as_ref(),
                        self.facet_path,
                        Some(&probe_measurement),
                    )
                    .await?
            } else {
                probe_measurement.local_overflow_value().guide
            };
        let total_overflow = prepared_runtime
            .compiled_subplot
            .total_overflow_from_precomputed_guide_overflow(
                self.eval_ctx,
                &fixed_plot_area_layout_spec(final_subplot_plot_width, final_subplot_plot_height),
                &adjusted_scales,
                final_subplot_plot_width,
                final_subplot_plot_height,
                &guide_overflow,
                self.facet_path,
            )
            .await?;

        Ok(FacetBandProbeSynthesis {
            cell_probe_summary: FacetCellProbeSummary {
                guide_overflow,
                total_overflow,
                max_child_padding: band_layout_plan.padding_inner_px,
            },
        })
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
            let available_scales = self.scales.keys().cloned().collect::<Vec<_>>();
            AvengerChartError::InternalError(format!(
                "{} (axis={:?}, facet_path={:?}, available_scales={:?})",
                self.axis_ops.missing_scale_err,
                self.axis_ops.axis,
                self.facet_path,
                available_scales
            ))
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
                matches!(
                    self.eval_ctx.facet_runtime_sizing_mode(),
                    FacetRuntimeSizingMode::FixedSubplot { .. }
                ),
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

    fn fixed_subtree_plot_area_for_cell_path(
        &self,
        cell_path: &[ScalarValue],
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> (f32, f32) {
        if let Some(node) = self.eval_ctx.facet_tree.node_at_path(cell_path) {
            synthesize_subtree_plot_area_from_leaf_size(node, leaf_plot_width, leaf_plot_height)
        } else {
            (leaf_plot_width, leaf_plot_height)
        }
    }

    fn resolve_subplot_band_size(
        &self,
        resolved: &FacetBandResolvedNode<'_>,
        cell_values: &[ScalarValue],
    ) -> f32 {
        match self.eval_ctx.facet_runtime_sizing_mode() {
            FacetRuntimeSizingMode::CanvasFit => resolved.subplot_band_size,
            FacetRuntimeSizingMode::FixedSubplot {
                leaf_plot_width,
                leaf_plot_height,
            } => {
                let mut max_main_size = 0.0f32;
                for value in cell_values {
                    let mut cell_path = self.facet_path.to_vec();
                    cell_path.push(value.clone());
                    let (plot_width, plot_height) = self.fixed_subtree_plot_area_for_cell_path(
                        &cell_path,
                        leaf_plot_width,
                        leaf_plot_height,
                    );
                    max_main_size =
                        max_main_size.max(self.axis_ops.main_size(plot_width, plot_height));
                }
                if max_main_size > 0.0 {
                    max_main_size
                } else {
                    resolved.subplot_band_size
                }
            }
        }
    }

    fn synthesize_cell_attributes(
        &self,
        resolved: &FacetBandResolvedNode<'_>,
        cell_values: &[ScalarValue],
    ) -> Result<FacetBandSemantics, AvengerChartError> {
        FacetBandSemantics::from_tree_and_values(
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

    fn derive_local_layout_from_probe(
        &self,
        plan: &FacetBandMeasurePlan,
        overflow_summary: &OverflowProbeSummary,
        initial_subplot_band_size: f32,
        band_scale: &ConfiguredScale,
        empty_cell_policy: FacetEmptyCellPolicy,
    ) -> Result<(FacetBandPlan, f32, f32, f32), AvengerChartError> {
        let band_layout_plan = self.derive_layout_plan(
            &plan.cells,
            &plan.cell_values,
            overflow_summary,
            empty_cell_policy,
        );
        let final_subplot_band_size = if matches!(
            self.eval_ctx.facet_runtime_sizing_mode(),
            FacetRuntimeSizingMode::FixedSubplot { .. }
        ) {
            initial_subplot_band_size
        } else {
            self.build_pass2_scale(
                band_scale,
                &band_layout_plan,
                &plan.cell_values,
                initial_subplot_band_size,
            )?
        };
        let (subplot_plot_width, subplot_plot_height) = self
            .axis_ops
            .measure_dims(self.plot_other_axis_size, final_subplot_band_size);
        Ok((
            band_layout_plan,
            final_subplot_band_size,
            subplot_plot_width,
            subplot_plot_height,
        ))
    }

    async fn synthesize_local_layout(
        &self,
        mut plan: FacetBandMeasurePlan,
        overflow_summary: &OverflowProbeSummary,
        initial_subplot_band_size: f32,
        compiled_subplot: &Arc<CompiledPlot>,
        subplot_eval_ctx: &EvaluationContext,
        nested_measure_ctx: &FacetBandNestedMeasureContext,
        band_scale: &ConfiguredScale,
        empty_cell_policy: FacetEmptyCellPolicy,
        perf_counters: &mut FacetPipelinePerfCounters,
    ) -> Result<FacetLocalLayoutOutcome, AvengerChartError> {
        let (band_layout_plan, final_subplot_band_size, subplot_plot_width, subplot_plot_height) =
            self.derive_local_layout_from_probe(
                &plan,
                overflow_summary,
                initial_subplot_band_size,
                band_scale,
                empty_cell_policy,
            )?;

        measure_cells_final_and_extents(
            &mut plan.cells,
            subplot_plot_width,
            subplot_plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            nested_measure_ctx,
            empty_cell_policy,
            perf_counters,
        )
        .await?;

        Ok(FacetLocalLayoutOutcome {
            band_layout_plan,
            final_subplot_main_or_cross_size: final_subplot_band_size,
            cells: plan.cells,
        })
    }

    async fn synthesize_local_attributes(
        &self,
        overflow_synthesis: &FacetBandOverflowSynthesis,
        overflow_runtime: FacetBandOverflowRuntime,
        prepared_runtime: &FacetBandPreparedRuntime,
        perf_counters: &mut FacetPipelinePerfCounters,
    ) -> Result<(FacetBandLocalSynthesis, FacetBandMeasuredRuntime), AvengerChartError> {
        let plan = FacetBandMeasurePlan {
            cell_values: overflow_synthesis
                .prepared_synthesis
                .cell_synthesis
                .cell_values
                .clone(),
            cells: overflow_runtime.cells,
            scale_artifacts: prepared_runtime.scale_artifacts.clone(),
        };
        let local_layout_outcome = self
            .synthesize_local_layout(
                plan,
                &overflow_synthesis.overflow_probe_summary,
                prepared_runtime.initial_subplot_band_size,
                &prepared_runtime.compiled_subplot,
                &prepared_runtime.subplot_eval_ctx,
                &prepared_runtime.nested_measure_ctx,
                &prepared_runtime.original_band_scale,
                overflow_synthesis
                    .prepared_synthesis
                    .cell_synthesis
                    .empty_cell_policy,
                perf_counters,
            )
            .await?;

        let channel_sharing_levels = collect_channel_sharing_levels(&local_layout_outcome.cells);
        let mut measurements = Vec::with_capacity(local_layout_outcome.cells.len());
        let mut local_domain_extents = Vec::with_capacity(local_layout_outcome.cells.len());
        for mut cell in local_layout_outcome.cells {
            measurements.push(cell.measurement.take().expect(
                "FacetBand attributes invariant violated: missing final cell measurement in measured-runtime sidecars",
            ));
            local_domain_extents.push(cell.local_domain_extents);
        }

        let local_synthesis = FacetBandLocalSynthesis {
            overflow_synthesis: overflow_synthesis.clone(),
            band_layout_plan: local_layout_outcome.band_layout_plan,
            final_subplot_cross_size: local_layout_outcome.final_subplot_main_or_cross_size,
            channel_sharing_levels,
        };
        let sidecars = FacetBandMeasuredRuntime {
            measurements,
            local_domain_extents,
        };
        Ok((local_synthesis, sidecars))
    }

    fn assemble_coord_measurement(
        &self,
        local_synthesis: FacetBandLocalSynthesis,
        measured_runtime: FacetBandMeasuredRuntime,
        prepared_runtime: &FacetBandPreparedRuntime,
    ) -> Box<dyn CoordMeasurement> {
        let FacetBandLocalSynthesis {
            overflow_synthesis,
            band_layout_plan,
            final_subplot_cross_size,
            channel_sharing_levels,
        } = local_synthesis;
        let FacetBandOverflowSynthesis {
            prepared_synthesis, ..
        } = overflow_synthesis;
        let FacetBandPreparedSynthesis { cell_synthesis, .. } = prepared_synthesis;
        let crate::facet::band_attributes::FacetBandSemantics {
            facet_depth,
            coordination_field_identity,
            empty_cell_policy,
            cells,
            ..
        } = cell_synthesis;

        assert_eq!(
            cells.len(),
            prepared_runtime.data_overrides.len(),
            "FacetBand attributes/sidecar invariant violated: cell-synthesis cell count must equal prepared-synthesis data overrides"
        );
        assert_eq!(
            cells.len(),
            measured_runtime.measurements.len(),
            "FacetBand attributes/sidecar invariant violated: cell-synthesis cell count must equal measured-runtime measurements"
        );
        assert_eq!(
            cells.len(),
            measured_runtime.local_domain_extents.len(),
            "FacetBand attributes/sidecar invariant violated: cell-synthesis cell count must equal measured-runtime local extents"
        );

        let cell_runtimes: Vec<FacetCellRuntime> = cells
            .into_iter()
            .zip(prepared_runtime.data_overrides.iter().cloned())
            .zip(
                measured_runtime
                    .measurements
                    .into_iter()
                    .zip(measured_runtime.local_domain_extents),
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
        let base = FacetBandCoordMeasurement {
            axis: self.axis_ops.axis,
            cells: cell_runtimes,
            shared_scale_builder: prepared_runtime
                .scale_artifacts
                .shared_scale_builder
                .clone(),
            coordinated_overflow: CoordinatedOverflow::default(),
            compiled_subplot: prepared_runtime.compiled_subplot.clone(),
            subplot_cross_size: final_subplot_cross_size,
            facet_depth,
            original_band_scale: prepared_runtime.original_band_scale.clone(),
            local_layout,
            coordinated_layout: None,
            coordination_field_identity,
            channel_sharing_levels,
            empty_cell_policy,
        };

        if matches!(
            self.eval_ctx.facet_runtime_sizing_mode(),
            FacetRuntimeSizingMode::FixedSubplot { .. }
        ) {
            let fixed_main_axis_positions = compute_fixed_main_axis_positions(
                self.axis_ops.axis,
                &base.cells,
                &base.local_layout,
            );
            Box::new(FacetBandCoordMeasurementFixed {
                base,
                fixed_main_axis_positions,
            })
        } else {
            Box::new(base)
        }
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
    match eval_ctx.facet_runtime_sizing_mode() {
        FacetRuntimeSizingMode::CanvasFit => {
            crate::facet::coord_canvas_fit::measure_facet_row_canvas_fit(
                scales,
                plot_width,
                eval_ctx,
                data,
                compiled_marks,
                facet_path,
            )
            .await
        }
        FacetRuntimeSizingMode::FixedSubplot { .. } => {
            crate::facet::coord_fixed_subplot::measure_facet_row_fixed_subplot(
                scales,
                plot_width,
                eval_ctx,
                data,
                compiled_marks,
                facet_path,
            )
            .await
        }
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
        match eval_ctx.facet_runtime_sizing_mode() {
            FacetRuntimeSizingMode::CanvasFit => {
                crate::facet::coord_canvas_fit::measure_facet_column_canvas_fit(
                    scales,
                    plot_height,
                    eval_ctx,
                    data,
                    compiled_marks,
                    facet_path,
                )
                .await
            }
            FacetRuntimeSizingMode::FixedSubplot { .. } => {
                crate::facet::coord_fixed_subplot::measure_facet_column_fixed_subplot(
                    scales,
                    plot_height,
                    eval_ctx,
                    data,
                    compiled_marks,
                    facet_path,
                )
                .await
            }
        }
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

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            "column" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
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
    use crate::render::types::LegendMeasurement;
    use crate::scales::{Linear, Scale};
    use crate::theme::Theme;
    use avenger_scales::scales::{band::BandScale, linear::LinearScale};
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::sync::Arc;
    use taffy::Size;

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

    fn make_linear_scale_with_binding(
        range: (f32, f32),
        binding: ScaleRangeBinding,
    ) -> ConfiguredScaleWithSpec {
        ConfiguredScaleWithSpec::with_range_binding(
            Scale::<Linear>::new().into_auto(),
            LinearScale::configured((0.0, 1.0), range),
            binding,
        )
    }

    fn assert_configured_range_close(scale: &ConfiguredScaleWithSpec, expected: (f32, f32)) {
        let actual = scale.configured().numeric_interval_range().unwrap();
        assert!((actual.0 - expected.0).abs() <= 0.01, "{actual:?}");
        assert!((actual.1 - expected.1).abs() <= 0.01, "{actual:?}");
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 0.01,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn retarget_scale_ranges_for_plot_area_uses_binding_not_span_similarity() {
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            make_linear_scale_with_binding(
                (0.0, 157.34482),
                ScaleRangeBinding::plot_area(
                    PlotAreaRangeEndpoint::ZERO,
                    PlotAreaRangeEndpoint::WIDTH,
                ),
            ),
        );
        scales.insert(
            "y".to_string(),
            make_linear_scale_with_binding(
                (156.72412, 0.0),
                ScaleRangeBinding::plot_area(
                    PlotAreaRangeEndpoint::HEIGHT,
                    PlotAreaRangeEndpoint::ZERO,
                ),
            ),
        );

        let retarget_count = retarget_scale_ranges_for_plot_area(&mut scales, 157.34482, 505.0);

        assert_eq!(retarget_count, 1);
        assert_configured_range_close(scales.get("x").unwrap(), (0.0, 157.34482));
        assert_configured_range_close(scales.get("y").unwrap(), (505.0, 0.0));
    }

    #[test]
    fn retarget_scale_ranges_for_plot_area_leaves_independent_ranges_alone() {
        let mut scales = HashMap::new();
        scales.insert(
            "size".to_string(),
            make_linear_scale_with_binding((10.0, 200.0), ScaleRangeBinding::Independent),
        );

        let retarget_count = retarget_scale_ranges_for_plot_area(&mut scales, 500.0, 500.0);

        assert_eq!(retarget_count, 0);
        assert_configured_range_close(scales.get("size").unwrap(), (10.0, 200.0));
    }

    fn sample_layout_solution(
        legends: IndexMap<String, LayoutBounds>,
        legends_by_position: IndexMap<LegendPosition, Vec<String>>,
    ) -> crate::render::LayoutSolution {
        crate::render::LayoutSolution {
            taffy_layout: crate::layout::LayoutResult {
                plot_area: LayoutBounds {
                    x: 10.0,
                    y: 20.0,
                    width: 80.0,
                    height: 157.0,
                },
                guide_overflows: HashMap::new(),
                legends,
                legends_by_position,
                title: None,
                subtitle: None,
            },
            canvas_size: (200.0, 240.0),
            overflow: OverflowSpaceRequirement::default(),
            total_overflow: OverflowSpaceRequirement::default(),
            legend_info: crate::layout::LegendLayoutInfo::default(),
        }
    }

    #[test]
    fn apply_layout_side_slab_reanchors_right_legend_after_guide_coordination() {
        let mut legends = IndexMap::new();
        legends.insert(
            "fill".to_string(),
            LayoutBounds {
                x: 90.0,
                y: 20.0,
                width: 61.0,
                height: 157.0,
            },
        );
        let mut legends_by_position = IndexMap::new();
        legends_by_position.insert(LegendPosition::Right, vec!["fill".to_string()]);
        let mut layout = sample_layout_solution(legends, legends_by_position);

        apply_layout_side_slab(&mut layout, AxisPosition::Right, 5.0, 66.0);

        assert_close(layout.overflow.right, 5.0);
        assert_close(layout.total_overflow.right, 66.0);
        assert_close(layout.canvas_size.0, 266.0);
        let guide = layout
            .taffy_layout
            .guide_overflows
            .get(&AxisPosition::Right)
            .expect("expected right guide overflow bounds");
        assert_close(guide.x, 90.0);
        assert_close(guide.y, 20.0);
        assert_close(guide.width, 5.0);
        assert_close(guide.height, 157.0);

        let bounds = layout
            .taffy_layout
            .legends
            .get("fill")
            .expect("expected fill legend bounds");
        assert_close(bounds.x, 95.0);
        assert_close(bounds.y, 20.0);
        assert_close(bounds.width, 61.0);
        assert_close(bounds.height, 157.0);
    }

    #[test]
    fn apply_layout_side_slab_preserves_measured_bottom_guide_for_legend_only_slab() {
        let mut legends = IndexMap::new();
        legends.insert(
            "color".to_string(),
            LayoutBounds {
                x: 10.0,
                y: 211.0,
                width: 80.0,
                height: 48.0,
            },
        );
        let mut legends_by_position = IndexMap::new();
        legends_by_position.insert(LegendPosition::Bottom, vec!["color".to_string()]);
        let mut layout = sample_layout_solution(legends, legends_by_position);
        layout.overflow.bottom = 34.0;
        layout.total_overflow.bottom = 82.0;

        apply_layout_side_slab(&mut layout, AxisPosition::Bottom, 0.0, 48.0);

        assert_close(layout.overflow.bottom, 34.0);
        assert_close(layout.total_overflow.bottom, 82.0);
        let guide = layout
            .taffy_layout
            .guide_overflows
            .get(&AxisPosition::Bottom)
            .expect("expected bottom guide overflow bounds");
        assert_close(guide.y, 177.0);
        assert_close(guide.height, 34.0);

        let bounds = layout
            .taffy_layout
            .legends
            .get("color")
            .expect("expected color legend bounds");
        assert_close(bounds.y, 211.0);
        assert_close(bounds.height, 48.0);
    }

    #[test]
    fn retarget_layout_resizes_flexible_right_legend_to_new_plot_height() {
        let mut legends = IndexMap::new();
        legends.insert(
            "fill".to_string(),
            LayoutBounds {
                x: 90.0,
                y: 20.0,
                width: 61.0,
                height: 157.0,
            },
        );
        let mut legends_by_position = IndexMap::new();
        legends_by_position.insert(LegendPosition::Right, vec!["fill".to_string()]);
        let mut layout = sample_layout_solution(legends, legends_by_position);

        let mut legend_measurements = LegendMeasurements::new();
        legend_measurements.insert(
            "fill".to_string(),
            LegendMeasurement {
                size: Size {
                    width: 61.0,
                    height: 157.0,
                },
                flexible: true,
                position: LegendPosition::Right,
            },
        );

        retarget_layout_for_resized_plot_area(
            &mut layout,
            &legend_measurements,
            100.0,
            246.0,
            20.0,
            89.0,
        );

        let bounds = layout
            .taffy_layout
            .legends
            .get("fill")
            .expect("expected fill legend bounds");
        assert_close(bounds.x, 110.0);
        assert_close(bounds.y, 20.0);
        assert_close(bounds.width, 61.0);
        assert_close(bounds.height, 246.0);
    }

    #[test]
    fn retarget_layout_preserves_fixed_legend_size_when_flexible_legend_grows() {
        let mut legends = IndexMap::new();
        legends.insert(
            "shape".to_string(),
            LayoutBounds {
                x: 90.0,
                y: 20.0,
                width: 80.0,
                height: 40.0,
            },
        );
        legends.insert(
            "fill".to_string(),
            LayoutBounds {
                x: 90.0,
                y: 60.0,
                width: 61.0,
                height: 117.0,
            },
        );
        let mut legends_by_position = IndexMap::new();
        legends_by_position.insert(
            LegendPosition::Right,
            vec!["shape".to_string(), "fill".to_string()],
        );
        let mut layout = sample_layout_solution(legends, legends_by_position);

        let mut legend_measurements = LegendMeasurements::new();
        legend_measurements.insert(
            "shape".to_string(),
            LegendMeasurement {
                size: Size {
                    width: 80.0,
                    height: 40.0,
                },
                flexible: false,
                position: LegendPosition::Right,
            },
        );
        legend_measurements.insert(
            "fill".to_string(),
            LegendMeasurement {
                size: Size {
                    width: 61.0,
                    height: 117.0,
                },
                flexible: true,
                position: LegendPosition::Right,
            },
        );

        retarget_layout_for_resized_plot_area(
            &mut layout,
            &legend_measurements,
            80.0,
            246.0,
            0.0,
            89.0,
        );

        let shape_bounds = layout
            .taffy_layout
            .legends
            .get("shape")
            .expect("expected shape legend bounds");
        assert_close(shape_bounds.y, 20.0);
        assert_close(shape_bounds.height, 40.0);

        let fill_bounds = layout
            .taffy_layout
            .legends
            .get("fill")
            .expect("expected fill legend bounds");
        assert_close(fill_bounds.y, 60.0);
        assert_close(fill_bounds.height, 206.0);
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

    async fn build_non_leaf_probe_fixture() -> Result<FacetBandPipelineFixture, AvengerChartError> {
        let session = SessionContext::new();
        let data_df = session
            .sql(
                "SELECT * FROM (VALUES \
                 ('G1', 'A', 1.0, 10.0), \
                 ('G1', 'B', 2.0, 20.0), \
                 ('G2', 'A', 3.0, 30.0), \
                 ('G2', 'B', 4.0, 40.0) \
                 ) AS t(group_id, team_id, x, y)",
            )
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        let leaf_subplot = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill("#4682b4")
                .size(42.0),
        );
        let nested_subplot = Plot::<FacetColumn>::new().mark(
            Facet::new()
                .col_with(col("team_id"), |c| c)
                .subplot(leaf_subplot),
        );
        let compiled_plot = Plot::<FacetColumn>::new()
            .data(data_df.clone())
            .mark(
                Facet::new()
                    .col_with(col("group_id"), |c| c)
                    .subplot(nested_subplot),
            )
            .compile(&session)
            .await?;

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

        Ok(FacetBandPipelineFixture {
            scales,
            eval_ctx,
            data_df,
            compiled_marks: compiled_plot.marks.clone(),
            plot_other_axis_size: plot_height,
        })
    }

    async fn build_synthesis_measurement_fixture()
    -> Result<(Arc<CompiledPlot>, FacetPreparedRuntimeInputs), AvengerChartError> {
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
        let prepared_synthesis = prepare_measurement_inputs(
            cell_plans,
            &facet_path,
            &data_df,
            &compiled_subplot,
            FacetEmptyCellPolicy::Hole,
            &eval_ctx,
        )
        .await?;

        Ok((compiled_subplot, prepared_synthesis))
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
    fn apply_facet_band_scale_adjustment_honors_zero_padding_layout() {
        let base = make_band_scale((0.0, 200.0)).with_option("padding_inner", 0.1);
        let mut scales = HashMap::from([(
            "column".to_string(),
            ConfiguredScaleWithSpec::with_range_binding(
                Scale::<Band>::new().into_auto(),
                base,
                ScaleRangeBinding::plot_area(
                    PlotAreaRangeEndpoint::ZERO,
                    PlotAreaRangeEndpoint::WIDTH,
                ),
            ),
        )]);
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            outer_start: 0.0,
            outer_end: 0.0,
            n: 1,
        };

        apply_facet_band_scale_adjustment(
            FacetAxis::Column,
            &mut scales,
            &make_band_scale((0.0, 200.0)),
            &layout,
            &[s("a")],
            false,
            false,
        );

        let adjusted = scales.get("column").unwrap().configured();
        assert_eq!(
            adjusted
                .config
                .options
                .get("padding_inner_px")
                .unwrap()
                .as_f32()
                .unwrap(),
            0.0
        );
        assert_eq!(bandwidth(&adjusted.config).unwrap(), 200.0);
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
        assert_eq!(outcome.remeasure_skipped_cell_count, 0);
        assert_eq!(outcome.remeasured_non_empty_cell_count, 0);
        assert_eq!(outcome.remeasured_with_coordinated_extents_count, 0);
        assert!((facet_band.subplot_cross_size - before_cross_size).abs() > 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn apply_coordinated_overflow_with_plan_retargets_cells_when_required()
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

        assert!(!outcome.remeasure_triggered);
        assert_eq!(expected_cells, facet_band.cells.len());
        assert_eq!(outcome.remeasured_cell_count, 0);
        assert_eq!(outcome.remeasure_skipped_cell_count, 0);
        assert_eq!(outcome.remeasured_non_empty_cell_count, 0);
        assert_eq!(outcome.remeasured_with_coordinated_extents_count, 0);
        assert!(first_after < first_before);
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
            cell_probe_summaries: vec![],
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
    fn derive_band_overflow_from_probe_matches_coord_merge_semantics_column() {
        let cell_overflows = vec![
            (
                OverflowSpaceRequirement {
                    top: 1.0,
                    right: 2.0,
                    bottom: 3.0,
                    left: 4.0,
                },
                OverflowSpaceRequirement {
                    top: 10.0,
                    right: 20.0,
                    bottom: 30.0,
                    left: 40.0,
                },
            ),
            (
                OverflowSpaceRequirement {
                    top: 6.0,
                    right: 7.0,
                    bottom: 8.0,
                    left: 9.0,
                },
                OverflowSpaceRequirement {
                    top: 16.0,
                    right: 17.0,
                    bottom: 18.0,
                    left: 19.0,
                },
            ),
            (
                OverflowSpaceRequirement {
                    top: 2.0,
                    right: 12.0,
                    bottom: 4.0,
                    left: 5.0,
                },
                OverflowSpaceRequirement {
                    top: 12.0,
                    right: 22.0,
                    bottom: 14.0,
                    left: 15.0,
                },
            ),
        ];
        let renderable_mask = vec![true, false, true];
        let overflow =
            derive_band_overflow_from_probe(FacetAxis::Column, &renderable_mask, &cell_overflows);
        assert_eq!(overflow.guide.left, 4.0);
        assert_eq!(overflow.guide.right, 12.0);
        assert_eq!(overflow.guide.top, 2.0);
        assert_eq!(overflow.guide.bottom, 4.0);
        assert_eq!(overflow.total.left, 40.0);
        assert_eq!(overflow.total.right, 22.0);
        assert_eq!(overflow.total.top, 12.0);
        assert_eq!(overflow.total.bottom, 30.0);
    }

    #[test]
    fn derive_band_overflow_from_probe_matches_coord_merge_semantics_row() {
        let cell_overflows = vec![
            (
                OverflowSpaceRequirement {
                    top: 3.0,
                    right: 4.0,
                    bottom: 5.0,
                    left: 1.0,
                },
                OverflowSpaceRequirement {
                    top: 13.0,
                    right: 14.0,
                    bottom: 15.0,
                    left: 11.0,
                },
            ),
            (
                OverflowSpaceRequirement {
                    top: 9.0,
                    right: 2.0,
                    bottom: 12.0,
                    left: 8.0,
                },
                OverflowSpaceRequirement {
                    top: 19.0,
                    right: 12.0,
                    bottom: 22.0,
                    left: 18.0,
                },
            ),
            (
                OverflowSpaceRequirement {
                    top: 6.0,
                    right: 7.0,
                    bottom: 10.0,
                    left: 3.0,
                },
                OverflowSpaceRequirement {
                    top: 16.0,
                    right: 17.0,
                    bottom: 20.0,
                    left: 13.0,
                },
            ),
        ];
        let renderable_mask = vec![true, false, true];
        let overflow =
            derive_band_overflow_from_probe(FacetAxis::Row, &renderable_mask, &cell_overflows);
        assert_eq!(overflow.guide.top, 3.0);
        assert_eq!(overflow.guide.bottom, 10.0);
        assert_eq!(overflow.guide.left, 3.0);
        assert_eq!(overflow.guide.right, 7.0);
        assert_eq!(overflow.total.top, 13.0);
        assert_eq!(overflow.total.bottom, 20.0);
        assert_eq!(overflow.total.left, 13.0);
        assert_eq!(overflow.total.right, 17.0);
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
            cell_probe_summaries: vec![],
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
            cell_probe_summaries: vec![],
            max_child_padding: 18.0,
        };
        let renderable_cells = vec![true, true];

        assert_eq!(
            derive_padding_inner_px_from_probe(FacetAxis::Column, &pass1, &renderable_cells),
            18.0
        );
    }

    #[test]
    fn derive_layout_plan_applies_minimum_gap_between_renderable_cells() {
        let pass1 = OverflowProbeSummary {
            cell_overflows: vec![
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement::default(),
                ),
                (
                    OverflowSpaceRequirement::default(),
                    OverflowSpaceRequirement::default(),
                ),
            ],
            cell_probe_summaries: vec![],
            max_child_padding: 0.0,
        };
        let renderable_cells = vec![true, true];

        assert_eq!(
            derive_padding_inner_px_from_probe(FacetAxis::Column, &pass1, &renderable_cells),
            padding_policy::MIN_SUBPLOT_MAIN_GAP
        );
    }

    #[test]
    fn derive_layout_plan_skips_minimum_gap_for_single_renderable_cell() {
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
            cell_probe_summaries: vec![],
            max_child_padding: 0.0,
        };
        let renderable_cells = vec![false, true];

        assert_eq!(
            derive_padding_inner_px_from_probe(FacetAxis::Column, &pass1, &renderable_cells),
            0.0
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
    async fn overflow_synthesis_overflow_probe_leaf_capture_preserves_extents()
    -> Result<(), AvengerChartError> {
        let (compiled_subplot, prepared_synthesis) = build_synthesis_measurement_fixture().await?;
        let FacetPreparedRuntimeInputs {
            mut plan,
            subplot_eval_ctx,
            nested_measure_ctx,
        } = prepared_synthesis;
        let cells = &mut plan.cells;
        let is_leaf = is_leaf_subplot(&compiled_subplot);

        let before: Vec<(bool, usize)> = cells
            .iter()
            .map(|cell| (cell.measurement.is_none(), cell.local_domain_extents.len()))
            .collect();
        let mut perf_counters = FacetPipelinePerfCounters::default();

        let overflow_synthesis = synthesize_overflow_probe(
            cells,
            140.0,
            140.0,
            &compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
            FacetEmptyCellPolicy::Hole,
            is_leaf,
            &mut perf_counters,
        )
        .await?;

        assert_eq!(overflow_synthesis.cell_overflows.len(), 3);

        let after: Vec<(bool, usize)> = cells
            .iter()
            .map(|cell| (cell.measurement.is_none(), cell.local_domain_extents.len()))
            .collect();
        if is_leaf {
            assert!(after.iter().all(|(measurement_none, _)| !*measurement_none));
        } else {
            assert_eq!(before, after);
        }

        Ok(())
    }

    #[tokio::test]
    async fn non_leaf_phase5_probe_path_records_full_measure_calls() -> Result<(), AvengerChartError>
    {
        let fixture = build_non_leaf_probe_fixture().await?;
        let pipeline = FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            &fixture.scales,
            fixture.plot_other_axis_size,
            &fixture.eval_ctx,
            Some(&fixture.data_df),
            &fixture.compiled_marks,
            &[],
        );
        let mut perf_counters = FacetPipelinePerfCounters::default();

        let synthesis = pipeline
            .synthesize_band_probe_only(&mut perf_counters)
            .await?;
        assert!(synthesis.cell_probe_summary.max_child_padding >= 0.0);
        assert!(perf_counters.phase5_non_leaf_probe_aggregate_count > 0);
        assert!(perf_counters.phase5_non_leaf_full_measure_count > 0);

        Ok(())
    }

    #[tokio::test]
    async fn local_synthesis_local_layout_finalization_populates_measurements_and_extents()
    -> Result<(), AvengerChartError> {
        let (compiled_subplot, prepared_synthesis) = build_synthesis_measurement_fixture().await?;
        let FacetPreparedRuntimeInputs {
            mut plan,
            subplot_eval_ctx,
            nested_measure_ctx,
        } = prepared_synthesis;
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let is_leaf = is_leaf_subplot(&compiled_subplot);
        synthesize_overflow_probe(
            &mut plan.cells,
            140.0,
            140.0,
            &compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
            FacetEmptyCellPolicy::Hole,
            is_leaf,
            &mut perf_counters,
        )
        .await?;

        measure_cells_final_and_extents(
            &mut plan.cells,
            140.0,
            140.0,
            &compiled_subplot,
            &subplot_eval_ctx,
            &nested_measure_ctx,
            FacetEmptyCellPolicy::Hole,
            &mut perf_counters,
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
    async fn prepared_synthesis_ir_sidecars_align_with_cells() -> Result<(), AvengerChartError> {
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
                panic!("expected ready facet node for prepared_synthesis attributes test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_synthesis = pipeline.synthesize_cell_attributes(&resolved, &cell_values)?;
        let (prepared_synthesis, sidecars) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis.clone(),
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;

        assert_eq!(
            prepared_synthesis.cell_synthesis.cells.len(),
            sidecars.data_overrides.len()
        );
        assert_eq!(
            prepared_synthesis.cell_synthesis.cells.len(),
            prepared_synthesis.renderable_mask.len()
        );
        assert_eq!(prepared_synthesis.cell_synthesis.cell_values, cell_values);
        Ok(())
    }

    #[tokio::test]
    async fn overflow_synthesis_ir_probe_is_non_mutating() -> Result<(), AvengerChartError> {
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
                panic!("expected ready facet node for overflow_synthesis attributes test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_synthesis = pipeline.synthesize_cell_attributes(&resolved, &cell_values)?;
        let (prepared_synthesis, sidecars) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis,
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
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let (overflow_synthesis, _overflow_runtime) = synthesize_overflow_probe_attributes(
            &prepared_synthesis,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;

        assert_eq!(before_rows, sidecars.data_overrides.len());
        assert_eq!(
            overflow_synthesis
                .overflow_probe_summary
                .cell_overflows
                .len(),
            prepared_synthesis.cell_synthesis.cells.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_synthesis_ir_finalization_produces_layout_and_measurements()
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
                panic!("expected ready facet node for local_synthesis attributes test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_synthesis = pipeline.synthesize_cell_attributes(&resolved, &cell_values)?;
        let (prepared_synthesis, sidecars) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis,
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
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let (overflow_synthesis, overflow_runtime) = synthesize_overflow_probe_attributes(
            &prepared_synthesis,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;
        let (local_synthesis, measured_runtime) = pipeline
            .synthesize_local_attributes(
                &overflow_synthesis,
                overflow_runtime,
                &sidecars,
                &mut perf_counters,
            )
            .await?;

        assert_eq!(
            local_synthesis.band_layout_plan.n,
            prepared_synthesis.cell_synthesis.cells.len()
        );
        assert_eq!(
            measured_runtime.measurements.len(),
            prepared_synthesis.cell_synthesis.cells.len()
        );
        assert_eq!(
            measured_runtime.local_domain_extents.len(),
            prepared_synthesis.cell_synthesis.cells.len()
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
                panic!("expected ready facet node for coord-measurement attributes test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_synthesis = pipeline.synthesize_cell_attributes(&resolved, &cell_values)?;
        let (prepared_synthesis, sidecars) = synthesize_prepared_attributes_and_runtime(
            cell_synthesis,
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
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let (overflow_synthesis, overflow_runtime) = synthesize_overflow_probe_attributes(
            &prepared_synthesis,
            &sidecars,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;
        let (local_synthesis, measured_runtime) = pipeline
            .synthesize_local_attributes(
                &overflow_synthesis,
                overflow_runtime,
                &sidecars,
                &mut perf_counters,
            )
            .await?;
        let expected_n = local_synthesis.band_layout_plan.n;
        let expected_size = local_synthesis.final_subplot_cross_size;
        let measurement =
            pipeline.assemble_coord_measurement(local_synthesis, measured_runtime, &sidecars);
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
    async fn cell_synthesis_ir_semantics_match_cell_plan_projection()
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
        let cell_synthesis = pipeline.synthesize_cell_attributes(&resolved, &cell_values)?;
        let projected_plans =
            build_facet_cell_plans(&fixture.eval_ctx.facet_tree, &[], &cell_values)?;

        let cell_synthesis_plans: Vec<FacetCellPlan> = cell_synthesis
            .cells
            .iter()
            .map(FacetCellPlan::from)
            .collect();
        assert_eq!(cell_synthesis_plans.len(), projected_plans.len());
        for (cell_synthesis_plan, projected_plan) in
            cell_synthesis_plans.iter().zip(projected_plans.iter())
        {
            assert_eq!(cell_synthesis_plan.value, projected_plan.value);
            assert_eq!(cell_synthesis_plan.full_path, projected_plan.full_path);
            assert_eq!(
                cell_synthesis_plan.in_domain_slot,
                projected_plan.in_domain_slot
            );
            assert_eq!(
                cell_synthesis_plan.has_data_rows,
                projected_plan.has_data_rows
            );
            assert_eq!(cell_synthesis_plan.empty_kind, projected_plan.empty_kind);
            assert_eq!(
                cell_synthesis_plan.filter_predicate,
                projected_plan.filter_predicate
            );
        }
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
