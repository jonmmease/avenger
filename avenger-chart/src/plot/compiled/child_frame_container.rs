//! Generic view over measured child-frame containers.
//!
//! The view is intentionally read-only. It lets generic layout/debug consumers
//! inspect measured child frames without knowing whether the producer was a
//! facet, concat, coordinate-positioned container, or another composition.

use std::collections::HashSet;

use crate::{
    concat::ConcatCoordMeasurement,
    facet::coord::FacetBandCoordMeasurement,
    layout::Size,
    partition::format_partition_value,
    positioned_subplot::{PositionedCoordMeasurement, PositionedPlacementSolution},
};
use avenger_chart_core::{
    AvengerChartError, CoordMeasurement, FrameAllocation, LayoutBounds, OverflowSpaceRequirement,
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

/// Solved readback for one child frame in parent plot-area coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChildFrameRegion {
    pub(crate) child_index: usize,
    /// Child plot/content rectangle in parent plot-area coordinates.
    pub(crate) content: LayoutBounds,
    /// Parent-granted slot in parent plot-area coordinates.
    pub(crate) slot: LayoutBounds,
    /// Optional plot-area size adopted by the child at render time.
    pub(crate) content_size_override: Option<Size>,
    /// Optional coordinated edge targets adopted by the child at render time.
    pub(crate) edge_targets: Option<crate::layout::EdgeTargets>,
}

impl ChildFrameRegion {
    pub(crate) fn plot_origin(&self) -> [f32; 2] {
        [self.content.x, self.content.y]
    }

    pub(crate) fn project_child_bounds(
        &self,
        child_plot_bounds: LayoutBounds,
        child_bounds: LayoutBounds,
    ) -> LayoutBounds {
        project_child_local_rect(self.content, child_plot_bounds, child_bounds)
    }
}

/// Read-only child-frame container projection for generic layout consumers.
///
/// A view is valid only when child indices and solved regions form a
/// one-to-one mapping. Region coordinates are expressed in the parent plot
/// area's coordinate space; child-local bounds project from the child's
/// solved content rectangle.
#[derive(Debug)]
pub struct ChildFrameContainerView<'a> {
    children: Vec<ChildFrameChildView<'a>>,
    regions: Vec<ChildFrameRegion>,
    content_size: Size,
}

impl<'a> ChildFrameContainerView<'a> {
    fn new(
        children: Vec<ChildFrameChildView<'a>>,
        content_size: Size,
        regions: Vec<ChildFrameRegion>,
    ) -> Self {
        Self {
            children,
            regions,
            content_size,
        }
    }

    fn from_placement(
        children: Vec<ChildFrameChildView<'a>>,
        placement: PositionedPlacementSolution,
    ) -> Result<Self, AvengerChartError> {
        let regions = child_regions_from_placement(&children, &placement)?;
        let content_size = placement.content_size;
        Ok(Self {
            children,
            regions,
            content_size,
        })
    }

    pub(crate) fn content_size(&self) -> Size {
        self.content_size
    }

    pub(crate) fn child_regions(&self) -> &[ChildFrameRegion] {
        &self.regions
    }

    pub(crate) fn child_region(&self, child_index: usize) -> Option<&ChildFrameRegion> {
        self.regions
            .iter()
            .find(|region| region.child_index == child_index)
    }

    pub(crate) fn project_child_bounds(
        &self,
        child_index: usize,
        child_bounds: LayoutBounds,
    ) -> Result<LayoutBounds, AvengerChartError> {
        let region = self.child_region(child_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing child-frame region for child index {}",
                child_index
            ))
        })?;
        let child = self.child_measurement(child_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing child-frame measurement for child index {}",
                child_index
            ))
        })?;
        Ok(region.project_child_bounds(*child.layout.plot_area_bounds(), child_bounds))
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

pub(crate) fn project_child_local_rect(
    child_content: LayoutBounds,
    child_plot_bounds: LayoutBounds,
    child_bounds: LayoutBounds,
) -> LayoutBounds {
    LayoutBounds {
        x: child_content.x + child_bounds.x - child_plot_bounds.x,
        y: child_content.y + child_bounds.y - child_plot_bounds.y,
        width: child_bounds.width,
        height: child_bounds.height,
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
    let regions = concat.plot_child_frame_regions()?;
    validate_child_regions(&regions, &children, Some(&child_debug_labels))?;
    let content_size = concat.child_frame_content_size()?;
    let view = ChildFrameContainerView::new(children, content_size, regions);
    view.validate_scope_keys()?;
    view.validate_child_regions(Some(&child_debug_labels))?;
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
    let view = ChildFrameContainerView::from_placement(children, placement)?;
    view.validate_scope_keys()?;
    view.validate_child_regions(None)?;
    Ok(view)
}

pub(crate) fn child_frame_container_view_from_facet<'a>(
    _measurement: &'a ComponentsMeasurement,
    facet_band: &'a FacetBandCoordMeasurement,
) -> Result<ChildFrameContainerView<'a>, AvengerChartError> {
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

    let (content_size, regions) = facet_band.current_child_frame_regions()?;
    validate_child_regions(&regions, &children, None)?;
    let view = ChildFrameContainerView::new(children, content_size, regions);
    view.validate_scope_keys()?;
    view.validate_child_regions(None)?;
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

    fn validate_child_regions(
        &self,
        child_debug_labels: Option<&[String]>,
    ) -> Result<(), AvengerChartError> {
        validate_child_regions(&self.regions, &self.children, child_debug_labels)
    }
}

