//! Generic view over measured child-frame containers.
//!
//! The view is intentionally read-only. It lets generic layout/debug consumers
//! inspect measured child frames without knowing whether the producer was a
//! facet, concat, coordinate-positioned container, or another composition.

use std::collections::HashSet;

use avenger_chart_core::{
    AvengerChartError, CoordMeasurement, FrameAllocation, OverflowSpaceRequirement,
};

use crate::{
    concat::ConcatCoordMeasurement,
    container::{PlacementSolution, project_child_rect},
    facet::{coord::FacetBandCoordMeasurement, placement::resolve_facet_child_frame_placement},
    partition::format_partition_value,
    positioned_subplot::PositionedCoordMeasurement,
};

use super::{ChildFrameScopeKey, ComponentsMeasurement, CoordinationKind, CoordinationScopeKey};

/// One child frame inside a measured child-frame container.
#[derive(Debug, Clone)]
struct ChildFrameChildView<'a> {
    child_index: usize,
    scope_key: ChildFrameScopeKey,
    label: Option<String>,
    measurement: &'a ComponentsMeasurement,
}

/// Read-only child-frame container projection for generic layout consumers.
///
/// A view is valid only when child indices and render placements form a
/// one-to-one mapping. Render origins are frame origins in the parent content
/// coordinate space; they are not child plot-area origins.
#[derive(Debug)]
pub struct ChildFrameContainerView<'a> {
    children: Vec<ChildFrameChildView<'a>>,
    placement: PlacementSolution,
}

impl<'a> ChildFrameContainerView<'a> {
    fn new(children: Vec<ChildFrameChildView<'a>>, placement: PlacementSolution) -> Self {
        Self {
            children,
            placement,
        }
    }

    pub(crate) fn placement(&self) -> &PlacementSolution {
        &self.placement
    }

    pub(crate) fn child_measurement(
        &self,
        child_index: usize,
    ) -> Option<&'a ComponentsMeasurement> {
        self.child(child_index).map(|child| child.measurement)
    }

    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<&ChildFrameScopeKey> {
        self.child(child_index).map(|child| &child.scope_key)
    }

    pub(crate) fn child_label(&self, child_index: usize) -> Option<&str> {
        self.child(child_index)
            .and_then(|child| child.label.as_deref())
    }

    pub(crate) fn child_scope_keys(&self) -> impl Iterator<Item = &ChildFrameScopeKey> {
        self.children.iter().map(|child| &child.scope_key)
    }

    pub(crate) fn child_frame_allocations(&self) -> Vec<FrameAllocation> {
        self.children
            .iter()
            .map(|child| child.measurement.frame_allocation)
            .collect()
    }

    fn child(&self, child_index: usize) -> Option<&ChildFrameChildView<'a>> {
        self.children
            .iter()
            .find(|child| child.child_index == child_index)
    }
}

impl ComponentsMeasurement {
    pub(crate) fn child_frame_container_view(
        &self,
    ) -> Result<Option<ChildFrameContainerView<'_>>, AvengerChartError> {
        child_frame_container_view_for_coord_measurement(self.coord_measurement.as_ref(), self)
    }
}

pub(crate) fn child_frame_container_view_for_coord_measurement<'a>(
    coord_measurement: &'a dyn CoordMeasurement,
    measurement: &'a ComponentsMeasurement,
) -> Result<Option<ChildFrameContainerView<'a>>, AvengerChartError> {
    if let Some(concat) = coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
    {
        return Ok(Some(child_frame_container_view_from_concat(concat)?));
    }

    if let Some(positioned) = coord_measurement
        .as_any()
        .downcast_ref::<PositionedCoordMeasurement>()
    {
        return Ok(Some(child_frame_container_view_from_positioned(
            positioned,
        )?));
    }

    if let Some(facet_band) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    {
        return Ok(Some(child_frame_container_view_from_facet(
            measurement,
            facet_band,
        )?));
    }

    Ok(None)
}

