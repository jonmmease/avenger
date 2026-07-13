//! Core types for rendering pipeline

use std::{collections::HashMap, sync::Arc, time::Duration};

use avenger_chart_core::MaterializationRequest;
use avenger_chart_core::{AvengerChartError, CoordinateSystemTransform, SceneQueryDatumField};
use avenger_resource::ResourceRequest;
use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    arrow::{
        array::new_null_array,
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
};

pub use avenger_chart_legend::{LegendMeasurement, LegendMeasurements};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
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

/// Checkpoints inside the global facet coordination cycle
/// (snapshot -> solve -> install channels -> adopt geometry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinationCheckpoint {
    /// The coordination solution has been installed on every band
    /// (channel values readable; geometry not yet adopted).
    ChannelsInstalled,
    /// The solve's implied geometry has been adopted: plot areas and
    /// scale ranges moved, chrome frozen at its epoch measurements.
    Adopted,
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
    /// Canonical evaluation while bypassing reusable measurement/profile caches.
    ///
    /// This is useful when a host wants a fresh settled measurement instead of
    /// reusing guide, legend, text, or layout-profile state from the session.
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
    /// Whether to build a scene-graph R-tree in the returned `EvaluatedPlot`.
    ///
    /// App hosts that immediately build their own R-tree from the returned scene
    /// graph can disable this to avoid duplicate interaction-index work.
    pub build_scene_rtree: bool,
    /// Priority bias applied to async materialization requests emitted during this
    /// evaluation.
    #[doc(hidden)]
    pub materialization_priority: f32,
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
            build_scene_rtree: true,
            materialization_priority: 0.0,
        }
    }
}

/// Physical result-cache activity attributed to one evaluation
/// (counter deltas across the evaluation, plus end-of-evaluation state).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysicalCacheMetricsDelta {
    /// Cache hits served during this evaluation.
    pub hits: u64,
    /// Cache misses during this evaluation.
    pub misses: u64,
    /// Writes admitted during this evaluation.
    pub admitted_writes: u64,
    /// Writes committed during this evaluation.
    pub committed_writes: u64,
    /// Writes discarded during this evaluation.
    pub discarded_writes: u64,
    /// Committed entries retained at the END of the evaluation (state).
    pub entries: usize,
    /// Approximate retained bytes at the END of the evaluation (state).
    pub bytes: usize,
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
    /// Physical result-cache activity for this evaluation, when a cache is
    /// installed on the session context (`None` on cacheless contexts).
    /// Set once per evaluation at the session level; not merged by
    /// `merge_from`.
    pub physical_cache: Option<PhysicalCacheMetricsDelta>,
}

impl EvaluationMetrics {
    /// Accumulate another metrics record's counters and timings into `self`.
    ///
    /// Used to fold a per-task-local metrics collector (e.g. one installed for a
    /// parallel facet-cell build) back into the shared parent collector with a
    /// single lock, instead of locking the shared collector on every `record_*`.
    /// `mode` is intentionally not merged (it describes the run, not a delta).
    pub(crate) fn merge_from(&mut self, other: &EvaluationMetrics) {
        self.facet_layout.merge_from(&other.facet_layout);
        self.pipeline.merge_from(&other.pipeline);
        self.timings.merge_from(&other.timings);
    }

