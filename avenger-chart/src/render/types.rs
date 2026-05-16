//! Core types for rendering pipeline

use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::{
    guide::OverflowSpaceRequirement,
    layout::{FrameLayout, LayoutBounds, LegendLayoutInfo, Size2D},
    legend::LegendPosition,
};

/// Selects which layout snapshot to render during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutSnapshot {
    /// Final snapshot used by the default render pipeline.
    Final,
    /// Render a whole-chart checkpoint before final output.
    Whole(WholeChartSnapshot),
    /// Render a selected facet subtree at a local measurement checkpoint.
    FacetSubtree(FacetSubtreeSnapshot),
}

/// Whole-chart layout checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WholeChartSnapshot {
    /// After recursive local measurement, before global coordination.
    LocalMeasured,
    /// During global facet coordination.
    Coordination(CoordinationCheckpoint),
    /// During final canvas/plot-area-sized realization or optional refinement.
    Refinement {
        /// Zero is the mandatory realization pass; positive values are optional refinement passes.
        iteration: usize,
        checkpoint: RefinementCheckpoint,
    },
}

/// Checkpoints inside the global facet coordination cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinationCheckpoint {
    /// Initial layout requirements have been aggregated and distributed.
    InitialRequirementsApplied,
    /// Affected measurements have been retargeted.
    RetargetComplete,
    /// Retargeted overflow and layout requirements have been reconciled.
    RetargetedRequirementsApplied,
    /// Coordinated plot-area and scale-range updates have been propagated to descendants.
    FinalPropagationComplete,
}

/// Checkpoints inside the final layout realization/refinement loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefinementCheckpoint {
    /// Candidate guide overflow and chart layout have been recomputed for this iteration.
    CandidateLayoutMeasured,
    /// The candidate layout has been installed and plot area sizes retargeted.
    PlotAreaRetargeted,
    /// Overflow growth triggered another coordination cycle for the next iteration.
    Recoordinated,
}

/// Render a single facet subtree rather than the full chart.
#[derive(Debug, Clone, PartialEq)]
pub struct FacetSubtreeSnapshot {
    pub selector: FacetSubtreeSelector,
    pub checkpoint: FacetSubtreeCheckpoint,
}

/// Selector for locating a facet subtree in the measured tree.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetSubtreeSelector {
    /// Select by facet value path, e.g. `["Ops", "Support"]`.
    ByFacetPath(Vec<ScalarValue>),
    /// Select by child indices in the measured coordination tree.
    ByCoordinationNodePath(Vec<usize>),
}

/// Local facet-subtree checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacetSubtreeCheckpoint {
    /// Estimated-size overflow probe, before local plot-area retargeting.
    EstimatedOverflowProbe,
    /// Locally retargeted layout, before global coordination.
    LocalRetargetedLayout,
    /// After global coordination, before final realization/refinement.
    CoordinatedLayout,
    /// Final selected subtree.
    FinalLayout,
}

/// Selects which layout debug overlay geometry to render.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayoutDebugOverlayMode {
    /// Do not render layout debug overlays.
    #[default]
    Off,
    /// Render frame components: plot area, guide overflows, legends, titles, and subtitles.
    Components,
    /// Render frame/content allocation and demand: frame, content, owned slabs, residual overflow, and child allocations.
    AllocationDemand,
    /// Render both component bounds and allocation/demand geometry.
    All,
}

impl LayoutDebugOverlayMode {
    pub(crate) fn components_enabled(self) -> bool {
        matches!(self, Self::Components | Self::All)
    }

    pub(crate) fn allocation_demand_enabled(self) -> bool {
        matches!(self, Self::AllocationDemand | Self::All)
    }

    pub(crate) fn enabled(self) -> bool {
        self != Self::Off
    }
}