pub(crate) fn child_frame_container_view_from_concat(
    concat: &ConcatCoordMeasurement,
) -> Result<ChildFrameContainerView<'_>, AvengerChartError> {
    let placement = concat.child_frame_placement();
    let child_debug_labels = concat
        .children()
        .iter()
        .map(|child| child.debug_label())
        .collect::<Vec<_>>();
    let children = concat
        .children()
        .iter()
        .map(|child| {
            let scope_key = concat.child_scope_key(child.child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing concat scope key for child index {}",
                    child.child_index
                ))
            })?;
            Ok(ChildFrameChildView {
                child_index: child.child_index,
                scope_key,
                label: child.label.clone(),
                measurement: &child.measurement,
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    validate_container_placements(&placement, &children, Some(&child_debug_labels))?;
    let view = ChildFrameContainerView::new(children, placement);
    view.validate_scope_keys()?;
    Ok(view)
}

pub(crate) fn child_frame_container_view_from_positioned(
    positioned: &PositionedCoordMeasurement,
) -> Result<ChildFrameContainerView<'_>, AvengerChartError> {
    let placement = positioned.child_frame_placement();
    let children = positioned
        .children()
        .iter()
        .map(|child| {
            let scope_key = positioned
                .child_scope_key(child.child_index)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing positioned scope key for child index {}",
                        child.child_index
                    ))
                })?;
            Ok(ChildFrameChildView {
                child_index: child.child_index,
                scope_key,
                label: child.label.clone(),
                measurement: &child.measurement,
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    validate_container_placements(&placement, &children, None)?;
    let view = ChildFrameContainerView::new(children, placement);
    view.validate_scope_keys()?;
    Ok(view)
}

pub(crate) fn child_frame_container_view_from_facet<'a>(
    measurement: &'a ComponentsMeasurement,
    facet_band: &'a FacetBandCoordMeasurement,
) -> Result<ChildFrameContainerView<'a>, AvengerChartError> {
    let placement = resolve_facet_child_frame_placement(measurement, facet_band)?;
    let children = facet_band
        .cells
        .iter()
        .enumerate()
        .map(|(child_index, cell)| {
            let scope_key = facet_band.child_scope_key(child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing facet scope key for child index {child_index}"
                ))
            })?;
            Ok(ChildFrameChildView {
                child_index,
                scope_key,
                label: Some(format_partition_value(&cell.plan.value)),
                measurement: &cell.measurement,
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    validate_container_placements(&placement, &children, None)?;
    let view = ChildFrameContainerView::new(children, placement);
    view.validate_scope_keys()?;
    Ok(view)
}

impl ChildFrameContainerView<'_> {
    fn validate_scope_keys(&self) -> Result<(), AvengerChartError> {
        let scope_count = self.child_scope_keys().count();
        if scope_count != self.children.len() {
            return Err(AvengerChartError::InternalError(format!(
                "Child-frame scope count {scope_count} did not match child count {}",
                self.children.len()
            )));
        }

        let mut seen = HashSet::with_capacity(self.children.len());
        let mut container_coordination_key: Option<CoordinationScopeKey> = None;
        for child in &self.children {
            let Some(scope_key) = self.child_scope_key(child.child_index) else {
                let label = self.child_label(child.child_index).unwrap_or("<unlabeled>");
                return Err(AvengerChartError::InternalError(format!(
                    "Missing child-frame scope key for child index {} ({label})",
                    child.child_index,
                )));
            };
            let current_container_key =
                CoordinationScopeKey::child_frame_container(CoordinationKind::ChildSize, scope_key);
            if let Some(expected) = &container_coordination_key {
                if expected != &current_container_key {
                    let label = self.child_label(child.child_index).unwrap_or("<unlabeled>");
                    return Err(AvengerChartError::InternalError(format!(
                        "Child-frame scope key for child index {} ({label}) belongs to a different container: {:?}",
                        child.child_index, scope_key,
                    )));
                }
            } else {
                container_coordination_key = Some(current_container_key);
            }

            let child_coordination_key =
                CoordinationScopeKey::child_frame(CoordinationKind::ChildSize, scope_key);
            if !seen.insert(child_coordination_key) {
                let label = self.child_label(child.child_index).unwrap_or("<unlabeled>");
                return Err(AvengerChartError::InternalError(format!(
                    "Duplicate child-frame scope key for child index {} ({label}): {:?}",
                    child.child_index, scope_key,
                )));
            }
        }
        Ok(())
    }
}