    pub(crate) fn record_preview_attempt_duration(&mut self, duration: Duration) {
        self.timings.preview_attempt_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_structure_reflow_duration(&mut self, duration: Duration) {
        self.timings.preview_structure_reflow_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_layout_setup_duration(&mut self, duration: Duration) {
        self.timings.preview_layout_setup_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_measurement_clone_duration(&mut self, duration: Duration) {
        self.timings.preview_measurement_clone_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_scale_refresh_duration(&mut self, duration: Duration) {
        self.timings.preview_scale_refresh_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_scale_context_setup_duration(&mut self, duration: Duration) {
        self.timings.preview_scale_context_setup_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_scale_cache_key_duration(&mut self, duration: Duration) {
        self.timings.preview_scale_cache_key_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_scale_cache_lookup_duration(&mut self, duration: Duration) {
        self.timings.preview_scale_cache_lookup_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_scale_build_duration(&mut self, duration: Duration) {
        self.timings.preview_scale_build_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_measurement_retarget_duration(&mut self, duration: Duration) {
        self.timings.preview_measurement_retarget_us += duration_micros_u64(duration);
    }

    pub(crate) fn record_preview_facet_domain_override_duration(&mut self, duration: Duration) {
        self.timings.preview_facet_domain_override_us += duration_micros_u64(duration);
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

    pub(crate) fn record_facet_tree_profile_reuse(&mut self) {
        self.pipeline.facet_tree_profile_reuses += 1;
    }

    pub(crate) fn record_facet_semantic_cache_hits(&mut self, count: usize) {
        self.pipeline.facet_semantic_cache_hits += count;
    }

    pub(crate) fn record_facet_semantic_cache_misses(&mut self, count: usize) {
        self.pipeline.facet_semantic_cache_misses += count;
    }

    pub(crate) fn record_facet_scale_builder_precompute_cache_hit(&mut self) {
        self.pipeline.facet_scale_builder_precompute_cache_hits += 1;
    }

    pub(crate) fn record_facet_scale_builder_precompute_cache_miss(&mut self) {
        self.pipeline.facet_scale_builder_precompute_cache_misses += 1;
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

    pub(crate) fn record_facet_cells_built(&mut self, count: usize) {
        self.pipeline.facet_cells_built += count;
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

    pub(crate) fn record_widget_item_collect(&mut self) {
        self.pipeline.widget_item_collects += 1;
    }

    pub(crate) fn record_widget_item_cache_hit(&mut self) {
        self.pipeline.widget_item_cache_hits += 1;
    }

    pub(crate) fn record_widget_item_cache_miss(&mut self) {
        self.pipeline.widget_item_cache_misses += 1;
    }

    pub(crate) fn record_materialization_request_emitted(&mut self) {
        self.pipeline.materialization_requests_emitted += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_cache_hit(&mut self) {
        self.pipeline.materialization_cache_hits += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_cache_miss(&mut self) {
        self.pipeline.materialization_cache_misses += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_queued(&mut self) {
        self.pipeline.materialization_queued += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_running(&mut self) {
        self.pipeline.materialization_running += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_ready_used(&mut self) {
        self.pipeline.materialization_ready_used += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_stale_fallback_used(&mut self) {
        self.pipeline.materialization_stale_fallback_used += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_view_scalar_sync(&mut self) {
        self.pipeline.view_scalar_syncs += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_view_scalar_async_use(&mut self) {
        self.pipeline.view_scalar_async_uses += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_error(&mut self) {
        self.pipeline.materialization_errors += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn record_materialization_completion(&mut self) {
        self.pipeline.materialization_completions += 1;
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
    /// Time spent evaluating layout/sizing inputs while attempting Preview reuse.
    pub preview_layout_setup_us: u64,
    /// Time spent cloning the cached measurement before retargeting it.
    pub preview_measurement_clone_us: u64,
    /// Time spent refreshing configured non-facet scales for a reused Preview measurement.
    pub preview_scale_refresh_us: u64,
    /// Time spent constructing the lightweight core scale-evaluation context.
    pub preview_scale_context_setup_us: u64,
    /// Time spent building the top-level scale-domain cache key during Preview.
    pub preview_scale_cache_key_us: u64,
    /// Time spent looking up the cached scale-domain builder during Preview.
    pub preview_scale_cache_lookup_us: u64,
    /// Time spent constructing configured scales from cached domain artifacts.
    pub preview_scale_build_us: u64,
    /// Time spent retargeting the reused measurement to the requested plot area.
    pub preview_measurement_retarget_us: u64,
    /// Time spent applying active raw-domain overrides to reused facet cells.
    pub preview_facet_domain_override_us: u64,
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

impl EvaluationTimingMetrics {
    pub(crate) fn merge_from(&mut self, other: &EvaluationTimingMetrics) {
        self.preview_attempt_us += other.preview_attempt_us;
        self.preview_structure_reflow_us += other.preview_structure_reflow_us;
        self.preview_layout_setup_us += other.preview_layout_setup_us;
        self.preview_measurement_clone_us += other.preview_measurement_clone_us;
        self.preview_scale_refresh_us += other.preview_scale_refresh_us;
        self.preview_scale_context_setup_us += other.preview_scale_context_setup_us;
        self.preview_scale_cache_key_us += other.preview_scale_cache_key_us;
        self.preview_scale_cache_lookup_us += other.preview_scale_cache_lookup_us;
        self.preview_scale_build_us += other.preview_scale_build_us;
        self.preview_measurement_retarget_us += other.preview_measurement_retarget_us;
        self.preview_facet_domain_override_us += other.preview_facet_domain_override_us;
        self.measure_cells_overflow_probe_us += other.measure_cells_overflow_probe_us;
        self.refresh_reused_profile_layout_us += other.refresh_reused_profile_layout_us;
        self.guide_overflow_measure_us += other.guide_overflow_measure_us;
        self.build_plot_components_us += other.build_plot_components_us;
        self.components_to_evaluated_plot_us += other.components_to_evaluated_plot_us;
    }
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
    /// Number of times Preview reused the facet tree stored in the current
    /// layout profile because only raw-domain pan params changed.
    pub facet_tree_profile_reuses: usize,
    /// Number of session semantic facet cache hits while building facet trees.
    pub facet_semantic_cache_hits: usize,
    /// Number of session semantic facet cache misses while building facet trees.
    pub facet_semantic_cache_misses: usize,
    /// Number of session facet scale-builder-precompute store hits.
    pub facet_scale_builder_precompute_cache_hits: usize,
    /// Number of session facet scale-builder-precompute store misses.
    pub facet_scale_builder_precompute_cache_misses: usize,
    /// Number of scale-builder construction requests observed by the chart
    /// runtime. This counts calls to the current domain-inference pipeline,
    /// not chart geometry refreshes.
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
    /// Number of preview evaluations that reused a previous layout profile.
    pub preview_profile_reuses: usize,
    /// Number of preview evaluations that could not reuse a previous layout profile.
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
    /// Number of facet cells dispatched through the per-cell build path (each is an
    /// independent task that runs in parallel on a multi-thread runtime). Summed
    /// across all facet bands in the evaluation.
    pub facet_cells_built: usize,
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
    /// Widget item-plan collect calls. One composed widget item relation is
    /// collected once per evaluation revision and shared by measurement,
    /// scale inference, and every part mark.
    pub widget_item_collects: usize,
    /// Prepared widget item relations reused from a `PlotSession` cache.
    pub widget_item_cache_hits: usize,
    /// Prepared widget item relations missing from a `PlotSession` cache.
    pub widget_item_cache_misses: usize,
    /// Async materialization requests emitted during evaluation.
    pub materialization_requests_emitted: usize,
    /// Materialization cache hits for desired keys.
    pub materialization_cache_hits: usize,
    /// Materialization cache misses for desired keys.
    pub materialization_cache_misses: usize,
    /// Materialization requests newly queued.
    pub materialization_queued: usize,
    /// Materialization requests already running.
    pub materialization_running: usize,
    /// Ready materialization results used for the desired key.
    pub materialization_ready_used: usize,
    /// Last-ready stale materialization results used while a desired key is pending.
    pub materialization_stale_fallback_used: usize,
    /// Number of view-scoped scalar aggregations executed synchronously
    /// inside an evaluation (exact evals and preview cold starts). Preview
    /// evaluations in RetargetCached views should keep this at zero — their
    /// scalars come from ready/stale materialized results.
    pub view_scalar_syncs: usize,
    /// Number of view-scoped scalar aggregations served from a ready or
    /// stale materialized result instead of executing synchronously.
    pub view_scalar_async_uses: usize,
    /// Materialization errors observed during evaluation.
    pub materialization_errors: usize,
    /// Async materialization completions observed by the chart runtime.
    pub materialization_completions: usize,
}

impl EvaluationPipelineMetrics {
    pub(crate) fn merge_from(&mut self, other: &EvaluationPipelineMetrics) {
        self.facet_tree_builds += other.facet_tree_builds;
        self.facet_tree_profile_reuses += other.facet_tree_profile_reuses;
        self.facet_semantic_cache_hits += other.facet_semantic_cache_hits;
        self.facet_semantic_cache_misses += other.facet_semantic_cache_misses;
        self.facet_scale_builder_precompute_cache_hits +=
            other.facet_scale_builder_precompute_cache_hits;
        self.facet_scale_builder_precompute_cache_misses +=
            other.facet_scale_builder_precompute_cache_misses;
        self.scale_builder_builds += other.scale_builder_builds;
        self.scale_domain_cache_hits += other.scale_domain_cache_hits;
        self.scale_domain_cache_misses += other.scale_domain_cache_misses;
        self.scale_domain_collects += other.scale_domain_collects;
        self.guide_overflow_cache_hits += other.guide_overflow_cache_hits;
        self.guide_overflow_cache_misses += other.guide_overflow_cache_misses;
        self.guide_overflow_measure_calls += other.guide_overflow_measure_calls;
        self.legend_plan_builds += other.legend_plan_builds;
        self.legend_measurements += other.legend_measurements;
        self.legend_measurement_cache_hits += other.legend_measurement_cache_hits;
        self.legend_measurement_cache_misses += other.legend_measurement_cache_misses;
        self.text_measurement_cache_hits += other.text_measurement_cache_hits;
        self.text_measurement_cache_misses += other.text_measurement_cache_misses;
        self.preview_profile_reuses += other.preview_profile_reuses;
        self.preview_profile_misses += other.preview_profile_misses;
        self.preview_fallbacks += other.preview_fallbacks;
        self.preview_profile_fallback_reasons
            .extend(other.preview_profile_fallback_reasons.iter().cloned());
        self.preview_structure_reflow_reuses += other.preview_structure_reflow_reuses;
        self.preview_structure_reflow_misses += other.preview_structure_reflow_misses;
        self.preview_data_mark_reuses += other.preview_data_mark_reuses;
        self.preview_data_mark_reuse_misses += other.preview_data_mark_reuse_misses;
        self.facet_cell_measurement_profile_reuses += other.facet_cell_measurement_profile_reuses;
        self.facet_cell_measurement_profile_misses += other.facet_cell_measurement_profile_misses;
        self.facet_cell_measurement_profile_chrome_refreshes +=
            other.facet_cell_measurement_profile_chrome_refreshes;
        self.facet_cells_built += other.facet_cells_built;
        self.skipped_component_measure_calls += other.skipped_component_measure_calls;
        self.mark_data_collects += other.mark_data_collects;
        self.mark_data_full_collects += other.mark_data_full_collects;
        self.mark_data_array_collects += other.mark_data_array_collects;
        self.mark_data_scalar_collects += other.mark_data_scalar_collects;
        self.widget_item_collects += other.widget_item_collects;
        self.widget_item_cache_hits += other.widget_item_cache_hits;
        self.widget_item_cache_misses += other.widget_item_cache_misses;
        self.materialization_requests_emitted += other.materialization_requests_emitted;
        self.materialization_cache_hits += other.materialization_cache_hits;
        self.materialization_cache_misses += other.materialization_cache_misses;
        self.materialization_queued += other.materialization_queued;
        self.materialization_running += other.materialization_running;
        self.materialization_ready_used += other.materialization_ready_used;
        self.materialization_stale_fallback_used += other.materialization_stale_fallback_used;
        self.materialization_errors += other.materialization_errors;
        self.materialization_completions += other.materialization_completions;
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
    pub(crate) fn merge_from(&mut self, other: &FacetLayoutMetrics) {
        self.plot_component_measure_calls += other.plot_component_measure_calls;
        if self.plot_component_measure_calls_by_facet_depth.len()
            < other.plot_component_measure_calls_by_facet_depth.len()
        {
            self.plot_component_measure_calls_by_facet_depth
                .resize(other.plot_component_measure_calls_by_facet_depth.len(), 0);
        }
        for (slot, value) in self
            .plot_component_measure_calls_by_facet_depth
            .iter_mut()
            .zip(other.plot_component_measure_calls_by_facet_depth.iter())
        {
            *slot += value;
        }
        self.facet_band_measure_runs += other.facet_band_measure_runs;
        self.estimated_overflow_leaf_measure_count += other.estimated_overflow_leaf_measure_count;
        self.estimated_overflow_non_leaf_aggregate_count +=
            other.estimated_overflow_non_leaf_aggregate_count;
        self.estimated_overflow_non_leaf_full_measure_count +=
            other.estimated_overflow_non_leaf_full_measure_count;
        self.refinement_pass_count += other.refinement_pass_count;
        self.refinement_converged |= other.refinement_converged;
        self.refinement_hit_max_passes |= other.refinement_hit_max_passes;
    }
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
    /// Declared chrome the frame was solved from. Retained so coordination
    /// retargets re-solve and re-project instead of mutating rects.
    pub(crate) chrome: crate::layout::FrameChrome,
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

/// Identifies one evaluated interaction scope within an `EvaluatedPlot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InteractionScopeId(pub usize);

/// What kind of interaction surface a scope represents.
///
/// V1 only produces `Coordinate` scopes for Cartesian plot areas. `Container`
/// is reserved for future facet-title/slot tools that will reuse this layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionScopeKind {
    Coordinate,
    LegendColorbar,
    Container,
}

/// Kind of child-frame container segment that owns an interaction scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluatedChildFrameKind {
    HConcat,
    VConcat,
    GridConcat,
    WrapConcat,
}

/// Public, evaluated child-frame metadata for an interaction scope.
///
/// This is intentionally semantic metadata rather than an internal scene-graph
/// mark path. It lets tools and tests reason about concat/repeat placement
/// without relying on private child-frame scope keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedChildFrameSegment {
    pub kind: EvaluatedChildFrameKind,
    pub child_index: usize,
    pub key: Option<String>,
    pub label: Option<String>,
    pub row: Option<usize>,
    pub column: Option<usize>,
    pub row_count: Option<usize>,
    pub column_count: Option<usize>,
    pub row_span: Option<usize>,
    pub column_span: Option<usize>,
    pub slot_index: Option<usize>,
}

/// A measured interaction scope exported from final plot/facet layout.
///
/// Bounds are in scene/canvas coordinates. Scales and the coordinate transform
/// let the runtime invert pointer positions back into data/domain space.
#[derive(Clone)]
pub struct EvaluatedInteractionScope {
    pub id: InteractionScopeId,
    pub kind: InteractionScopeKind,
    /// Stable content-derived id for this interaction scope.
    pub scope_id: String,
    /// Scope bounds in scene/canvas coordinates.
    pub bounds: LayoutBounds,
    pub plot_area_width: f32,
    pub plot_area_height: f32,
    /// Full physical facet path for this scope (empty at the root).
    pub facet_path: Vec<ScalarValue>,
    /// Logical facet values for this scope, excluding structural wrap rows.
    pub logical_facet_values: Vec<ScalarValue>,
    /// Facet coord-node path for this scope (empty at the root).
    pub coord_node_path: Vec<usize>,
    /// Authored subplot ids for ancestor subplot marks, outermost first.
    pub subplot_id_path: Vec<String>,
    /// Semantic child-frame path for ancestor concat-like containers.
    pub child_frame_path: Vec<EvaluatedChildFrameSegment>,
    /// Coordinate transform used to invert local points for this scope.
    pub coord_transform: Box<dyn CoordinateSystemTransform>,
    /// Coordinate channels this scope can invert.
    pub channels: Vec<String>,
    /// Configured scales keyed by coordinate channel.
    pub scales: HashMap<String, ConfiguredScale>,
    /// Logical sharing-owner paths keyed by sharing level for this scope.
    pub sharing_owner_paths: HashMap<u8, Vec<ScalarValue>>,
}

impl EvaluatedInteractionScope {
    pub(crate) fn prepend_coord_node_path(&mut self, child_index: usize) {
        self.coord_node_path.insert(0, child_index);
        self.scope_id =
            interaction_scope_content_id(&self.coord_node_path, &self.logical_facet_values);
    }

    pub(crate) fn prepend_subplot_id(&mut self, id: Option<&str>) {
        let Some(id) = id else {
            return;
        };
        self.subplot_id_path.insert(0, id.to_string());
    }

    pub(crate) fn prepend_child_frame_segment(&mut self, segment: EvaluatedChildFrameSegment) {
        self.child_frame_path.insert(0, segment);
    }
}

fn interaction_scope_content_id(
    coord_node_path: &[usize],
    logical_facet_values: &[ScalarValue],
) -> String {
    let coord_path = coord_node_path
        .iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(".");
    let facet_path = logical_facet_values
        .iter()
        .map(scalar_value_for_scope_id)
        .collect::<Vec<_>>()
        .join("/");
    format!("coord:{coord_path};facet:{facet_path}")
}

fn scalar_value_for_scope_id(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => value.clone(),
        _ => value.to_string(),
    }
}

impl std::fmt::Debug for EvaluatedInteractionScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvaluatedInteractionScope")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("scope_id", &self.scope_id)
            .field("bounds", &self.bounds)
            .field("plot_area_width", &self.plot_area_width)
            .field("plot_area_height", &self.plot_area_height)
            .field("facet_path", &self.facet_path)
            .field("logical_facet_values", &self.logical_facet_values)
            .field("coord_node_path", &self.coord_node_path)
            .field("subplot_id_path", &self.subplot_id_path)
            .field("child_frame_path", &self.child_frame_path)
            .field("channels", &self.channels)
            .field("sharing_owner_paths", &self.sharing_owner_paths)
            .finish_non_exhaustive()
    }
}

/// All interaction scopes produced by one evaluation.
#[derive(Clone, Debug, Default)]
pub struct EvaluatedInteractionState {
    pub scopes: Vec<EvaluatedInteractionScope>,
}

/// Logical datum rows retained for one rendered scene mark.
#[derive(Clone, Debug)]
pub struct EvaluatedEventDatumRows {
    /// Final scene-graph mark path for the rendered mark.
    pub mark_path: Vec<usize>,
    /// Authored subplot ids for ancestor subplot marks, outermost first.
    pub subplot_id_path: Vec<String>,
    /// Logical rows in the same order as rendered mark instances.
    pub rows: RecordBatch,
}

impl EvaluatedEventDatumRows {
    pub(crate) fn prepend_subplot_id(&mut self, id: Option<&str>) {
        let Some(id) = id else {
            return;
        };
        self.subplot_id_path.insert(0, id.to_string());
    }
}

/// Datum lookup table produced by the most recent chart evaluation.
#[derive(Clone, Debug, Default)]
pub struct EvaluatedEventDatumState {
    pub rows: Vec<EvaluatedEventDatumRows>,
}

impl EvaluatedEventDatumState {
    pub fn datum_for_mark_instance(
        &self,
        mark_instance: Option<&MarkInstance>,
        field: &str,
    ) -> Option<ScalarValue> {
        let mark_instance = mark_instance?;
        let row_index = mark_instance.instance_index?;
        let rows = self
            .rows
            .iter()
            .find(|rows| rows.mark_path == mark_instance.mark_path)?;
        let column = rows.rows.column_by_name(field)?;
        if row_index >= column.len() {
            return None;
        }
        ScalarValue::try_from_array(column, row_index).ok()
    }

    pub fn datums_for_mark_instances(
        &self,
        instances: impl IntoIterator<Item = MarkInstance>,
        fields: &[SceneQueryDatumField],
        unique_by: &[String],
    ) -> Result<RecordBatch, AvengerChartError> {
        let schema_fields = fields
            .iter()
            .map(|field| {
                let data_type = self
                    .rows
                    .iter()
                    .find_map(|rows| {
                        rows.rows
                            .schema()
                            .field_with_name(&field.datum_field)
                            .ok()
                            .map(|field| field.data_type().clone())
                    })
                    .ok_or_else(|| {
                        AvengerChartError::InvalidArgument(format!(
                            "Scene geometry query requested datum field '{}' but no retained event datum rows expose it",
                            field.datum_field
                        ))
                    })?;
                Ok(Field::new(field.id.clone(), data_type, true))
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let schema = Arc::new(Schema::new(schema_fields));

        let unique_fields = if unique_by.is_empty() {
            fields
                .iter()
                .map(|field| field.id.clone())
                .collect::<Vec<_>>()
        } else {
            unique_by.to_vec()
        };
        let unique_indices = unique_fields
            .iter()
            .map(|field| {
                fields
                    .iter()
                    .position(|datum_field| &datum_field.id == field)
                    .ok_or_else(|| {
                        AvengerChartError::InvalidArgument(format!(
                            "Scene geometry query unique field '{}' is not one of the requested datum fields",
                            field
                        ))
                    })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        let mut rows_out: Vec<Vec<ScalarValue>> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for instance in instances {
            let Some(row_index) = instance.instance_index else {
                continue;
            };
            let Some(rows) = self
                .rows
                .iter()
                .find(|rows| rows.mark_path == instance.mark_path)
            else {
                continue;
            };
            let mut row = Vec::with_capacity(fields.len());
            let mut missing = false;
            for field in fields {
                let Some(column) = rows.rows.column_by_name(&field.datum_field) else {
                    missing = true;
                    break;
                };
                if row_index >= column.len() {
                    missing = true;
                    break;
                }
                let value = ScalarValue::try_from_array(column, row_index).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "Failed to read scene query datum field '{}': {}",
                        field.datum_field, err
                    ))
                })?;
                row.push(value);
            }
            if missing {
                continue;
            }
            let unique_key = unique_indices
                .iter()
                .map(|index| scalar_unique_key(&row[*index]))
                .collect::<Option<Vec<_>>>();
            let Some(unique_key) = unique_key else {
                continue;
            };
            if seen.insert(unique_key.join("\u{1f}")) {
                rows_out.push(row);
            }
        }

        let columns = fields
            .iter()
            .enumerate()
            .map(|(field_index, _)| {
                let values = rows_out
                    .iter()
                    .map(|row| row[field_index].clone())
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    Ok(new_null_array(schema.field(field_index).data_type(), 0))
                } else {
                    ScalarValue::iter_to_array(values.into_iter()).map_err(|err| {
                        AvengerChartError::InvalidArgument(format!(
                            "Failed to build scene query datum batch: {}",
                            err
                        ))
                    })
                }
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        RecordBatch::try_new(schema, columns).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Failed to build scene query datum batch: {}",
                err
            ))
        })
    }

    pub fn subplot_id_path_for_mark_path(&self, mark_path: &[usize]) -> Option<&[String]> {
        self.rows
            .iter()
            .find(|rows| rows.mark_path == mark_path)
            .map(|rows| rows.subplot_id_path.as_slice())
    }
}

fn scalar_unique_key(value: &ScalarValue) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(format!("{value:?}"))
    }
}

/// Result of evaluating a plot to scene graph components
pub struct EvaluatedPlot {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// External resources requested while evaluating scene output.
    pub resource_requests: Vec<ResourceRequest>,
    /// Async materializations requested while evaluating scene output.
    pub materialization_requests: Vec<MaterializationRequest>,
    /// Spatial index for efficient hit testing
    pub rtree: Option<SceneGraphRTree>,
    /// Interaction scopes for event routing and coordinate inversion.
    pub interaction: EvaluatedInteractionState,
    /// Logical datum rows addressable by rendered mark instance.
    pub event_datums: EvaluatedEventDatumState,
    /// Prefetch-retarget planners published by coordinate guides for this
    /// evaluation (hover-driven prefetch retargeting; one per scope).
    pub prefetch_planners: Vec<Arc<dyn avenger_resource::PrefetchRetargetPlanner>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::{
        array::{Int32Array, StringArray},
        datatypes::DataType,
    };

    fn datum_state() -> EvaluatedEventDatumState {
        let schema = Arc::new(Schema::new(vec![
            Field::new("source_id", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]));
        let rows = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "b", "c"])),
                Arc::new(StringArray::from(vec!["A", "B", "B", "C"])),
                Arc::new(Int32Array::from(vec![1, 2, 3, 4])),
            ],
        )
        .expect("build batch");

        EvaluatedEventDatumState {
            rows: vec![EvaluatedEventDatumRows {
                mark_path: vec![2, 0],
                subplot_id_path: Vec::new(),
                rows,
            }],
        }
    }

    fn mark(index: usize) -> MarkInstance {
        MarkInstance {
            name: "points".to_string(),
            mark_path: vec![2, 0],
            instance_index: Some(index),
        }
    }

    #[test]
    fn datums_for_mark_instances_dedupes_by_requested_tuple() {
        let batch = datum_state()
            .datums_for_mark_instances(
                [mark(0), mark(1), mark(2), mark(3)],
                &[
                    SceneQueryDatumField::new("id").datum("source_id"),
                    SceneQueryDatumField::new("category"),
                ],
                &["id".to_string()],
            )
            .expect("query datum batch");

        assert_eq!(batch.num_rows(), 3);
        let ids = (0..batch.num_rows())
            .map(|row| {
                ScalarValue::try_from_array(batch.column_by_name("id").unwrap(), row)
                    .expect("id value")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                ScalarValue::Utf8(Some("a".to_string())),
                ScalarValue::Utf8(Some("b".to_string())),
                ScalarValue::Utf8(Some("c".to_string())),
            ]
        );
    }

    #[test]
    fn datums_for_mark_instances_reports_unknown_unique_field() {
        let err = datum_state()
            .datums_for_mark_instances(
                [mark(0)],
                &[SceneQueryDatumField::new("id").datum("source_id")],
                &["missing".to_string()],
            )
            .expect_err("unknown unique field should error");
        assert!(err.to_string().contains("unique field 'missing'"));
    }

    #[test]
    fn datums_for_mark_instances_skips_missing_instance_indexes() {
        let batch = datum_state()
            .datums_for_mark_instances(
                [
                    MarkInstance {
                        name: "points".to_string(),
                        mark_path: vec![2, 0],
                        instance_index: None,
                    },
                    mark(1),
                ],
                &[SceneQueryDatumField::new("id").datum("source_id")],
                &[],
            )
            .expect("query datum batch");

        assert_eq!(batch.num_rows(), 1);
        let value =
            ScalarValue::try_from_array(batch.column_by_name("id").unwrap(), 0).expect("id value");
        assert_eq!(value, ScalarValue::Utf8(Some("b".to_string())));
    }

    #[test]
    fn datums_for_mark_instances_skips_marks_missing_requested_fields() {
        let eligible_schema = Arc::new(Schema::new(vec![
            Field::new("row_id", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]));
        let eligible_rows = RecordBatch::try_new(
            eligible_schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Int32Array::from(vec![1, 2])),
            ],
        )
        .expect("eligible batch");
        let annotation_schema = Arc::new(Schema::new(vec![Field::new(
            "label",
            DataType::Utf8,
            false,
        )]));
        let annotation_rows = RecordBatch::try_new(
            annotation_schema,
            vec![Arc::new(StringArray::from(vec!["note"]))],
        )
        .expect("annotation batch");
        let state = EvaluatedEventDatumState {
            rows: vec![
                EvaluatedEventDatumRows {
                    mark_path: vec![0],
                    subplot_id_path: Vec::new(),
                    rows: eligible_rows,
                },
                EvaluatedEventDatumRows {
                    mark_path: vec![1],
                    subplot_id_path: Vec::new(),
                    rows: annotation_rows,
                },
            ],
        };

        let batch = state
            .datums_for_mark_instances(
                [
                    MarkInstance {
                        name: "points".to_string(),
                        mark_path: vec![0],
                        instance_index: Some(1),
                    },
                    MarkInstance {
                        name: "annotation".to_string(),
                        mark_path: vec![1],
                        instance_index: Some(0),
                    },
                ],
                &[SceneQueryDatumField::new("row_id")],
                &[],
            )
            .expect("query datum batch");

        assert_eq!(batch.num_rows(), 1);
        let value = ScalarValue::try_from_array(batch.column_by_name("row_id").unwrap(), 0)
            .expect("row_id value");
        assert_eq!(value, ScalarValue::Utf8(Some("b".to_string())));
    }
}
