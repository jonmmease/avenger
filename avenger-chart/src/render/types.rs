//! Core types for rendering pipeline

use avenger_scenegraph::scene_graph::SceneGraph;
use indexmap::IndexMap;

/// Measurement and layout information for a single legend
#[derive(Debug, Clone)]
pub struct LegendMeasurement {
    /// Size of the legend (width, height)
    pub size: taffy::Size<f32>,
    /// Whether the legend height is flexible (e.g., for colorbars)
    pub flexible: bool,
    /// Position of the legend
    pub position: crate::legend::LegendPosition,
}

/// Type for legend measurements used in layout computation
pub type LegendMeasurements = IndexMap<String, LegendMeasurement>;

/// Result of layout computation from Taffy
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Complete layout with all component positions
    pub taffy_layout: crate::layout::LayoutResult,
    /// Computed canvas size (may differ from requested when using plot_size)
    pub canvas_size: (f32, f32),
    /// Guide overflow measured during layout (top, right, bottom, left)
    pub overflow: crate::guide::OverflowSpaceRequirement,
    /// Legend bounding box dimensions for cross-subplot alignment
    pub legend_info: crate::layout::LegendLayoutInfo,
}

impl LayoutSolution {
    /// Get the plot area bounds
    pub fn plot_area_bounds(&self) -> &crate::layout::LayoutBounds {
        &self.taffy_layout.plot_area
    }
}

/// Result of evaluating a plot to scene graph components
pub struct EvaluatedPlot {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<avenger_geometry::rtree::SceneGraphRTree>,
}
