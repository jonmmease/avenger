//! Core types for rendering pipeline

use avenger_scenegraph::scene_graph::SceneGraph;

/// Estimated proportion of plot area relative to total size for initial scale computation.
/// This is used before layout is calculated to build scales with approximate dimensions.
/// The actual plot area is typically 70-85% of total size after padding for axes/legends.
pub(crate) const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

/// Padding around a plot area
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Result of layout computation, containing padding and Taffy layout
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Padding around the plot area
    pub padding: Padding,
    /// The actual plot area rectangle (x, y, width, height)
    pub plot_area: (f32, f32, f32, f32),
    /// Taffy layout result for dynamic positioning
    pub taffy_layout: crate::layout::LayoutResult,
}

impl LayoutSolution {
    /// Get the plot area bounds as a tuple
    pub fn plot_area_bounds(&self) -> (f32, f32, f32, f32) {
        self.plot_area
    }
}

/// Result of rendering a plot to scene graph components
pub struct RenderResult {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<avenger_geometry::rtree::SceneGraphRTree>,
}