fn child_regions_from_placement(
    children: &[ChildFrameChildView<'_>],
    placement: &PositionedPlacementSolution,
) -> Result<Vec<ChildFrameRegion>, AvengerChartError> {
    placement
        .placements()
        .iter()
        .map(|render_placement| {
            let child = children
                .iter()
                .find(|child| child.child_index == render_placement.child_index)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Child-frame placement index {} did not resolve to a child measurement",
                        render_placement.child_index
                    ))
                })?;
            let content_size = Size::new(
                child.measurement.plot_area_width,
                child.measurement.plot_area_height,
            );
            let content = LayoutBounds {
                x: render_placement.origin[0],
                y: render_placement.origin[1],
                width: content_size.width,
                height: content_size.height,
            };
            Ok(ChildFrameRegion {
                child_index: render_placement.child_index,
                content,
                slot: content,
                content_size_override: None,
                edge_targets: None,
            })
        })
        .collect()
}

/// Compute parent-frame overflow required by already measured child frames in a
/// generic container view.
pub(crate) fn child_frame_container_overflow(
    plot_width: f32,
    plot_height: f32,
    container: &ChildFrameContainerView<'_>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    child_frame_container_overflow_from_regions(
        plot_width,
        plot_height,
        container.content_size(),
        container.child_regions(),
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
pub(crate) fn child_frame_container_overflow_from_regions<'a>(
    plot_width: f32,
    plot_height: f32,
    content_size: Size,
    regions: &[ChildFrameRegion],
    mut child_measurement: impl FnMut(usize) -> Result<&'a ComponentsMeasurement, AvengerChartError>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let mut min_x = 0.0f32;
    let mut min_y = 0.0f32;
    let mut max_x = content_size.width.max(plot_width);
    let mut max_y = content_size.height.max(plot_height);

    for region in regions {
        let child_measurement = child_measurement(region.child_index)?;
        let child_plot_bounds = *child_measurement.layout.plot_area_bounds();
        let frame_bounds =
            region.project_child_bounds(child_plot_bounds, child_measurement.frame_allocation.rect);

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

fn validate_child_regions(
    regions: &[ChildFrameRegion],
    children: &[ChildFrameChildView<'_>],
    child_debug_labels: Option<&[String]>,
) -> Result<(), AvengerChartError> {
    if regions.len() != children.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Child-frame container region count mismatch: regions={}, children={}",
            regions.len(),
            children.len()
        )));
    }

    let mut seen_regions = HashSet::with_capacity(regions.len());
    for region in regions {
        if !seen_regions.insert(region.child_index) {
            return Err(AvengerChartError::InternalError(format!(
                "Duplicate child-frame container region for child index {}",
                region.child_index
            )));
        }

        if !children
            .iter()
            .any(|child| child.child_index == region.child_index)
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
                "Child-frame container region index {} did not resolve to a child measurement; available children: {}",
                region.child_index, available
            )));
        }
    }

    for child in children {
        if !seen_regions.contains(&child.child_index) {
            let label = child.label.as_deref().unwrap_or("<unlabeled>");
            return Err(AvengerChartError::InternalError(format!(
                "Missing child-frame region for child index {} ({label})",
                child.child_index
            )));
        }
    }

    Ok(())
}

fn validate_container_placements(
    placement: &PositionedPlacementSolution,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_child_local_rect_uses_solved_content_as_reference() {
        let solved_child_content = LayoutBounds {
            x: 120.0,
            y: 80.0,
            width: 200.0,
            height: 100.0,
        };
        let child_plot_bounds = LayoutBounds {
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 100.0,
        };
        let child_legend_bounds = LayoutBounds {
            x: 225.0,
            y: 35.0,
            width: 40.0,
            height: 30.0,
        };

        assert_eq!(
            project_child_local_rect(solved_child_content, child_plot_bounds, child_legend_bounds),
            LayoutBounds {
                x: 335.0,
                y: 95.0,
                width: 40.0,
                height: 30.0,
            }
        );
    }

    #[test]
    fn child_frame_region_projects_from_content_rect() {
        let region = ChildFrameRegion {
            child_index: 7,
            content: LayoutBounds {
                x: 50.0,
                y: 60.0,
                width: 150.0,
                height: 90.0,
            },
            slot: LayoutBounds {
                x: 40.0,
                y: 55.0,
                width: 170.0,
                height: 100.0,
            },
            content_size_override: None,
            edge_targets: None,
        };

        let projected = region.project_child_bounds(
            LayoutBounds {
                x: -5.0,
                y: 10.0,
                width: 150.0,
                height: 90.0,
            },
            LayoutBounds {
                x: -15.0,
                y: 0.0,
                width: 20.0,
                height: 12.0,
            },
        );

        assert_eq!(
            projected,
            LayoutBounds {
                x: 40.0,
                y: 50.0,
                width: 20.0,
                height: 12.0,
            }
        );
    }
}
