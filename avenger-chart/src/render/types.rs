//! Core types for rendering pipeline

use avenger_scenegraph::scene_graph::SceneGraph;

/// Estimated proportion of plot area relative to total size for initial scale computation.
/// This is used before layout is calculated to build scales with approximate dimensions.
/// The actual plot area is typically 70-85% of total size after padding for axes/legends.
pub(crate) const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

/// Result of layout computation from Taffy
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Complete layout with all component positions
    pub taffy_layout: crate::layout::LayoutResult,
    /// Computed canvas size (may differ from requested when using plot_size)
    pub canvas_size: (f32, f32),
}

impl LayoutSolution {
    /// Get the plot area bounds
    pub fn plot_area_bounds(&self) -> &crate::layout::LayoutBounds {
        &self.taffy_layout.plot_area
    }
}

/// Result of rendering a plot to scene graph components
pub struct RenderResult {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<avenger_geometry::rtree::SceneGraphRTree>,
}
