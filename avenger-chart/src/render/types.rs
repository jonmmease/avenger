//! Core types for rendering pipeline

use std::time::Duration;

use datafusion::common::ScalarValue;

pub use avenger_chart_legend::{LegendMeasurement, LegendMeasurements};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::{
    guide::OverflowSpaceRequirement,
    layout::{FrameLayout, LayoutBounds, LegendLayoutInfo},
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

/// Evaluation strategy requested by a reusable plot session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvaluationMode {
    /// Canonical evaluation semantics. Valid caches may be reused.
    #[default]
    Exact,
    /// Low-latency interaction mode for transient updates.
    ///
    /// `PlotSession` disables optional facet refinement passes in this mode so
    /// the settled `Exact` evaluation can perform the slower convergence work.
    Preview,
    /// Canonical evaluation while bypassing measurement-profile caches.
    ///
    /// Phase 1 has no measurement-profile cache yet, so this also falls back to
    /// exact evaluation while preserving the requested mode in metrics.
    ForceRemeasure,
}

/// Reason a Preview evaluation could not reuse the current layout profile.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewProfileFallbackReason {
    /// The session has not produced an exact layout profile yet.
    NoPriorProfile,
    /// Preview profile reuse only supports final layout snapshots.
    NonFinalSnapshot,
    /// The facet tree changed in a way that is not a supported logical reflow.
    PhysicalStructureMismatch,
    /// A responsive wrap tree changed logical slots, ordering, or membership.
    LogicalStructureMismatch,
    /// A logical reflow was possible in principle, but no terminal cell profiles
    /// were available to reuse.
    MissingTerminalProfile,
    /// Profile dependency parameters are known to be incompatible with the
    /// current request.
    IncompatibleParams,
    /// Profile scale signatures are known to be incompatible with the current
    /// request.
    IncompatibleScales,
    /// The profiled child structure is not supported by the current reuse path.
    UnsupportedChildStructure,
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
/// One mandatory pass is always performed. The default allows two refinement passes;
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
            max_refinement_passes: 2,
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
    /// Requested evaluation mode for this run.
    pub mode: EvaluationMode,
    /// Metrics for recursive facet layout measurement.
    pub facet_layout: FacetLayoutMetrics,
    /// Metrics for top-level evaluation pipeline work that future sessions
    /// should cache, reuse, or bypass.
    pub pipeline: EvaluationPipelineMetrics,
    /// Wall-clock timings for the most important evaluation phases.
    pub timings: EvaluationTimingMetrics,
}

