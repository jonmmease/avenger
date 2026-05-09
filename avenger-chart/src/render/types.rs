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

/// Opt-in counters for evaluation performance diagnostics.
///
/// This is intentionally not part of the normal evaluated plot output. Use it
/// from focused tests or benchmarks when changing evaluation algorithms.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationMetrics {
    /// Metrics for recursive facet layout measurement.
    pub facet_layout: FacetLayoutMetrics,
}

impl EvaluationMetrics {
    pub(crate) fn record_plot_component_measure_call(&mut self, facet_depth: usize) {
        self.facet_layout
            .record_plot_component_measure_call(facet_depth);
    }

    pub(crate) fn record_facet_band_measure_run(
        &mut self,
        phase5_leaf_measure_count: usize,
        phase5_non_leaf_probe_aggregate_count: usize,
        phase5_non_leaf_full_measure_count: usize,
        phase6_full_measure_count: usize,
    ) {
        self.facet_layout.record_facet_band_measure_run(
            phase5_leaf_measure_count,
            phase5_non_leaf_probe_aggregate_count,
            phase5_non_leaf_full_measure_count,
            phase6_full_measure_count,
        );
    }
}

/// Opt-in counters for recursive facet layout measurement diagnostics.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FacetLayoutMetrics {
    /// Number of times `CompiledPlot::measure_plot_components` ran.
    pub plot_component_measure_calls: usize,
    /// `measure_plot_components` calls grouped by `facet_path.len()`.
    pub plot_component_measure_calls_by_facet_depth: Vec<usize>,
    /// Number of completed facet-band measurement pipelines.
    pub facet_band_measure_runs: usize,
    /// Number of leaf cell measurements in phase 5 overflow probes.
    pub phase5_leaf_measure_count: usize,
    /// Number of non-leaf synthesized probe aggregates in phase 5.
    pub phase5_non_leaf_probe_aggregate_count: usize,
    /// Number of full subtree measurements used to synthesize non-leaf phase 5 probes.
    pub phase5_non_leaf_full_measure_count: usize,
    /// Number of full cell measurements in phase 6 finalization.
    pub phase6_full_measure_count: usize,
}

impl FacetLayoutMetrics {
    pub(crate) fn record_plot_component_measure_call(&mut self, facet_depth: usize) {
        self.plot_component_measure_calls += 1;
        if self.plot_component_measure_calls_by_facet_depth.len() <= facet_depth {
            self.plot_component_measure_calls_by_facet_depth
                .resize(facet_depth + 1, 0);
        }
        self.plot_component_measure_calls_by_facet_depth[facet_depth] += 1;
    }

    pub(crate) fn record_facet_band_measure_run(
        &mut self,
        phase5_leaf_measure_count: usize,
        phase5_non_leaf_probe_aggregate_count: usize,
        phase5_non_leaf_full_measure_count: usize,
        phase6_full_measure_count: usize,
    ) {
        self.facet_band_measure_runs += 1;
        self.phase5_leaf_measure_count += phase5_leaf_measure_count;
        self.phase5_non_leaf_probe_aggregate_count += phase5_non_leaf_probe_aggregate_count;
        self.phase5_non_leaf_full_measure_count += phase5_non_leaf_full_measure_count;
        self.phase6_full_measure_count += phase6_full_measure_count;
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
