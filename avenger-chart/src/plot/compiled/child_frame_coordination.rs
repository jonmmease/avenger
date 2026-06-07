//! Read-only layout coordination nodes for measured child-frame containers.
//!
//! These nodes describe measured container requirements without mutating the
//! measurement tree. Later coordination phases can group compatible nodes,
//! merge their requirements, and apply aligned solutions through concrete
//! container adapters.

use crate::{
    concat::{ConcatCoordMeasurement, GridShape, GridSlotRect, GridTrackRequirements},
    plot::compiled::{
        ChildFrameKey, ComponentsMeasurement, ContainerPathSegment,
        container_path_without_facet_segments,
    },
};

use avenger_chart_core::AvengerChartError;

/// One physical measured occurrence of a child-frame container.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameContainerInstanceKey {
    pub(crate) container_path: Vec<ContainerPathSegment>,
}

impl ChildFrameContainerInstanceKey {
    pub(crate) fn new(container_path: Vec<ContainerPathSegment>) -> Self {
        Self { container_path }
    }
}

/// Semantic identity used to align equivalent container instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameContainerTemplateKey {
    pub(crate) container_path_template: Vec<ContainerPathSegment>,
}

impl ChildFrameContainerTemplateKey {
    fn from_instance(
        instance_key: &ChildFrameContainerInstanceKey,
        scope: LayoutCoordinationScope,
    ) -> Self {
        let container_path_template = match scope {
            LayoutCoordinationScope::TemplatePathWithoutFacetSegments => {
                container_path_without_facet_segments(&instance_key.container_path)
            }
        };
        Self {
            container_path_template,
        }
    }
}

/// Kind of child-frame container described by a layout coordination node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameContainerKind {
    GridConcat,
}

/// Rule used to project a physical instance into a reusable template key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LayoutCoordinationScope {
    TemplatePathWithoutFacetSegments,
}

/// One child slot inside a layout coordination node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChildFrameLayoutSlot {
    pub(crate) child_index: usize,
    pub(crate) child_key: ChildFrameKey,
    pub(crate) slot: GridSlotRect,
}

/// Grouping key for compatible alignment nodes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayoutAlignmentKey {
    pub(crate) template_key: ChildFrameContainerTemplateKey,
    pub(crate) kind: ChildFrameContainerKind,
    pub(crate) topology: ChildFrameLayoutTopology,
}

/// Physical topology that must match for nodes to share one aligned solution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameLayoutTopology {
    Grid {
        shape: GridShape,
        slots: Vec<ChildFrameLayoutSlotTopology>,
    },
}

/// Slot topology omits measured sizes but preserves child identity and slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameLayoutSlotTopology {
    pub(crate) child_key: ChildFrameKey,
    pub(crate) slot: GridSlotRect,
}

/// Read-only measured layout coordination node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChildFrameLayoutCoordinationNode {
    pub(crate) instance_key: ChildFrameContainerInstanceKey,
    pub(crate) template_key: ChildFrameContainerTemplateKey,
    pub(crate) kind: ChildFrameContainerKind,
    pub(crate) alignment_scope: LayoutCoordinationScope,
    pub(crate) topology: ChildFrameLayoutTopology,
    pub(crate) slots: Vec<ChildFrameLayoutSlot>,
    pub(crate) requirements: ChildFrameLayoutRequirements,
}

impl ChildFrameLayoutCoordinationNode {
    pub(crate) fn alignment_key(&self) -> LayoutAlignmentKey {
        LayoutAlignmentKey {
            template_key: self.template_key.clone(),
            kind: self.kind,
            topology: self.topology.clone(),
        }
    }
}

/// Local requirements exported by a measured child-frame container.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChildFrameLayoutRequirements {
    Grid(GridTrackRequirements),
}

pub(crate) fn grid_layout_coordination_node(
    concat: &ConcatCoordMeasurement,
) -> Result<Option<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let Some(shape) = concat.grid_shape() else {
        return Ok(None);
    };

    let instance_key = ChildFrameContainerInstanceKey::new(concat.container_path()?);
    let alignment_scope = LayoutCoordinationScope::TemplatePathWithoutFacetSegments;
    let template_key =
        ChildFrameContainerTemplateKey::from_instance(&instance_key, alignment_scope);
    let slots = concat.grid_layout_slots()?;
    let topology = ChildFrameLayoutTopology::Grid {
        shape,
        slots: slots
            .iter()
            .map(|slot| ChildFrameLayoutSlotTopology {
                child_key: slot.child_key.clone(),
                slot: slot.slot,
            })
            .collect(),
    };
    let requirements = ChildFrameLayoutRequirements::Grid(concat.grid_track_requirements()?);

    Ok(Some(ChildFrameLayoutCoordinationNode {
        instance_key,
        template_key,
        kind: ChildFrameContainerKind::GridConcat,
        alignment_scope,
        topology,
        slots,
        requirements,
    }))
}

pub(crate) fn collect_child_frame_layout_coordination_nodes(
    measurement: &ComponentsMeasurement,
) -> Result<Vec<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let mut nodes = Vec::new();
    collect_child_frame_layout_coordination_nodes_into(measurement, &mut nodes)?;
    Ok(nodes)
}

fn collect_child_frame_layout_coordination_nodes_into(
    measurement: &ComponentsMeasurement,
    nodes: &mut Vec<ChildFrameLayoutCoordinationNode>,
) -> Result<(), AvengerChartError> {
    if let Some(concat) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
    {
        if let Some(node) = grid_layout_coordination_node(concat)? {
            nodes.push(node);
        }
    }

    if let Some(container) = measurement.child_frame_container_view()? {
        for placement in container.placement().render_placements() {
            let child = container.child_measurement(placement.child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame measurement for child index {} while collecting layout coordination nodes",
                    placement.child_index
                ))
            })?;
            collect_child_frame_layout_coordination_nodes_into(child, nodes)?;
        }
    }

    Ok(())
}
