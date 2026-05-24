//! Facet band measurement and local layout pipeline (FacetColumn and FacetRow).
//!
//! This module builds per-cell semantics, measures estimated overflow, derives
//! local facet band layout, and retargets child measurements before global
//! coordination aligns matching facet requirements.
//!
use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, band::bandwidth};
#[cfg(test)]
use datafusion::logical_expr::lit;
use datafusion::{common::ScalarValue, dataframe::DataFrame, scalar::ScalarValue as DfScalarValue};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::{
    chart_core::AxisPosition,
    container::{ChildFrameKey, ChildFrameScopeKey, ContainerPathSegment},
    coords::{
        CellDomainInfo, CoordMeasureRequest, CoordMeasurement, CoordinateSystem,
        CoordinateSystemTransform, CoordinatedLayout, CoordinatedOverflow, FacetAxis, PaddingSpec,
        PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::FacetDirection,
    facet::{
        band_attributes::{
            FacetBandLocalLayout, FacetBandMeasuredRuntime, FacetBandOverflowProbe,
            FacetBandPreparedInputs, FacetBandPreparedRuntime, FacetBandSemantics,
            OverflowProbeSummary,
        },
        coord_row::compute_band_layout,
        coordination_plans::{
            AxisSlab, BandRetargetAction, CoordinationNodeKey, FacetOwnershipRequirement,
            PlotAreaSize, PlotAreaTarget, RetargetNodeActions, RetargetNodeOutcome,
            RetargetNodeRequirements,
        },
        domain_coordination::{
            aggregate_domain_extents, coordinated_extents_for_cell, domain_infos_for_cell,
        },
        empty_cell_policy::FacetEmptyCellPolicy,
        guide::FacetColGuideConfig,
        layout_plan::{
            FacetBandPlan, FacetCellEmptyKind, FacetCellPlan, compute_padding_from_overflows,
            effective_edge_indices, effective_edge_indices_for_values_at_path,
        },
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        overflow_projection::{
            FacetCellOverflowInput, FacetOverflowPurpose, FacetOverflowResolutionPhase,
            FacetOverflowSlabs, aggregate_facet_band_overflow, boundary_profiles_for_measurement,
            compute_padding_from_boundary_profiles, resolve_facet_overflow,
        },
        ownership_policy::{has_holes_from_cells, resolve_facet_ownership_policy},
        padding_policy, path_math,
        placement::{
            FacetBandExplicitPlacement, FacetBandPlacement, FacetBandPlacementModel,
            compute_explicit_facet_band_placement, resolve_scale_backed_facet_band_placement,
        },
        probe_summary::FacetCellProbeSummary,
        scale_precompute::{
            FacetScaleNodeArtifacts, FacetScaleNodeKey, build_node_artifacts, canonicalize_path,
            ensure_subtree_precomputed,
        },
        subtree_plot_area::{LeafPlotAreaSize, estimate_path_plot_area_from_leaf_size},
    },
    layout::{
        EdgeSlabs, EvaluatedLayoutSpec, OwnedEdgeSlabs, apply_frame_side_slab, overflow_side_value,
        retarget_frame_layout_for_plot_area,
    },
    marks::CompiledMark,
    plot::compiled::{
        CompiledPlot, ComponentsMeasurement, CoordinationKind, CoordinationScopeKey, SharingLevel,
        fixed_child_plot_area_layout_spec, measure_child_frame_plot_with_builder,
        scales::build_scale_builder_from_marks,
    },
    render::{EvaluationContext, FacetSubtreeCheckpoint, FacetSubtreeSelector},
    scales::{
        ConfiguredScaleWithSpec, PlotAreaRangeEndpoint, ScaleBuilder, ScaleRangeBinding,
        domain_extent::DomainExtent,
    },
};

#[cfg(test)]
use crate::coords::OverflowSpaceRequirement;
#[cfg(test)]
use crate::facet::coordination_plans::CellRetargetAction;
#[cfg(test)]
use crate::facet::overflow_projection::{
    FacetBandNoRenderablePolicy, aggregate_facet_band_overflow_with_policy,
};
#[cfg(test)]
use crate::facet::probe_summary::FacetBandProbeLayout;

pub use crate::facet::coord_row::FacetRow;

/// Domain extent with associated channel-domain sharing level.
///
/// Used to track domain extents per cell along with the channel-domain sharing
/// level for coordinating plot scale domains across cells at the appropriate
/// hierarchy level.
#[derive(Clone, Debug)]
pub(crate) struct ChannelDomainExtent {
    /// The domain extent (bounds + optional radius padding)
    pub extent: DomainExtent,
    /// Channel-domain sharing level (0=Free, N=Level(N), 255=Shared).
    pub(crate) domain_sharing_level: SharingLevel,
}

/// Runtime state for a single enumerated facet cell.
#[derive(Debug)]
pub(crate) struct FacetCellRuntime {
    /// Canonical plan metadata (value, path, emptiness, filter intent).
    pub(crate) plan: FacetCellPlan,
    /// Per-cell dataframe override used by render and coordinated refinement paths.
    pub(crate) data_override: DataFrame,
    /// Final subplot measurement used for rendering.
    pub(crate) measurement: ComponentsMeasurement,
    /// Per-channel local domain extents extracted before overflow measurement.
    #[cfg(test)]
    pub(crate) local_domain_extents: HashMap<String, ChannelDomainExtent>,
    /// Coordinated per-channel extents applied to initial cell measurement.
    #[cfg(test)]
    pub(crate) coordinated_domain_extents: HashMap<String, DomainExtent>,
}

/// Which rendered legend slabs are already owned by the parent facet slot.
///
/// The actual owned pixel values depend on the active coordinated layout and
/// the current overflow source, so this stores ownership policy rather than a
/// stale measured slab.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FacetBandAllocationOwnership {
    pub(crate) parent_owns_main_axis_start_slab: bool,
    pub(crate) parent_owns_main_axis_end_slab: bool,
    pub(crate) parent_owns_cross_axis_legend_slabs: bool,
    pub(crate) realized_owned_legend_slabs: Option<OwnedEdgeSlabs>,
}

impl FacetBandAllocationOwnership {
    pub(crate) fn from_policy(
        parent_owns_main_axis_outer_slabs: bool,
        parent_owns_cross_axis_legend_slabs: bool,
    ) -> Self {
        Self::from_axis_policy(
            parent_owns_main_axis_outer_slabs,
            parent_owns_main_axis_outer_slabs,
            parent_owns_cross_axis_legend_slabs,
        )
    }

    pub(crate) fn from_axis_policy(
        parent_owns_main_axis_start_slab: bool,
        parent_owns_main_axis_end_slab: bool,
        parent_owns_cross_axis_legend_slabs: bool,
    ) -> Self {
        Self {
            parent_owns_main_axis_start_slab,
            parent_owns_main_axis_end_slab,
            parent_owns_cross_axis_legend_slabs,
            realized_owned_legend_slabs: None,
        }
    }

    pub(crate) fn with_realized_owned_legend_slabs(self, owned_slabs: OwnedEdgeSlabs) -> Self {
        Self {
            realized_owned_legend_slabs: Some(owned_slabs),
            ..self
        }
    }

    pub(crate) fn without_realized_owned_legend_slabs(self) -> Self {
        Self {
            realized_owned_legend_slabs: None,
            ..self
        }
    }

    pub(crate) fn owned_legend_slabs(
        self,
        axis: FacetAxis,
        overflow: &CoordinatedOverflow,
        layout: &CoordinatedLayout,
    ) -> OwnedEdgeSlabs {
        if let Some(owned_slabs) = self.realized_owned_legend_slabs {
            return owned_slabs;
        }

        let slabs = FacetOverflowSlabs::from_coordinated(overflow);
        let mut owned = EdgeSlabs::default();

        match axis {
            FacetAxis::Column => {
                if self.parent_owns_main_axis_start_slab {
                    owned.left = slabs.legend.left.min(layout.outer_start.max(0.0));
                }
                if self.parent_owns_main_axis_end_slab {
                    owned.right = slabs.legend.right.min(layout.outer_end.max(0.0));
                }
            }
            FacetAxis::Row => {
                if self.parent_owns_main_axis_start_slab {
                    owned.top = slabs.legend.top.min(layout.outer_start.max(0.0));
                }
                if self.parent_owns_main_axis_end_slab {
                    owned.bottom = slabs.legend.bottom.min(layout.outer_end.max(0.0));
                }
            }
        }

        if self.parent_owns_cross_axis_legend_slabs {
            match axis {
                FacetAxis::Column => {
                    owned.top = slabs.legend.top;
                    owned.bottom = slabs.legend.bottom;
                }
                FacetAxis::Row => {
                    owned.left = slabs.legend.left;
                    owned.right = slabs.legend.right;
                }
            }
        }

        owned
    }
}

/// Measurement data for a facet band coordinate system.
///
/// This captures the computed padding from overflow measurement and precomputed
/// subplot measurements. Marks use the stored measurements directly without re-measuring.
pub struct FacetBandCoordMeasurement {
    /// Facet axis orientation for this measurement.
    pub axis: FacetAxis,
    /// Enumerated cell runtime state.
    pub(crate) cells: Vec<FacetCellRuntime>,
    /// Generic child-frame container path that owns this facet band.
    pub(crate) scope_path_prefix: Vec<ContainerPathSegment>,
    /// ScaleBuilder for plot scales (caches data extents for rebuilding with updated dimensions).
    /// Used with DynamicScaleProvider to correctly compute radius-aware domains.
    pub shared_scale_builder: ScaleBuilder,
    /// Stable field identity for coordination grouping at this facet level.
    /// This is typically the facet row or column field name.
    pub coordination_field_identity: String,
    /// Coordinated overflow values aggregated across ALL facets at this nesting level.
    /// Populated by facet coordination after local measurement.
    pub coordinated_overflow: CoordinatedOverflow,
    /// Coordinated sibling-boundary/allocation overflow with true chart-edge
    /// slabs removed.
    ///
    /// Explicit sibling gaps and parent-projected child frame allocations use
    /// this boundary contract so globally outer axes/titles do not become
    /// interior spacing.
    pub(crate) coordinated_boundary_overflow: Option<CoordinatedOverflow>,
    /// Coordinated facet guide anchor overflow for this visual lane.
    ///
    /// Unlike `coordinated_overflow`, this is scoped by the orthogonal lane so
    /// guides align within a row/column lane without borrowing hidden axis
    /// chrome from unrelated lanes.
    pub(crate) coordinated_guide_anchor_overflow: Option<CoordinatedOverflow>,
    /// Overflow measured from this facet band's rendered children before coordination.
    ///
    /// This is the stable "what this subtree actually renders" value. It must
    /// not be recomputed from child layouts after coordinated alignment slabs
    /// are projected into those child layouts.
    pub measured_overflow: Option<CoordinatedOverflow>,
    /// Reference to compiled subplot for retargeting after coordination.
    /// Used by retarget actions when coordinated layout changes child sizing.
    pub compiled_subplot: Arc<CompiledPlot>,
    /// Subplot cross-axis plot-area size for coordinated retargeting.
    pub subplot_cross_size: f32,
    /// Facet depth in the hierarchy (1 = outermost, 2 = nested, etc.)
    /// INVARIANT: facet_depth == full_cell_path.len() for any cell
    /// Used for channel-domain sharing comparison: sharing >= facet_depth means global.
    pub facet_depth: u8,
    /// Original facet band scale before local or coordinated layout rewrites.
    /// Used to recompute subplot size when coordinated layout differs from local layout.
    pub original_band_scale: ConfiguredScale,
    /// Local layout values (pre-coordination).
    pub local_layout: CoordinatedLayout,
    /// Interior padding required by axis and facet guides, excluding legend-only slabs.
    pub guide_padding_inner_px: f32,
    /// Coordinated layout values (post-coordination). None before coordination.
    pub coordinated_layout: Option<CoordinatedLayout>,
    /// Effective policy used when deciding whether empty cells are renderable.
    pub empty_cell_policy: FacetEmptyCellPolicy,
    /// Rendered legend slabs owned by this facet band's parent allocation.
    pub(crate) allocation_ownership: FacetBandAllocationOwnership,
    /// Placement model used to position this facet band's cells.
    pub(crate) placement_model: FacetBandPlacementModel,
}

impl FacetBandCoordMeasurement {
    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<ChildFrameScopeKey> {
        self.cells
            .get(child_index)
            .map(|cell| self.child_scope_key_for_cell(cell))
    }

    pub(crate) fn child_scope_key_for_cell(&self, cell: &FacetCellRuntime) -> ChildFrameScopeKey {
        ChildFrameScopeKey::new(
            self.scope_path_prefix.clone(),
            ChildFrameKey::FacetValue {
                axis: self.axis,
                level: self.facet_depth,
                value: cell.plan.value.clone(),
            },
        )
    }

    pub(crate) fn uses_explicit_placement(&self) -> bool {
        matches!(self.placement_model, FacetBandPlacementModel::Explicit(_))
    }

    pub(crate) fn owned_legend_slabs_for_overflow(
        &self,
        overflow: &CoordinatedOverflow,
        layout: &CoordinatedLayout,
    ) -> OwnedEdgeSlabs {
        self.allocation_ownership
            .owned_legend_slabs(self.axis, overflow, layout)
    }

    pub(crate) fn set_realized_owned_legend_slabs(&mut self, owned_slabs: OwnedEdgeSlabs) {
        self.allocation_ownership = self
            .allocation_ownership
            .with_realized_owned_legend_slabs(owned_slabs);
    }

    pub(crate) fn recompute_explicit_placement_if_needed(&mut self) {
        if self.uses_explicit_placement() {
            self.recompute_explicit_placement();
        }
    }

    pub(crate) fn recompute_explicit_placement(&mut self) {
        let active_layout = self
            .coordinated_layout
            .as_ref()
            .unwrap_or(&self.local_layout);
        let mut placement =
            compute_explicit_facet_band_placement(self.axis, &self.cells, active_layout);
        let slabs = FacetOverflowSlabs::from_coordinated(self.active_boundary_overflow());
        let cross_axis_start_offset = match self.axis {
            FacetAxis::Column => slabs.legend.top,
            FacetAxis::Row => slabs.legend.left,
        };
        placement.cross_axis_size += cross_axis_start_offset.max(0.0);
        self.placement_model = FacetBandPlacementModel::Explicit(placement);
    }

