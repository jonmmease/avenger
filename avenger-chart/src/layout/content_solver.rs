//! Content layout abstractions below the chart frame.
//!
//! A frame solver owns chart chrome around one content rectangle. A content
//! solver owns what happens inside that rectangle. A regular chart is the
//! degenerate single-plot content case. Child-frame content is the multi-child
//! content case that produces child frame allocations.

use crate::{
    error::AvengerChartError,
    layout::{FrameAllocation, FrameDemand, LayoutBounds},
};

/// Allocation granted to the content inside a chart frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentAllocation {
    pub frame: FrameAllocation,
    pub content_rect: LayoutBounds,
}

impl ContentAllocation {
    pub fn new(frame: FrameAllocation, content_rect: LayoutBounds) -> Self {
        Self {
            frame,
            content_rect,
        }
    }
}

/// Measured content demand inside a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentDemand {
    pub frame_demand: FrameDemand,
    pub child_frame_allocations: Vec<FrameAllocation>,
}

impl ContentDemand {
    pub fn new(frame_demand: FrameDemand, child_frame_allocations: Vec<FrameAllocation>) -> Self {
        Self {
            frame_demand,
            child_frame_allocations,
        }
    }

    pub fn single_plot(frame_demand: FrameDemand) -> Self {
        Self::new(frame_demand, Vec::new())
    }
}

/// Realized content layout inside a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentLayout {
    pub allocation: ContentAllocation,
    pub frame_demand: FrameDemand,
    pub child_frame_allocations: Vec<FrameAllocation>,
}

impl ContentLayout {
    pub fn new(
        allocation: ContentAllocation,
        frame_demand: FrameDemand,
        child_frame_allocations: Vec<FrameAllocation>,
    ) -> Self {
        Self {
            allocation,
            frame_demand,
            child_frame_allocations,
        }
    }
}

/// Content solver interface shared by single-plot and child-frame content.
pub trait ContentLayoutSolver {
    type Measurement;
    type Plan;

    fn measure_content_demand(
        &self,
        allocation: &ContentAllocation,
        measurement: &Self::Measurement,
    ) -> Result<ContentDemand, AvengerChartError>;

    fn coordinate_content(
        &self,
        allocation: &ContentAllocation,
        demand: &ContentDemand,
    ) -> Result<Self::Plan, AvengerChartError>;

