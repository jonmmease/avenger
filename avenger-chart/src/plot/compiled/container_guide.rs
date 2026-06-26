//! Internal guide helpers for child-frame containers.
//!
//! Public coordinate guides still belong to each coordinate system. The helpers
//! here centralize the generic work those guides need once they have a measured
//! child-frame container view.

use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError, guide::OverflowSpaceRequirement, layout::LayoutBounds, theme::Theme,
};

use super::{
    ChildFrameContainerView, ContainerLabelPlacement,
    child_frame_container::child_frame_container_overflow,
    container_labels::{
        container_label_items_from_child_frame_container, measure_container_label_slab,
        render_container_labels,
    },
};

/// Measure child-frame container overflow plus container-owned label slabs.
pub(crate) fn measure_child_frame_container_guide_overflow(
    plot_width: f32,
    plot_height: f32,
    container: &ChildFrameContainerView<'_>,
    label_placement: Option<ContainerLabelPlacement>,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let mut overflow = child_frame_container_overflow(plot_width, plot_height, container)?;
    let Some(label_placement) = label_placement else {
        return Ok(overflow);
    };

    let label_slab = measure_container_label_slab(
        label_placement,
        &container_label_items_from_child_frame_container(container)?,
        theme,
        params,
    );
    match label_placement {
        ContainerLabelPlacement::Top => overflow.top += label_slab,
        ContainerLabelPlacement::Left => overflow.left += label_slab,
    }
    Ok(overflow)
}

/// Render labels owned by a child-frame container guide.
pub(crate) fn render_child_frame_container_guide_labels(
    container: &ChildFrameContainerView<'_>,
    label_placement: Option<ContainerLabelPlacement>,
    plot_bounds: &LayoutBounds,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let Some(label_placement) = label_placement else {
        return Ok(Vec::new());
    };

    Ok(render_container_labels(
        label_placement,
        &container_label_items_from_child_frame_container(container)?,
        plot_bounds,
        theme,
        params,
    ))
}