    pub(crate) fn preserve_empty_slot_plot_area_if_explicit(
        &mut self,
        plot_width: f32,
        plot_height: f32,
    ) {
        if !self.uses_explicit_placement() {
            return;
        }
        if !self.cells.is_empty() {
            return;
        }

        let (main_axis_size, cross_axis_size) = match self.axis {
            FacetAxis::Column => (plot_width, plot_height),
            FacetAxis::Row => (plot_height, plot_width),
        };
        self.placement_model = FacetBandPlacementModel::Explicit(FacetBandExplicitPlacement {
            main_axis_positions: Vec::new(),
            main_axis_size: main_axis_size.max(0.0),
            cross_axis_size: cross_axis_size.max(0.0),
        });
    }

    /// Return the realized plot-area extent for this facet subtree.
    pub(crate) fn plot_area_extent(&self) -> (f32, f32) {
        let FacetBandPlacementModel::Explicit(explicit_placement) = &self.placement_model else {
            return match self.axis {
                FacetAxis::Column => (self.subplot_cross_size, 0.0),
                FacetAxis::Row => (0.0, self.subplot_cross_size),
            };
        };
        match self.axis {
            FacetAxis::Column => (
                explicit_placement.main_axis_size,
                explicit_placement.cross_axis_size,
            ),
            FacetAxis::Row => (
                explicit_placement.cross_axis_size,
                explicit_placement.main_axis_size,
            ),
        }
    }

    pub(crate) fn resolved_placement_from_configured_scales(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Result<FacetBandPlacement, AvengerChartError> {
        match &self.placement_model {
            FacetBandPlacementModel::Explicit(explicit) => {
                FacetBandPlacement::from_explicit(self.axis, explicit, &self.cells)
            }
            FacetBandPlacementModel::ScaleBacked => {
                let configured = scales.get(self.axis.scale_name()).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing {} scale while resolving facet placement",
                        self.axis.scale_name()
                    ))
                })?;
                let cell_values = self.cell_values().cloned().collect::<Vec<_>>();
                resolve_scale_backed_facet_band_placement(
                    self.axis,
                    configured,
                    &cell_values,
                    self.cells.len(),
                    self.coordinated_subplot_cross_size(),
                )
            }
        }
    }

    pub(crate) fn resolved_placement_from_scale_specs(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Result<FacetBandPlacement, AvengerChartError> {
        match &self.placement_model {
            FacetBandPlacementModel::Explicit(explicit) => {
                FacetBandPlacement::from_explicit(self.axis, explicit, &self.cells)
            }
            FacetBandPlacementModel::ScaleBacked => {
                let configured = scales.get(self.axis.scale_name()).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing {} scale while resolving facet placement",
                        self.axis.scale_name()
                    ))
                })?;
                let cell_values = self.cell_values().cloned().collect::<Vec<_>>();
                resolve_scale_backed_facet_band_placement(
                    self.axis,
                    configured.configured(),
                    &cell_values,
                    self.cells.len(),
                    self.coordinated_subplot_cross_size(),
                )
            }
        }
    }
}

pub(crate) fn facet_band_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&FacetBandCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
}

pub(crate) fn facet_band_mut(
    coord_measurement: &mut dyn CoordMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
}

/// Probe-only facet band measurement used during estimated-overflow aggregation.
///
/// This carries enough local information for guide overflow measurement and band-scale
/// layout rewrites without materializing child `ComponentsMeasurement` trees.
#[derive(Debug, Clone)]
pub(crate) struct FacetBandProbeMeasurement {
    pub(crate) axis: FacetAxis,
    pub(crate) cell_values: Vec<ScalarValue>,
    pub(crate) cell_has_data_rows: Vec<bool>,
    pub(crate) measured_overflow: CoordinatedOverflow,
    pub(crate) original_band_scale: ConfiguredScale,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) fixed_plot_area_lock: bool,
    pub(crate) allocation_ownership: FacetBandAllocationOwnership,
}

impl FacetBandProbeMeasurement {
    pub(crate) fn cell_values(&self) -> impl Iterator<Item = &ScalarValue> {
        self.cell_values.iter()
    }

    #[cfg(test)]
    pub(crate) fn measured_overflow_value(&self) -> CoordinatedOverflow {
        self.measured_overflow.clone()
    }

    pub(crate) fn owned_legend_slabs_for_overflow(
        &self,
        overflow: &CoordinatedOverflow,
        layout: &CoordinatedLayout,
    ) -> OwnedEdgeSlabs {
        self.allocation_ownership
            .owned_legend_slabs(self.axis, overflow, layout)
    }
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

    pub fn measured_overflow_value(&self) -> Option<CoordinatedOverflow> {
        self.measured_overflow.clone()
    }

    #[cfg(test)]
    pub(crate) fn measured_rendered_subtree_overflow_value(&self) -> Option<CoordinatedOverflow> {
        resolve_facet_overflow(
            self,
            FacetOverflowResolutionPhase::Measurement,
            FacetOverflowPurpose::RenderedSubtree,
        )
        .map(|resolved| resolved.overflow)
    }

    pub(crate) fn measured_sibling_boundary_overflow_value(&self) -> Option<CoordinatedOverflow> {
        resolve_facet_overflow(
            self,
            FacetOverflowResolutionPhase::Measurement,
            FacetOverflowPurpose::SiblingBoundary { axis: self.axis },
        )
        .map(|resolved| resolved.overflow)
    }

    pub fn local_layout_value(&self) -> CoordinatedLayout {
        self.local_layout.clone()
    }

    pub fn set_coordinated_overflow_value(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_overflow = overflow;
        self.coordinated_guide_anchor_overflow = None;
        self.allocation_ownership = self
            .allocation_ownership
            .without_realized_owned_legend_slabs();
    }

    pub(crate) fn coordinated_boundary_overflow_value(&self) -> Option<&CoordinatedOverflow> {
        self.coordinated_boundary_overflow.as_ref()
    }

    pub(crate) fn coordinated_guide_anchor_overflow_value(&self) -> Option<&CoordinatedOverflow> {
        self.coordinated_guide_anchor_overflow.as_ref()
    }

    pub(crate) fn active_boundary_overflow(&self) -> &CoordinatedOverflow {
        self.coordinated_boundary_overflow
            .as_ref()
            .unwrap_or(&self.coordinated_overflow)
    }

    pub fn set_coordinated_boundary_overflow_value(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_boundary_overflow = Some(overflow);
        self.allocation_ownership = self
            .allocation_ownership
            .without_realized_owned_legend_slabs();
    }

    pub fn set_coordinated_guide_anchor_overflow_value(&mut self, overflow: CoordinatedOverflow) {
        self.coordinated_guide_anchor_overflow = Some(overflow);
    }

    pub fn set_coordinated_layout_value(&mut self, layout: CoordinatedLayout) {
        self.coordinated_layout = Some(layout);
        self.allocation_ownership = self
            .allocation_ownership
            .without_realized_owned_legend_slabs();
    }

    /// Realize the parent-owned coordinated cross-axis slabs on each child frame.
    ///
    /// Child layouts may be rebuilt during retargeting/refinement after global
    /// coordination. Re-apply the coordinated slabs at those realization
    /// boundaries so sibling child frames at the same facet level use the same
    /// frame allocation.
    pub(crate) fn realize_coordinated_child_frame_allocations(&mut self) {
        let coordinated = self.active_boundary_overflow().clone();
        let local_child_overflow = self
            .measured_overflow
            .clone()
            .unwrap_or_else(|| coordinated.clone());
        let sides = match self.axis {
            FacetAxis::Column => [AxisPosition::Top, AxisPosition::Bottom],
            FacetAxis::Row => [AxisPosition::Left, AxisPosition::Right],
        };

        for cell in &mut self.cells {
            let child_uses_parent_guide_slab =
                child_uses_parent_cross_axis_guide_slab(self.axis, &cell.measurement);
            for side in sides {
                let allocation_overflow = if child_uses_parent_guide_slab {
                    &coordinated
                } else {
                    &local_child_overflow
                };
                apply_measurement_side_slab(
                    &mut cell.measurement,
                    side,
                    overflow_side_value(&allocation_overflow.guide, side),
                    overflow_side_value(&allocation_overflow.total, side),
                );
            }
            sync_measurement_owned_slabs_from_coord(&mut cell.measurement);
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
                "FacetBand set_parent_bandwidth updated original band scale range"
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
        self.realize_coordinated_child_frame_allocations();
        self.recompute_explicit_placement_if_needed();

        Ok(())
    }

    pub(crate) fn retarget_parent_plot_area_policy_no_remeasure(
        &mut self,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
        eval_ctx: &EvaluationContext,
        new_plot_area_width: f32,
        new_plot_area_height: f32,
    ) -> Result<(), AvengerChartError> {
        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        if policy
            .facet_band_dimension(self.axis)
            .is_canvas_constrained()
        {
            let parent_main_size = match self.axis {
                FacetAxis::Column => new_plot_area_width,
                FacetAxis::Row => new_plot_area_height,
            }
            .max(1.0);
            self.original_band_scale = self
                .original_band_scale
                .clone()
                .with_range_interval((0.0, parent_main_size));
        }
        self.apply_scale_adjustments(scales);

        if policy
            .facet_band_dimension(self.axis)
            .is_canvas_constrained()
        {
            let band_scale = scales.get(self.axis.scale_name()).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "FacetBand policy retarget missing {} scale",
                    self.axis.scale_name()
                ))
            })?;
            self.subplot_cross_size = bandwidth(&band_scale.configured().config).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to get policy retargeted facet bandwidth: {}",
                    e
                ))
            })?;
        }

        let (legend_start, legend_end) =
            legend_axis_overflow(self.axis, &self.coordinated_overflow);
        let orthogonal_canvas_constrained = policy
            .facet_orthogonal_dimension(self.axis)
            .is_canvas_constrained();
        let orthogonal_size = |size: f32| {
            if orthogonal_canvas_constrained {
                adjusted_size_for_legend_overflow(size.max(1.0), legend_start, legend_end)
            } else {
                size.max(1.0)
            }
        };

        let compiled_subplot = self.compiled_subplot.clone();
        for cell in &mut self.cells {
            let mut target_plot_area_width = cell.measurement.plot_area_width;
            let mut target_plot_area_height = cell.measurement.plot_area_height;
            match self.axis {
                FacetAxis::Column => {
                    if policy.width.is_canvas_constrained() {
                        target_plot_area_width = self.subplot_cross_size;
                    }
                    if policy.height.is_canvas_constrained() {
                        target_plot_area_height = orthogonal_size(new_plot_area_height);
                    }
                }
                FacetAxis::Row => {
                    if policy.width.is_canvas_constrained() {
                        target_plot_area_width = orthogonal_size(new_plot_area_width);
                    }
                    if policy.height.is_canvas_constrained() {
                        target_plot_area_height = self.subplot_cross_size;
                    }
                }
            }
            retarget_measurement_plot_area_policy_no_remeasure(
                &mut cell.measurement,
                compiled_subplot.as_ref(),
                eval_ctx,
                &cell.plan.full_path,
                target_plot_area_width,
                target_plot_area_height,
            )?;
        }
        self.realize_coordinated_child_frame_allocations();
        self.recompute_explicit_placement_if_needed();

        Ok(())
    }
}