    fn realize_content(
        &self,
        allocation: ContentAllocation,
        demand: ContentDemand,
        plan: Self::Plan,
    ) -> Result<ContentLayout, AvengerChartError>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SinglePlotContentMeasurement {
    pub frame_demand: FrameDemand,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SinglePlotContentPlan;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SinglePlotContentSolver;

impl ContentLayoutSolver for SinglePlotContentSolver {
    type Measurement = SinglePlotContentMeasurement;
    type Plan = SinglePlotContentPlan;

    fn measure_content_demand(
        &self,
        _allocation: &ContentAllocation,
        measurement: &Self::Measurement,
    ) -> Result<ContentDemand, AvengerChartError> {
        Ok(ContentDemand::single_plot(measurement.frame_demand))
    }

    fn coordinate_content(
        &self,
        _allocation: &ContentAllocation,
        demand: &ContentDemand,
    ) -> Result<Self::Plan, AvengerChartError> {
        if !demand.child_frame_allocations.is_empty() {
            return Err(AvengerChartError::InternalError(
                "single-plot content cannot own child frame allocations".to_string(),
            ));
        }
        Ok(SinglePlotContentPlan)
    }

    fn realize_content(
        &self,
        allocation: ContentAllocation,
        demand: ContentDemand,
        _plan: Self::Plan,
    ) -> Result<ContentLayout, AvengerChartError> {
        Ok(ContentLayout::new(
            allocation,
            demand.frame_demand,
            Vec::new(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChildFrameContentMeasurement {
    pub frame_demand: FrameDemand,
    pub child_frame_allocations: Vec<FrameAllocation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChildFrameContentPlan {
    pub child_frame_allocations: Vec<FrameAllocation>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChildFrameContentSolver;

impl ContentLayoutSolver for ChildFrameContentSolver {
    type Measurement = ChildFrameContentMeasurement;
    type Plan = ChildFrameContentPlan;

    fn measure_content_demand(
        &self,
        _allocation: &ContentAllocation,
        measurement: &Self::Measurement,
    ) -> Result<ContentDemand, AvengerChartError> {
        Ok(ContentDemand::new(
            measurement.frame_demand,
            measurement.child_frame_allocations.clone(),
        ))
    }

    fn coordinate_content(
        &self,
        _allocation: &ContentAllocation,
        demand: &ContentDemand,
    ) -> Result<Self::Plan, AvengerChartError> {
        Ok(ChildFrameContentPlan {
            child_frame_allocations: demand.child_frame_allocations.clone(),
        })
    }

    fn realize_content(
        &self,
        allocation: ContentAllocation,
        demand: ContentDemand,
        plan: Self::Plan,
    ) -> Result<ContentLayout, AvengerChartError> {
        Ok(ContentLayout::new(
            allocation,
            demand.frame_demand,
            plan.child_frame_allocations,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{
        EdgeSlabs, FrameDimensionSizing, FrameSizingPolicy, LayoutBounds, OwnedEdgeSlabs,
    };

    fn allocation(owned_slabs: OwnedEdgeSlabs) -> ContentAllocation {
        ContentAllocation::new(
            FrameAllocation {
                rect: LayoutBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 160.0,
                },
                sizing: FrameSizingPolicy {
                    width: FrameDimensionSizing::CanvasConstrained { canvas_size: 200.0 },
                    height: FrameDimensionSizing::ContentSized {
                        content_size: 100.0,
                    },
                },
                owned_slabs,
            },
            LayoutBounds {
                x: 20.0,
                y: 30.0,
                width: 120.0,
                height: 100.0,
            },
        )
    }

    #[test]
    fn single_plot_content_solver_realizes_without_children() {
        let solver = SinglePlotContentSolver;
        let allocation = allocation(EdgeSlabs::default());
        let frame_demand = FrameDemand {
            rendered_envelope: EdgeSlabs::new(1.0, 2.0, 3.0, 4.0),
            ..Default::default()
        };
        let demand = solver
            .measure_content_demand(&allocation, &SinglePlotContentMeasurement { frame_demand })
            .unwrap();
        let plan = solver.coordinate_content(&allocation, &demand).unwrap();
        let layout = solver.realize_content(allocation, demand, plan).unwrap();

        assert_eq!(layout.child_frame_allocations, Vec::new());
        assert_eq!(layout.frame_demand.rendered_envelope.right, 2.0);
        assert_eq!(layout.allocation.content_rect.width, 120.0);
    }

    #[test]
    fn child_frame_content_solver_preserves_child_allocations() {
        let solver = ChildFrameContentSolver;
        let allocation = allocation(EdgeSlabs::default());
        let child_allocation = FrameAllocation {
            rect: LayoutBounds {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            },
            sizing: allocation.frame.sizing,
            owned_slabs: EdgeSlabs::new(0.0, 5.0, 0.0, 0.0),
        };
        let demand = solver
            .measure_content_demand(
                &allocation,
                &ChildFrameContentMeasurement {
                    frame_demand: FrameDemand::default(),
                    child_frame_allocations: vec![child_allocation],
                },
            )
            .unwrap();
        let plan = solver.coordinate_content(&allocation, &demand).unwrap();
        let layout = solver.realize_content(allocation, demand, plan).unwrap();

        assert_eq!(layout.child_frame_allocations, vec![child_allocation]);
        assert_eq!(layout.child_frame_allocations[0].owned_slabs.right, 5.0);
    }
}