/// Runtime evaluation options for selecting layout snapshots and debug overlays.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationOptions {
    /// Which layout snapshot to render.
    pub layout_snapshot: LayoutSnapshot,
    /// Which layout debug overlay to draw when no env override is set.
    pub debug_layout_overlay: LayoutDebugOverlayMode,
    /// Controls optional facet layout refinement after the mandatory measure-once pass.
    pub facet_layout_refinement: FacetLayoutRefinement,
}

/// Controls optional repeated facet measurement/layout passes.
///
/// One mandatory pass is always performed. The default runs one refinement pass;
/// set `max_refinement_passes` to zero for the fastest measure-once path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FacetLayoutRefinement {
    /// Number of additional measure/retarget refinement passes after the mandatory pass.
    pub max_refinement_passes: usize,
    /// Minimum overflow growth that should be treated as layout-significant.
    pub overflow_growth_epsilon: f32,
}

impl Default for FacetLayoutRefinement {
    fn default() -> Self {
        Self {
            max_refinement_passes: 1,
            overflow_growth_epsilon: 0.5,
        }
    }
}

impl Default for EvaluationOptions {
    fn default() -> Self {
        Self {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::Off,
            facet_layout_refinement: FacetLayoutRefinement::default(),
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
        estimated_overflow_leaf_measure_count: usize,
        estimated_overflow_non_leaf_aggregate_count: usize,
        estimated_overflow_non_leaf_full_measure_count: usize,
    ) {
        self.facet_layout.record_facet_band_measure_run(
            estimated_overflow_leaf_measure_count,
            estimated_overflow_non_leaf_aggregate_count,
            estimated_overflow_non_leaf_full_measure_count,
        );
    }

    pub(crate) fn record_facet_refinement_pass(&mut self) {
        self.facet_layout.refinement_pass_count += 1;
    }

    pub(crate) fn record_facet_refinement_converged(&mut self) {
        self.facet_layout.refinement_converged = true;
    }

    pub(crate) fn record_facet_refinement_hit_max_passes(&mut self) {
        self.facet_layout.refinement_hit_max_passes = true;
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
    /// Number of leaf cell measurements during estimated-overflow probes.
    pub estimated_overflow_leaf_measure_count: usize,
    /// Number of non-leaf estimated-overflow probe aggregates.
    pub estimated_overflow_non_leaf_aggregate_count: usize,
    /// Number of full subtree measurements used to synthesize non-leaf estimated-overflow probes.
    pub estimated_overflow_non_leaf_full_measure_count: usize,
    /// Number of top-level facet refinement passes after the mandatory pass.
    pub refinement_pass_count: usize,
    /// Whether iterative facet refinement stopped because no overflow grew.
    pub refinement_converged: bool,
    /// Whether iterative facet refinement consumed the configured pass budget.
    pub refinement_hit_max_passes: bool,
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
        estimated_overflow_leaf_measure_count: usize,
        estimated_overflow_non_leaf_aggregate_count: usize,
        estimated_overflow_non_leaf_full_measure_count: usize,
    ) {
        self.facet_band_measure_runs += 1;
        self.estimated_overflow_leaf_measure_count += estimated_overflow_leaf_measure_count;
        self.estimated_overflow_non_leaf_aggregate_count +=
            estimated_overflow_non_leaf_aggregate_count;
        self.estimated_overflow_non_leaf_full_measure_count +=
            estimated_overflow_non_leaf_full_measure_count;
    }
}

/// Measurement and layout information for a single legend
#[derive(Debug, Clone)]
pub struct LegendMeasurement {
    /// Size of the legend (width, height)
    pub size: Size2D,
    /// Whether the legend height is flexible (e.g., for colorbars)
    pub flexible: bool,
    /// Position of the legend
    pub position: LegendPosition,
}

/// Type for legend measurements used in layout computation
pub type LegendMeasurements = IndexMap<String, LegendMeasurement>;

/// Result of frame layout computation.
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Complete layout with all component positions
    pub frame_layout: FrameLayout,
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
        &self.frame_layout.plot_area
    }
}

/// Result of evaluating a plot to scene graph components
pub struct EvaluatedPlot {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<SceneGraphRTree>,
}
