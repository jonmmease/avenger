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
    layout::{EdgeSlabs, FrameDemand, OwnedEdgeSlabs},
    plot::compiled::ComponentsMeasurement,
};

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
    /// Global coordination contract for aligned guide anchors and sibling gaps.
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
            resolve_projected_facet_overflow(measurement, source, axis, |axis| {
                FacetOverflowProjection::GuideAnchor { axis }
            })
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

    let first_idx = *renderable_indices
        .first()
        .expect("renderable indices must not be empty");
    let last_idx = *renderable_indices
        .last()
        .expect("renderable indices must not be empty");
    let first = &cells[first_idx];
    let last = &cells[last_idx];

    let max_guide_top = renderable_indices
        .iter()
        .map(|idx| cells[*idx].guide.top)
        .fold(0.0, f32::max);
    let max_guide_bottom = renderable_indices
        .iter()
        .map(|idx| cells[*idx].guide.bottom)
        .fold(0.0, f32::max);
    let max_guide_left = renderable_indices
        .iter()
        .map(|idx| cells[*idx].guide.left)
        .fold(0.0, f32::max);
    let max_guide_right = renderable_indices
        .iter()
        .map(|idx| cells[*idx].guide.right)
        .fold(0.0, f32::max);

    let max_total_top = renderable_indices
        .iter()
        .map(|idx| cells[*idx].total.top)
        .fold(0.0, f32::max);
    let max_total_bottom = renderable_indices
        .iter()
        .map(|idx| cells[*idx].total.bottom)
        .fold(0.0, f32::max);
    let max_total_left = renderable_indices
        .iter()
        .map(|idx| cells[*idx].total.left)
        .fold(0.0, f32::max);
    let max_total_right = renderable_indices
        .iter()
        .map(|idx| cells[*idx].total.right)
        .fold(0.0, f32::max);

    let guide = match axis {
        FacetAxis::Column => OverflowSpaceRequirement {
            top: max_guide_top,
            bottom: max_guide_bottom,
            left: first.guide.left,
            right: last.guide.right,
        },
        FacetAxis::Row => OverflowSpaceRequirement {
            top: first.guide.top,
            bottom: last.guide.bottom,
            left: max_guide_left,
            right: max_guide_right,
        },
    };

    let total = match axis {
        FacetAxis::Column => OverflowSpaceRequirement {
            top: max_total_top,
            bottom: max_total_bottom,
            left: first.total.left,
            right: last.total.right,
        },
        FacetAxis::Row => OverflowSpaceRequirement {
            top: first.total.top,
            bottom: last.total.bottom,
            left: max_total_left,
            right: max_total_right,
        },
    };

    Some(CoordinatedOverflow { guide, total })
}

