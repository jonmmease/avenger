//! Shared facet overflow projection semantics.
//!
//! Facet measurements keep two overflow values:
//! - `guide`: axis and facet-guide overflow that directly surrounds the plot area.
//! - `total`: guide overflow plus rendered content outside the guide slab, such as legends.
//!
//! Raw measured overflow and coordinated overflow are both represented by
//! `CoordinatedOverflow`. The full value is the rendered subtree envelope:
//! guide overflow plus rendered content outside the guide slab, such as
//! legends and colorbars. Callers should pass that value upward when an
//! ancestor needs to reserve space for everything the subtree renders.
//!
//! When a parent facet allocation already owns some rendered side slabs, the
//! subtree reports only the residual rendered envelope. The residual calculation
//! is expressed as `FrameDemand - OwnedEdgeSlabs` and deliberately subtracts
//! only legend/rendered slabs, never axis or facet-guide slabs.
//!
//! The projections here name narrower questions where a phase intentionally
//! hides part of the full rendered envelope:
//! - `GuideAnchor` keeps the boundary slabs used to anchor facet guides.
//!   Coordinated guide anchors are scoped to visual lanes by the coordination
//!   pass so guides align within a lane without borrowing unrelated sibling
//!   axis chrome.
//! - `SiblingBoundary` keeps only the rendered boundary slabs that can affect
//!   adjacent facet-cell spacing.
//!
//! Fixed plot-area placement additionally needs `FacetBoundaryDemand`: the
//! amount of space a rendered cell requires before and after its plot area along
//! the facet band's main axis.

use crate::{
    coords::{
        CoordMeasurement, CoordinatedLayout, CoordinatedOverflow, FacetAxis,
        OverflowSpaceRequirement,
    },
    facet::coord::{
        FacetBandProbeMeasurement, facet_band_ref as facet_band_from_coord,
        renderable_for_empty_policy,
    },
    facet::layout_plan::effective_edge_indices,
    layout::{
        EdgeDemand, EdgeGrant, EdgeSlabs, Edges, FrameDemand, OwnedEdgeSlabs, Size as LayoutSize,
    },
    plot::compiled::ComponentsMeasurement,
};