fn measured_overflow_from_cells(
    axis: FacetAxis,
    cells: &[FacetCellRuntime],
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Option<CoordinatedOverflow> {
    let overflow_inputs = cells
        .iter()
        .map(|cell| {
            let cell_overflow = parent_cell_overflow_summary(&cell.measurement);
            FacetCellOverflowInput {
                renderable: renderable_for_empty_policy(
                    empty_cell_policy,
                    !cell.plan.has_data_rows,
                ),
                guide: cell_overflow.guide_overflow,
                total: cell_overflow.total_overflow,
            }
        })
        .collect::<Vec<_>>();
    aggregate_facet_band_overflow(axis, &overflow_inputs)
}

fn parent_cell_overflow_summary(measurement: &ComponentsMeasurement) -> FacetCellProbeSummary {
    let mut guide_overflow = measurement.layout.overflow.clone();
    let mut total_overflow = measurement.layout.total_overflow.clone();
    let boundary_profiles = boundary_profiles_for_measurement(measurement);
    let mut max_child_padding = 0.0f32;

    if let Some(facet_band) = facet_band_ref(measurement.coord_measurement.as_ref()) {
        max_child_padding = facet_band.active_layout().padding_inner_px;
        if let Some(boundary) = facet_band.measured_sibling_boundary_overflow_value() {
            guide_overflow = guide_overflow.max_components(&boundary.guide);
            total_overflow = total_overflow.max_components(&boundary.total);
        }
    }

    FacetCellProbeSummary {
        guide_overflow,
        total_overflow,
        boundary_profiles,
        max_child_padding,
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
        ScaleLayoutRewriteMode::Render {
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
}

impl FacetBandCoordMeasurement {
    pub(crate) fn apply_scale_adjustments(
        &self,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    ) {
        if self.uses_explicit_placement() {
            return;
        }

        // Apply facet band layout adjustments at render-time:
        // - band-scale domain override from cell_values
        // - coordinated padding/outer edges
        // - band_n override when coordinated layout expects more slots than the local slot set
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

impl CoordMeasurement for FacetBandProbeMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl FacetBandProbeMeasurement {
    pub(crate) fn apply_scale_adjustments(
        &self,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    ) {
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
    Measurement {
        side_specific_outer_edges: bool,
    },
    Render {
        allow_zero_padding_override: bool,
        side_specific_outer_edges: bool,
    },
    Retarget {
        side_specific_outer_edges: bool,
    },
}

impl ScaleLayoutRewriteMode {
    #[inline]
    fn set_padding_always(self) -> bool {
        matches!(
            self,
            ScaleLayoutRewriteMode::Measurement { .. } | ScaleLayoutRewriteMode::Retarget { .. }
        )
    }

    #[inline]
    fn allow_zero_padding_override(self) -> bool {
        match self {
            ScaleLayoutRewriteMode::Render {
                allow_zero_padding_override,
                ..
            } => allow_zero_padding_override,
            _ => false,
        }
    }

    #[inline]
    fn side_specific_outer_edges(self) -> bool {
        match self {
            ScaleLayoutRewriteMode::Measurement {
                side_specific_outer_edges,
            }
            | ScaleLayoutRewriteMode::Render {
                side_specific_outer_edges,
                ..
            }
            | ScaleLayoutRewriteMode::Retarget {
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

    if let Some(domain_values) = domain_override
        && !domain_values.is_empty()
        && let Ok(domain_array) = ScalarValue::iter_to_array(domain_values.iter().cloned())
    {
        updated = updated.with_domain(domain_array);
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

fn legend_axis_overflow(axis: FacetAxis, coordinated: &CoordinatedOverflow) -> (f32, f32) {
    let slabs = FacetOverflowSlabs::from_coordinated(coordinated);
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

fn collect_channel_domain_sharing_levels_from_infos(
    infos: &[CellDomainInfo],
) -> HashMap<String, SharingLevel> {
    let mut sharing_levels = HashMap::new();
    for info in infos {
        let sharing_level = SharingLevel::from_raw(info.domain_sharing_level);
        sharing_levels
            .entry(info.channel.clone())
            .and_modify(|existing: &mut SharingLevel| *existing = (*existing).max(sharing_level))
            .or_insert(sharing_level);
    }
    sharing_levels
}

fn is_leaf_subplot(compiled_subplot: &CompiledPlot) -> bool {
    !compiled_subplot
        .marks
        .iter()
        .any(|mark| facet_subplot_ref(mark.as_ref()).is_some())
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
    pub(crate) fn coordination_scope_key_for_depth(&self, depth: usize) -> CoordinationScopeKey {
        CoordinationScopeKey::container_group(
            CoordinationKind::ChildSize,
            depth,
            format!(
                "{}:{}",
                self.axis.coordination_key_prefix(),
                self.coordination_field_identity
            ),
        )
    }

    pub fn guide_padding_inner_px_value(&self) -> f32 {
        self.guide_padding_inner_px
    }

    fn require_coordinated_layout_subplot_cross_size(
        &self,
        node_id: &CoordinationNodeKey,
    ) -> Result<f32, AvengerChartError> {
        let layout = self.coordinated_layout.as_ref().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Facet retarget requirements for node path {:?} need coordinated layout, but none is present",
                node_id.path
            ))
        })?;
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
            ScaleLayoutRewriteMode::Retarget {
                side_specific_outer_edges: true,
            },
        );
        bandwidth(&scale.config).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to get coordinated bandwidth for node path {:?}: {}",
                node_id.path, e
            ))
        })
    }

    pub(crate) fn derive_retarget_requirements(
        &self,
        node_id: CoordinationNodeKey,
    ) -> Result<RetargetNodeRequirements, AvengerChartError> {
        let (legend_start, legend_end) =
            legend_axis_overflow(self.axis, &self.coordinated_overflow);
        let legend_main_axis_slab = AxisSlab {
            start: legend_start,
            end: legend_end,
        };
        let has_legend_overflow = legend_main_axis_slab.has_slab();
        let layout_changed =
            has_coordinated_layout_change(&self.local_layout, self.coordinated_layout.as_ref());
        let policy = resolve_facet_ownership_policy(
            self.empty_cell_policy,
            has_holes_from_cells(self.cells.iter().map(|cell| cell.plan.is_empty)),
        );
        let child_plot_areas = self
            .cells
            .iter()
            .map(|cell| {
                PlotAreaSize::new(
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                )
            })
            .collect::<Vec<_>>();
        let target_subplot_cross_size = if layout_changed {
            self.require_coordinated_layout_subplot_cross_size(&node_id)?
        } else {
            self.subplot_cross_size
        };

        Ok(RetargetNodeRequirements {
            node_id,
            axis: self.axis,
            child_count: self.cells.len(),
            coordinated_overflow: self.coordinated_overflow.clone(),
            coordinated_layout: self.coordinated_layout.clone(),
            layout_changed,
            legend_main_axis_slab,
            has_legend_overflow,
            ownership: FacetOwnershipRequirement {
                has_holes: policy.has_holes,
                axis_owner_ignore_empty_cells: policy.axis_owner_ignore_empty_cells,
            },
            child_plot_areas,
            target_subplot_cross_size,
        })
    }

    fn apply_coordinated_layout_cross_size(&mut self) -> Result<(), AvengerChartError> {
        let layout = self.coordinated_layout.as_ref().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Facet retarget action requested coordinated layout for {} facet at depth {}, but no coordinated layout is present",
                self.axis.scale_name(),
                self.facet_depth
            ))
        })?;
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
            ScaleLayoutRewriteMode::Retarget {
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
            "FacetBand apply_retarget_actions layout coordination"
        );

        self.subplot_cross_size = new_subplot_cross_size;
        Ok(())
    }

    pub(crate) async fn apply_retarget_actions(
        &mut self,
        eval_ctx: &EvaluationContext,
        actions: &RetargetNodeActions,
    ) -> Result<RetargetNodeOutcome, AvengerChartError> {
        if actions.axis != self.axis {
            return Err(AvengerChartError::InternalError(format!(
                "Retarget action axis mismatch: planned {:?}, actual {:?}",
                actions.axis, self.axis
            )));
        }

        if actions.child_actions.len() != self.cells.len() {
            return Err(AvengerChartError::InternalError(format!(
                "Retarget action child count mismatch for node path {:?}: planned child_actions={}, actual_children={}",
                actions.node_id.path,
                actions.child_actions.len(),
                self.cells.len()
            )));
        }

        let subplot_cross_size_before = self.subplot_cross_size;
        let legend_slabs = FacetOverflowSlabs::from_coordinated(&self.coordinated_overflow);

        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            coordination_field = %self.coordination_field_identity,
            legend_start = legend_axis_overflow(self.axis, &self.coordinated_overflow).0,
            legend_end = legend_axis_overflow(self.axis, &self.coordinated_overflow).1,
            legend_top = legend_slabs.legend.top,
            legend_right = legend_slabs.legend.right,
            legend_bottom = legend_slabs.legend.bottom,
            legend_left = legend_slabs.legend.left,
            "FacetBand apply_retarget_actions legend slab sides"
        );

        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            coordination_field = %self.coordination_field_identity,
            band_action = ?actions.band_action,
            child_action_count = actions.child_actions.len(),
            coordinated_guide_top = self.coordinated_overflow.guide.top,
            coordinated_guide_right = self.coordinated_overflow.guide.right,
            coordinated_guide_bottom = self.coordinated_overflow.guide.bottom,
            coordinated_guide_left = self.coordinated_overflow.guide.left,
            coordinated_total_top = self.coordinated_overflow.total.top,
            coordinated_total_right = self.coordinated_overflow.total.right,
            coordinated_total_bottom = self.coordinated_overflow.total.bottom,
            coordinated_total_left = self.coordinated_overflow.total.left,
            "FacetBand apply_retarget_actions coordinated inputs"
        );

        let band_layout_applied = matches!(
            actions.band_action,
            BandRetargetAction::ApplyCoordinatedLayout
        );
        if band_layout_applied {
            self.apply_coordinated_layout_cross_size()?;
        }

        debug!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            plot_area_action_count = actions.child_actions.iter().filter(|action| action.retargets_plot_area()).count(),
            "FacetBand apply_retarget_actions executing child actions"
        );

        let compiled_subplot = self.compiled_subplot.clone();
        let mut plot_area_retarget_count = 0usize;
        let mut width_retarget_count = 0usize;
        let mut height_retarget_count = 0usize;
        for idx in 0..self.cells.len() {
            let action = &actions.child_actions[idx];
            let cell = &mut self.cells[idx];
            let current_plot_area = PlotAreaSize::new(
                cell.measurement.plot_area_width,
                cell.measurement.plot_area_height,
            );
            if let Some(target) = action.plot_area_target {
                width_retarget_count += usize::from(target.width.is_some());
                height_retarget_count += usize::from(target.height.is_some());
                let PlotAreaTarget { width, height } = target;
                let target = PlotAreaTarget { width, height }.resolve(current_plot_area);
                retarget_measurement_plot_area_no_remeasure(
                    &mut cell.measurement,
                    compiled_subplot.as_ref(),
                    eval_ctx,
                    &cell.plan.full_path,
                    target.width,
                    target.height,
                )?;
                plot_area_retarget_count += 1;
            }
        }
        self.realize_coordinated_child_frame_allocations();
        trace!(
            axis = ?self.axis,
            facet_depth = self.facet_depth,
            cell_count = self.cells.len(),
            plot_area_retarget_count,
            width_retarget_count,
            height_retarget_count,
            "FacetBand apply_retarget_actions retargeted cells without remeasurement"
        );

        Ok(RetargetNodeOutcome {
            subplot_cross_size_before,
            subplot_cross_size_after: self.subplot_cross_size,
            band_layout_applied,
            plot_area_retarget_count,
            width_retarget_count,
            height_retarget_count,
        })
    }
}

/// Scale selection strategy for measuring a facet cell.
enum FacetCellMeasurementMode<'a> {
    /// Use child facet slot-sharing rules (Free/Level(N)/Shared) for nested facet cells.
    ChildFacetSlots {
        child_facet_slot_sharing: Option<SharingLevel>,
        child_facet_depth: u8,
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
}

struct FacetCellDraft {
    plan: FacetCellPlan,
    data_override: DataFrame,
    measurement: Option<ComponentsMeasurement>,
    local_domain_extents: HashMap<String, ChannelDomainExtent>,
    coordinated_domain_extents: HashMap<String, DomainExtent>,
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

/// Planning data prepared once before running the facet-band measurement pipeline.
struct FacetBandMeasurePlan {
    cell_values: Vec<ScalarValue>,
    cells: Vec<FacetCellDraft>,
    scale_artifacts: Arc<FacetScaleNodeArtifacts>,
}

/// Shared nested-facet measurement context used by overflow probes and retargeted layout.
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
    estimated_overflow_leaf_measure_count: usize,
    estimated_overflow_non_leaf_aggregate_count: usize,
    estimated_overflow_non_leaf_full_measure_count: usize,
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

fn apply_measurement_side_slab(
    measurement: &mut ComponentsMeasurement,
    side: AxisPosition,
    guide: f32,
    total: f32,
) {
    apply_frame_side_slab(&mut measurement.layout, side, guide, total);
    measurement.sync_canvas_size_from_layout();
}

fn child_uses_parent_cross_axis_guide_slab(
    parent_axis: FacetAxis,
    child: &ComponentsMeasurement,
) -> bool {
    facet_band_ref(child.coord_measurement.as_ref())
        .is_some_and(|child_facet_band| child_facet_band.axis == parent_axis)
}

fn refresh_measurement_frame_allocation_rect(measurement: &mut ComponentsMeasurement) {
    measurement.refresh_frame_allocation_rect();
}

fn sync_measurement_owned_slabs_from_coord(measurement: &mut ComponentsMeasurement) {
    let owned_slabs = if let Some(facet_band) =
        facet_band_ref(measurement.coord_measurement.as_ref())
    {
        let active_layout = facet_band.active_layout();
        facet_band.owned_legend_slabs_for_overflow(&facet_band.coordinated_overflow, active_layout)
    } else {
        EdgeSlabs::default()
    };

    measurement.frame_allocation.owned_slabs = owned_slabs;
    if let Some(facet_band) = facet_band_mut(measurement.coord_measurement.as_mut()) {
        facet_band.set_realized_owned_legend_slabs(owned_slabs);
    }
}

fn facet_direction_to_axis(direction: FacetDirection) -> FacetAxis {
    match direction {
        FacetDirection::Row => FacetAxis::Row,
        FacetDirection::Column => FacetAxis::Column,
    }
}

fn facet_scope_path_prefix(
    base_path: &[ContainerPathSegment],
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
) -> Result<Vec<ContainerPathSegment>, AvengerChartError> {
    if facet_path.is_empty() {
        return Ok(base_path.to_vec());
    }

    let resolved = facet_tree.resolve_path_info(facet_path).ok_or_else(|| {
        AvengerChartError::InternalError(format!(
            "Facet scope path prefix requested for unresolved facet path {facet_path:?}"
        ))
    })?;

    if resolved.level_directions.len() < facet_path.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet scope path {facet_path:?} has {} directions for {} values",
            resolved.level_directions.len(),
            facet_path.len()
        )));
    }

    let facet_path_segments = facet_path
        .iter()
        .enumerate()
        .map(|(level_index, value)| {
            ContainerPathSegment::facet_value(
                facet_direction_to_axis(resolved.level_directions[level_index]),
                (level_index + 1) as u8,
                value.clone(),
            )
        })
        .collect::<Vec<_>>();

    // Recursive facet measurement carries ancestor cells in the generic
    // evaluation context. Direct tests and older measurement entry points may
    // only provide the value path, so keep both forms equivalent without
    // encoding the same ancestor twice.
    if base_path.ends_with(&facet_path_segments) {
        return Ok(base_path.to_vec());
    }

    let mut prefix = base_path.to_vec();
    prefix.extend(facet_path_segments);
    Ok(prefix)
}