impl EvaluationMetrics {
    pub(crate) fn record_preview_attempt_duration(&mut self, duration: Duration) {
        self.timings.preview_attempt_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_structure_reflow_duration(&mut self, duration: Duration) {
        self.timings.preview_structure_reflow_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_measure_cells_overflow_probe_duration(&mut self, duration: Duration) {
        self.timings.measure_cells_overflow_probe_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_refresh_reused_profile_layout_duration(&mut self, duration: Duration) {
        self.timings.refresh_reused_profile_layout_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_guide_overflow_measure_duration(&mut self, duration: Duration) {
        self.timings.guide_overflow_measure_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_build_plot_components_duration(&mut self, duration: Duration) {
        self.timings.build_plot_components_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_components_to_evaluated_plot_duration(&mut self, duration: Duration) {
        self.timings.components_to_evaluated_plot_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_facet_tree_build(&mut self) {
        self.pipeline.facet_tree_builds += 1;
    }

    pub(crate) fn record_facet_semantic_cache_hits(&mut self, count: usize) {
        self.pipeline.facet_semantic_cache_hits += count;
    }

    pub(crate) fn record_facet_semantic_cache_misses(&mut self, count: usize) {
        self.pipeline.facet_semantic_cache_misses += count;
    }

    pub(crate) fn record_facet_scale_precompute_cache_hit(&mut self) {
        self.pipeline.facet_scale_precompute_cache_hits += 1;
    }

    pub(crate) fn record_facet_scale_precompute_cache_miss(&mut self) {
        self.pipeline.facet_scale_precompute_cache_misses += 1;
    }

    pub(crate) fn record_scale_builder_build(&mut self) {
        self.pipeline.scale_builder_builds += 1;
    }

    pub(crate) fn record_scale_domain_cache_hit(&mut self) {
        self.pipeline.scale_domain_cache_hits += 1;
    }

    pub(crate) fn record_scale_domain_cache_miss(&mut self) {
        self.pipeline.scale_domain_cache_misses += 1;
    }

    pub(crate) fn record_scale_domain_collect(&mut self) {
        self.pipeline.scale_domain_collects += 1;
    }

    pub(crate) fn record_guide_overflow_measure_call(&mut self) {
        self.pipeline.guide_overflow_measure_calls += 1;
    }

    pub(crate) fn record_guide_overflow_cache_hit(&mut self) {
        self.pipeline.guide_overflow_cache_hits += 1;
    }

    pub(crate) fn record_guide_overflow_cache_miss(&mut self) {
        self.pipeline.guide_overflow_cache_misses += 1;
    }

    pub(crate) fn record_legend_plan_build(&mut self) {
        self.pipeline.legend_plan_builds += 1;
    }

    pub(crate) fn record_legend_measurements(&mut self, count: usize) {
        self.pipeline.legend_measurements += count;
    }

    pub(crate) fn record_legend_measurement_cache_hit(&mut self) {
        self.pipeline.legend_measurement_cache_hits += 1;
    }

    pub(crate) fn record_legend_measurement_cache_miss(&mut self) {
        self.pipeline.legend_measurement_cache_misses += 1;
    }

    pub(crate) fn record_text_measurement_cache_hit(&mut self) {
        self.pipeline.text_measurement_cache_hits += 1;
    }

    pub(crate) fn record_text_measurement_cache_miss(&mut self) {
        self.pipeline.text_measurement_cache_misses += 1;
    }

    pub(crate) fn record_preview_profile_reuse(&mut self) {
        self.pipeline.preview_profile_reuses += 1;
    }

    pub(crate) fn record_preview_profile_miss(&mut self) {
        self.pipeline.preview_profile_misses += 1;
    }

    pub(crate) fn record_preview_fallback(&mut self) {
        self.pipeline.preview_fallbacks += 1;
    }

    pub(crate) fn record_preview_profile_fallback_reasons(
        &mut self,
        reasons: impl IntoIterator<Item = PreviewProfileFallbackReason>,
    ) {
        self.pipeline
            .preview_profile_fallback_reasons
            .extend(reasons);
    }

    pub(crate) fn record_preview_structure_reflow_reuse(&mut self) {
        self.pipeline.preview_structure_reflow_reuses += 1;
    }

    pub(crate) fn record_preview_structure_reflow_miss(&mut self) {
        self.pipeline.preview_structure_reflow_misses += 1;
    }

    pub(crate) fn record_preview_data_mark_reuse(&mut self) {
        self.pipeline.preview_data_mark_reuses += 1;
    }

    pub(crate) fn record_preview_data_mark_reuse_miss(&mut self) {
        self.pipeline.preview_data_mark_reuse_misses += 1;
    }

    pub(crate) fn record_facet_cell_measurement_profile_reuse(&mut self) {
        self.pipeline.facet_cell_measurement_profile_reuses += 1;
    }

    pub(crate) fn record_facet_cell_measurement_profile_miss(&mut self) {
        self.pipeline.facet_cell_measurement_profile_misses += 1;
    }

    pub(crate) fn record_facet_cell_measurement_profile_chrome_refresh(&mut self) {
        self.pipeline
            .facet_cell_measurement_profile_chrome_refreshes += 1;
    }

    pub(crate) fn record_skipped_component_measure_calls(&mut self, count: usize) {
        self.pipeline.skipped_component_measure_calls += count;
    }

    pub(crate) fn record_mark_data_full_collect(&mut self) {
        self.pipeline.mark_data_collects += 1;
        self.pipeline.mark_data_full_collects += 1;
    }

    pub(crate) fn record_mark_data_array_collect(&mut self) {
        self.pipeline.mark_data_collects += 1;
        self.pipeline.mark_data_array_collects += 1;
    }

    pub(crate) fn record_mark_data_scalar_collect(&mut self) {
        self.pipeline.mark_data_collects += 1;
        self.pipeline.mark_data_scalar_collects += 1;
    }

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

/// Opt-in wall-clock timings for evaluation pipeline diagnostics.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationTimingMetrics {
    /// Total time spent attempting Preview reuse before returning reused output
    /// or falling back to Exact evaluation.
    pub preview_attempt_us: u64,
    /// Time spent in logical structure reflow when a responsive facet wrap
    /// changes physical row/column layout but keeps the same logical cells.
    pub preview_structure_reflow_us: u64,
    /// Time spent probing facet cell overflow during facet-band measurement.
    pub measure_cells_overflow_probe_us: u64,
    /// Time spent refreshing guide/layout chrome on reused profile cells.
    pub refresh_reused_profile_layout_us: u64,
    /// Time spent measuring guide overflow.
    pub guide_overflow_measure_us: u64,
    /// Time spent rendering plot components from measurements.
    pub build_plot_components_us: u64,
    /// Time spent assembling final scene graph and R-tree from plot components.
    pub components_to_evaluated_plot_us: u64,
}

pub(crate) fn duration_micros_u64(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Opt-in counters for evaluation pipeline diagnostics.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationPipelineMetrics {
    /// Number of times a top-level `EvaluatedFacetTree` was built.
    pub facet_tree_builds: usize,
    /// Number of session semantic facet cache hits while building facet trees.
    pub facet_semantic_cache_hits: usize,
    /// Number of session semantic facet cache misses while building facet trees.
    pub facet_semantic_cache_misses: usize,
    /// Number of session facet scale-precompute store hits.
    pub facet_scale_precompute_cache_hits: usize,
    /// Number of session facet scale-precompute store misses.
    pub facet_scale_precompute_cache_misses: usize,
    /// Number of scale-builder construction requests observed by the chart
    /// runtime. This counts calls to the current domain-inference pipeline,
    /// not configured scale range rebuilds.
    pub scale_builder_builds: usize,
    /// Number of scale-domain cache hits in a reusable `PlotSession`.
    pub scale_domain_cache_hits: usize,
    /// Number of scale-domain cache misses in a reusable `PlotSession`.
    pub scale_domain_cache_misses: usize,
    /// Number of DataFusion collect calls made while inferring scale domains.
    pub scale_domain_collects: usize,
    /// Number of exact guide-overflow profile cache hits.
    pub guide_overflow_cache_hits: usize,
    /// Number of exact guide-overflow profile cache misses.
    pub guide_overflow_cache_misses: usize,
    /// Number of coordinate-guide overflow measurements.
    pub guide_overflow_measure_calls: usize,
    /// Number of legend plan builds.
    pub legend_plan_builds: usize,
    /// Number of legend groups measured while building plans.
    pub legend_measurements: usize,
    /// Number of legend measurement-profile cache hits.
    pub legend_measurement_cache_hits: usize,
    /// Number of legend measurement-profile cache misses.
    pub legend_measurement_cache_misses: usize,
    /// Number of text layout measurement cache hits.
    pub text_measurement_cache_hits: usize,
    /// Number of text layout measurement cache misses.
    pub text_measurement_cache_misses: usize,
    /// Number of preview evaluations that reused a previous measurement profile.
    pub preview_profile_reuses: usize,
    /// Number of preview evaluations that could not reuse a previous measurement profile.
    pub preview_profile_misses: usize,
    /// Number of preview evaluations that fell back to exact measurement.
    pub preview_fallbacks: usize,
    /// Typed reasons that Preview could not use the current layout profile.
    pub preview_profile_fallback_reasons: Vec<PreviewProfileFallbackReason>,
    /// Number of preview evaluations that reused profiles while rebuilding a changed layout tree.
    pub preview_structure_reflow_reuses: usize,
    /// Number of preview evaluations that considered structure reflow but could not use it.
    pub preview_structure_reflow_misses: usize,
    /// Number of preview evaluations that reused rendered data/facet subtree marks.
    pub preview_data_mark_reuses: usize,
    /// Number of preview evaluations that could not reuse rendered data/facet subtree marks.
    pub preview_data_mark_reuse_misses: usize,
    /// Number of facet cell measurements reused from a layout profile.
    pub facet_cell_measurement_profile_reuses: usize,
    /// Number of facet cell profile lookups that missed during layout-profile preview.
    pub facet_cell_measurement_profile_misses: usize,
    /// Number of reused facet cell profiles whose guide/layout chrome was refreshed.
    pub facet_cell_measurement_profile_chrome_refreshes: usize,
    /// Number of component measurement calls skipped by preview reuse.
    pub skipped_component_measure_calls: usize,
    /// Number of DataFusion collect calls made while preparing mark render data.
    pub mark_data_collects: usize,
    /// Mark data collect calls for marks requesting full data batches.
    pub mark_data_full_collects: usize,
    /// Mark data collect calls for array channel batches.
    pub mark_data_array_collects: usize,
    /// Mark data collect calls for scalar channel batches.
    pub mark_data_scalar_collects: usize,
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
