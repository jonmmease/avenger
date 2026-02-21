//! Core types for rendering pipeline

use indexmap::IndexMap;
use taffy::Size;

use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::{
    guide::OverflowSpaceRequirement,
    layout::{LayoutBounds, LayoutResult, LegendLayoutInfo},
    legend::LegendPosition,
};

/// Selects which layout snapshot to render during evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSnapshot {
    /// Initial local measurement snapshot before global coordination.
    Initial,
    /// Snapshot after global coordination, before optional canvas refinement.
    Coordinated,
    /// Final snapshot used by the default render pipeline.
    Final,
}

/// Runtime evaluation options for selecting layout snapshots and debug overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationOptions {
    /// Which layout snapshot to render.
    pub layout_snapshot: LayoutSnapshot,
    /// Whether to draw layout debug overlays when no env override is set.
    pub debug_layout_lines: bool,
}

impl Default for EvaluationOptions {
    fn default() -> Self {
        Self {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_lines: false,
        }
    }
}

/// Measurement and layout information for a single legend
#[derive(Debug, Clone)]
pub struct LegendMeasurement {
    /// Size of the legend (width, height)
    pub size: Size<f32>,
    /// Whether the legend height is flexible (e.g., for colorbars)
    pub flexible: bool,
    /// Position of the legend
    pub position: LegendPosition,
}

/// Type for legend measurements used in layout computation
pub type LegendMeasurements = IndexMap<String, LegendMeasurement>;

/// Result of layout computation from Taffy
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Complete layout with all component positions
    pub taffy_layout: LayoutResult,
    /// Computed canvas size (may differ from requested when using plot_size)
    pub canvas_size: (f32, f32),
    /// Guide-only overflow (axes, tick labels, axis titles).
    /// Used for cross-subplot alignment in grid facets.
    pub overflow: OverflowSpaceRequirement,
    /// Total overflow including guide overflow plus legend dimensions.
    /// Used for outer facet positioning and canvas sizing.
    pub total_overflow: OverflowSpaceRequirement,
    /// Legend bounding box dimensions for cross-subplot alignment
    pub legend_info: LegendLayoutInfo,
}

impl LayoutSolution {
    /// Get the plot area bounds
    pub fn plot_area_bounds(&self) -> &LayoutBounds {
        &self.taffy_layout.plot_area
    }
}

/// Result of evaluating a plot to scene graph components
pub struct EvaluatedPlot {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<SceneGraphRTree>,
}