pub(crate) fn rendered_subtree_overflow_from_coord_measurement(
    measurement: &dyn CoordMeasurement,
    source: FacetOverflowSource,
) -> Option<CoordinatedOverflow> {
    match source {
        FacetOverflowSource::MeasuredLocal => {
            if let Some(facet_measurement) = facet_band_from_coord(measurement) {
                let active_layout = facet_measurement
                    .coordinated_layout
                    .as_ref()
                    .unwrap_or(&facet_measurement.local_layout);
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
                let active_layout = facet_measurement
                    .coordinated_layout
                    .as_ref()
                    .unwrap_or(&facet_measurement.local_layout);
                return Some(rendered_subtree_overflow_for_facet_band(
                    facet_measurement,
                    &facet_measurement.coordinated_overflow,
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
    let active_layout = facet_measurement
        .coordinated_layout
        .as_ref()
        .unwrap_or(&facet_measurement.local_layout);
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
    CoordinatedOverflow {
        guide: measurement.layout.overflow.clone(),
        total: measurement.layout.total_overflow.clone(),
    }
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
    boundary_demand_components_from_overflow(axis, measurement, &overflow)
}

pub(crate) fn overflow_from_boundary_demand(
    axis: FacetAxis,
    demand: FacetBoundaryDemand,
) -> OverflowSpaceRequirement {
    match axis {
        FacetAxis::Column => OverflowSpaceRequirement {
            left: demand.before,
            right: demand.after,
            ..Default::default()
        },
        FacetAxis::Row => OverflowSpaceRequirement {
            top: demand.before,
            bottom: demand.after,
            ..Default::default()
        },
    }
}

fn boundary_demand_components_from_overflow(
    axis: FacetAxis,
    measurement: &ComponentsMeasurement,
    overflow: &CoordinatedOverflow,
) -> FacetBoundaryDemandComponents {
    const LEGEND_EDGE_BREATHING_ROOM: f32 = 8.0;

    let guide = &overflow.guide;
    let total = &overflow.total;
    let (
        legend_left_from_bounds,
        legend_right_from_bounds,
        legend_top_from_bounds,
        legend_bottom_from_bounds,
    ) = legend_bounds_overflow_edges(measurement);
    let (
        coordinated_legend_left,
        coordinated_legend_right,
        coordinated_legend_top,
        coordinated_legend_bottom,
    ) = coordinated_legend_overflow_edges(measurement);

    let legend_left = (total.left - guide.left)
        .max(0.0)
        .max(coordinated_legend_left)
        .max(legend_left_from_bounds);
    let legend_right = (total.right - guide.right)
        .max(0.0)
        .max(coordinated_legend_right)
        .max(legend_right_from_bounds);
    let legend_top = (total.top - guide.top)
        .max(0.0)
        .max(coordinated_legend_top)
        .max(legend_top_from_bounds);
    let legend_bottom = (total.bottom - guide.bottom)
        .max(0.0)
        .max(coordinated_legend_bottom)
        .max(legend_bottom_from_bounds);

    match axis {
        FacetAxis::Column => FacetBoundaryDemandComponents {
            guide: FacetBoundaryDemand {
                before: guide.left.max(0.0),
                after: guide.right.max(0.0),
            },
            total: FacetBoundaryDemand {
                before: total.left.max(0.0)
                    + if legend_left > 0.0 {
                        LEGEND_EDGE_BREATHING_ROOM
                    } else {
                        0.0
                    },
                after: total.right.max(0.0)
                    + if legend_right > 0.0 {
                        LEGEND_EDGE_BREATHING_ROOM
                    } else {
                        0.0
                    },
            },
        },
        FacetAxis::Row => FacetBoundaryDemandComponents {
            guide: FacetBoundaryDemand {
                before: guide.top.max(0.0),
                after: guide.bottom.max(0.0),
            },
            total: FacetBoundaryDemand {
                before: total.top.max(0.0)
                    + if legend_top > 0.0 {
                        LEGEND_EDGE_BREATHING_ROOM
                    } else {
                        0.0
                    },
                after: total.bottom.max(0.0)
                    + if legend_bottom > 0.0 {
                        LEGEND_EDGE_BREATHING_ROOM
                    } else {
                        0.0
                    },
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
            total.left = overflow.guide.left;
            total.right = overflow.guide.right;
            total.top = overflow.guide.top;
        }
        FacetAxis::Row => {
            total.top = overflow.guide.top;
            total.bottom = overflow.guide.bottom;
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
                    facet_measurement.coordinated_overflow.clone(),
                )
            })
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

    for bounds in measurement.layout.frame_layout.legends.values() {
        left = left.max((-bounds.x).max(0.0));
        right = right.max((bounds.x + bounds.width - plot_width).max(0.0));
        top = top.max((-bounds.y).max(0.0));
        bottom = bottom.max((bounds.y + bounds.height - plot_height).max(0.0));
    }

    (left, right, top, bottom)
}

fn coordinated_legend_overflow_edges(measurement: &ComponentsMeasurement) -> (f32, f32, f32, f32) {
    let Some(facet_band) = facet_band_from_coord(measurement.coord_measurement.as_ref()) else {
        return (0.0, 0.0, 0.0, 0.0);
    };

    let slabs = FacetOverflowSlabs::from_coordinated(facet_band.active_boundary_overflow());
    (
        slabs.legend.left,
        slabs.legend.right,
        slabs.legend.top,
        slabs.legend.bottom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn sibling_boundary_projection_preserves_only_cross_axis_end_legend_slab() {
        let raw = overflow((10.0, 20.0, 30.0, 40.0), (110.0, 120.0, 130.0, 140.0));

        let row = project_facet_overflow(
            &raw,
            FacetOverflowProjection::SiblingBoundary {
                axis: FacetAxis::Row,
            },
        );
        assert_eq!(row.total.top, raw.guide.top);
        assert_eq!(row.total.bottom, raw.guide.bottom);
        assert_eq!(row.total.left, raw.guide.left);
        assert_eq!(row.total.right, raw.total.right);

        let column = project_facet_overflow(
            &raw,
            FacetOverflowProjection::SiblingBoundary {
                axis: FacetAxis::Column,
            },
        );
        assert_eq!(column.total.left, raw.guide.left);
        assert_eq!(column.total.right, raw.guide.right);
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