fn current_facet_path_segment(
    axis: FacetAxis,
    facet_depth: u8,
    value: ScalarValue,
) -> ContainerPathSegment {
    ContainerPathSegment::facet_value(axis, facet_depth, value)
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
    refresh_measurement_frame_allocation_rect(measurement);
    retarget_frame_layout_for_plot_area(
        &mut measurement.layout,
        &measurement.legend_plan.measurements,
        new_plot_area_width,
        new_plot_area_height,
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
    sync_measurement_owned_slabs_from_coord(measurement);
}

fn update_measurement_plot_area_metadata_for_policy(
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
    let policy = eval_ctx.facet_runtime_sizing_mode().policy();
    let is_root = facet_path.is_empty();
    let canvas_width_delta = if is_root && policy.width.is_canvas_constrained() {
        0.0
    } else {
        width_delta
    };
    let canvas_height_delta = if is_root && policy.height.is_canvas_constrained() {
        0.0
    } else {
        height_delta
    };

    measurement.plot_area_width = new_plot_area_width;
    measurement.plot_area_height = new_plot_area_height;
    measurement.canvas_size.0 = (measurement.canvas_size.0 + canvas_width_delta).max(1.0);
    measurement.canvas_size.1 = (measurement.canvas_size.1 + canvas_height_delta).max(1.0);
    measurement.layout.canvas_size = measurement.canvas_size;
    refresh_measurement_frame_allocation_rect(measurement);
    retarget_frame_layout_for_plot_area(
        &mut measurement.layout,
        &measurement.legend_plan.measurements,
        new_plot_area_width,
        new_plot_area_height,
    );

    if !is_root || policy.width.is_leaf_plot_area_sized() {
        measurement.params.insert(
            "width".to_string(),
            ScalarValue::Float32(Some(new_plot_area_width)),
        );
    }
    if !is_root || policy.height.is_leaf_plot_area_sized() {
        measurement.params.insert(
            "height".to_string(),
            ScalarValue::Float32(Some(new_plot_area_height)),
        );
    }

    measurement.clip = compiled_plot.resolved_clip_region(
        eval_ctx,
        facet_path,
        &measurement.scales,
        new_plot_area_width,
        new_plot_area_height,
    );
    sync_measurement_owned_slabs_from_coord(measurement);
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

    if let Some(facet_band) = facet_band_mut(measurement.coord_measurement.as_mut()) {
        if facet_band.uses_explicit_placement() {
            facet_band.recompute_explicit_placement_if_needed();
        } else {
            facet_band.retarget_parent_plot_area_no_remeasure(
                &mut measurement.scales,
                eval_ctx,
                new_plot_area_width,
                new_plot_area_height,
            )?;
        }
    } else {
        crate::coords::apply_coord_measurement_scale_adjustments(
            measurement.coord_measurement.as_ref(),
            &mut measurement.scales,
        );
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

pub(crate) fn retarget_measurement_plot_area_policy_no_remeasure(
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

    if let Some(facet_band) = facet_band_mut(measurement.coord_measurement.as_mut()) {
        facet_band.retarget_parent_plot_area_policy_no_remeasure(
            &mut measurement.scales,
            eval_ctx,
            new_plot_area_width,
            new_plot_area_height,
        )?;
    } else {
        crate::coords::apply_coord_measurement_scale_adjustments(
            measurement.coord_measurement.as_ref(),
            &mut measurement.scales,
        );
    }

    update_measurement_plot_area_metadata_for_policy(
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

fn derive_padding_inner_px_from_probe(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
) -> f32 {
    derive_padding_inner_px_from_probe_overflows(axis, pass1, pass1_renderable_cells, true)
}

fn derive_guide_padding_inner_px_from_probe(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
) -> f32 {
    derive_padding_inner_px_from_probe_overflows(axis, pass1, pass1_renderable_cells, false)
}

fn derive_padding_inner_px_from_probe_overflows(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
    include_total_overflow: bool,
) -> f32 {
    let boundary_profiles = pass1
        .cell_probe_summaries
        .iter()
        .map(|summary| summary.boundary_profiles.clone())
        .collect::<Vec<_>>();
    let required_parent_padding = compute_padding_from_boundary_profiles(
        axis,
        &boundary_profiles,
        pass1_renderable_cells,
        include_total_overflow,
    )
    .unwrap_or_else(|| {
        derive_padding_inner_px_from_probe_overflow_slabs(
            axis,
            pass1,
            pass1_renderable_cells,
            include_total_overflow,
        )
    });
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

fn derive_padding_inner_px_from_probe_overflow_slabs(
    axis: FacetAxis,
    pass1: &OverflowProbeSummary,
    pass1_renderable_cells: &[bool],
    include_total_overflow: bool,
) -> f32 {
    let cell_overflows = pass1
        .cell_overflows
        .iter()
        .map(|(guide, total)| {
            if include_total_overflow {
                total.clone()
            } else {
                guide.clone()
            }
        })
        .collect::<Vec<_>>();
    compute_padding_from_overflows(axis, &cell_overflows, pass1_renderable_cells)
}

#[cfg(test)]
pub(crate) fn derive_band_overflow_from_probe(
    axis: FacetAxis,
    renderable_mask: &[bool],
    cell_overflows: &[(OverflowSpaceRequirement, OverflowSpaceRequirement)],
) -> CoordinatedOverflow {
    let use_renderable_mask = renderable_mask.len() == cell_overflows.len();
    let overflow_inputs = cell_overflows
        .iter()
        .enumerate()
        .map(|(idx, (guide, total))| FacetCellOverflowInput {
            renderable: !use_renderable_mask || renderable_mask[idx],
            guide: guide.clone(),
            total: total.clone(),
        })
        .collect::<Vec<_>>();

    aggregate_facet_band_overflow_with_policy(
        axis,
        &overflow_inputs,
        FacetBandNoRenderablePolicy::FirstCell,
    )
    .unwrap_or_default()
}

pub(crate) fn renderable_for_empty_policy(policy: FacetEmptyCellPolicy, is_empty: bool) -> bool {
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
    plot_area_sized_mode: bool,
    orthogonal_dimension_canvas_constrained: bool,
) -> Box<dyn CoordMeasurement> {
    let base = FacetBandCoordMeasurement {
        axis,
        cells: Vec::new(),
        scope_path_prefix: Vec::new(),
        shared_scale_builder: ScaleBuilder::default(),
        coordinated_overflow: CoordinatedOverflow::default(),
        coordinated_boundary_overflow: None,
        coordinated_guide_anchor_overflow: None,
        measured_overflow: None,
        compiled_subplot: compiled_subplot.clone(),
        subplot_cross_size: 0.0,
        facet_depth: facet_path.len() as u8 + 1,
        original_band_scale: band_scale.configured().clone(),
        local_layout: CoordinatedLayout::default(),
        guide_padding_inner_px: 0.0,
        coordinated_layout: None,
        coordination_field_identity: axis.scale_name().to_string(),
        empty_cell_policy,
        allocation_ownership: FacetBandAllocationOwnership::from_policy(
            !facet_path.is_empty() || plot_area_sized_mode,
            orthogonal_dimension_canvas_constrained,
        ),
        placement_model: if plot_area_sized_mode {
            FacetBandPlacementModel::Explicit(FacetBandExplicitPlacement::default())
        } else {
            FacetBandPlacementModel::ScaleBacked
        },
    };
    Box::new(base)
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

fn resolve_nested_scale_plan<'a>(
    cell: &FacetCellPlan,
    child_facet_slot_sharing: Option<SharingLevel>,
    child_facet_depth: u8,
    ancestor_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    per_cell_scale_builder_cache: &'a HashMap<Vec<ScalarValue>, ScaleBuilder>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    data_df: &DataFrame,
) -> Result<NestedScalePlan<'a>, AvengerChartError> {
    if !cell.in_domain_slot {
        return Ok(NestedScalePlan::EmptySharedNoData);
    }

    match child_facet_slot_sharing {
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
                    "Facet child-slot measurement missing per-cell cached builder; falling back to on-demand build"
                );
                Ok(NestedScalePlan::PerCellBuilderFallback)
            }
        }
        Some(sharing_level) if sharing_level < child_facet_depth => {
            let ancestor_key = path_math::child_facet_slot_ancestor_key(
                &cell.full_path,
                sharing_level,
                child_facet_depth,
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
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    match plan {
        NestedScalePlan::EmptySharedNoData => measure_child_frame_plot_with_builder(
            compiled_subplot,
            subplot_eval_ctx,
            subplot_layout_spec,
            shared_scale_builder,
            None,
            &cell.full_path,
            &[coordinated_domain_extents],
        )
        .await
        .map(|measurement| MeasuredFacetCell { measurement }),
        NestedScalePlan::PerCellCachedBuilder { cached_builder } => {
            measure_child_frame_plot_with_builder(
                compiled_subplot,
                subplot_eval_ctx,
                subplot_layout_spec,
                cached_builder,
                Some(data_override),
                &cell.full_path,
                &[coordinated_domain_extents],
            )
            .await
            .map(|measurement| MeasuredFacetCell { measurement })
        }
        NestedScalePlan::PerCellBuilderFallback => {
            let cell_scale_builder = build_scale_builder_from_marks(
                &compiled_subplot.marks,
                &compiled_subplot.scale_specs,
                &compiled_subplot.coord_transform,
                &compiled_subplot.data,
                Some(data_override.clone()),
                eval_ctx,
                compiled_subplot.get_theme().as_ref(),
            )
            .await?;
            let measurement = measure_child_frame_plot_with_builder(
                compiled_subplot,
                subplot_eval_ctx,
                subplot_layout_spec,
                &cell_scale_builder,
                Some(data_override),
                &cell.full_path,
                &[coordinated_domain_extents],
            )
            .await?;

            Ok(MeasuredFacetCell { measurement })
        }
        NestedScalePlan::AncestorCachedBuilder {
            cached_builder,
            ancestor_filtered_df,
        } => measure_child_frame_plot_with_builder(
            compiled_subplot,
            subplot_eval_ctx,
            subplot_layout_spec,
            cached_builder,
            Some(&ancestor_filtered_df),
            &cell.full_path,
            &[coordinated_domain_extents],
        )
        .await
        .map(|measurement| MeasuredFacetCell { measurement }),
        NestedScalePlan::Shared => measure_child_frame_plot_with_builder(
            compiled_subplot,
            subplot_eval_ctx,
            subplot_layout_spec,
            shared_scale_builder,
            Some(data_override),
            &cell.full_path,
            &[coordinated_domain_extents],
        )
        .await
        .map(|measurement| MeasuredFacetCell { measurement }),
    }
}

/// Measure a single facet cell using a selected scale strategy.
///
/// This is the shared measurement engine used by estimated-overflow probes,
/// local layout finalization, and coordinated refinement.
async fn measure_facet_cell(
    cell: &FacetCellPlan,
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    subplot_layout_spec: &EvaluatedLayoutSpec,
    mode: FacetCellMeasurementMode<'_>,
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    match mode {
        FacetCellMeasurementMode::ChildFacetSlots {
            child_facet_slot_sharing,
            child_facet_depth,
            ancestor_scale_builder_cache,
            per_cell_scale_builder_cache,
            shared_scale_builder,
            facet_tree,
            data_df,
            eval_ctx,
        } => {
            let plan = resolve_nested_scale_plan(
                cell,
                child_facet_slot_sharing,
                child_facet_depth,
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
                coordinated_domain_extents,
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
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<MeasuredFacetCell, AvengerChartError> {
    let subplot_layout_spec =
        fixed_child_plot_area_layout_spec(subplot_plot_width, subplot_plot_height);
    let mode = FacetCellMeasurementMode::ChildFacetSlots {
        child_facet_slot_sharing: nested_ctx.scale_artifacts.child_facet_slot_sharing,
        child_facet_depth: nested_ctx.scale_artifacts.child_facet_depth,
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
        coordinated_domain_extents,
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
                .child_facet_slot_sharing
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
                coordinated_domain_extents: HashMap::new(),
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
    prepared_inputs: &FacetBandPreparedInputs,
    runtime_state: &FacetBandPreparedRuntime,
) -> Vec<FacetCellDraft> {
    assert_eq!(
        prepared_inputs.cell_semantics.cells.len(),
        runtime_state.data_overrides.len(),
        "FacetBand prepared inputs/executor invariant violated: mismatched cell/data_override lengths"
    );

    prepared_inputs
        .cell_semantics
        .cells
        .iter()
        .zip(runtime_state.data_overrides.iter())
        .map(|(cell, data_override)| FacetCellDraft {
            plan: FacetCellPlan::from(cell),
            data_override: data_override.clone(),
            measurement: None,
            local_domain_extents: HashMap::new(),
            coordinated_domain_extents: HashMap::new(),
        })
        .collect()
}

async fn prepare_band_inputs_and_runtime(
    cell_semantics: FacetBandSemantics,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    band_scale: &ConfiguredScaleWithSpec,
    initial_subplot_band_size: f32,
    eval_ctx: &EvaluationContext,
) -> Result<(FacetBandPreparedInputs, FacetBandPreparedRuntime), AvengerChartError> {
    let cell_plans: Vec<FacetCellPlan> = cell_semantics
        .cells
        .iter()
        .map(FacetCellPlan::from)
        .collect();
    let measurement_inputs = prepare_measurement_inputs(
        cell_plans,
        &cell_semantics.node_id.facet_path,
        data_df,
        compiled_subplot,
        cell_semantics.empty_cell_policy,
        eval_ctx,
    )
    .await?;

    let renderable_mask = renderable_mask_for_cells(
        &measurement_inputs.plan.cells,
        cell_semantics.empty_cell_policy,
    );
    let data_overrides = measurement_inputs
        .plan
        .cells
        .iter()
        .map(|cell| cell.data_override.clone())
        .collect::<Vec<_>>();
    let scale_artifacts = measurement_inputs.plan.scale_artifacts.clone();

    let prepared_inputs = FacetBandPreparedInputs {
        cell_semantics: cell_semantics.clone(),
        renderable_mask,
        scale_artifacts_key: FacetScaleNodeKey::new(
            compiled_subplot,
            &cell_semantics.node_id.facet_path,
        ),
    };
    let runtime_state = FacetBandPreparedRuntime {
        data_overrides,
        subplot_eval_ctx: measurement_inputs.subplot_eval_ctx,
        nested_measure_ctx: measurement_inputs.nested_measure_ctx,
        compiled_subplot: compiled_subplot.clone(),
        original_band_scale: band_scale.configured().clone(),
        initial_subplot_band_size,
        scale_artifacts,
    };
    Ok((prepared_inputs, runtime_state))
}

async fn build_overflow_probe(
    prepared_inputs: &FacetBandPreparedInputs,
    runtime_state: &FacetBandPreparedRuntime,
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    perf_counters: &mut FacetPipelinePerfCounters,
) -> Result<(FacetBandOverflowProbe, FacetBandOverflowRuntime), AvengerChartError> {
    assert_eq!(
        prepared_inputs.renderable_mask.len(),
        prepared_inputs.cell_semantics.cells.len(),
        "FacetBand prepared inputs invariant violated: renderable mask length mismatch"
    );
    let expected_key = FacetScaleNodeKey::new(
        &runtime_state.compiled_subplot,
        &prepared_inputs.cell_semantics.node_id.facet_path,
    );
    assert_eq!(
        prepared_inputs.scale_artifacts_key, expected_key,
        "FacetBand prepared inputs invariant violated: scale artifacts key mismatch"
    );

    let mut cells = prepared_cells_as_drafts(prepared_inputs, runtime_state);
    coordinate_cell_domains_before_measurement(
        &mut cells,
        prepared_inputs.cell_semantics.facet_depth,
        &runtime_state.compiled_subplot,
        &runtime_state.nested_measure_ctx,
    )
    .await?;
    let overflow_probe_summary = measure_overflow_probe(
        prepared_inputs.cell_semantics.node_id.axis,
        prepared_inputs.cell_semantics.facet_depth,
        &mut cells,
        subplot_plot_width,
        subplot_plot_height,
        &runtime_state.compiled_subplot,
        &runtime_state.subplot_eval_ctx,
        &runtime_state.nested_measure_ctx,
        prepared_inputs.cell_semantics.empty_cell_policy,
        is_leaf_subplot(&runtime_state.compiled_subplot),
        perf_counters,
    )
    .await?;

    Ok((
        FacetBandOverflowProbe {
            prepared_inputs: prepared_inputs.clone(),
            overflow_probe_summary,
        },
        FacetBandOverflowRuntime { cells },
    ))
}

async fn build_extent_builder_for_cell(
    cell: &FacetCellDraft,
    data_override: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    nested_ctx: &FacetBandNestedMeasureContext,
) -> Result<ScaleBuilder, AvengerChartError> {
    let canonical_full_path = canonicalize_path(&cell.plan.full_path);
    if let Some(cached_builder) = nested_ctx
        .scale_artifacts
        .per_cell_scale_builder_cache
        .get(&canonical_full_path)
        .or_else(|| {
            nested_ctx
                .scale_artifacts
                .per_cell_scale_builder_cache
                .get(&cell.plan.full_path)
        })
    {
        return Ok(cached_builder.clone());
    }

    build_scale_builder_from_marks(
        &compiled_subplot.marks,
        &compiled_subplot.scale_specs,
        &compiled_subplot.coord_transform,
        &compiled_subplot.data,
        Some(data_override.clone()),
        &nested_ctx.eval_ctx,
        compiled_subplot.get_theme().as_ref(),
    )
    .await
}

async fn coordinate_cell_domains_before_measurement(
    cells: &mut [FacetCellDraft],
    facet_depth: u8,
    compiled_subplot: &Arc<CompiledPlot>,
    nested_ctx: &FacetBandNestedMeasureContext,
) -> Result<HashMap<String, SharingLevel>, AvengerChartError> {
    for cell in cells.iter_mut() {
        let local_extents = if cell.plan.has_data_rows {
            let extent_builder = build_extent_builder_for_cell(
                cell,
                &cell.data_override,
                compiled_subplot,
                nested_ctx,
            )
            .await?;
            annotate_domain_extents(
                extent_builder.extract_domain_extents(&["x", "y", "x2", "y2"]),
                nested_ctx,
            )
        } else {
            HashMap::new()
        };
        cell.local_domain_extents = local_extents;
    }

    let current_domain_infos = cells
        .iter()
        .flat_map(|cell| {
            domain_infos_for_cell(
                &cell.plan.full_path,
                &cell.local_domain_extents,
                facet_depth,
            )
        })
        .collect::<Vec<_>>();

    let mut domain_infos = nested_ctx
        .eval_ctx
        .facet_scale_precompute_store()
        .domain_infos();
    domain_infos.extend(current_domain_infos.iter().cloned());
    if domain_infos.is_empty() {
        return Ok(HashMap::new());
    }
    let channel_domain_sharing_levels =
        collect_channel_domain_sharing_levels_from_infos(&domain_infos);
    let unified = aggregate_domain_extents(&domain_infos);

    for cell in cells.iter_mut() {
        cell.coordinated_domain_extents = coordinated_extents_for_cell(
            &cell.plan.full_path,
            &cell.local_domain_extents,
            &channel_domain_sharing_levels,
            facet_depth,
            &unified,
        );
        if cell.local_domain_extents.is_empty() && !cell.coordinated_domain_extents.is_empty() {
            trace!(
                cell_path = ?cell.plan.full_path,
                coordinated_domain_count = cell.coordinated_domain_extents.len(),
                "FacetBand prefinement domain coordination filled empty-cell extents"
            );
        }
    }

    Ok(channel_domain_sharing_levels)
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
        FacetSubtreeSelector::ByCoordinationNodePath(path) => {
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
            let domain_sharing_level = nested_ctx
                .facet_tree
                .channel_domain_sharing_level_typed(&channel);
            (
                channel,
                ChannelDomainExtent {
                    extent,
                    domain_sharing_level,
                },
            )
        })
        .collect()
}

async fn measure_cells_overflow_probe(
    axis: FacetAxis,
    facet_depth: u8,
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
        let cell_eval_ctx = cell_eval_ctx
            .with_facet_coord_node_path_appended(idx)
            .with_child_frame_container_path_appended(current_facet_path_segment(
                axis,
                facet_depth,
                cell.plan.value.clone(),
            ));
        let (probe_plot_width, probe_plot_height) = cell_eval_ctx
            .facet_probe_size_override(&cell.plan.full_path)
            .unwrap_or((subplot_plot_width, subplot_plot_height));

        let cell_summary = if is_leaf_node {
            perf_counters.estimated_overflow_leaf_measure_count += 1;
            let measured = Box::pin(measure_nested_cell(
                &cell.plan,
                &cell.data_override,
                probe_plot_width,
                probe_plot_height,
                compiled_subplot,
                &cell_eval_ctx,
                nested_ctx,
                &cell.coordinated_domain_extents,
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
            let cell_probe_summary = parent_cell_overflow_summary(&measured.measurement);
            cell.measurement = Some(measured.measurement);
            cell_probe_summary
        } else {
            perf_counters.estimated_overflow_non_leaf_aggregate_count += 1;
            perf_counters.estimated_overflow_non_leaf_full_measure_count += 1;
            let measured = Box::pin(measure_nested_cell(
                &cell.plan,
                &cell.data_override,
                probe_plot_width,
                probe_plot_height,
                compiled_subplot,
                &cell_eval_ctx,
                nested_ctx,
                &cell.coordinated_domain_extents,
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
            let cell_probe_summary = parent_cell_overflow_summary(&measured.measurement);
            cell.measurement = Some(measured.measurement);
            trace!(
                cell_index = idx,
                "FacetBand non-leaf estimated-overflow full measure"
            );
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

async fn measure_overflow_probe(
    axis: FacetAxis,
    facet_depth: u8,
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
        axis,
        facet_depth,
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

fn retarget_cells_to_final_plot_area(
    cells: &mut [FacetCellDraft],
    subplot_plot_width: f32,
    subplot_plot_height: f32,
    compiled_subplot: &Arc<CompiledPlot>,
    subplot_eval_ctx: &EvaluationContext,
    empty_cell_policy: FacetEmptyCellPolicy,
) -> Result<(), AvengerChartError> {
    for (idx, cell) in cells.iter_mut().enumerate() {
        if !cell.plan.has_data_rows {
            trace!(
                cell_index = idx,
                cell_value = ?cell.plan.value,
                "FacetBand retargeted layout empty cell"
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
                "FacetBand retargeted-layout invariant violated: missing estimated-overflow measurement for cell {:?}",
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
            "FacetBand finalized retargeted cell measurement without remeasurement"
        );
    }

    Ok(())
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
///         Subplot::new(
///             Plot::<Cartesian>::new().mark(Symbol::new()...),
///         )
///         .column(col("year")),
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
                missing_mark_err: "FacetColumn coord requires a facet column subplot mark",
            },
            FacetAxis::Row => Self {
                axis,
                scale_name: "row",
                facet_label: "FacetRow",
                missing_scale_err: "No row scale found",
                missing_mark_err: "FacetRow coord requires a facet row subplot mark",
            },
        }
    }

    fn matches_mark(self, facet_mark: &FacetSubplotRef<'_>) -> bool {
        matches!(
            (self.axis, facet_mark),
            (FacetAxis::Column, FacetSubplotRef::Col(_))
                | (FacetAxis::Row, FacetSubplotRef::Row(_))
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
    current_facet_slot_sharing: SharingLevel,
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
                self.eval_ctx
                    .facet_runtime_sizing_mode()
                    .facet_band_is_leaf_plot_area_sized(self.axis_ops.axis),
                self.eval_ctx
                    .facet_runtime_sizing_mode()
                    .policy()
                    .facet_orthogonal_dimension(self.axis_ops.axis)
                    .is_canvas_constrained(),
            ));
        }
        let subplot_band_size = self.resolve_subplot_band_size(&resolved, &cell_values);

        let data_df = self.data.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "{} measure requires data",
                self.axis_ops.facet_label
            ))
        })?;

        // Step 2: Precompute -- precompute subtree scale builders used by child facet slot sharing.
        ensure_subtree_precomputed(
            self.compiled_marks,
            self.facet_path,
            data_df,
            &self.eval_ctx.facet_tree,
            self.eval_ctx,
        )
        .await?;

        // Step 3: Build cell semantics -- build geometry-independent per-cell semantics.
        let cell_semantics = self.build_cell_semantics(&resolved, &cell_values)?;

        // Step 4: Prepare layout inputs and runtime state.
        let (prepared_inputs, prepared_runtime) = prepare_band_inputs_and_runtime(
            cell_semantics.clone(),
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

        // Step 5: Build overflow probe -- non-mutating probe at estimated slot size.
        let (overflow_probe, overflow_runtime) = build_overflow_probe(
            &prepared_inputs,
            &prepared_runtime,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;

        // Step 6: Build local layout -- finalize band layout and retarget cells.
        let (local_layout, measured_runtime) = self
            .build_local_layout(&overflow_probe, overflow_runtime, &prepared_runtime)
            .await?;

        let coord_measurement =
            self.assemble_coord_measurement(local_layout, measured_runtime, &prepared_runtime)?;

        debug!(
            axis = ?self.axis_ops.axis,
            facet_path = ?self.facet_path,
            estimated_overflow_leaf_measure_count = perf_counters.estimated_overflow_leaf_measure_count,
            estimated_overflow_non_leaf_aggregate_count = perf_counters.estimated_overflow_non_leaf_aggregate_count,
            estimated_overflow_non_leaf_full_measure_count = perf_counters.estimated_overflow_non_leaf_full_measure_count,
            "FacetBand layout measurement counters"
        );
        self.eval_ctx.record_facet_band_measure_run(
            perf_counters.estimated_overflow_leaf_measure_count,
            perf_counters.estimated_overflow_non_leaf_aggregate_count,
            perf_counters.estimated_overflow_non_leaf_full_measure_count,
        );

        // Step 7: Assemble -- package runtime cell state for coordination and rendering.
        Ok(coord_measurement)
    }

    #[cfg(test)]
    async fn build_band_probe_only(
        &self,
        perf_counters: &mut FacetPipelinePerfCounters,
    ) -> Result<FacetBandProbeLayout, AvengerChartError> {
        let resolved = match self.resolve_node_or_empty()? {
            ResolveBandNodeOutcome::Empty(_) => {
                return Ok(FacetBandProbeLayout {
                    cell_probe_summary: FacetCellProbeSummary {
                        guide_overflow: OverflowSpaceRequirement::default(),
                        total_overflow: OverflowSpaceRequirement::default(),
                        boundary_profiles:
                            crate::facet::overflow_projection::FacetBoundaryProfiles::default(),
                        max_child_padding: 0.0,
                    },
                });
            }
            ResolveBandNodeOutcome::Ready(resolved) => resolved,
        };
        let cell_values = self.enumerate_cell_values(&resolved);
        if cell_values.is_empty() {
            return Ok(FacetBandProbeLayout {
                cell_probe_summary: FacetCellProbeSummary {
                    guide_overflow: OverflowSpaceRequirement::default(),
                    total_overflow: OverflowSpaceRequirement::default(),
                    boundary_profiles:
                        crate::facet::overflow_projection::FacetBoundaryProfiles::default(),
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

        let cell_semantics = self.build_cell_semantics(&resolved, &cell_values)?;
        let (prepared_inputs, prepared_runtime) = prepare_band_inputs_and_runtime(
            cell_semantics,
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
        let (overflow_probe, overflow_runtime) = build_overflow_probe(
            &prepared_inputs,
            &prepared_runtime,
            subplot_plot_width,
            subplot_plot_height,
            perf_counters,
        )
        .await?;

        let plan = FacetBandMeasurePlan {
            cell_values: overflow_probe
                .prepared_inputs
                .cell_semantics
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
            &overflow_probe.overflow_probe_summary,
            prepared_runtime.initial_subplot_band_size,
            &prepared_runtime.original_band_scale,
            overflow_probe
                .prepared_inputs
                .cell_semantics
                .empty_cell_policy,
        )?;

        let mut final_probe_cells =
            prepared_cells_as_drafts(&overflow_probe.prepared_inputs, &prepared_runtime);
        let final_overflow_summary = measure_overflow_probe(
            overflow_probe.prepared_inputs.cell_semantics.node_id.axis,
            overflow_probe.prepared_inputs.cell_semantics.facet_depth,
            &mut final_probe_cells,
            final_subplot_plot_width,
            final_subplot_plot_height,
            &prepared_runtime.compiled_subplot,
            &prepared_runtime.subplot_eval_ctx,
            &prepared_runtime.nested_measure_ctx,
            overflow_probe
                .prepared_inputs
                .cell_semantics
                .empty_cell_policy,
            is_leaf_subplot(&prepared_runtime.compiled_subplot),
            perf_counters,
        )
        .await?;

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_layout_plan.padding_inner_px,
            guide_slot_gap_px: band_layout_plan.guide_padding_inner_px,
            outer_start: band_layout_plan.outer_start,
            outer_end: band_layout_plan.outer_end,
            n: band_layout_plan.n,
        };
        let measured_overflow = derive_band_overflow_from_probe(
            self.axis_ops.axis,
            &overflow_probe.prepared_inputs.renderable_mask,
            &final_overflow_summary.cell_overflows,
        );
        let orthogonal_dimension_canvas_constrained = self
            .eval_ctx
            .facet_runtime_sizing_mode()
            .policy()
            .facet_orthogonal_dimension(self.axis_ops.axis)
            .is_canvas_constrained();
        let fixed_plot_area_lock = self
            .eval_ctx
            .facet_runtime_sizing_mode()
            .facet_band_is_leaf_plot_area_sized(self.axis_ops.axis);
        let parent_owns_main_axis_start_slab = !self.scale_backed_edge_slab_is_chart_overflow(true);
        let parent_owns_main_axis_end_slab = !self.scale_backed_edge_slab_is_chart_overflow(false);
        let probe_measurement = FacetBandProbeMeasurement {
            axis: self.axis_ops.axis,
            cell_values: plan.cell_values.clone(),
            cell_has_data_rows: plan
                .cells
                .iter()
                .map(|cell| cell.plan.has_data_rows)
                .collect(),
            measured_overflow,
            original_band_scale: prepared_runtime.original_band_scale.clone(),
            local_layout,
            empty_cell_policy: overflow_probe
                .prepared_inputs
                .cell_semantics
                .empty_cell_policy,
            fixed_plot_area_lock,
            allocation_ownership: FacetBandAllocationOwnership::from_axis_policy(
                parent_owns_main_axis_start_slab,
                parent_owns_main_axis_end_slab,
                orthogonal_dimension_canvas_constrained,
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
                let sharing_context = crate::guide::GuideSharingContext::new(
                    self.eval_ctx.facet_tree.as_ref(),
                    self.facet_path,
                    self.eval_ctx.child_frame_sharing_path(),
                );
                compiled_guide
                    .measure_overflow(
                        &configured_scales,
                        final_subplot_plot_width,
                        final_subplot_plot_height,
                        prepared_runtime.compiled_subplot.get_theme().as_ref(),
                        &self.eval_ctx.params,
                        self.data,
                        self.eval_ctx.session_context.as_ref(),
                        sharing_context,
                        Some(&probe_measurement),
                    )
                    .await?
            } else {
                probe_measurement.measured_overflow_value().guide
            };
        let total_overflow = prepared_runtime
            .compiled_subplot
            .total_overflow_from_precomputed_guide_overflow(
                self.eval_ctx,
                &fixed_child_plot_area_layout_spec(
                    final_subplot_plot_width,
                    final_subplot_plot_height,
                ),
                &adjusted_scales,
                final_subplot_plot_width,
                final_subplot_plot_height,
                &guide_overflow,
                self.facet_path,
            )
            .await?;

        Ok(FacetBandProbeLayout {
            cell_probe_summary: FacetCellProbeSummary {
                boundary_profiles:
                    crate::facet::overflow_projection::FacetBoundaryProfiles::single_from_overflow(
                        guide_overflow.clone(),
                        total_overflow.clone(),
                    ),
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
                let mark = facet_subplot_ref(m.as_ref())?;
                self.axis_ops.matches_mark(&mark).then_some(mark)
            })
            .ok_or_else(|| {
                AvengerChartError::InternalError(self.axis_ops.missing_mark_err.to_string())
            })?;

        let (compiled_subplot, current_facet_slot_sharing, empty_cell_policy) = match facet_mark {
            FacetSubplotRef::Col(mark) => (
                mark.compiled_subplot(),
                mark.facet_slot_sharing()
                    .map(SharingLevel::from)
                    .unwrap_or(SharingLevel::FREE),
                mark.facet_empty_cell_policy(),
            ),
            FacetSubplotRef::Row(mark) => (
                mark.compiled_subplot(),
                mark.facet_slot_sharing()
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
                self.eval_ctx
                    .facet_runtime_sizing_mode()
                    .facet_band_is_leaf_plot_area_sized(self.axis_ops.axis),
                self.eval_ctx
                    .facet_runtime_sizing_mode()
                    .policy()
                    .facet_orthogonal_dimension(self.axis_ops.axis)
                    .is_canvas_constrained(),
            )));
        };

        Ok(ResolveBandNodeOutcome::Ready(FacetBandResolvedNode {
            compiled_subplot,
            band_scale,
            subplot_band_size,
            current_facet_slot_sharing,
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
            .enumerate_values_for_facet(self.facet_path, resolved.current_facet_slot_sharing.raw())
            .unwrap_or_else(fallback)
    }

    fn fixed_subtree_plot_area_for_cell_path(
        &self,
        cell_path: &[ScalarValue],
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> crate::facet::subtree_plot_area::PlotAreaSize {
        estimate_path_plot_area_from_leaf_size(
            &self.eval_ctx.facet_tree,
            cell_path,
            LeafPlotAreaSize::new(leaf_plot_width, leaf_plot_height),
        )
    }

    fn resolve_subplot_band_size(
        &self,
        resolved: &FacetBandResolvedNode<'_>,
        cell_values: &[ScalarValue],
    ) -> f32 {
        let mode = self.eval_ctx.facet_runtime_sizing_mode();
        if !mode.facet_band_is_leaf_plot_area_sized(self.axis_ops.axis) {
            return resolved.subplot_band_size;
        }

        let policy = mode.policy();
        let leaf_plot_width = policy
            .leaf_plot_width()
            .unwrap_or(resolved.subplot_band_size.max(1.0));
        let leaf_plot_height = policy
            .leaf_plot_height()
            .unwrap_or(resolved.subplot_band_size.max(1.0));
        let mut max_main_size = 0.0f32;
        for value in cell_values {
            let mut cell_path = self.facet_path.to_vec();
            cell_path.push(value.clone());
            let plot_area_size = self.fixed_subtree_plot_area_for_cell_path(
                &cell_path,
                leaf_plot_width,
                leaf_plot_height,
            );
            max_main_size = max_main_size.max(plot_area_size.main_size(self.axis_ops.axis));
        }
        if max_main_size > 0.0 {
            max_main_size
        } else {
            resolved.subplot_band_size
        }
    }

    fn build_cell_semantics(
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

        let mut padding_inner_px =
            derive_padding_inner_px_from_probe(self.axis_ops.axis, pass1, &pass1_renderable_cells);
        let mut guide_padding_inner_px = derive_guide_padding_inner_px_from_probe(
            self.axis_ops.axis,
            pass1,
            &pass1_renderable_cells,
        );
        if let Some(feedback) = self
            .eval_ctx
            .facet_padding_feedback(self.eval_ctx.facet_coord_node_path())
        {
            padding_inner_px = padding_inner_px.max(feedback.padding_inner_px);
            guide_padding_inner_px = guide_padding_inner_px.max(feedback.guide_padding_inner_px);
            trace!(
                axis = ?self.axis_ops.axis,
                facet_coord_node_path = ?self.eval_ctx.facet_coord_node_path(),
                feedback_padding_inner_px = feedback.padding_inner_px,
                feedback_guide_padding_inner_px = feedback.guide_padding_inner_px,
                padding_inner_px,
                guide_padding_inner_px,
                "FacetBand applied realized padding feedback"
            );
        }

        let (first_edge_idx, last_edge_idx) =
            effective_edge_indices(&pass1_renderable_cells, pass1.cell_overflows.len())
                .unwrap_or((0, 0));
        // Edge legend slabs are not part of this facet band's own scale range.
        // They are routed upward as sibling-boundary demand, or as root residual
        // demand at the true chart edge, so leaf plot-area sizes stay uniform
        // across sibling facet subtrees.
        let (raw_outer_start, raw_outer_end) =
            self.axis_ops
                .derive_outer_edges(pass1, first_edge_idx, last_edge_idx);
        let start_edge_is_chart_overflow = self.scale_backed_edge_slab_is_chart_overflow(true);
        let end_edge_is_chart_overflow = self.scale_backed_edge_slab_is_chart_overflow(false);
        let outer_start = 0.0;
        let outer_end = 0.0;
        debug!(
            axis = ?self.axis_ops.axis,
            padding_inner_px,
            guide_padding_inner_px,
            outer_start,
            outer_end,
            raw_outer_start,
            raw_outer_end,
            cell_count = cell_values.len(),
            first_edge_idx,
            last_edge_idx,
            start_edge_is_chart_overflow,
            end_edge_is_chart_overflow,
            "FacetBand derived local layout"
        );

        FacetBandPlan {
            padding_inner_px,
            guide_padding_inner_px,
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
            guide_slot_gap_px: band_plan.guide_padding_inner_px,
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
            ScaleLayoutRewriteMode::Measurement {
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
            "FacetBand local layout scale bandwidth"
        );

        Ok(final_subplot_band_size)
    }

    fn scale_backed_edge_slab_is_chart_overflow(&self, is_start: bool) -> bool {
        if !self
            .eval_ctx
            .facet_runtime_sizing_mode()
            .policy()
            .facet_band_dimension(self.axis_ops.axis)
            .is_canvas_constrained()
        {
            return false;
        }

        let facet_tree = &self.eval_ctx.facet_tree;
        let Some(resolved) = facet_tree.resolve_path_info(self.facet_path) else {
            return self.facet_path.is_empty();
        };

        let mut ancestor_path = Vec::new();
        for (depth, value) in self.facet_path.iter().enumerate() {
            let Some(direction) = resolved.level_directions.get(depth).copied() else {
                return false;
            };
            let ancestor_axis = match direction {
                crate::facet::FacetDirection::Column => FacetAxis::Column,
                crate::facet::FacetDirection::Row => FacetAxis::Row,
            };

            if ancestor_axis == self.axis_ops.axis {
                let Some(node) = facet_tree.node_at_path(&ancestor_path) else {
                    return false;
                };
                let values = node.values().cloned().collect::<Vec<_>>();
                let Some((first_edge_idx, last_edge_idx)) =
                    effective_edge_indices_for_values_at_path(facet_tree, &ancestor_path, &values)
                else {
                    return false;
                };
                let Some(index) = resolved.indices.get(depth).copied() else {
                    return false;
                };
                let expected = if is_start {
                    first_edge_idx
                } else {
                    last_edge_idx
                };
                if index != expected {
                    return false;
                }
            }

            ancestor_path.push(value.clone());
        }

        true
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
        let final_subplot_band_size = if self
            .eval_ctx
            .facet_runtime_sizing_mode()
            .facet_band_is_leaf_plot_area_sized(self.axis_ops.axis)
        {
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

    async fn build_local_layout_from_probe(
        &self,
        mut plan: FacetBandMeasurePlan,
        overflow_summary: &OverflowProbeSummary,
        initial_subplot_band_size: f32,
        compiled_subplot: &Arc<CompiledPlot>,
        subplot_eval_ctx: &EvaluationContext,
        _nested_measure_ctx: &FacetBandNestedMeasureContext,
        band_scale: &ConfiguredScale,
        empty_cell_policy: FacetEmptyCellPolicy,
    ) -> Result<FacetLocalLayoutOutcome, AvengerChartError> {
        let (band_layout_plan, final_subplot_band_size, subplot_plot_width, subplot_plot_height) =
            self.derive_local_layout_from_probe(
                &plan,
                overflow_summary,
                initial_subplot_band_size,
                band_scale,
                empty_cell_policy,
            )?;

        retarget_cells_to_final_plot_area(
            &mut plan.cells,
            subplot_plot_width,
            subplot_plot_height,
            compiled_subplot,
            subplot_eval_ctx,
            empty_cell_policy,
        )?;

        Ok(FacetLocalLayoutOutcome {
            band_layout_plan,
            final_subplot_main_or_cross_size: final_subplot_band_size,
            cells: plan.cells,
        })
    }

    async fn build_local_layout(
        &self,
        overflow_probe: &FacetBandOverflowProbe,
        overflow_runtime: FacetBandOverflowRuntime,
        prepared_runtime: &FacetBandPreparedRuntime,
    ) -> Result<(FacetBandLocalLayout, FacetBandMeasuredRuntime), AvengerChartError> {
        let plan = FacetBandMeasurePlan {
            cell_values: overflow_probe
                .prepared_inputs
                .cell_semantics
                .cell_values
                .clone(),
            cells: overflow_runtime.cells,
            scale_artifacts: prepared_runtime.scale_artifacts.clone(),
        };
        let local_layout_outcome = self
            .build_local_layout_from_probe(
                plan,
                &overflow_probe.overflow_probe_summary,
                prepared_runtime.initial_subplot_band_size,
                &prepared_runtime.compiled_subplot,
                &prepared_runtime.subplot_eval_ctx,
                &prepared_runtime.nested_measure_ctx,
                &prepared_runtime.original_band_scale,
                overflow_probe
                    .prepared_inputs
                    .cell_semantics
                    .empty_cell_policy,
            )
            .await?;

        let mut measurements = Vec::with_capacity(local_layout_outcome.cells.len());
        let mut local_domain_extents = Vec::with_capacity(local_layout_outcome.cells.len());
        let mut coordinated_domain_extents = Vec::with_capacity(local_layout_outcome.cells.len());
        for mut cell in local_layout_outcome.cells {
            measurements.push(cell.measurement.take().expect(
                "FacetBand layout invariant violated: missing final cell measurement in measured runtime state",
            ));
            local_domain_extents.push(cell.local_domain_extents);
            coordinated_domain_extents.push(cell.coordinated_domain_extents);
        }

        let local_layout = FacetBandLocalLayout {
            overflow_probe: overflow_probe.clone(),
            band_layout_plan: local_layout_outcome.band_layout_plan,
            final_subplot_cross_size: local_layout_outcome.final_subplot_main_or_cross_size,
        };
        let runtime_state = FacetBandMeasuredRuntime {
            measurements,
            local_domain_extents,
            coordinated_domain_extents,
        };
        Ok((local_layout, runtime_state))
    }

    fn assemble_coord_measurement(
        &self,
        local_layout: FacetBandLocalLayout,
        measured_runtime: FacetBandMeasuredRuntime,
        prepared_runtime: &FacetBandPreparedRuntime,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        let FacetBandLocalLayout {
            overflow_probe,
            band_layout_plan,
            final_subplot_cross_size,
        } = local_layout;
        let FacetBandOverflowProbe {
            prepared_inputs, ..
        } = overflow_probe;
        let FacetBandPreparedInputs { cell_semantics, .. } = prepared_inputs;
        let crate::facet::band_attributes::FacetBandSemantics {
            facet_depth,
            coordination_field_identity,
            empty_cell_policy,
            cells,
            ..
        } = cell_semantics;

        assert_eq!(
            cells.len(),
            prepared_runtime.data_overrides.len(),
            "FacetBand layout invariant violated: cell semantics cell count must equal prepared inputs data overrides"
        );
        assert_eq!(
            cells.len(),
            measured_runtime.measurements.len(),
            "FacetBand layout invariant violated: cell semantics cell count must equal measured runtime measurements"
        );
        assert_eq!(
            cells.len(),
            measured_runtime.local_domain_extents.len(),
            "FacetBand layout invariant violated: cell semantics cell count must equal measured runtime local extents"
        );
        assert_eq!(
            cells.len(),
            measured_runtime.coordinated_domain_extents.len(),
            "FacetBand layout invariant violated: cell semantics cell count must equal measured runtime coordinated extents"
        );

        let cell_runtimes: Vec<FacetCellRuntime> = cells
            .into_iter()
            .zip(prepared_runtime.data_overrides.iter().cloned())
            .zip(
                measured_runtime
                    .measurements
                    .into_iter()
                    .zip(measured_runtime.local_domain_extents)
                    .zip(measured_runtime.coordinated_domain_extents),
            )
            .map(
                |(
                    (cell, data_override),
                    ((measurement, local_domain_extents), coordinated_domain_extents),
                )| {
                    #[cfg(not(test))]
                    let _ = (local_domain_extents, coordinated_domain_extents);
                    FacetCellRuntime {
                        data_override,
                        plan: FacetCellPlan::from(&cell),
                        measurement,
                        #[cfg(test)]
                        local_domain_extents,
                        #[cfg(test)]
                        coordinated_domain_extents,
                    }
                },
            )
            .collect();

        let local_layout = CoordinatedLayout {
            padding_inner_px: band_layout_plan.padding_inner_px,
            guide_slot_gap_px: band_layout_plan.guide_padding_inner_px,
            outer_start: band_layout_plan.outer_start,
            outer_end: band_layout_plan.outer_end,
            n: band_layout_plan.n,
        };
        let measured_overflow =
            measured_overflow_from_cells(self.axis_ops.axis, &cell_runtimes, empty_cell_policy);
        let scope_path_prefix = facet_scope_path_prefix(
            self.eval_ctx.child_frame_container_path(),
            self.eval_ctx.facet_tree.as_ref(),
            self.facet_path,
        )?;
        let orthogonal_dimension_canvas_constrained = self
            .eval_ctx
            .facet_runtime_sizing_mode()
            .policy()
            .facet_orthogonal_dimension(self.axis_ops.axis)
            .is_canvas_constrained();
        let base = FacetBandCoordMeasurement {
            axis: self.axis_ops.axis,
            cells: cell_runtimes,
            scope_path_prefix,
            shared_scale_builder: prepared_runtime
                .scale_artifacts
                .shared_scale_builder
                .clone(),
            coordinated_overflow: CoordinatedOverflow::default(),
            coordinated_boundary_overflow: None,
            coordinated_guide_anchor_overflow: None,
            measured_overflow,
            compiled_subplot: prepared_runtime.compiled_subplot.clone(),
            subplot_cross_size: final_subplot_cross_size,
            facet_depth,
            original_band_scale: prepared_runtime.original_band_scale.clone(),
            local_layout,
            guide_padding_inner_px: band_layout_plan.guide_padding_inner_px,
            coordinated_layout: None,
            coordination_field_identity,
            empty_cell_policy,
            allocation_ownership: FacetBandAllocationOwnership::from_axis_policy(
                !self.scale_backed_edge_slab_is_chart_overflow(true),
                !self.scale_backed_edge_slab_is_chart_overflow(false),
                orthogonal_dimension_canvas_constrained,
            ),
            placement_model: if self
                .eval_ctx
                .facet_runtime_sizing_mode()
                .facet_band_is_leaf_plot_area_sized(self.axis_ops.axis)
            {
                FacetBandPlacementModel::Explicit(FacetBandExplicitPlacement::default())
            } else {
                FacetBandPlacementModel::ScaleBacked
            },
        };

        let mut measurement = base;
        measurement.recompute_explicit_placement_if_needed();
        Ok(Box::new(measurement))
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
        request: CoordMeasureRequest<'_>,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        FacetBandMeasurePipeline::new(
            FacetAxisOps::for_axis(FacetAxis::Column),
            request.scales(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
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
    use crate::chart_core::LegendPosition;
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::layout::{LayoutBounds, Size2D};
    use crate::prelude::*;
    use crate::render::LegendMeasurements;
    use crate::render::types::LegendMeasurement;
    use crate::scales::{Linear, Scale};
    use crate::theme::Theme;
    use avenger_scales::scales::{band::BandScale, linear::LinearScale};
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::sync::Arc;

    fn make_band_scale(range: (f32, f32)) -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ])
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
            frame_layout: crate::layout::FrameLayout {
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
    fn apply_frame_side_slab_reanchors_right_legend_after_guide_coordination() {
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

        apply_frame_side_slab(&mut layout, AxisPosition::Right, 5.0, 66.0);

        assert_close(layout.overflow.right, 5.0);
        assert_close(layout.total_overflow.right, 66.0);
        assert_close(layout.canvas_size.0, 266.0);
        let guide = layout
            .frame_layout
            .guide_overflows
            .get(&AxisPosition::Right)
            .expect("expected right guide overflow bounds");
        assert_close(guide.x, 90.0);
        assert_close(guide.y, 20.0);
        assert_close(guide.width, 5.0);
        assert_close(guide.height, 157.0);

        let bounds = layout
            .frame_layout
            .legends
            .get("fill")
            .expect("expected fill legend bounds");
        assert_close(bounds.x, 95.0);
        assert_close(bounds.y, 20.0);
        assert_close(bounds.width, 61.0);
        assert_close(bounds.height, 157.0);
    }

    #[test]
    fn apply_frame_side_slab_preserves_measured_bottom_guide_for_legend_only_slab() {
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

        apply_frame_side_slab(&mut layout, AxisPosition::Bottom, 0.0, 48.0);

        assert_close(layout.overflow.bottom, 34.0);
        assert_close(layout.total_overflow.bottom, 82.0);
        let guide = layout
            .frame_layout
            .guide_overflows
            .get(&AxisPosition::Bottom)
            .expect("expected bottom guide overflow bounds");
        assert_close(guide.y, 177.0);
        assert_close(guide.height, 34.0);

        let bounds = layout
            .frame_layout
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
                size: Size2D {
                    width: 61.0,
                    height: 157.0,
                },
                flexible: true,
                position: LegendPosition::Right,
            },
        );

        retarget_frame_layout_for_plot_area(&mut layout, &legend_measurements, 100.0, 246.0);

        let bounds = layout
            .frame_layout
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
                size: Size2D {
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
                size: Size2D {
                    width: 61.0,
                    height: 117.0,
                },
                flexible: true,
                position: LegendPosition::Right,
            },
        );

        retarget_frame_layout_for_plot_area(&mut layout, &legend_measurements, 80.0, 246.0);

        let shape_bounds = layout
            .frame_layout
            .legends
            .get("shape")
            .expect("expected shape legend bounds");
        assert_close(shape_bounds.y, 20.0);
        assert_close(shape_bounds.height, 40.0);

        let fill_bounds = layout
            .frame_layout
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

    #[test]
    fn facet_scope_path_prefix_preserves_ancestor_axis_and_level() -> Result<(), AvengerChartError>
    {
        let tree = sample_tree_for_cell_plan_tests();
        let prefix = facet_scope_path_prefix(&[], &tree, &[s("Group")])?;

        assert_eq!(
            prefix,
            vec![ContainerPathSegment::facet_value(
                FacetAxis::Row,
                1,
                s("Group")
            )]
        );
        Ok(())
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
                    .mark(Subplot::new(inner_subplot.clone()).col_with(col("col_group"), |c| c))
                    .compile(&session)
                    .await?
            }
            FacetAxis::Row => {
                Plot::<FacetRow>::new()
                    .data(data_df.clone())
                    .mark(Subplot::new(inner_subplot).row_with(col("row_group"), |c| c))
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
            &eval_ctx,
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
        let nested_subplot = Plot::<FacetColumn>::new()
            .mark(Subplot::new(leaf_subplot).col_with(col("team_id"), |c| c));
        let compiled_plot = Plot::<FacetColumn>::new()
            .data(data_df.clone())
            .mark(Subplot::new(nested_subplot).col_with(col("group_id"), |c| c))
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
            &eval_ctx,
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

    async fn build_measurement_fixture()
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
        let prepared_inputs = prepare_measurement_inputs(
            cell_plans,
            &facet_path,
            &data_df,
            &compiled_subplot,
            FacetEmptyCellPolicy::Hole,
            &eval_ctx,
        )
        .await?;

        Ok((compiled_subplot, prepared_inputs))
    }

    async fn build_coord_measurement_for_retarget_tests(
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
            guide_slot_gap_px: 4.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 2,
        };
        let coordinated = CoordinatedLayout {
            padding_inner_px: 10.0,
            guide_slot_gap_px: 14.0,
            outer_start: 5.0,
            outer_end: 6.0,
            n: 4,
        };
        let selected = layout_from_measurement_or_local(&local, Some(&coordinated));
        assert_eq!(selected.padding_inner_px, coordinated.padding_inner_px);
        assert_eq!(selected.guide_slot_gap_px, coordinated.guide_slot_gap_px);
        assert_eq!(selected.outer_start, coordinated.outer_start);
        assert_eq!(selected.outer_end, coordinated.outer_end);
        assert_eq!(selected.n, coordinated.n);

        let fallback = layout_from_measurement_or_local(&local, None);
        assert_eq!(fallback.padding_inner_px, local.padding_inner_px);
        assert_eq!(fallback.guide_slot_gap_px, local.guide_slot_gap_px);
        assert_eq!(fallback.outer_start, local.outer_start);
        assert_eq!(fallback.outer_end, local.outer_end);
        assert_eq!(fallback.n, local.n);
    }

    #[test]
    fn realized_allocation_owned_slabs_override_policy_estimate() {
        let overflow = CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: 1.0,
                right: 2.0,
                bottom: 3.0,
                left: 4.0,
            },
            total: OverflowSpaceRequirement {
                top: 11.0,
                right: 22.0,
                bottom: 33.0,
                left: 44.0,
            },
        };
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            guide_slot_gap_px: 0.0,
            outer_start: 0.0,
            outer_end: 0.0,
            n: 2,
        };
        let realized = EdgeSlabs::new(5.0, 6.0, 7.0, 8.0);
        let ownership = FacetBandAllocationOwnership::from_policy(false, false)
            .with_realized_owned_legend_slabs(realized);

        assert_eq!(
            ownership.owned_legend_slabs(FacetAxis::Column, &overflow, &layout),
            realized
        );
        assert_eq!(
            ownership
                .without_realized_owned_legend_slabs()
                .owned_legend_slabs(FacetAxis::Column, &overflow, &layout),
            EdgeSlabs::default()
        );
    }

    #[test]
    fn apply_facet_band_scale_layout_applies_domain_padding_and_range() {
        let base = make_band_scale((0.0, 300.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 12.0,
            guide_slot_gap_px: 99.0,
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
            ScaleLayoutRewriteMode::Measurement {
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
    fn coordinated_layout_merge_uses_shared_outer_edges() {
        let mut merged = CoordinatedLayout {
            padding_inner_px: 6.0,
            guide_slot_gap_px: 7.0,
            outer_start: 11.0,
            outer_end: 12.0,
            n: 2,
        };
        let coordinated = CoordinatedLayout {
            padding_inner_px: 18.0,
            guide_slot_gap_px: 27.0,
            outer_start: 91.0,
            outer_end: 92.0,
            n: 4,
        };

        merged.merge(&coordinated);
        assert_eq!(merged.padding_inner_px, coordinated.padding_inner_px);
        assert_eq!(merged.guide_slot_gap_px, coordinated.guide_slot_gap_px);
        assert_eq!(merged.n, coordinated.n);
        assert_eq!(merged.outer_start, coordinated.outer_start);
        assert_eq!(merged.outer_end, coordinated.outer_end);
    }

    #[test]
    fn apply_facet_band_scale_layout_zero_padding_override_is_optional() {
        let base = make_band_scale((0.0, 200.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            guide_slot_gap_px: 0.0,
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
            ScaleLayoutRewriteMode::Render {
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
            ScaleLayoutRewriteMode::Render {
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
            guide_slot_gap_px: 0.0,
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
            guide_slot_gap_px: 12.0,
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
            ScaleLayoutRewriteMode::Measurement {
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
            guide_slot_gap_px: 6.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 3,
        };
        let same = CoordinatedLayout {
            padding_inner_px: 6.0,
            guide_slot_gap_px: 99.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 3,
        };
        let changed = CoordinatedLayout {
            padding_inner_px: 6.0,
            guide_slot_gap_px: 6.0,
            outer_start: 1.0,
            outer_end: 2.0,
            n: 4,
        };

        assert!(!has_coordinated_layout_change(&local, None));
        assert!(!has_coordinated_layout_change(&local, Some(&same)));
        assert!(has_coordinated_layout_change(&local, Some(&changed)));
    }

    #[tokio::test]
    async fn derive_retarget_requirements_records_layout_change_without_retarget_policy()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let mut coordinated = facet_band.local_layout.clone();
        coordinated.n = coordinated.n.saturating_add(1);
        coordinated.padding_inner_px += 7.0;
        facet_band.set_coordinated_layout_value(coordinated);

        let requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        assert!(requirements.layout_changed);
        assert!(!requirements.has_legend_overflow);
        assert_eq!(requirements.child_count, facet_band.cells.len());
        assert_eq!(
            requirements.child_plot_areas.len(),
            requirements.child_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn orthogonal_child_allocations_use_measured_overflow_after_alignment()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Row).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let measured_before = facet_band
            .measured_overflow_value()
            .expect("fixture should have measured overflow");

        let mut coordinated = measured_before.clone();
        coordinated.guide.left += 100.0;
        coordinated.total.left += 100.0;
        coordinated.guide.right += 100.0;
        coordinated.total.right += 100.0;
        facet_band.set_coordinated_overflow_value(coordinated);
        facet_band.realize_coordinated_child_frame_allocations();

        let measured_after = facet_band
            .measured_overflow_value()
            .expect("fixture should still have measured overflow");
        assert_eq!(measured_after.guide.left, measured_before.guide.left);
        assert_eq!(measured_after.total.left, measured_before.total.left);
        assert_eq!(measured_after.guide.right, measured_before.guide.right);
        assert_eq!(measured_after.total.right, measured_before.total.right);

        let projected_left_guide = facet_band
            .child_measurements_iter()
            .map(|child| child.layout.overflow.left)
            .fold(0.0f32, f32::max);
        assert_close(projected_left_guide, measured_before.guide.left);
        let projected_left_total = facet_band
            .child_measurements_iter()
            .map(|child| child.layout.total_overflow.left)
            .fold(0.0f32, f32::max);
        assert_close(projected_left_total, measured_before.total.left);
        Ok(())
    }

    #[tokio::test]
    async fn derive_retarget_requirements_records_legend_slab_as_fact()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.coordinated_overflow.total.top = 12.0;

        let requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        assert!(requirements.has_legend_overflow);
        assert_eq!(requirements.legend_main_axis_slab.start, 12.0);
        assert_eq!(requirements.legend_main_axis_slab.end, 0.0);
        assert_eq!(requirements.legend_main_axis_slab.total(), 12.0);
        Ok(())
    }

    #[tokio::test]
    async fn derive_retarget_requirements_axis_owner_ignore_empty_cells_matches_holes()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        facet_band.empty_cell_policy = FacetEmptyCellPolicy::Auto;

        for cell in facet_band.cells.iter_mut() {
            cell.plan.is_empty = false;
        }

        let no_holes_requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        assert!(!no_holes_requirements.ownership.has_holes);
        assert!(
            !no_holes_requirements
                .ownership
                .axis_owner_ignore_empty_cells
        );

        facet_band.cells[0].plan.is_empty = true;
        let with_holes_requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        assert!(with_holes_requirements.ownership.has_holes);
        assert!(
            with_holes_requirements
                .ownership
                .axis_owner_ignore_empty_cells
        );
        Ok(())
    }

    #[tokio::test]
    async fn apply_retarget_actions_layout_only_updates_cross_size() -> Result<(), AvengerChartError>
    {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Column).await?;
        let facet_band = measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
            .expect("expected FacetBandCoordMeasurement");
        let mut coordinated = facet_band.local_layout.clone();
        coordinated.n = coordinated.n.saturating_add(3);
        coordinated.padding_inner_px += 9.0;
        facet_band.set_coordinated_layout_value(coordinated);

        let before_cross_size = facet_band.subplot_cross_size;
        let requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        let actions = RetargetNodeActions {
            node_id: requirements.node_id.clone(),
            axis: requirements.axis,
            band_action: BandRetargetAction::ApplyCoordinatedLayout,
            child_actions: vec![CellRetargetAction::preserve(); requirements.child_count],
        };
        let outcome = facet_band
            .apply_retarget_actions(&eval_ctx, &actions)
            .await?;

        assert!((outcome.subplot_cross_size_before - before_cross_size).abs() <= 0.01);
        assert!(outcome.band_layout_applied);
        assert_eq!(outcome.plot_area_retarget_count, 0);
        assert!((facet_band.subplot_cross_size - before_cross_size).abs() > 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn apply_retarget_actions_retargets_cells_when_requested() -> Result<(), AvengerChartError>
    {
        let (mut measurement, eval_ctx) =
            build_coord_measurement_for_retarget_tests(FacetAxis::Column).await?;
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
        let requirements =
            facet_band.derive_retarget_requirements(CoordinationNodeKey::new(Vec::new()))?;
        let plot_area_target = PlotAreaTarget {
            width: None,
            height: Some((first_before - requirements.legend_main_axis_slab.total()).max(1.0)),
        };
        let actions = RetargetNodeActions {
            node_id: requirements.node_id.clone(),
            axis: requirements.axis,
            band_action: BandRetargetAction::Preserve,
            child_actions: vec![
                CellRetargetAction::retarget_plot_area(plot_area_target);
                requirements.child_count
            ],
        };

        let outcome = facet_band
            .apply_retarget_actions(&eval_ctx, &actions)
            .await?;
        let first_after = facet_band
            .cells
            .first()
            .expect("expected at least one facet cell")
            .measurement
            .plot_area_height;

        assert_eq!(expected_cells, facet_band.cells.len());
        assert_eq!(outcome.plot_area_retarget_count, expected_cells);
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
    async fn overflow_probe_leaf_capture_preserves_extents() -> Result<(), AvengerChartError> {
        let (compiled_subplot, prepared_inputs) = build_measurement_fixture().await?;
        let FacetPreparedRuntimeInputs {
            mut plan,
            subplot_eval_ctx,
            nested_measure_ctx,
        } = prepared_inputs;
        let cells = &mut plan.cells;
        let is_leaf = is_leaf_subplot(&compiled_subplot);
        let facet_depth = cells
            .first()
            .map(|cell| cell.plan.full_path.len() as u8)
            .unwrap_or(0);
        coordinate_cell_domains_before_measurement(
            cells,
            facet_depth,
            &compiled_subplot,
            &nested_measure_ctx,
        )
        .await?;

        let before: Vec<(bool, usize)> = cells
            .iter()
            .map(|cell| (cell.measurement.is_none(), cell.local_domain_extents.len()))
            .collect();
        let mut perf_counters = FacetPipelinePerfCounters::default();

        let overflow_probe = measure_overflow_probe(
            FacetAxis::Column,
            facet_depth,
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

        assert_eq!(overflow_probe.cell_overflows.len(), 3);

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
    async fn non_leaf_estimated_overflow_probe_path_records_full_measure_calls()
    -> Result<(), AvengerChartError> {
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

        let probe_layout = pipeline.build_band_probe_only(&mut perf_counters).await?;
        assert!(probe_layout.cell_probe_summary.max_child_padding >= 0.0);
        assert!(perf_counters.estimated_overflow_non_leaf_aggregate_count > 0);
        assert!(perf_counters.estimated_overflow_non_leaf_full_measure_count > 0);

        Ok(())
    }

    #[tokio::test]
    async fn local_layout_finalization_populates_measurements_and_extents()
    -> Result<(), AvengerChartError> {
        let (compiled_subplot, prepared_inputs) = build_measurement_fixture().await?;
        let FacetPreparedRuntimeInputs {
            mut plan,
            subplot_eval_ctx,
            nested_measure_ctx,
        } = prepared_inputs;
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let is_leaf = is_leaf_subplot(&compiled_subplot);
        let facet_depth = plan
            .cells
            .first()
            .map(|cell| cell.plan.full_path.len() as u8)
            .unwrap_or(0);
        coordinate_cell_domains_before_measurement(
            &mut plan.cells,
            facet_depth,
            &compiled_subplot,
            &nested_measure_ctx,
        )
        .await?;
        measure_overflow_probe(
            FacetAxis::Column,
            facet_depth,
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

        retarget_cells_to_final_plot_area(
            &mut plan.cells,
            140.0,
            140.0,
            &compiled_subplot,
            &subplot_eval_ctx,
            FacetEmptyCellPolicy::Hole,
        )?;

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
    async fn prepared_inputs_runtime_state_aligns_with_cells() -> Result<(), AvengerChartError> {
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
                panic!("expected ready facet node for prepared_inputs pipeline test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_semantics = pipeline.build_cell_semantics(&resolved, &cell_values)?;
        let (prepared_inputs, runtime_state) = prepare_band_inputs_and_runtime(
            cell_semantics.clone(),
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;

        assert_eq!(
            prepared_inputs.cell_semantics.cells.len(),
            runtime_state.data_overrides.len()
        );
        assert_eq!(
            prepared_inputs.cell_semantics.cells.len(),
            prepared_inputs.renderable_mask.len()
        );
        assert_eq!(prepared_inputs.cell_semantics.cell_values, cell_values);
        Ok(())
    }

    #[tokio::test]
    async fn overflow_probe_is_non_mutating() -> Result<(), AvengerChartError> {
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
                panic!("expected ready facet node for overflow_probe pipeline test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_semantics = pipeline.build_cell_semantics(&resolved, &cell_values)?;
        let (prepared_inputs, runtime_state) = prepare_band_inputs_and_runtime(
            cell_semantics,
            &fixture.data_df,
            resolved.compiled_subplot,
            resolved.band_scale,
            resolved.subplot_band_size,
            &fixture.eval_ctx,
        )
        .await?;

        let before_rows = runtime_state.data_overrides.len();
        let (subplot_plot_width, subplot_plot_height) = pipeline
            .axis_ops
            .measure_dims(fixture.plot_other_axis_size, resolved.subplot_band_size);
        let mut perf_counters = FacetPipelinePerfCounters::default();
        let (overflow_probe, _overflow_runtime) = build_overflow_probe(
            &prepared_inputs,
            &runtime_state,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;

        assert_eq!(before_rows, runtime_state.data_overrides.len());
        assert_eq!(
            overflow_probe.overflow_probe_summary.cell_overflows.len(),
            prepared_inputs.cell_semantics.cells.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_layout_finalization_produces_layout_and_measurements()
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
                panic!("expected ready facet node for local_layout pipeline test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_semantics = pipeline.build_cell_semantics(&resolved, &cell_values)?;
        let (prepared_inputs, runtime_state) = prepare_band_inputs_and_runtime(
            cell_semantics,
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
        let (overflow_probe, overflow_runtime) = build_overflow_probe(
            &prepared_inputs,
            &runtime_state,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;
        let (local_layout, measured_runtime) = pipeline
            .build_local_layout(&overflow_probe, overflow_runtime, &runtime_state)
            .await?;

        assert_eq!(
            local_layout.band_layout_plan.n,
            prepared_inputs.cell_semantics.cells.len()
        );
        assert_eq!(
            measured_runtime.measurements.len(),
            prepared_inputs.cell_semantics.cells.len()
        );
        assert_eq!(
            measured_runtime.local_domain_extents.len(),
            prepared_inputs.cell_semantics.cells.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn layout_to_coord_measurement_preserves_axis_and_layout_fields()
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
                panic!("expected ready facet node for coord-measurement pipeline test");
            }
        };
        let cell_values = pipeline.enumerate_cell_values(&resolved);
        let cell_semantics = pipeline.build_cell_semantics(&resolved, &cell_values)?;
        let (prepared_inputs, runtime_state) = prepare_band_inputs_and_runtime(
            cell_semantics,
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
        let (overflow_probe, overflow_runtime) = build_overflow_probe(
            &prepared_inputs,
            &runtime_state,
            subplot_plot_width,
            subplot_plot_height,
            &mut perf_counters,
        )
        .await?;
        let (local_layout, measured_runtime) = pipeline
            .build_local_layout(&overflow_probe, overflow_runtime, &runtime_state)
            .await?;
        let expected_n = local_layout.band_layout_plan.n;
        let expected_size = local_layout.final_subplot_cross_size;
        let measurement =
            pipeline.assemble_coord_measurement(local_layout, measured_runtime, &runtime_state)?;
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
    async fn cell_semantics_match_cell_plan_projection() -> Result<(), AvengerChartError> {
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
        let cell_semantics = pipeline.build_cell_semantics(&resolved, &cell_values)?;
        let projected_plans =
            build_facet_cell_plans(&fixture.eval_ctx.facet_tree, &[], &cell_values)?;

        let cell_semantics_plans: Vec<FacetCellPlan> = cell_semantics
            .cells
            .iter()
            .map(FacetCellPlan::from)
            .collect();
        assert_eq!(cell_semantics_plans.len(), projected_plans.len());
        for (cell_semantics_plan, projected_plan) in
            cell_semantics_plans.iter().zip(projected_plans.iter())
        {
            assert_eq!(cell_semantics_plan.value, projected_plan.value);
            assert_eq!(cell_semantics_plan.full_path, projected_plan.full_path);
            assert_eq!(
                cell_semantics_plan.in_domain_slot,
                projected_plan.in_domain_slot
            );
            assert_eq!(
                cell_semantics_plan.has_data_rows,
                projected_plan.has_data_rows
            );
            assert_eq!(cell_semantics_plan.empty_kind, projected_plan.empty_kind);
            assert_eq!(
                cell_semantics_plan.filter_predicate,
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
            let first_scope = facet_band
                .child_scope_key(0)
                .expect("first facet cell should have a scope key");
            assert!(first_scope.container_path.is_empty());
            assert_eq!(
                &first_scope.child_key,
                &ChildFrameKey::FacetValue {
                    axis,
                    level: 1,
                    value: facet_band.cells[0].plan.value.clone()
                }
            );
        }

        Ok(())
    }
}
