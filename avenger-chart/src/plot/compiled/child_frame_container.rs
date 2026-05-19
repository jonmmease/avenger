//! Generic view over measured child-frame containers.
//!
//! The view is intentionally read-only. It lets generic layout/debug consumers
//! inspect measured child frames without knowing whether the producer was a
//! facet, concat, coordinate-positioned container, or another composition.

use crate::{
    error::AvengerChartError,
    facet::{coord::facet_band_ref, placement::resolve_facet_child_frame_placement},
    layout::{ChildFramePlacementResult, FrameAllocation},
};

use super::ComponentsMeasurement;

/// One child frame inside a measured child-frame container.
#[derive(Debug, Clone, Copy)]
struct ChildFrameChildView<'a> {
    child_index: usize,
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
            .map(|(child_index, cell)| ChildFrameChildView {
                child_index,
                measurement: &cell.measurement,
            })
            .collect();

        Ok(Some(ChildFrameContainerView::new(children, placement)))
    }
}