/// The unified layout type used for facet band overflow composition.
pub(crate) type UnifiedLayout = avenger_layout::Layout<usize>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetOverflowProjection {
    GuideAnchor { axis: FacetAxis },
    SiblingBoundary { axis: FacetAxis },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetOverflowSource {
    /// Stable local requirements captured during measurement.
    MeasuredLocal,
    /// Current local frame envelope after final guide placement has been rebuilt.
    RealizedLocal,
    /// Purpose-specific coordination contract for aligned guide anchors and sibling gaps.
    Coordinated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetOverflowResolutionPhase {
    /// Collect local requirements from measured or probe geometry.
    Measurement,
    /// Realize final geometry from the coordinated contract when available.
    Final,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetOverflowPurpose {
    /// Full rendered subtree envelope that an ancestor must reserve.
    RenderedSubtree,
    /// Boundary slabs used to anchor facet guide labels/titles.
    GuideAnchor { axis: FacetAxis },
    /// Boundary slabs used to size gaps between sibling facet cells.
    SiblingBoundary { axis: FacetAxis },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetOverflowResolvedSource {
    MeasuredLocal,
    RealizedLocal,
    Coordinated,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedFacetOverflow {
    pub(crate) overflow: CoordinatedOverflow,
    pub(crate) source: FacetOverflowResolvedSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FacetBandNoRenderablePolicy {
    DefaultOverflow,
    #[cfg(test)]
    FirstCell,
}

impl FacetBandNoRenderablePolicy {
    fn use_first_cell_when_none_renderable(self) -> bool {
        match self {
            Self::DefaultOverflow => false,
            #[cfg(test)]
            Self::FirstCell => true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FacetOverflowSlabs {
    pub guide: OverflowSpaceRequirement,
    pub legend: OverflowSpaceRequirement,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetCellOverflowInput {
    pub(crate) renderable: bool,
    pub(crate) guide: OverflowSpaceRequirement,
    pub(crate) total: OverflowSpaceRequirement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FacetBoundaryDemand {
    pub(crate) before: f32,
    pub(crate) after: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FacetBoundaryDemandComponents {
    pub(crate) guide: FacetBoundaryDemand,
    pub(crate) total: FacetBoundaryDemand,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FacetBoundaryLane {
    pub(crate) lane_key: Vec<usize>,
    pub(crate) guide: FacetBoundaryDemand,
    pub(crate) total: FacetBoundaryDemand,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FacetAxisBoundaryProfile {
    pub(crate) lanes: Vec<FacetBoundaryLane>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FacetBoundaryProfiles {
    pub(crate) row: FacetAxisBoundaryProfile,
    pub(crate) column: FacetAxisBoundaryProfile,
}

impl FacetAxisBoundaryProfile {
    fn push_lane(&mut self, lane: FacetBoundaryLane) {
        if let Some(existing) = self
            .lanes
            .iter_mut()
            .find(|existing| existing.lane_key == lane.lane_key)
        {
            existing.guide.before = existing.guide.before.max(lane.guide.before);
            existing.guide.after = existing.guide.after.max(lane.guide.after);
            existing.total.before = existing.total.before.max(lane.total.before);
            existing.total.after = existing.total.after.max(lane.total.after);
        } else {
            self.lanes.push(lane);
        }
    }

    fn push_lanes_with_prefix(&mut self, prefix: usize, profile: &Self) {
        for lane in &profile.lanes {
            let mut lane_key = Vec::with_capacity(lane.lane_key.len() + 1);
            lane_key.push(prefix);
            lane_key.extend(lane.lane_key.iter().cloned());
            self.push_lane(FacetBoundaryLane {
                lane_key,
                guide: lane.guide,
                total: lane.total,
            });
        }
    }

    fn add_uniform_extra(&mut self, guide: FacetBoundaryDemand, total: FacetBoundaryDemand) {
        if self.lanes.is_empty()
            && (guide.before > 0.0 || guide.after > 0.0 || total.before > 0.0 || total.after > 0.0)
        {
            self.lanes.push(FacetBoundaryLane::default());
        }

        for lane in &mut self.lanes {
            lane.guide.before += guide.before;
            lane.guide.after += guide.after;
            lane.total.before += total.before;
            lane.total.after += total.after;
            lane.total.before = lane.total.before.max(lane.guide.before);
            lane.total.after = lane.total.after.max(lane.guide.after);
        }
    }
}

impl FacetBoundaryProfiles {
    pub(crate) fn single_from_overflow(
        guide: OverflowSpaceRequirement,
        total: OverflowSpaceRequirement,
    ) -> Self {
        Self {
            row: FacetAxisBoundaryProfile {
                lanes: vec![FacetBoundaryLane {
                    lane_key: Vec::new(),
                    guide: FacetBoundaryDemand {
                        before: guide.top.max(0.0),
                        after: guide.bottom.max(0.0),
                    },
                    total: FacetBoundaryDemand {
                        before: total.top.max(0.0),
                        after: total.bottom.max(0.0),
                    },
                }],
            },
            column: FacetAxisBoundaryProfile {
                lanes: vec![FacetBoundaryLane {
                    lane_key: Vec::new(),
                    guide: FacetBoundaryDemand {
                        before: guide.left.max(0.0),
                        after: guide.right.max(0.0),
                    },
                    total: FacetBoundaryDemand {
                        before: total.left.max(0.0),
                        after: total.right.max(0.0),
                    },
                }],
            },
        }
    }

    pub(crate) fn profile(&self, axis: FacetAxis) -> &FacetAxisBoundaryProfile {
        match axis {
            FacetAxis::Row => &self.row,
            FacetAxis::Column => &self.column,
        }
    }

    fn to_overflow(&self) -> CoordinatedOverflow {
        let mut guide = OverflowSpaceRequirement::default();
        let mut total = OverflowSpaceRequirement::default();
        for lane in &self.row.lanes {
            guide.top = guide.top.max(lane.guide.before);
            guide.bottom = guide.bottom.max(lane.guide.after);
            total.top = total.top.max(lane.total.before);
            total.bottom = total.bottom.max(lane.total.after);
        }
        for lane in &self.column.lanes {
            guide.left = guide.left.max(lane.guide.before);
            guide.right = guide.right.max(lane.guide.after);
            total.left = total.left.max(lane.total.before);
            total.right = total.right.max(lane.total.after);
        }
        CoordinatedOverflow { guide, total }
    }

    fn add_direct_extra_from_layout(
        &mut self,
        layout_guide: &OverflowSpaceRequirement,
        layout_total: &OverflowSpaceRequirement,
    ) {
        let child = self.to_overflow();
        self.row.add_uniform_extra(
            FacetBoundaryDemand {
                before: (layout_guide.top - child.guide.top).max(0.0),
                after: (layout_guide.bottom - child.guide.bottom).max(0.0),
            },
            FacetBoundaryDemand {
                before: (layout_total.top - child.total.top).max(0.0),
                after: (layout_total.bottom - child.total.bottom).max(0.0),
            },
        );
        self.column.add_uniform_extra(
            FacetBoundaryDemand {
                before: (layout_guide.left - child.guide.left).max(0.0),
                after: (layout_guide.right - child.guide.right).max(0.0),
            },
            FacetBoundaryDemand {
                before: (layout_total.left - child.total.left).max(0.0),
                after: (layout_total.right - child.total.right).max(0.0),
            },
        );
    }
}

impl FacetOverflowSlabs {
    pub(crate) fn from_coordinated(overflow: &CoordinatedOverflow) -> Self {
        let legend = OverflowSpaceRequirement {
            top: (overflow.total.top - overflow.guide.top).max(0.0),
            right: (overflow.total.right - overflow.guide.right).max(0.0),
            bottom: (overflow.total.bottom - overflow.guide.bottom).max(0.0),
            left: (overflow.total.left - overflow.guide.left).max(0.0),
        };
        Self {
            guide: overflow.guide.clone(),
            legend,
        }
    }

    #[inline]
    pub(crate) fn legend_vertical(&self) -> (f32, f32) {
        (self.legend.top, self.legend.bottom)
    }

    #[inline]
    pub(crate) fn legend_horizontal(&self) -> (f32, f32) {
        (self.legend.left, self.legend.right)
    }
}

pub(crate) fn project_facet_overflow(
    overflow: &CoordinatedOverflow,
    projection: FacetOverflowProjection,
) -> CoordinatedOverflow {
    match projection {
        FacetOverflowProjection::GuideAnchor { axis }
        | FacetOverflowProjection::SiblingBoundary { axis } => {
            sibling_boundary_overflow(axis, overflow)
        }
    }
}

pub(crate) fn resolve_facet_overflow(
    measurement: &dyn CoordMeasurement,
    phase: FacetOverflowResolutionPhase,
    purpose: FacetOverflowPurpose,
) -> Option<ResolvedFacetOverflow> {
    match phase {
        FacetOverflowResolutionPhase::Measurement => resolve_facet_overflow_from_source(
            measurement,
            FacetOverflowSource::MeasuredLocal,
            FacetOverflowResolvedSource::MeasuredLocal,
            purpose,
        ),
        FacetOverflowResolutionPhase::Final => match purpose {
            FacetOverflowPurpose::RenderedSubtree => resolve_facet_overflow_from_source(
                measurement,
                FacetOverflowSource::RealizedLocal,
                FacetOverflowResolvedSource::RealizedLocal,
                purpose,
            )
            .or_else(|| {
                resolve_facet_overflow_from_source(
                    measurement,
                    FacetOverflowSource::MeasuredLocal,
                    FacetOverflowResolvedSource::MeasuredLocal,
                    purpose,
                )
            }),
            FacetOverflowPurpose::GuideAnchor { .. }
            | FacetOverflowPurpose::SiblingBoundary { .. } => resolve_facet_overflow_from_source(
                measurement,
                FacetOverflowSource::Coordinated,
                FacetOverflowResolvedSource::Coordinated,
                purpose,
            )
            .or_else(|| {
                resolve_facet_overflow_from_source(
                    measurement,
                    FacetOverflowSource::MeasuredLocal,
                    FacetOverflowResolvedSource::MeasuredLocal,
                    purpose,
                )
            }),
        },
    }
}

fn resolved_rendered_subtree_overflow(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
) -> Option<CoordinatedOverflow> {
    match source {
        FacetOverflowSource::MeasuredLocal => {
            rendered_subtree_overflow_from_coord_measurement(measurement, source)
        }
        FacetOverflowSource::RealizedLocal => {
            realized_rendered_subtree_overflow_from_coord_measurement(measurement)
        }
        FacetOverflowSource::Coordinated => {
            rendered_subtree_overflow_from_coord_measurement(measurement, source)
        }
    }
}

fn resolve_facet_overflow_from_source(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
    resolved_source: FacetOverflowResolvedSource,
    purpose: FacetOverflowPurpose,
) -> Option<ResolvedFacetOverflow> {
    let overflow = match purpose {
        FacetOverflowPurpose::RenderedSubtree => {
            resolved_rendered_subtree_overflow(measurement, source)
        }
        FacetOverflowPurpose::GuideAnchor { axis } => {
            resolve_guide_anchor_overflow(measurement, source, axis)
        }
        FacetOverflowPurpose::SiblingBoundary { axis } => {
            resolve_sibling_boundary_overflow(measurement, source, axis)
        }
    }?;

    Some(ResolvedFacetOverflow {
        overflow,
        source: resolved_source,
    })
}

fn resolve_projected_facet_overflow(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
    fallback_axis: FacetAxis,
    projection: impl FnOnce(FacetAxis) -> FacetOverflowProjection,
) -> Option<CoordinatedOverflow> {
    if let Some((axis, overflow)) = facet_measurement_overflow(measurement, source) {
        return Some(project_facet_overflow(&overflow, projection(axis)));
    }

    match source {
        FacetOverflowSource::MeasuredLocal | FacetOverflowSource::RealizedLocal => None,
        FacetOverflowSource::Coordinated => measurement
            .coordinated_overflow()
            .map(|overflow| project_facet_overflow(overflow, projection(fallback_axis))),
    }
}

fn resolve_guide_anchor_overflow(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
    fallback_axis: FacetAxis,
) -> Option<CoordinatedOverflow> {
    if matches!(source, FacetOverflowSource::Coordinated)
        && let Some(facet_measurement) = facet_band_from_coord(measurement)
        && let Some(guide_anchor_overflow) =
            facet_measurement.coordinated_guide_anchor_overflow_value()
    {
        return Some(guide_anchor_overflow.clone());
    }

    resolve_projected_facet_overflow(measurement, source, fallback_axis, |axis| {
        FacetOverflowProjection::GuideAnchor { axis }
    })
}

fn resolve_sibling_boundary_overflow(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
    fallback_axis: FacetAxis,
) -> Option<CoordinatedOverflow> {
    if matches!(source, FacetOverflowSource::Coordinated)
        && let Some(facet_measurement) = facet_band_from_coord(measurement)
        && let Some(boundary_overflow) = facet_measurement.coordinated_boundary_overflow_value()
    {
        return Some(project_facet_overflow(
            boundary_overflow,
            FacetOverflowProjection::SiblingBoundary {
                axis: facet_measurement.axis,
            },
        ));
    }

    resolve_projected_facet_overflow(measurement, source, fallback_axis, |axis| {
        FacetOverflowProjection::SiblingBoundary { axis }
    })
}

/// Build the 1xN (Column) / Nx1 (Row) layout for a facet band's overflow:
/// cells as measured leaves (guide as inner, legend as outer), plus the
/// band's own chrome (labels/titles on the inner layer, band-level legends
/// on the outer layer) declared on the band itself.
pub(crate) fn band_overflow_node(
    axis: FacetAxis,
    cells: &[(OverflowSpaceRequirement, OverflowSpaceRequirement)],
    stacked_inner_edges: Edges<f32>,
    stacked_outer_edges: Edges<f32>,
) -> UnifiedLayout {
    use avenger_layout::Side;
    let leaves = cells.iter().map(|(guide, total)| {
        let mut leaf = UnifiedLayout::leaf(LayoutSize::default());
        for (side, guide_value, total_value) in [
            (Side::Top, guide.top, total.top),
            (Side::Right, guide.right, total.right),
            (Side::Bottom, guide.bottom, total.bottom),
            (Side::Left, guide.left, total.left),
        ] {
            leaf = leaf.demand(
                side,
                EdgeDemand::new(
                    guide_value,
                    (total_value - guide_value).max(0.0),
                    total_value,
                ),
            );
        }
        leaf
    });
    let mut band = match axis {
        FacetAxis::Column => UnifiedLayout::row(leaves),
        FacetAxis::Row => UnifiedLayout::column(leaves),
    };
    for (side, inner, outer) in [
        (
            avenger_layout::Side::Top,
            stacked_inner_edges.top,
            stacked_outer_edges.top,
        ),
        (
            avenger_layout::Side::Right,
            stacked_inner_edges.right,
            stacked_outer_edges.right,
        ),
        (
            avenger_layout::Side::Bottom,
            stacked_inner_edges.bottom,
            stacked_outer_edges.bottom,
        ),
        (
            avenger_layout::Side::Left,
            stacked_inner_edges.left,
            stacked_outer_edges.left,
        ),
    ] {
        band = band.inner(side, inner).outer(side, outer);
    }
    band
}

/// Measured envelope of a band layout as a coordinated overflow value:
/// guide from the layered inner layer, total from the geometric (raw
/// rendered) view.
pub(crate) fn band_node_envelope(node: &UnifiedLayout) -> CoordinatedOverflow {
    let solved = node
        .solve(&avenger_layout::SolveOptions::default())
        .expect("facet band leaves are single-span and indexed within the band shape");
    let envelope = solved.envelope();
    CoordinatedOverflow {
        guide: OverflowSpaceRequirement {
            top: envelope.layered.top.inner,
            right: envelope.layered.right.inner,
            bottom: envelope.layered.bottom.inner,
            left: envelope.layered.left.inner,
        },
        total: OverflowSpaceRequirement {
            top: envelope.geometric_total.top,
            right: envelope.geometric_total.right,
            bottom: envelope.geometric_total.bottom,
            left: envelope.geometric_total.left,
        },
    }
}

pub(crate) fn aggregate_facet_band_overflow(
    axis: FacetAxis,
    cells: &[FacetCellOverflowInput],
) -> Option<CoordinatedOverflow> {
    aggregate_facet_band_overflow_with_policy(
        axis,
        cells,
        FacetBandNoRenderablePolicy::DefaultOverflow,
    )
}

pub(crate) fn aggregate_facet_band_overflow_with_policy(
    axis: FacetAxis,
    cells: &[FacetCellOverflowInput],
    no_renderable_policy: FacetBandNoRenderablePolicy,
) -> Option<CoordinatedOverflow> {
    if cells.is_empty() {
        return None;
    }

    let mut renderable_indices = cells
        .iter()
        .enumerate()
        .filter_map(|(idx, cell)| cell.renderable.then_some(idx))
        .collect::<Vec<_>>();
    if renderable_indices.is_empty() {
        if no_renderable_policy.use_first_cell_when_none_renderable() {
            renderable_indices.push(0);
        } else {
            return Some(CoordinatedOverflow::default());
        }
    }

    // Renderable filtering and the no-renderable policy are chart policy
    // (applied above); the first/last-vs-max envelope math itself is the
    // measured tree envelope: each renderable cell becomes a leaf of a
    // 1xN (Column) or Nx1 (Row) layout node, and the per-side aggregation
    // falls out of the slot edge roles.
    let cell_envelopes = renderable_indices
        .iter()
        .map(|&cell_index| {
            let cell = &cells[cell_index];
            (cell.guide.clone(), cell.total.clone())
        })
        .collect::<Vec<_>>();
    let node = band_overflow_node(axis, &cell_envelopes, Edges::default(), Edges::default());
    Some(band_node_envelope(&node))
}

/// Per-side layered demand view of a coordinated overflow: guide chrome is
/// `inner`, legend chrome is `outer`.
pub(crate) fn overflow_edge_demands(overflow: &CoordinatedOverflow) -> Edges<EdgeGrant> {
    let side = |guide: f32, total: f32| EdgeGrant::new(guide, (total - guide).max(0.0), total);
    Edges::new(
        side(overflow.guide.top, overflow.total.top),
        side(overflow.guide.right, overflow.total.right),
        side(overflow.guide.bottom, overflow.total.bottom),
        side(overflow.guide.left, overflow.total.left),
    )
}

pub(crate) fn overflow_from_edge_demands(demands: Edges<EdgeGrant>) -> CoordinatedOverflow {
    CoordinatedOverflow {
        guide: OverflowSpaceRequirement {
            top: demands.top.inner,
            right: demands.right.inner,
            bottom: demands.bottom.inner,
            left: demands.left.inner,
        },
        total: OverflowSpaceRequirement {
            top: demands.top.total,
            right: demands.right.total,
            bottom: demands.bottom.total,
            left: demands.left.total,
        },
    }
}

pub(crate) fn rendered_subtree_overflow_from_coord_measurement(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
) -> Option<CoordinatedOverflow> {
    match source {
        FacetOverflowSource::MeasuredLocal => {
            if let Some(facet_measurement) = facet_band_from_coord(measurement) {
                let active_layout = facet_measurement.active_layout();
                return facet_measurement.measured_overflow_value().map(|overflow| {
                    rendered_subtree_overflow_for_facet_band(
                        facet_measurement,
                        &overflow,
                        active_layout,
                    )
                });
            }
            measurement
                .as_any()
                .downcast_ref::<FacetBandProbeMeasurement>()
                .map(|facet_measurement| {
                    let owned_slabs = facet_measurement.owned_legend_slabs_for_overflow(
                        &facet_measurement.measured_overflow,
                        &facet_measurement.local_layout,
                    );
                    residual_overflow_for_owned_legend_slabs(
                        &facet_measurement.measured_overflow,
                        owned_slabs,
                    )
                })
        }
        FacetOverflowSource::RealizedLocal => {
            realized_rendered_subtree_overflow_from_coord_measurement(measurement)
        }
        FacetOverflowSource::Coordinated => {
            if let Some(facet_measurement) = facet_band_from_coord(measurement) {
                let active_layout = facet_measurement.active_layout();
                return Some(rendered_subtree_overflow_for_facet_band(
                    facet_measurement,
                    facet_measurement.active_overflow(),
                    active_layout,
                ));
            }
            measurement.coordinated_overflow().cloned()
        }
    }
}

fn realized_rendered_subtree_overflow_from_coord_measurement(
    measurement: &dyn CoordMeasurement,
) -> Option<CoordinatedOverflow> {
    let facet_measurement = facet_band_from_coord(measurement)?;
    let active_layout = facet_measurement.active_layout();
    let overflow = realized_facet_band_overflow(facet_measurement)?;
    Some(rendered_subtree_overflow_for_facet_band(
        facet_measurement,
        &overflow,
        active_layout,
    ))
}

fn realized_facet_band_overflow(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
) -> Option<CoordinatedOverflow> {
    let overflow_inputs = facet_measurement
        .cells
        .iter()
        .map(|cell| {
            let overflow = realized_cell_overflow(&cell.measurement);
            FacetCellOverflowInput {
                renderable: renderable_for_empty_policy(
                    facet_measurement.empty_cell_policy,
                    !cell.plan.has_data_rows,
                ),
                guide: overflow.guide,
                total: overflow.total,
            }
        })
        .collect::<Vec<_>>();
    aggregate_facet_band_overflow(facet_measurement.axis, &overflow_inputs)
}

fn realized_cell_overflow(measurement: &ComponentsMeasurement) -> CoordinatedOverflow {
    let mut overflow = CoordinatedOverflow {
        guide: measurement.layout.overflow.clone(),
        total: measurement.layout.total_overflow.clone(),
    };

    if let Some(facet_measurement) = facet_band_from_coord(measurement.coord_measurement.as_ref())
        && let Some(realized) = realized_facet_band_overflow(facet_measurement)
    {
        let active_layout = facet_measurement.active_layout();
        let rendered =
            rendered_subtree_overflow_for_facet_band(facet_measurement, &realized, active_layout);
        overflow.guide = overflow.guide.max_components(&rendered.guide);
        overflow.total = overflow.total.max_components(&rendered.total);
    }

    overflow
}

pub(crate) fn rendered_boundary_demand_for_measurement(
    axis: FacetAxis,
    measurement: &ComponentsMeasurement,
) -> FacetBoundaryDemand {
    realized_boundary_demand_components_for_measurement(axis, measurement).total
}

pub(crate) fn realized_boundary_demand_components_for_measurement(
    axis: FacetAxis,
    measurement: &ComponentsMeasurement,
) -> FacetBoundaryDemandComponents {
    let overflow = realized_sibling_boundary_overflow_for_measurement(measurement);
    boundary_demand_components_from_overflow(axis, &overflow)
}

pub(crate) fn boundary_profiles_for_measurement(
    measurement: &ComponentsMeasurement,
) -> FacetBoundaryProfiles {
    let Some(facet_measurement) = facet_band_from_coord(measurement.coord_measurement.as_ref())
    else {
        return FacetBoundaryProfiles::single_from_overflow(
            measurement.layout.overflow.clone(),
            measurement.layout.total_overflow.clone(),
        );
    };

    let mut profiles = boundary_profiles_for_facet_band(facet_measurement);
    profiles.add_direct_extra_from_layout(
        &measurement.layout.overflow,
        &measurement.layout.total_overflow,
    );
    profiles
}

pub(crate) fn compute_padding_from_boundary_profiles(
    axis: FacetAxis,
    profiles: &[FacetBoundaryProfiles],
    renderable_cells: &[bool],
    include_total_overflow: bool,
) -> Option<f32> {
    let Some((first_renderable_idx, last_renderable_idx)) =
        effective_edge_indices(renderable_cells, profiles.len())
    else {
        return Some(0.0);
    };

    if first_renderable_idx >= last_renderable_idx {
        return Some(0.0);
    }

    let renderable_indices = (first_renderable_idx..=last_renderable_idx)
        .filter(|idx| renderable_cells.get(*idx).copied().unwrap_or(false))
        .collect::<Vec<_>>();
    let mut padding_max = 0.0f32;
    let mut saw_lane = false;

    for boundary in renderable_indices.windows(2) {
        let [before_idx, after_idx] = boundary else {
            continue;
        };
        let Some(before_profile) = profiles
            .get(*before_idx)
            .map(|profiles| profiles.profile(axis))
        else {
            continue;
        };
        let Some(after_profile) = profiles
            .get(*after_idx)
            .map(|profiles| profiles.profile(axis))
        else {
            continue;
        };

        for before_lane in &before_profile.lanes {
            saw_lane = true;
            let outgoing = boundary_after(before_lane, include_total_overflow);
            let matching_after = after_profile
                .lanes
                .iter()
                .find(|after_lane| after_lane.lane_key == before_lane.lane_key);
            let boundary_padding = if let Some(after_lane) = matching_after {
                outgoing + boundary_before(after_lane, include_total_overflow)
            } else {
                outgoing
            };
            padding_max = padding_max.max(boundary_padding);
        }

        for after_lane in &after_profile.lanes {
            saw_lane = true;
            if before_profile
                .lanes
                .iter()
                .any(|before_lane| before_lane.lane_key == after_lane.lane_key)
            {
                continue;
            }
            padding_max = padding_max.max(boundary_before(after_lane, include_total_overflow));
        }
    }

    saw_lane.then_some(padding_max)
}

fn boundary_before(lane: &FacetBoundaryLane, include_total_overflow: bool) -> f32 {
    if include_total_overflow {
        lane.total.before
    } else {
        lane.guide.before
    }
}

fn boundary_after(lane: &FacetBoundaryLane, include_total_overflow: bool) -> f32 {
    if include_total_overflow {
        lane.total.after
    } else {
        lane.guide.after
    }
}

fn boundary_profiles_for_facet_band(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
) -> FacetBoundaryProfiles {
    FacetBoundaryProfiles {
        row: boundary_profile_for_facet_band_axis(facet_measurement, FacetAxis::Row),
        column: boundary_profile_for_facet_band_axis(facet_measurement, FacetAxis::Column),
    }
}

fn boundary_profile_for_facet_band_axis(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
    query_axis: FacetAxis,
) -> FacetAxisBoundaryProfile {
    let renderable_cells = facet_measurement
        .cells
        .iter()
        .map(|cell| {
            renderable_for_empty_policy(
                facet_measurement.empty_cell_policy,
                !cell.plan.has_data_rows,
            )
        })
        .collect::<Vec<_>>();

    if facet_measurement.axis == query_axis {
        return same_axis_boundary_profile(facet_measurement, query_axis, &renderable_cells);
    }

    orthogonal_axis_boundary_profile(facet_measurement, query_axis, &renderable_cells)
}

fn same_axis_boundary_profile(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
    query_axis: FacetAxis,
    renderable_cells: &[bool],
) -> FacetAxisBoundaryProfile {
    let Some((first_idx, last_idx)) =
        effective_edge_indices(renderable_cells, facet_measurement.cells.len())
    else {
        return FacetAxisBoundaryProfile::default();
    };

    let mut profile = FacetAxisBoundaryProfile::default();
    if let Some(first_cell) = facet_measurement.cells.get(first_idx)
        && renderable_cells.get(first_idx).copied().unwrap_or(false)
    {
        let child_profiles = boundary_profiles_for_measurement(&first_cell.measurement);
        for lane in &child_profiles.profile(query_axis).lanes {
            profile.push_lane(FacetBoundaryLane {
                lane_key: lane.lane_key.clone(),
                guide: FacetBoundaryDemand {
                    before: lane.guide.before,
                    after: 0.0,
                },
                total: FacetBoundaryDemand {
                    before: lane.total.before,
                    after: 0.0,
                },
            });
        }
    }

    if let Some(last_cell) = facet_measurement.cells.get(last_idx)
        && renderable_cells.get(last_idx).copied().unwrap_or(false)
    {
        let child_profiles = boundary_profiles_for_measurement(&last_cell.measurement);
        for lane in &child_profiles.profile(query_axis).lanes {
            profile.push_lane(FacetBoundaryLane {
                lane_key: lane.lane_key.clone(),
                guide: FacetBoundaryDemand {
                    before: 0.0,
                    after: lane.guide.after,
                },
                total: FacetBoundaryDemand {
                    before: 0.0,
                    after: lane.total.after,
                },
            });
        }
    }

    profile
}

fn orthogonal_axis_boundary_profile(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
    query_axis: FacetAxis,
    renderable_cells: &[bool],
) -> FacetAxisBoundaryProfile {
    let mut profile = FacetAxisBoundaryProfile::default();
    for (idx, cell) in facet_measurement.cells.iter().enumerate() {
        if !renderable_cells.get(idx).copied().unwrap_or(false) {
            continue;
        }
        let child_profiles = boundary_profiles_for_measurement(&cell.measurement);
        profile.push_lanes_with_prefix(idx, child_profiles.profile(query_axis));
    }
    profile
}

fn boundary_demand_components_from_overflow(
    axis: FacetAxis,
    overflow: &CoordinatedOverflow,
) -> FacetBoundaryDemandComponents {
    let guide = &overflow.guide;
    let total = &overflow.total;

    match axis {
        FacetAxis::Column => FacetBoundaryDemandComponents {
            guide: FacetBoundaryDemand {
                before: guide.left.max(0.0),
                after: guide.right.max(0.0),
            },
            total: FacetBoundaryDemand {
                before: total.left.max(0.0),
                after: total.right.max(0.0),
            },
        },
        FacetAxis::Row => FacetBoundaryDemandComponents {
            guide: FacetBoundaryDemand {
                before: guide.top.max(0.0),
                after: guide.bottom.max(0.0),
            },
            total: FacetBoundaryDemand {
                before: total.top.max(0.0),
                after: total.bottom.max(0.0),
            },
        },
    }
}

fn realized_sibling_boundary_overflow_for_measurement(
    measurement: &ComponentsMeasurement,
) -> CoordinatedOverflow {
    let mut overflow = CoordinatedOverflow {
        guide: measurement.layout.overflow.clone(),
        total: measurement.layout.total_overflow.clone(),
    };

    if let Some(facet_band) = facet_band_from_coord(measurement.coord_measurement.as_ref())
        && let Some(realized) = realized_facet_band_overflow(facet_band)
    {
        let boundary = project_facet_overflow(
            &realized,
            FacetOverflowProjection::SiblingBoundary {
                axis: facet_band.axis,
            },
        );
        overflow.guide = overflow.guide.max_components(&boundary.guide);
        overflow.total = overflow.total.max_components(&boundary.total);
    }

    overflow
}

fn sibling_boundary_overflow(
    axis: FacetAxis,
    overflow: &CoordinatedOverflow,
) -> CoordinatedOverflow {
    let mut total = overflow.total.clone();
    match axis {
        FacetAxis::Column => {
            total.left = overflow.total.left;
            total.right = overflow.total.right;
            total.top = overflow.guide.top;
        }
        FacetAxis::Row => {
            total.top = overflow.total.top;
            total.bottom = overflow.total.bottom;
            total.left = overflow.guide.left;
        }
    }
    CoordinatedOverflow {
        guide: overflow.guide.clone(),
        total,
    }
}

fn residual_overflow_for_owned_legend_slabs(
    overflow: &CoordinatedOverflow,
    owned_legend_slabs: OwnedEdgeSlabs,
) -> CoordinatedOverflow {
    let demand = FrameDemand::from_guide_and_rendered_envelope(
        overflow.guide.clone(),
        overflow.total.clone(),
    );
    CoordinatedOverflow {
        guide: overflow.guide.clone(),
        total: demand
            .residual_overflow_after_owned_legend_slabs(owned_legend_slabs)
            .into(),
    }
}

fn rendered_subtree_overflow_for_facet_band(
    facet_measurement: &crate::facet::coord::FacetBandCoordMeasurement,
    overflow: &CoordinatedOverflow,
    layout: &CoordinatedLayout,
) -> CoordinatedOverflow {
    let owned_slabs = facet_measurement.owned_legend_slabs_for_overflow(overflow, layout);

    if owned_slabs == EdgeSlabs::default() {
        return overflow.clone();
    }

    residual_overflow_for_owned_legend_slabs(overflow, owned_slabs)
}

fn facet_measurement_overflow(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
) -> Option<(FacetAxis, CoordinatedOverflow)> {
    match source {
        FacetOverflowSource::MeasuredLocal => {
            if let Some(facet_measurement) = facet_band_from_coord(measurement) {
                return facet_measurement
                    .measured_overflow_value()
                    .map(|overflow| (facet_measurement.axis, overflow));
            }
            measurement
                .as_any()
                .downcast_ref::<FacetBandProbeMeasurement>()
                .map(|facet_measurement| {
                    (
                        facet_measurement.axis,
                        facet_measurement.measured_overflow.clone(),
                    )
                })
        }
        FacetOverflowSource::RealizedLocal => {
            facet_band_from_coord(measurement).and_then(|facet_measurement| {
                realized_facet_band_overflow(facet_measurement)
                    .map(|overflow| (facet_measurement.axis, overflow))
            })
        }
        FacetOverflowSource::Coordinated => {
            facet_band_from_coord(measurement).map(|facet_measurement| {
                (
                    facet_measurement.axis,
                    facet_measurement.active_overflow().clone(),
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The aggregation IS the measured tree envelope (since Phase 9a), so
    /// equality must hold even in the mixed-dominance case where the
    /// layered law diverges.
    #[test]
    fn aggregation_equals_measured_envelope_on_mixed_dominance() {
        // Cell 0 is all guide on top (total 10); cell 1 is all legend on
        // top (total 8). Measured envelope: 10. Layered would lift to 18.
        let cells = [
            FacetCellOverflowInput {
                renderable: true,
                guide: OverflowSpaceRequirement {
                    top: 10.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
                total: OverflowSpaceRequirement {
                    top: 10.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            },
            FacetCellOverflowInput {
                renderable: true,
                guide: OverflowSpaceRequirement::default(),
                total: OverflowSpaceRequirement {
                    top: 8.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            },
        ];

        let aggregated = aggregate_facet_band_overflow_with_policy(
            FacetAxis::Column,
            &cells,
            FacetBandNoRenderablePolicy::DefaultOverflow,
        )
        .expect("non-empty cells should aggregate");

        assert_eq!(aggregated.guide.top, 10.0);
        assert_eq!(
            aggregated.total.top, 10.0,
            "measured envelope keeps the raw max total; the layered law would report 18"
        );
    }

    /// Acceptance test for the avenger-layout tree sweeps: the facet band's
    /// first/last-vs-max edge aggregation falls out of grid edge math when
    /// cells are leaves of a 1xN (Column) or Nx1 (Row) layout tree.
    ///
    /// The comparison holds when one cell dominates each side in both the
    /// guide and legend layers (as here). In the mixed case the two computed
    /// values intentionally diverge: this aggregation reports the measured
    /// envelope (max of per-cell totals), while the tree envelope reports
    /// the layered coordination view (max guide + max legend), which is what
    /// cells physically occupy after overflow patches are applied.
    #[test]
    fn tree_envelope_matches_facet_band_overflow_aggregation() {
        let cell_inputs = [
            FacetCellOverflowInput {
                renderable: true,
                guide: OverflowSpaceRequirement {
                    top: 4.0,
                    right: 9.0,
                    bottom: 1.0,
                    left: 15.0,
                },
                total: OverflowSpaceRequirement {
                    top: 6.0,
                    right: 12.0,
                    bottom: 2.0,
                    left: 18.0,
                },
            },
            FacetCellOverflowInput {
                renderable: true,
                guide: OverflowSpaceRequirement {
                    top: 7.0,
                    right: 3.0,
                    bottom: 8.0,
                    left: 2.0,
                },
                total: OverflowSpaceRequirement {
                    top: 10.0,
                    right: 5.0,
                    bottom: 11.0,
                    left: 3.0,
                },
            },
        ];

        for axis in [FacetAxis::Column, FacetAxis::Row] {
            let aggregated = aggregate_facet_band_overflow_with_policy(
                axis,
                &cell_inputs,
                FacetBandNoRenderablePolicy::DefaultOverflow,
            )
            .expect("non-empty cells should aggregate");

            let cells = cell_inputs
                .iter()
                .map(|cell| (cell.guide.clone(), cell.total.clone()))
                .collect::<Vec<_>>();
            let band = band_overflow_node(axis, &cells, Edges::default(), Edges::default());
            let solved = band
                .solve(&avenger_layout::SolveOptions::default())
                .expect("facet band layout should solve");
            let envelope = solved.envelope();

            assert_eq!(envelope.layered.top.inner, aggregated.guide.top, "{axis:?}");
            assert_eq!(
                envelope.layered.right.inner, aggregated.guide.right,
                "{axis:?}"
            );
            assert_eq!(
                envelope.layered.bottom.inner, aggregated.guide.bottom,
                "{axis:?}"
            );
            assert_eq!(
                envelope.layered.left.inner, aggregated.guide.left,
                "{axis:?}"
            );
            assert_eq!(envelope.layered.top.total, aggregated.total.top, "{axis:?}");
            assert_eq!(
                envelope.layered.right.total, aggregated.total.right,
                "{axis:?}"
            );
            assert_eq!(
                envelope.layered.bottom.total, aggregated.total.bottom,
                "{axis:?}"
            );
            assert_eq!(
                envelope.layered.left.total, aggregated.total.left,
                "{axis:?}"
            );
        }
    }

    fn overflow(guide: (f32, f32, f32, f32), total: (f32, f32, f32, f32)) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: guide.0,
                right: guide.1,
                bottom: guide.2,
                left: guide.3,
            },
            total: OverflowSpaceRequirement {
                top: total.0,
                right: total.1,
                bottom: total.2,
                left: total.3,
            },
        }
    }

    fn cell(
        renderable: bool,
        guide: (f32, f32, f32, f32),
        total: (f32, f32, f32, f32),
    ) -> FacetCellOverflowInput {
        FacetCellOverflowInput {
            renderable,
            guide: OverflowSpaceRequirement {
                top: guide.0,
                right: guide.1,
                bottom: guide.2,
                left: guide.3,
            },
            total: OverflowSpaceRequirement {
                top: total.0,
                right: total.1,
                bottom: total.2,
                left: total.3,
            },
        }
    }

    fn lane_key(slots: &[usize]) -> Vec<usize> {
        slots.to_vec()
    }

    fn row_profile_lane(
        lane_key: Vec<usize>,
        guide_before: f32,
        guide_after: f32,
        total_before: f32,
        total_after: f32,
    ) -> FacetBoundaryProfiles {
        FacetBoundaryProfiles {
            row: FacetAxisBoundaryProfile {
                lanes: vec![FacetBoundaryLane {
                    lane_key,
                    guide: FacetBoundaryDemand {
                        before: guide_before,
                        after: guide_after,
                    },
                    total: FacetBoundaryDemand {
                        before: total_before,
                        after: total_after,
                    },
                }],
            },
            column: FacetAxisBoundaryProfile::default(),
        }
    }

    fn column_profile_lane(
        lane_key: Vec<usize>,
        guide_before: f32,
        guide_after: f32,
        total_before: f32,
        total_after: f32,
    ) -> FacetBoundaryProfiles {
        FacetBoundaryProfiles {
            row: FacetAxisBoundaryProfile::default(),
            column: FacetAxisBoundaryProfile {
                lanes: vec![FacetBoundaryLane {
                    lane_key,
                    guide: FacetBoundaryDemand {
                        before: guide_before,
                        after: guide_after,
                    },
                    total: FacetBoundaryDemand {
                        before: total_before,
                        after: total_after,
                    },
                }],
            },
        }
    }

    fn assert_overflow_eq(actual: &CoordinatedOverflow, expected: &CoordinatedOverflow) {
        assert_eq!(actual.guide, expected.guide);
        assert_eq!(actual.total, expected.total);
    }

    #[test]
    fn rendered_subtree_projection_preserves_full_overflow() {
        let raw = overflow((1.0, 2.0, 3.0, 4.0), (5.0, 6.0, 7.0, 8.0));
        assert_eq!(raw.guide.top, 1.0);
        assert_eq!(raw.total.right, 6.0);
    }

    #[test]
    fn root_residual_retains_unowned_right_legend_slab() {
        let raw = overflow((0.0, 20.0, 0.0, 0.0), (0.0, 70.0, 0.0, 0.0));
        let projected = residual_overflow_for_owned_legend_slabs(&raw, EdgeSlabs::default());

        assert_eq!(projected.total.right, 70.0);
        assert_eq!(projected.guide.right, 20.0);
    }

    #[test]
    fn rendered_subtree_residual_removes_main_axis_legend_slab_owned_by_layout() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (110.0, 120.0, 130.0, 140.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            guide_slot_gap_px: 0.0,
            outer_start: 100.0,
            outer_end: 100.0,
            n: 2,
        };
        let owned_slabs =
            crate::facet::coord::FacetBandAllocationOwnership::from_policy(true, false)
                .owned_legend_slabs(FacetAxis::Column, &raw, &layout);
        let projected = residual_overflow_for_owned_legend_slabs(&raw, owned_slabs);

        assert_eq!(projected.total.left, raw.guide.left);
        assert_eq!(projected.total.right, raw.guide.right);
        assert_eq!(projected.total.top, raw.total.top);
        assert_eq!(projected.total.bottom, raw.total.bottom);
    }

    #[test]
    fn rendered_subtree_residual_removes_cross_axis_legend_slab_owned_by_canvas_slot() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (110.0, 120.0, 130.0, 140.0));
        let layout = CoordinatedLayout {
            padding_inner_px: 0.0,
            guide_slot_gap_px: 0.0,
            outer_start: 0.0,
            outer_end: 0.0,
            n: 2,
        };
        let owned_slabs =
            crate::facet::coord::FacetBandAllocationOwnership::from_policy(false, true)
                .owned_legend_slabs(FacetAxis::Column, &raw, &layout);
        let projected = residual_overflow_for_owned_legend_slabs(&raw, owned_slabs);

        assert_eq!(projected.total.top, raw.guide.top);
        assert_eq!(projected.total.bottom, raw.guide.bottom);
        assert_eq!(projected.total.left, raw.total.left);
        assert_eq!(projected.total.right, raw.total.right);
    }

    #[test]
    fn owned_legend_slabs_never_consume_facet_guide_slabs() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (15.0, 70.0, 45.0, 60.0));
        let projected = residual_overflow_for_owned_legend_slabs(
            &raw,
            EdgeSlabs::new(100.0, 100.0, 100.0, 100.0),
        );

        assert_eq!(projected.guide, raw.guide);
        assert_eq!(projected.total, raw.guide);
    }

    #[test]
    fn sibling_boundary_projection_preserves_same_axis_and_cross_axis_end_legend_slabs() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (110.0, 120.0, 130.0, 140.0));

        let row = project_facet_overflow(
            &raw,
            FacetOverflowProjection::SiblingBoundary {
                axis: FacetAxis::Row,
            },
        );
        assert_eq!(row.total.top, raw.total.top);
        assert_eq!(row.total.bottom, raw.total.bottom);
        assert_eq!(row.total.left, raw.guide.left);
        assert_eq!(row.total.right, raw.total.right);

        let column = project_facet_overflow(
            &raw,
            FacetOverflowProjection::SiblingBoundary {
                axis: FacetAxis::Column,
            },
        );
        assert_eq!(column.total.left, raw.total.left);
        assert_eq!(column.total.right, raw.total.right);
        assert_eq!(column.total.top, raw.guide.top);
        assert_eq!(column.total.bottom, raw.total.bottom);
    }

    #[test]
    fn guide_anchor_projection_matches_sibling_boundary_projection() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (110.0, 120.0, 130.0, 140.0));
        assert_overflow_eq(
            &project_facet_overflow(
                &raw,
                FacetOverflowProjection::GuideAnchor {
                    axis: FacetAxis::Row,
                },
            ),
            &project_facet_overflow(
                &raw,
                FacetOverflowProjection::SiblingBoundary {
                    axis: FacetAxis::Row,
                },
            ),
        );
    }

    #[test]
    fn overflow_slabs_extract_non_negative_legend_slab() {
        let raw = overflow((12.0, 9.0, 5.0, 3.0), (20.0, 10.0, 8.0, 1.0));
        let slabs = FacetOverflowSlabs::from_coordinated(&raw);
        assert_eq!(slabs.guide, raw.guide);
        assert_eq!(slabs.legend.top, 8.0);
        assert_eq!(slabs.legend.right, 1.0);
        assert_eq!(slabs.legend.bottom, 3.0);
        assert_eq!(slabs.legend.left, 0.0);
    }

    #[test]
    fn lane_padding_uses_one_sided_row_overflow_without_matching_lower_lane() {
        let profiles = vec![
            row_profile_lane(lane_key(&[0]), 0.0, 3.0, 0.0, 34.0),
            row_profile_lane(lane_key(&[1]), 7.0, 0.0, 41.0, 0.0),
        ];
        let renderable = vec![true, true];

        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Row, &profiles, &renderable, true),
            Some(41.0)
        );
        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Row, &profiles, &renderable, false),
            Some(7.0)
        );
    }

    #[test]
    fn lane_padding_counts_row_overflow_with_matching_lower_lane() {
        let profiles = vec![
            row_profile_lane(lane_key(&[0]), 0.0, 3.0, 0.0, 34.0),
            row_profile_lane(lane_key(&[0]), 7.0, 0.0, 41.0, 0.0),
        ];
        let renderable = vec![true, true];

        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Row, &profiles, &renderable, true),
            Some(75.0)
        );
        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Row, &profiles, &renderable, false),
            Some(10.0)
        );
    }

    #[test]
    fn lane_padding_uses_one_sided_column_overflow_without_matching_lanes() {
        let profiles = vec![
            column_profile_lane(lane_key(&[0]), 0.0, 23.0, 0.0, 31.0),
            column_profile_lane(lane_key(&[1]), 17.0, 0.0, 39.0, 0.0),
        ];
        let renderable = vec![true, true];

        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Column, &profiles, &renderable, true,),
            Some(39.0)
        );

        let profiles = vec![
            column_profile_lane(lane_key(&[0]), 0.0, 23.0, 0.0, 31.0),
            column_profile_lane(lane_key(&[0]), 17.0, 0.0, 39.0, 0.0),
        ];
        assert_eq!(
            compute_padding_from_boundary_profiles(FacetAxis::Column, &profiles, &renderable, true,),
            Some(70.0)
        );
    }

    #[test]
    fn aggregate_column_uses_first_last_main_edges_and_max_cross_edges() {
        let cells = vec![
            cell(true, (1.0, 2.0, 3.0, 4.0), (11.0, 12.0, 13.0, 14.0)),
            cell(false, (50.0, 60.0, 70.0, 80.0), (51.0, 61.0, 71.0, 81.0)),
            cell(true, (5.0, 6.0, 7.0, 8.0), (15.0, 16.0, 17.0, 18.0)),
        ];
        let aggregated = aggregate_facet_band_overflow(FacetAxis::Column, &cells).unwrap();

        assert_eq!(
            aggregated.guide,
            OverflowSpaceRequirement {
                top: 5.0,
                right: 6.0,
                bottom: 7.0,
                left: 4.0,
            }
        );
        assert_eq!(
            aggregated.total,
            OverflowSpaceRequirement {
                top: 15.0,
                right: 16.0,
                bottom: 17.0,
                left: 14.0,
            }
        );
    }

    #[test]
    fn aggregate_row_uses_first_last_main_edges_and_max_cross_edges() {
        let cells = vec![
            cell(true, (1.0, 2.0, 3.0, 4.0), (11.0, 12.0, 13.0, 14.0)),
            cell(false, (50.0, 60.0, 70.0, 80.0), (51.0, 61.0, 71.0, 81.0)),
            cell(true, (5.0, 6.0, 7.0, 8.0), (15.0, 16.0, 17.0, 18.0)),
        ];
        let aggregated = aggregate_facet_band_overflow(FacetAxis::Row, &cells).unwrap();

        assert_eq!(
            aggregated.guide,
            OverflowSpaceRequirement {
                top: 1.0,
                right: 6.0,
                bottom: 7.0,
                left: 8.0,
            }
        );
        assert_eq!(
            aggregated.total,
            OverflowSpaceRequirement {
                top: 11.0,
                right: 16.0,
                bottom: 17.0,
                left: 18.0,
            }
        );
    }

    #[test]
    fn aggregate_preserves_no_renderable_policies() {
        let cells = vec![
            cell(false, (1.0, 2.0, 3.0, 4.0), (11.0, 12.0, 13.0, 14.0)),
            cell(false, (5.0, 6.0, 7.0, 8.0), (15.0, 16.0, 17.0, 18.0)),
        ];

        assert_overflow_eq(
            &aggregate_facet_band_overflow(FacetAxis::Column, &cells).unwrap(),
            &CoordinatedOverflow::default(),
        );
        assert_eq!(
            aggregate_facet_band_overflow_with_policy(
                FacetAxis::Column,
                &cells,
                FacetBandNoRenderablePolicy::FirstCell,
            )
            .unwrap()
            .total
            .left,
            14.0
        );
    }
}
