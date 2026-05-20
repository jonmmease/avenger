//! Generic view over measured child-frame containers.
//!
//! The view is intentionally read-only. It lets generic layout/debug consumers
//! inspect measured child frames without knowing whether the producer was a
//! facet, concat, coordinate-positioned container, or another composition.

use std::collections::HashSet;

use crate::{
    concat::concat_coord_ref,
    error::AvengerChartError,
    facet::{coord::facet_band_ref, placement::resolve_facet_child_frame_placement},
    layout::{ChildFramePlacementResult, FrameAllocation},
};

use super::{ChildFrameScopeKey, ComponentsMeasurement};

/// One child frame inside a measured child-frame container.
#[derive(Debug, Clone)]
struct ChildFrameChildView<'a> {
    child_index: usize,
    scope_key: ChildFrameScopeKey,
    measurement: &'a ComponentsMeasurement,
}

/// Read-only child-frame container projection for generic layout consumers.
#[derive(Debug)]
pub(crate) struct ChildFrameContainerView<'a> {
    children: Vec<ChildFrameChildView<'a>>,
    placement: ChildFramePlacementResult,
}

impl<'a> ChildFrameContainerView<'a> {
    fn new(children: Vec<ChildFrameChildView<'a>>, placement: ChildFramePlacementResult) -> Self {
        Self {
            children,
            placement,
        }
    }

    pub(crate) fn placement(&self) -> &ChildFramePlacementResult {
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
        if let Some(concat) = concat_coord_ref(self.coord_measurement.as_ref()) {
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
                        measurement: &child.measurement,
                    })
                })
                .collect::<Result<Vec<_>, AvengerChartError>>()?;
            validate_container_placements(&placement, &children, Some(&child_debug_labels))?;
            let view = ChildFrameContainerView::new(children, placement);
            view.validate_scope_keys()?;
            return Ok(Some(view));
        }

        let Some(facet_band) = facet_band_ref(self.coord_measurement.as_ref()) else {
            return Ok(None);
        };
        let placement = resolve_facet_child_frame_placement(self, facet_band)?;
        if placement.render_placements().len() != facet_band.cells.len() {
            return Err(AvengerChartError::InternalError(format!(
                "Child-frame container placement count mismatch: placements={}, children={}",
                placement.render_placements().len(),
                facet_band.cells.len()
            )));
        }

        for render_placement in placement.render_placements() {
            if render_placement.child_index >= facet_band.cells.len() {
                return Err(AvengerChartError::InternalError(format!(
                    "Child-frame container placement index {} exceeded child count {}",
                    render_placement.child_index,
                    facet_band.cells.len()
                )));
            }
        }

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
                    measurement: &cell.measurement,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        validate_container_placements(&placement, &children, None)?;
        let view = ChildFrameContainerView::new(children, placement);
        view.validate_scope_keys()?;
        Ok(Some(view))
    }
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
        for child in &self.children {
            let Some(scope_key) = self.child_scope_key(child.child_index) else {
                return Err(AvengerChartError::InternalError(format!(
                    "Missing child-frame scope key for child index {}",
                    child.child_index
                )));
            };
            if !seen.insert(scope_key.clone()) {
                return Err(AvengerChartError::InternalError(format!(
                    "Duplicate child-frame scope key for child index {}: {:?}",
                    child.child_index, scope_key
                )));
            }
        }
        Ok(())
    }
}

fn validate_container_placements(
    placement: &ChildFramePlacementResult,
    children: &[ChildFrameChildView<'_>],
    child_debug_labels: Option<&[String]>,
) -> Result<(), AvengerChartError> {
    for render_placement in placement.render_placements() {
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
    Ok(())
}