/// Compute parent-frame overflow required by already measured child frames in a
/// generic container view.
pub(crate) fn child_frame_container_overflow(
    plot_width: f32,
    plot_height: f32,
    container: &ChildFrameContainerView<'_>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    child_frame_container_overflow_from_placements(
        plot_width,
        plot_height,
        container.placement(),
        |child_index| {
            container.child_measurement(child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame measurement for child index {}",
                    child_index
                ))
            })
        },
    )
}

/// Compute parent-frame overflow required by already measured child frames.
pub(crate) fn child_frame_container_overflow_from_placements<'a>(
    plot_width: f32,
    plot_height: f32,
    placement: &PlacementSolution,
    mut child_measurement: impl FnMut(usize) -> Result<&'a ComponentsMeasurement, AvengerChartError>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let mut min_x = 0.0f32;
    let mut min_y = 0.0f32;
    let mut max_x = placement.content_size.width.max(plot_width);
    let mut max_y = placement.content_size.height.max(plot_height);

    for render_placement in placement.placements() {
        let child_measurement = child_measurement(render_placement.child_index)?;
        let child_plot_bounds = *child_measurement.layout.plot_area_bounds();
        let frame_bounds = project_child_rect(
            [0.0, 0.0],
            render_placement.origin,
            child_plot_bounds,
            child_measurement.frame_allocation.rect,
        );

        min_x = min_x.min(frame_bounds.x);
        min_y = min_y.min(frame_bounds.y);
        max_x = max_x.max(frame_bounds.x + frame_bounds.width);
        max_y = max_y.max(frame_bounds.y + frame_bounds.height);
    }

    Ok(OverflowSpaceRequirement {
        top: (-min_y).max(0.0),
        right: (max_x - plot_width).max(0.0),
        bottom: (max_y - plot_height).max(0.0),
        left: (-min_x).max(0.0),
    })
}

fn validate_container_placements(
    placement: &PlacementSolution,
    children: &[ChildFrameChildView<'_>],
    child_debug_labels: Option<&[String]>,
) -> Result<(), AvengerChartError> {
    if placement.placements().len() != children.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Child-frame container placement count mismatch: placements={}, children={}",
            placement.placements().len(),
            children.len()
        )));
    }

    let mut seen_placements = HashSet::with_capacity(placement.placements().len());
    for render_placement in placement.placements() {
        if !seen_placements.insert(render_placement.child_index) {
            return Err(AvengerChartError::InternalError(format!(
                "Duplicate child-frame container placement for child index {}",
                render_placement.child_index
            )));
        }

        if !children
            .iter()
            .any(|child| child.child_index == render_placement.child_index)
        {
            let available = child_debug_labels
                .map(|labels| labels.join(", "))
                .unwrap_or_else(|| {
                    children
                        .iter()
                        .map(|child| child.child_index.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                });
            return Err(AvengerChartError::InternalError(format!(
                "Child-frame container placement index {} did not resolve to a child measurement; available children: {}",
                render_placement.child_index, available
            )));
        }
    }

    for child in children {
        if placement.child(child.child_index).is_none() {
            let label = child.label.as_deref().unwrap_or("<unlabeled>");
            return Err(AvengerChartError::InternalError(format!(
                "Missing child-frame placement for child index {} ({label})",
                child.child_index
            )));
        }
    }

    Ok(())
}
