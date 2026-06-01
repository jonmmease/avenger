//! Reusable evaluation session for a compiled plot.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use avenger_chart_core::{
    ChannelInfo, CompiledParamSpec, DefaultLogicalExprNodeExt, FacetWrapColumnMode, LegendChannel,
    LegendPosition, LogicalPlanNodeExt, Maybe, RadiusExpression, ScaleConfigSpec,
    ScaleDefaultDomain, ScaleDomain, SerializableExpr, Sharing,
};
use avenger_chart_scales::{PlotScaleSpec, ScaleBuilder};
use avenger_scales::scales::ConfiguredScale;
use avenger_text::{
    measurement::TextBounds,
    types::{FontStyle, FontWeight},
};
use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use indexmap::IndexMap;

use crate::{
    concat,
    error::AvengerChartError,
    facet::{
        evaluated_facet_tree::EvaluatedFacetTree,
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        scale_precompute::FacetScalePrecomputeStore,
    },
    guide::OverflowSpaceRequirement,
    layout::{LayoutSpec, Size2D, SizeMode},
    partition::PartitionSlotCache,
    plot::compiled::ChildFrameSharingPath,
    render::{
        EvaluatedPlot, EvaluationMetrics, EvaluationMode, EvaluationOptions,
        PreviewProfileFallbackReason, types::LegendMeasurement,
    },
    scales::ConfiguredScaleWithSpec,
};

use super::{
    CompiledPlot, LayoutProfileSnapshot, compiled_subplot_payload_child_plot,
    legends::PreparedLegendGroup,
};

pub(crate) type ScaleDomainCacheHandle = Arc<Mutex<ScaleDomainCache>>;
pub(crate) type FacetSemanticCacheHandle = Arc<Mutex<PartitionSlotCache>>;
pub(crate) type FacetScalePrecomputeCacheHandle = Arc<Mutex<FacetScalePrecomputeSessionCache>>;
pub(crate) type GuideOverflowCacheHandle = Arc<Mutex<GuideOverflowCache>>;
pub(crate) type LegendMeasurementCacheHandle = Arc<Mutex<LegendMeasurementCache>>;
pub(crate) type TextMeasurementCacheHandle = Arc<Mutex<TextMeasurementCache>>;

pub(crate) fn new_plot_session_cache_handles() -> (
    ScaleDomainCacheHandle,
    FacetSemanticCacheHandle,
    FacetScalePrecomputeCacheHandle,
    GuideOverflowCacheHandle,
    LegendMeasurementCacheHandle,
    TextMeasurementCacheHandle,
) {
    (
        Arc::new(Mutex::new(ScaleDomainCache::default())),
        Arc::new(Mutex::new(PartitionSlotCache::new())),
        Arc::new(Mutex::new(FacetScalePrecomputeSessionCache::default())),
        Arc::new(Mutex::new(GuideOverflowCache::default())),
        Arc::new(Mutex::new(LegendMeasurementCache::default())),
        Arc::new(Mutex::new(TextMeasurementCache::default())),
    )
}

/// Session-owned cache for scale-domain inference artifacts.
#[derive(Default)]
pub(crate) struct ScaleDomainCache {
    builders: HashMap<ScaleDomainCacheKey, Arc<ScaleBuilder>>,
}

impl ScaleDomainCache {
    pub(crate) fn get(&self, key: &ScaleDomainCacheKey) -> Option<Arc<ScaleBuilder>> {
        self.builders.get(key).cloned()
    }

    pub(crate) fn insert(
        &mut self,
        key: ScaleDomainCacheKey,
        builder: ScaleBuilder,
    ) -> Arc<ScaleBuilder> {
        let builder = Arc::new(builder);
        self.builders.insert(key, builder.clone());
        builder
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct ScaleDomainCacheKey {
    subject: ScaleDomainCacheSubject,
    scope: ScaleDomainCacheScope,
    params: Vec<(String, String)>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ScaleDomainCacheSubject {
    marks_ptr: usize,
    scale_specs_ptr: usize,
    data_ptr: usize,
    data_override_plan: Option<String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum ScaleDomainCacheScope {
    TopLevel,
    FacetPath(Vec<String>),
    ChildFrame {
        container_path: Vec<String>,
        data_selection: String,
    },
}

/// Session-owned cache of facet scale-precompute stores.
#[derive(Default)]
pub(crate) struct FacetScalePrecomputeSessionCache {
    stores: HashMap<FacetScalePrecomputeCacheKey, Arc<FacetScalePrecomputeStore>>,
}

impl FacetScalePrecomputeSessionCache {
    fn store_for_key(
        &mut self,
        key: FacetScalePrecomputeCacheKey,
    ) -> (Arc<FacetScalePrecomputeStore>, bool) {
        if let Some(store) = self.stores.get(&key) {
            return (store.clone(), true);
        }
        let store = Arc::new(FacetScalePrecomputeStore::default());
        self.stores.insert(key, store.clone());
        (store, false)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct FacetScalePrecomputeCacheKey {
    program_ptr: usize,
    facet_tree_structure: Vec<String>,
    params: Vec<(String, String)>,
}

/// Session-owned cache for exact guide-overflow measurement profiles.
#[derive(Default)]
pub(crate) struct GuideOverflowCache {
    overflows: HashMap<GuideOverflowCacheKey, OverflowSpaceRequirement>,
}

impl GuideOverflowCache {
    pub(crate) fn get(&self, key: &GuideOverflowCacheKey) -> Option<OverflowSpaceRequirement> {
        self.overflows.get(key).cloned()
    }

    pub(crate) fn insert(
        &mut self,
        key: GuideOverflowCacheKey,
        overflow: OverflowSpaceRequirement,
    ) {
        self.overflows.insert(key, overflow);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct GuideOverflowCacheKey {
    program_ptr: usize,
    guide_ptr: usize,
    estimate_width: u32,
    estimate_height: u32,
    scales: Vec<(String, String)>,
    params: Vec<(String, String)>,
    facet_path: Vec<String>,
    facet_tree_structure: Vec<String>,
    child_frame_sharing_path: String,
    data_override_plan: Option<String>,
}

/// Session-owned cache for exact legend measurement profiles.
#[derive(Default)]
pub(crate) struct LegendMeasurementCache {
    measurements: HashMap<LegendMeasurementCacheKey, LegendMeasurement>,
}

impl LegendMeasurementCache {
    pub(crate) fn get(&self, key: &LegendMeasurementCacheKey) -> Option<LegendMeasurement> {
        self.measurements.get(key).cloned()
    }

    pub(crate) fn insert(
        &mut self,
        key: LegendMeasurementCacheKey,
        measurement: LegendMeasurement,
    ) {
        self.measurements.insert(key, measurement);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct LegendMeasurementCacheKey {
    program_ptr: usize,
    layout_key: String,
    renderer_name: String,
    legend: String,
    channels: Vec<String>,
    available_width: u32,
    available_height: u32,
    position: String,
    params: Vec<(String, String)>,
}

/// Session-owned cache for exact text layout measurements.
#[derive(Default)]
pub(crate) struct TextMeasurementCache {
    measurements: HashMap<TextMeasurementCacheKey, TextBounds>,
}

impl TextMeasurementCache {
    pub(crate) fn get(&self, key: &TextMeasurementCacheKey) -> Option<TextBounds> {
        self.measurements.get(key).cloned()
    }

    pub(crate) fn insert(&mut self, key: TextMeasurementCacheKey, measurement: TextBounds) {
        self.measurements.insert(key, measurement);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct TextMeasurementCacheKey {
    text: String,
    font: String,
    font_size: u32,
    font_weight: String,
    font_style: String,
}

impl TextMeasurementCacheKey {
    pub(crate) fn new(
        text: &str,
        font: &str,
        font_size: f32,
        font_weight: &FontWeight,
        font_style: &FontStyle,
    ) -> Self {
        Self {
            text: text.to_string(),
            font: font.to_string(),
            font_size: font_size.to_bits(),
            font_weight: format!("{font_weight:?}"),
            font_style: format!("{font_style:?}"),
        }
    }
}

/// Identifies one scoped copy of a parameter at a specific facet owner path.
///
/// The root/global copy uses an empty `owner_path`. Facet-scoped copies use the
/// logical owner path resolved for the parameter's `Sharing` level.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ScopedParamKey {
    name: String,
    owner_path: Vec<ScalarValue>,
}

/// A single scoped parameter write produced by an event binding assignment.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopedParamAssignment {
    pub name: String,
    pub owner_path: Vec<ScalarValue>,
    pub value: ScalarValue,
    pub replace_scoped_values: bool,
}

/// Immutable snapshot of the scoped parameter store.
///
/// Captures both the root effective values and any facet-scoped overrides so a
/// gesture can resolve frozen start/previous params at any sharing scope.
#[derive(Clone, Debug, Default)]
pub struct ScopedParamStoreSnapshot {
    root: IndexMap<String, ScalarValue>,
    scoped: IndexMap<ScopedParamKey, ScalarValue>,
}

impl ScopedParamStoreSnapshot {
    /// Build a snapshot that only carries root (global/shared) param values.
    ///
    /// Useful for tests and for callers that operate purely at the root scope.
    pub fn from_root_params(root: IndexMap<String, ScalarValue>) -> Self {
        Self {
            root,
            scoped: IndexMap::new(),
        }
    }
}

/// Resolve the logical owner path for a sharing scope.
///
/// `Shared` always resolves to the root path `[]`. `Free`/`Level(0)` and
/// `Level(N)` look up the routed scope's owner path for that level, falling back
/// to root when the scope does not provide one (e.g. unfaceted plots).
fn owner_path_for_sharing(
    sharing: Sharing,
    sharing_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
) -> Vec<ScalarValue> {
    let level = sharing.to_level();
    if level == u8::MAX {
        return Vec::new();
    }
    sharing_owner_paths.get(&level).cloned().unwrap_or_default()
}

/// Session-owned store of sharing-scoped parameter values.
///
/// Root (global/shared) values live at the empty owner path. Facet-scoped values
/// live at their resolved logical owner path. Reads fall back to the registered
/// `CompiledParamSpec` default when no scoped value exists.
#[derive(Clone, Debug)]
pub(crate) struct ScopedParamStore {
    specs: IndexMap<String, CompiledParamSpec>,
    values: IndexMap<ScopedParamKey, ScalarValue>,
    revisions: IndexMap<String, u64>,
}

impl ScopedParamStore {
    fn new(specs: IndexMap<String, CompiledParamSpec>) -> Self {
        Self {
            specs,
            values: IndexMap::new(),
            revisions: IndexMap::new(),
        }
    }

    fn bump_revision(&mut self, name: &str) {
        *self.revisions.entry(name.to_string()).or_insert(0) += 1;
    }

    /// Build a flat effective param map for a scope's sharing-owner paths.
    fn effective_params_for_owner_paths(
        &self,
        sharing_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
    ) -> IndexMap<String, ScalarValue> {
        let mut result = IndexMap::with_capacity(self.specs.len());
        for (name, spec) in &self.specs {
            let owner_path = owner_path_for_sharing(spec.sharing, sharing_owner_paths);
            let key = ScopedParamKey {
                name: name.clone(),
                owner_path,
            };
            let value = self
                .values
                .get(&key)
                .cloned()
                .unwrap_or_else(|| spec.default.clone());
            result.insert(name.clone(), value);
        }
        // Preserve any root values for names that are not registered specs so the
        // historical flat `set_params` behavior (which kept extra keys) holds.
        for (key, value) in &self.values {
            if key.owner_path.is_empty() && !self.specs.contains_key(&key.name) {
                result.insert(key.name.clone(), value.clone());
            }
        }
        result
    }

    /// Build the root (owner path `[]`) effective param map.
    fn root_effective_params(&self) -> IndexMap<String, ScalarValue> {
        self.effective_params_for_owner_paths(&HashMap::new())
    }

    /// Build the effective param map for a faceted cell identified by `full_path`.
    ///
    /// Resolves each registered param's value at the owner path implied by its
    /// sharing level and the cell's position in `tree`: `Free` resolves to the
    /// full path (per-cell), `Shared` to root `[]`, and `Level(n)` to `n` logical
    /// levels up (FacetWrap weight-0 levels collapsed by the tree).
    pub(crate) fn effective_params_for_cell(
        &self,
        tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
    ) -> IndexMap<String, ScalarValue> {
        let mut owner_paths: HashMap<u8, Vec<ScalarValue>> = HashMap::new();
        for spec in self.specs.values() {
            let level = spec.sharing.to_level();
            owner_paths
                .entry(level)
                .or_insert_with(|| tree.sharing_owner_path(full_path, level));
        }
        self.effective_params_for_owner_paths(&owner_paths)
    }

    /// True when any scoped (non-root) assignment exists.
    ///
    /// Used as a global fast-path gate: when false, per-cell resolution is a
    /// no-op and the non-interactive measurement path is unchanged.
    pub(crate) fn has_scoped_overrides(&self) -> bool {
        self.values.keys().any(|key| !key.owner_path.is_empty())
    }

    /// Replace root values with `params`, clearing prior root values first.
    fn set_root_params(&mut self, params: IndexMap<String, ScalarValue>) {
        self.values.retain(|key, _| !key.owner_path.is_empty());
        for (name, value) in params {
            self.values.insert(
                ScopedParamKey {
                    name: name.clone(),
                    owner_path: Vec::new(),
                },
                value,
            );
            self.bump_revision(&name);
        }
    }

    /// Patch root values from `patch`, leaving unrelated root values intact.
    fn apply_root_patch(&mut self, patch: IndexMap<String, ScalarValue>) {
        for (name, value) in patch {
            let key = ScopedParamKey {
                name: name.clone(),
                owner_path: Vec::new(),
            };
            let changed = self.values.get(&key) != Some(&value);
            self.values.insert(key, value);
            if changed {
                self.bump_revision(&name);
            }
        }
    }

    /// Apply scoped assignments. Empty owner paths target root values.
    fn apply_scoped_patch(&mut self, patch: impl IntoIterator<Item = ScopedParamAssignment>) {
        for assignment in patch {
            if assignment.replace_scoped_values {
                let before = self.values.len();
                self.values.retain(|key, _| key.name != assignment.name);
                if self.values.len() != before {
                    self.bump_revision(&assignment.name);
                }
            }
            let key = ScopedParamKey {
                name: assignment.name.clone(),
                owner_path: assignment.owner_path,
            };
            let changed = self.values.get(&key) != Some(&assignment.value);
            self.values.insert(key, assignment.value);
            if changed {
                self.bump_revision(&assignment.name);
            }
        }
    }

    fn snapshot(&self) -> ScopedParamStoreSnapshot {
        ScopedParamStoreSnapshot {
            root: self.root_effective_params(),
            scoped: self.values.clone(),
        }
    }

    /// Effective params for a scope using a frozen snapshot rather than live values.
    fn effective_params_from_snapshot(
        &self,
        snapshot: &ScopedParamStoreSnapshot,
        sharing_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
    ) -> IndexMap<String, ScalarValue> {
        let mut result = IndexMap::with_capacity(self.specs.len());
        for (name, spec) in &self.specs {
            let owner_path = owner_path_for_sharing(spec.sharing, sharing_owner_paths);
            let value = if owner_path.is_empty() {
                snapshot
                    .root
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| spec.default.clone())
            } else {
                let key = ScopedParamKey {
                    name: name.clone(),
                    owner_path,
                };
                snapshot
                    .scoped
                    .get(&key)
                    .cloned()
                    .or_else(|| snapshot.root.get(name).cloned())
                    .unwrap_or_else(|| spec.default.clone())
            };
            result.insert(name.clone(), value);
        }
        result
    }

    /// Deterministic fingerprint of all materialized scoped values for `names`.
    ///
    /// Used by facet scale-precompute cache keys that span multiple facet owners.
    /// Includes a default marker so a missing scoped value is distinguished from a
    /// written one.
    // Consumed by facet-scoped cache keys when faceted scoped-param evaluation
    // lands; retained as part of the scoped-param store contract.
    #[allow(dead_code)]
    fn scoped_fingerprint_for_names(
        &self,
        names: &BTreeSet<String>,
    ) -> Vec<(String, Vec<String>, String)> {
        let mut fingerprint: Vec<(String, Vec<String>, String)> = Vec::new();
        for (key, value) in &self.values {
            if names.contains(&key.name) {
                let owner_path = key
                    .owner_path
                    .iter()
                    .map(|v| format!("{v:?}"))
                    .collect::<Vec<_>>();
                fingerprint.push((key.name.clone(), owner_path, format!("{value:?}")));
            }
        }
        for name in names {
            if let Some(spec) = self.specs.get(name) {
                fingerprint.push((
                    name.clone(),
                    vec!["<default>".to_string()],
                    format!("{:?}", spec.default),
                ));
            }
        }
        fingerprint.sort();
        fingerprint
    }
}

/// Request object for evaluating a `PlotSession`.
#[derive(Clone, Debug)]
pub struct EvaluationRequest {
    params: Option<IndexMap<String, ScalarValue>>,
    param_patch: Option<IndexMap<String, ScalarValue>>,
    mode: EvaluationMode,
    options: EvaluationOptions,
}

impl Default for EvaluationRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl EvaluationRequest {
    pub fn new() -> Self {
        Self {
            params: None,
            param_patch: None,
            mode: EvaluationMode::Exact,
            options: EvaluationOptions::default(),
        }
    }

    pub fn params(mut self, params: IndexMap<String, ScalarValue>) -> Self {
        self.params = Some(params);
        self
    }

    pub fn param_patch(mut self, param_patch: IndexMap<String, ScalarValue>) -> Self {
        self.param_patch = Some(param_patch);
        self
    }

    pub fn mode(mut self, mode: EvaluationMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn options(mut self, options: EvaluationOptions) -> Self {
        self.options = options;
        self
    }

    pub fn exact(self) -> Self {
        self.mode(EvaluationMode::Exact)
    }

    pub fn preview(self) -> Self {
        self.mode(EvaluationMode::Preview)
    }

    pub fn force_remeasure(self) -> Self {
        self.mode(EvaluationMode::ForceRemeasure)
    }
}

fn options_for_evaluation_mode(
    mode: EvaluationMode,
    mut options: EvaluationOptions,
) -> EvaluationOptions {
    if mode == EvaluationMode::Preview {
        options.facet_layout_refinement.max_refinement_passes = 0;
    }
    options
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EvaluationRequestSummary {
    mode: EvaluationMode,
    params: IndexMap<String, ScalarValue>,
}

/// Stateful runtime instance for evaluating one compiled plot repeatedly.
pub struct PlotSession {
    program: Arc<CompiledPlot>,
    ctx: Arc<SessionContext>,
    scoped_params: ScopedParamStore,
    /// Cached root (owner path `[]`) effective param map for `params()`.
    root_cache: IndexMap<String, ScalarValue>,
    last_request: Option<EvaluationRequestSummary>,
    layout_profile: Option<LayoutProfileSnapshot>,
    last_metrics: Option<EvaluationMetrics>,
    scale_domain_cache: ScaleDomainCacheHandle,
    facet_semantic_cache: FacetSemanticCacheHandle,
    facet_scale_precompute_cache: FacetScalePrecomputeCacheHandle,
    guide_overflow_cache: GuideOverflowCacheHandle,
    legend_measurement_cache: LegendMeasurementCacheHandle,
    text_measurement_cache: TextMeasurementCacheHandle,
}

impl PlotSession {
    pub(crate) fn new(program: Arc<CompiledPlot>, ctx: Arc<SessionContext>) -> Self {
        let mut scoped_params = ScopedParamStore::new(program.param_specs().clone());
        // Seed root values from compiled defaults so existing root params and any
        // undeclared default keys are present at the root owner path.
        scoped_params.set_root_params(program.get_default_params().clone());
        let root_cache = scoped_params.root_effective_params();
        let (
            scale_domain_cache,
            facet_semantic_cache,
            facet_scale_precompute_cache,
            guide_overflow_cache,
            legend_measurement_cache,
            text_measurement_cache,
        ) = new_plot_session_cache_handles();
        Self {
            program,
            ctx,
            scoped_params,
            root_cache,
            last_request: None,
            layout_profile: None,
            last_metrics: None,
            scale_domain_cache,
            facet_semantic_cache,
            facet_scale_precompute_cache,
            guide_overflow_cache,
            legend_measurement_cache,
            text_measurement_cache,
        }
    }

    fn refresh_root_cache(&mut self) {
        self.root_cache = self.scoped_params.root_effective_params();
    }

    /// Commit a fully merged root param map after a successful evaluation.
    fn commit_root_params(&mut self, params: IndexMap<String, ScalarValue>) {
        self.scoped_params.set_root_params(params);
        self.refresh_root_cache();
    }

    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.root_cache
    }

    pub fn set_params(&mut self, params: IndexMap<String, ScalarValue>) {
        let mut merged = self.program.get_default_params().clone();
        merged.extend(params);
        self.scoped_params.set_root_params(merged);
        self.refresh_root_cache();
    }

    pub fn apply_param_patch(&mut self, patch: IndexMap<String, ScalarValue>) {
        self.scoped_params.apply_root_patch(patch);
        self.refresh_root_cache();
    }

    /// Build the flat effective param map for a scope's sharing-owner paths.
    ///
    /// The root scope passes an empty map, which resolves every sharing level to
    /// the root owner path.
    pub fn effective_params_for_owner_paths(
        &self,
        sharing_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
    ) -> IndexMap<String, ScalarValue> {
        self.scoped_params
            .effective_params_for_owner_paths(sharing_owner_paths)
    }

    /// Apply scoped parameter assignments produced by an event binding.
    pub fn apply_scoped_param_patch(&mut self, patch: Vec<ScopedParamAssignment>) {
        let touches_root = patch
            .iter()
            .any(|assignment| assignment.owner_path.is_empty());
        self.scoped_params.apply_scoped_patch(patch);
        if touches_root {
            self.refresh_root_cache();
        }
    }

    /// Snapshot the entire scoped parameter store for gesture freezing.
    pub fn snapshot_scoped_params(&self) -> ScopedParamStoreSnapshot {
        self.scoped_params.snapshot()
    }

    /// Resolve effective params for a scope from a frozen scoped-store snapshot.
    pub fn effective_params_from_snapshot(
        &self,
        snapshot: &ScopedParamStoreSnapshot,
        sharing_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
    ) -> IndexMap<String, ScalarValue> {
        self.scoped_params
            .effective_params_from_snapshot(snapshot, sharing_owner_paths)
    }

    /// Deterministic scoped fingerprint for the requested param names.
    // Consumed by facet-scoped cache keys when faceted scoped-param evaluation
    // lands; retained as part of the scoped-param store contract.
    #[allow(dead_code)]
    pub(crate) fn scoped_fingerprint_for_names(
        &self,
        names: &BTreeSet<String>,
    ) -> Vec<(String, Vec<String>, String)> {
        self.scoped_params.scoped_fingerprint_for_names(names)
    }

    pub fn last_metrics(&self) -> Option<&EvaluationMetrics> {
        self.last_metrics.as_ref()
    }

    pub fn take_metrics(&mut self) -> Option<EvaluationMetrics> {
        self.last_metrics.take()
    }

    /// Build a shareable handle to the scoped store for per-cell evaluation,
    /// only when non-root (`Free`/`Level`) assignments exist. Returns `None` on
    /// the common path so evaluation stays allocation-free and unchanged.
    fn scoped_param_store_handle(&self) -> Option<Arc<ScopedParamStore>> {
        self.scoped_params
            .has_scoped_overrides()
            .then(|| Arc::new(self.scoped_params.clone()))
    }

    pub async fn evaluate(
        &mut self,
        request: EvaluationRequest,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        let (evaluated, _) = self.evaluate_with_metrics(request).await?;
        Ok(evaluated)
    }

    pub async fn evaluate_with_metrics(
        &mut self,
        request: EvaluationRequest,
    ) -> Result<(EvaluatedPlot, EvaluationMetrics), AvengerChartError> {
        let mode = request.mode;
        let next_params = self.params_for_request(&request);
        let options = options_for_evaluation_mode(mode, request.options);
        let use_measurement_profile_caches = mode != EvaluationMode::ForceRemeasure;
        let scoped_store = self.scoped_param_store_handle();

        if mode == EvaluationMode::Preview {
            let mut preview_fallback_reasons = Vec::new();
            let mut preview_attempt_duration = Duration::default();
            if let Some(layout_profile) = self.layout_profile.as_ref() {
                let preview_attempt_start = Instant::now();
                let attempt = self
                    .program
                    .evaluate_preview_with_layout_profile_and_metrics(
                        self.ctx.as_ref(),
                        Some(next_params.clone()),
                        options.clone(),
                        layout_profile,
                        self.scale_domain_cache.clone(),
                        self.facet_semantic_cache.clone(),
                        self.facet_scale_precompute_cache.clone(),
                        use_measurement_profile_caches.then(|| self.guide_overflow_cache.clone()),
                        use_measurement_profile_caches
                            .then(|| self.legend_measurement_cache.clone()),
                        use_measurement_profile_caches.then(|| self.text_measurement_cache.clone()),
                        scoped_store.clone(),
                    )
                    .await?;
                preview_attempt_duration += preview_attempt_start.elapsed();
                if let Some((evaluated, mut metrics, layout_profile)) = attempt.reused {
                    metrics.mode = mode;
                    metrics.record_preview_attempt_duration(preview_attempt_duration);
                    self.commit_root_params(next_params.clone());
                    self.last_request = Some(EvaluationRequestSummary {
                        mode,
                        params: next_params,
                    });
                    if let Some(layout_profile) = layout_profile {
                        self.layout_profile = Some(layout_profile);
                    }
                    self.last_metrics = Some(metrics.clone());
                    return Ok((evaluated, metrics));
                }
                preview_fallback_reasons.extend(attempt.fallback_reasons);
            } else {
                preview_fallback_reasons.push(PreviewProfileFallbackReason::NoPriorProfile);
            }

            let (evaluated, mut metrics, layout_profile) = self
                .program
                .evaluate_with_options_and_metrics_with_scale_domain_cache(
                    self.ctx.as_ref(),
                    Some(next_params.clone()),
                    options,
                    self.scale_domain_cache.clone(),
                    self.facet_semantic_cache.clone(),
                    self.facet_scale_precompute_cache.clone(),
                    use_measurement_profile_caches.then(|| self.guide_overflow_cache.clone()),
                    use_measurement_profile_caches.then(|| self.legend_measurement_cache.clone()),
                    use_measurement_profile_caches.then(|| self.text_measurement_cache.clone()),
                    scoped_store.clone(),
                )
                .await?;
            metrics.mode = mode;
            metrics.record_preview_attempt_duration(preview_attempt_duration);
            metrics.record_preview_profile_miss();
            metrics.record_preview_fallback();
            if preview_fallback_reasons.iter().any(|reason| {
                matches!(
                    reason,
                    PreviewProfileFallbackReason::PhysicalStructureMismatch
                        | PreviewProfileFallbackReason::LogicalStructureMismatch
                        | PreviewProfileFallbackReason::MissingTerminalProfile
                )
            }) {
                metrics.record_preview_structure_reflow_miss();
            }
            metrics.record_preview_profile_fallback_reasons(preview_fallback_reasons);
            self.commit_root_params(next_params.clone());
            self.last_request = Some(EvaluationRequestSummary {
                mode,
                params: next_params,
            });
            self.layout_profile = layout_profile;
            self.last_metrics = Some(metrics.clone());
            return Ok((evaluated, metrics));
        }

        let (evaluated, mut metrics, layout_profile) = self
            .program
            .evaluate_with_options_and_metrics_with_scale_domain_cache(
                self.ctx.as_ref(),
                Some(next_params.clone()),
                options,
                self.scale_domain_cache.clone(),
                self.facet_semantic_cache.clone(),
                self.facet_scale_precompute_cache.clone(),
                use_measurement_profile_caches.then(|| self.guide_overflow_cache.clone()),
                use_measurement_profile_caches.then(|| self.legend_measurement_cache.clone()),
                use_measurement_profile_caches.then(|| self.text_measurement_cache.clone()),
                scoped_store,
            )
            .await?;
        metrics.mode = mode;
        self.commit_root_params(next_params.clone());
        self.last_request = Some(EvaluationRequestSummary {
            mode,
            params: next_params,
        });
        self.layout_profile = layout_profile;
        self.last_metrics = Some(metrics.clone());
        Ok((evaluated, metrics))
    }

    fn params_for_request(&self, request: &EvaluationRequest) -> IndexMap<String, ScalarValue> {
        let mut params = self.program.get_default_params().clone();
        params.extend(
            request
                .params
                .clone()
                .unwrap_or_else(|| self.root_cache.clone()),
        );
        if let Some(patch) = &request.param_patch {
            params.extend(patch.clone());
        }
        params
    }
}

impl CompiledPlot {
    /// Instantiate this compiled plot as a reusable evaluation session.
    pub fn instantiate(self: Arc<Self>, ctx: Arc<SessionContext>) -> PlotSession {
        PlotSession::new(self, ctx)
    }

    /// Instantiate this compiled plot and initialize session params.
    pub fn instantiate_with_params(
        self: Arc<Self>,
        ctx: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
    ) -> PlotSession {
        let mut session = PlotSession::new(self, ctx);
        session.set_params(params);
        session
    }

    pub(crate) fn top_level_scale_domain_cache_key(
        &self,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> ScaleDomainCacheKey {
        scale_domain_cache_key_for_parts_with_scope(
            &self.marks,
            &self.scale_specs,
            &self.data,
            None,
            ctx,
            params,
            ScaleDomainCacheScope::TopLevel,
        )
    }

    pub(crate) fn facet_scale_precompute_store_from_session_cache(
        &self,
        cache: &FacetScalePrecomputeCacheHandle,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
    ) -> (Arc<FacetScalePrecomputeStore>, bool) {
        let relevant_params = facet_scale_precompute_dependency_params(self, ctx, params)
            .into_iter()
            .map(|name| {
                let value = params
                    .get(&name)
                    .or_else(|| params.get(&format!("${name}")))
                    .map(|value| format!("{value:?}"))
                    .unwrap_or_else(|| "<missing>".to_string());
                (name, value)
            })
            .collect();
        let key = FacetScalePrecomputeCacheKey {
            program_ptr: self as *const _ as usize,
            facet_tree_structure: facet_tree.structure_cache_key(),
            params: relevant_params,
        };
        cache
            .lock()
            .expect("facet scale precompute cache lock poisoned")
            .store_for_key(key)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn guide_overflow_cache_key(
        &self,
        guide_ptr: usize,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        estimate_width: f32,
        estimate_height: f32,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        data_override: Option<&DataFrame>,
    ) -> GuideOverflowCacheKey {
        let mut scale_signatures = scales
            .iter()
            .map(|(name, scale)| (name.clone(), configured_scale_signature(scale)))
            .collect::<Vec<_>>();
        scale_signatures.sort_by(|a, b| a.0.cmp(&b.0));

        GuideOverflowCacheKey {
            program_ptr: self as *const _ as usize,
            guide_ptr,
            estimate_width: estimate_width.to_bits(),
            estimate_height: estimate_height.to_bits(),
            scales: scale_signatures,
            params: params
                .iter()
                .map(|(name, value)| (name.clone(), format!("{value:?}")))
                .collect(),
            facet_path: facet_path
                .iter()
                .map(|value| format!("{value:?}"))
                .collect(),
            facet_tree_structure: facet_tree.structure_cache_key(),
            child_frame_sharing_path: format!("{child_frame_sharing_path:?}"),
            data_override_plan: data_override.map(|df| format!("{:?}", df.logical_plan())),
        }
    }

    pub(crate) fn guide_overflow_cache_key_for_discriminator(
        &self,
        guide_ptr: usize,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        estimate_width: f32,
        estimate_height: f32,
        params: &IndexMap<String, ScalarValue>,
        discriminator: String,
    ) -> GuideOverflowCacheKey {
        let mut scale_signatures = scales
            .iter()
            .map(|(name, scale)| (name.clone(), configured_scale_signature(scale)))
            .collect::<Vec<_>>();
        scale_signatures.sort_by(|a, b| a.0.cmp(&b.0));

        GuideOverflowCacheKey {
            program_ptr: self as *const _ as usize,
            guide_ptr,
            estimate_width: estimate_width.to_bits(),
            estimate_height: estimate_height.to_bits(),
            scales: scale_signatures,
            params: params
                .iter()
                .map(|(name, value)| (name.clone(), format!("{value:?}")))
                .collect(),
            facet_path: Vec::new(),
            facet_tree_structure: vec![discriminator],
            child_frame_sharing_path: String::new(),
            data_override_plan: None,
        }
    }

    pub(crate) fn legend_measurement_cache_key(
        &self,
        group: &PreparedLegendGroup,
        available_space: Size2D,
        position: LegendPosition,
        params: &IndexMap<String, ScalarValue>,
    ) -> LegendMeasurementCacheKey {
        LegendMeasurementCacheKey {
            program_ptr: self as *const _ as usize,
            layout_key: group.layout_key.clone(),
            renderer_name: group.renderer.name().to_string(),
            legend: format!("{:?}", group.legend),
            channels: group
                .channels
                .iter()
                .map(legend_channel_signature)
                .collect(),
            available_width: available_space.width.to_bits(),
            available_height: available_space.height.to_bits(),
            position: format!("{position:?}"),
            params: params
                .iter()
                .map(|(name, value)| (name.clone(), format!("{value:?}")))
                .collect(),
        }
    }
}

pub(crate) fn scale_domain_cache_key_for_parts_with_scope(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    scale_specs: &HashMap<String, PlotScaleSpec>,
    data: &Option<LogicalPlanNode>,
    data_override: Option<&DataFrame>,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    scope: ScaleDomainCacheScope,
) -> ScaleDomainCacheKey {
    let relevant_params = scale_domain_dependency_params(
        compiled_marks,
        scale_specs,
        data,
        data_override,
        ctx,
        params,
    );
    ScaleDomainCacheKey {
        subject: ScaleDomainCacheSubject {
            marks_ptr: compiled_marks.as_ptr() as usize,
            scale_specs_ptr: scale_specs as *const _ as usize,
            data_ptr: data
                .as_ref()
                .map(|node| node as *const LogicalPlanNode as usize)
                .unwrap_or(0),
            data_override_plan: data_override.map(|df| format!("{:?}", df.logical_plan())),
        },
        scope,
        params: relevant_params
            .into_iter()
            .map(|name| {
                let value = params
                    .get(&name)
                    .or_else(|| params.get(&format!("${name}")))
                    .map(|value| format!("{value:?}"))
                    .unwrap_or_else(|| "<missing>".to_string());
                (name, value)
            })
            .collect(),
    }
}

fn scale_domain_dependency_params(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    scale_specs: &HashMap<String, PlotScaleSpec>,
    data: &Option<LogicalPlanNode>,
    data_override: Option<&DataFrame>,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> BTreeSet<String> {
    let all_param_names = params.keys().cloned().collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();

    collect_plan_placeholders(data.as_ref(), ctx, &mut names, &all_param_names);
    if let Some(df) = data_override {
        collect_logical_plan_placeholders(df.logical_plan(), &mut names);
    }
    collect_marks_direct_dependency_placeholders(compiled_marks, ctx, &mut names, &all_param_names);

    for scale_spec in scale_specs.values() {
        match scale_spec {
            PlotScaleSpec::Local(config) => {
                collect_scale_config_placeholders(config, ctx, &mut names, &all_param_names);
            }
        }
    }

    names
}

pub(crate) fn changed_param_names(
    previous: &IndexMap<String, ScalarValue>,
    next: &IndexMap<String, ScalarValue>,
) -> BTreeSet<String> {
    let previous = normalized_param_values(previous);
    let next = normalized_param_values(next);
    previous
        .keys()
        .chain(next.keys())
        .filter(|name| previous.get(*name) != next.get(*name))
        .cloned()
        .collect()
}

fn normalized_param_values(params: &IndexMap<String, ScalarValue>) -> BTreeMap<String, String> {
    params
        .iter()
        .map(|(name, value)| {
            (
                name.trim_start_matches('$').to_string(),
                format!("{value:?}"),
            )
        })
        .collect()
}

pub(crate) fn layout_size_dependency_params(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> BTreeSet<String> {
    let all_param_names = normalized_param_values(params)
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    collect_layout_size_placeholders(&plot.layout_spec, ctx, &mut names, &all_param_names);
    // EvaluationContext injects resolved dimensions under these reserved names.
    // They may change during a resize even when the authored size param has a
    // different name, and they are safe for rendered data-mark retargeting.
    names.insert("width".to_string());
    names.insert("height".to_string());
    names
}

fn collect_layout_size_placeholders(
    layout_spec: &LayoutSpec,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    collect_size_mode_placeholders(&layout_spec.canvas, ctx, names, all_param_names);
    collect_size_mode_placeholders(&layout_spec.plot_area, ctx, names, all_param_names);
}

fn collect_size_mode_placeholders(
    mode: &SizeMode,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    match mode {
        SizeMode::Fixed { width, height } => {
            collect_serializable_expr_placeholders(width, ctx, names, all_param_names);
            collect_serializable_expr_placeholders(height, ctx, names, all_param_names);
        }
        SizeMode::Width(width) => {
            collect_serializable_expr_placeholders(width, ctx, names, all_param_names);
        }
        SizeMode::Height(height) => {
            collect_serializable_expr_placeholders(height, ctx, names, all_param_names);
        }
        SizeMode::Auto => {}
    }
}

fn collect_serializable_expr_placeholders(
    expr: &SerializableExpr,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    let node: LogicalExprNode = expr.clone().into();
    collect_expr_node_placeholders(Some(&node), ctx, names, all_param_names);
}

fn facet_scale_precompute_dependency_params(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> BTreeSet<String> {
    let all_param_names = params.keys().cloned().collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    collect_plot_dependency_placeholders(plot, ctx, &mut names, &all_param_names);
    names
}

#[derive(Clone, Copy)]
struct DependencyPlaceholderOptions {
    include_raw_domain: bool,
}

impl DependencyPlaceholderOptions {
    const ALL: Self = Self {
        include_raw_domain: true,
    };

    // Facet cell profile keys intentionally ignore params that are used only as
    // raw-domain scale overrides. Preview retargeting compares the cached and
    // current scale objects directly, so these params should move marks through
    // affine scale adjustments rather than invalidating the cached cell profile.
    const PROFILE_KEY: Self = Self {
        include_raw_domain: false,
    };
}

pub(crate) fn plot_profile_dependency_param_fingerprint(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Vec<(String, String)> {
    plot_dependency_param_fingerprint_with_options(
        plot,
        ctx,
        params,
        DependencyPlaceholderOptions::PROFILE_KEY,
    )
}

fn plot_dependency_param_fingerprint_with_options(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    options: DependencyPlaceholderOptions,
) -> Vec<(String, String)> {
    let all_param_names = params.keys().cloned().collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    collect_plot_dependency_placeholders_with_options(
        plot,
        ctx,
        &mut names,
        &all_param_names,
        options,
    );
    names
        .into_iter()
        .map(|name| {
            let value = params
                .get(&name)
                .or_else(|| params.get(&format!("${name}")))
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "<missing>".to_string());
            (name, value)
        })
        .collect()
}

fn configured_scale_signature(scale: &ConfiguredScaleWithSpec) -> String {
    configured_scale_runtime_signature(scale.configured())
}

fn configured_scale_runtime_signature(configured: &ConfiguredScale) -> String {
    let mut options = configured
        .config
        .options
        .iter()
        .map(|(name, value)| (name.clone(), format!("{value:?}")))
        .collect::<Vec<_>>();
    options.sort_by(|a, b| a.0.cmp(&b.0));
    format!(
        "type={};domain={:?};range={:?};options={:?}",
        configured.scale_impl.scale_type(),
        configured.domain(),
        configured.range(),
        options
    )
}

fn legend_channel_signature(channel: &LegendChannel) -> String {
    let mut related_channels = channel
        .related_channels
        .iter()
        .map(|(name, info)| (name.clone(), channel_info_signature(info)))
        .collect::<Vec<_>>();
    related_channels.sort_by(|a, b| a.0.cmp(&b.0));
    format!(
        "name={};expr={:?};scale={};channel_type={};sharing={:?};mark_type={};mark_index={};related={:?}",
        channel.name,
        channel.expression,
        configured_scale_runtime_signature(&channel.scale),
        channel.channel_type,
        channel.sharing_level,
        channel.mark_type,
        channel.mark_index,
        related_channels
    )
}

fn channel_info_signature(info: &ChannelInfo) -> String {
    match info {
        ChannelInfo::Scaled { expr, scale } => {
            format!(
                "scaled:expr={expr:?};scale={}",
                configured_scale_runtime_signature(scale)
            )
        }
        ChannelInfo::Constant { expr } => format!("constant:expr={expr:?}"),
    }
}

fn collect_plot_dependency_placeholders(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    collect_plot_dependency_placeholders_with_options(
        plot,
        ctx,
        names,
        all_param_names,
        DependencyPlaceholderOptions::ALL,
    );
}

fn collect_plot_dependency_placeholders_with_options(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    collect_plan_placeholders(plot.data.as_ref(), ctx, names, all_param_names);
    collect_marks_dependency_placeholders(&plot.marks, ctx, names, all_param_names, options);
    for scale_spec in plot.scale_specs.values() {
        match scale_spec {
            PlotScaleSpec::Local(config) => {
                collect_scale_config_placeholders_with_options(
                    config,
                    ctx,
                    names,
                    all_param_names,
                    options,
                );
            }
        }
    }
}

fn collect_marks_dependency_placeholders(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    for mark in compiled_marks {
        collect_mark_dependency_placeholders(mark.as_ref(), ctx, names, all_param_names, options);
    }
}

fn collect_marks_direct_dependency_placeholders(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    for mark in compiled_marks {
        collect_mark_direct_dependency_placeholders(
            mark.as_ref(),
            ctx,
            names,
            all_param_names,
            DependencyPlaceholderOptions::ALL,
        );
    }
}

fn collect_mark_dependency_placeholders(
    mark: &dyn avenger_chart_core::CompiledMark,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    collect_mark_direct_dependency_placeholders(mark, ctx, names, all_param_names, options);

    if let Some(facet_subplot) = facet_subplot_ref(mark) {
        collect_facet_subplot_dependency_placeholders(
            facet_subplot,
            ctx,
            names,
            all_param_names,
            options,
        );
    }
    if let Some(positioned_subplot) = mark.as_positioned_subplot() {
        if let Some(partition_expr) = positioned_subplot.partition_expr() {
            collect_expr_node_placeholders(Some(partition_expr), ctx, names, all_param_names);
        }
        collect_plot_dependency_placeholders_with_options(
            compiled_subplot_payload_child_plot(positioned_subplot.payload()),
            ctx,
            names,
            all_param_names,
            options,
        );
    }
    if let Some(concat_subplot) = concat::compiled_subplot(mark) {
        collect_plot_dependency_placeholders_with_options(
            concat_subplot.compiled_subplot(),
            ctx,
            names,
            all_param_names,
            options,
        );
    }
}

fn collect_mark_direct_dependency_placeholders(
    mark: &dyn avenger_chart_core::CompiledMark,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    collect_plan_placeholders(
        mark.data_context().logical_plan_node(),
        ctx,
        names,
        all_param_names,
    );
    for channel in mark.data_context().channels().values() {
        for expr in channel.all_exprs(ctx) {
            collect_expr_placeholders(&expr, names);
        }
        if let Some(config) = channel.get_scale_config() {
            collect_scale_config_placeholders_with_options(
                config,
                ctx,
                names,
                all_param_names,
                options,
            );
        }
    }
}

fn collect_facet_subplot_dependency_placeholders(
    facet_subplot: FacetSubplotRef<'_>,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    match facet_subplot {
        FacetSubplotRef::Row(mark) => {
            collect_expr_node_placeholders(mark.facet_order_expr(), ctx, names, all_param_names);
            collect_plot_dependency_placeholders_with_options(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
                options,
            );
        }
        FacetSubplotRef::Col(mark) => {
            collect_expr_node_placeholders(mark.facet_order_expr(), ctx, names, all_param_names);
            collect_plot_dependency_placeholders_with_options(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
                options,
            );
        }
        FacetSubplotRef::Wrap(mark) => {
            collect_expr_node_placeholders(mark.facet_order_expr(), ctx, names, all_param_names);
            match mark.facet_column_mode() {
                FacetWrapColumnMode::Auto => {}
                FacetWrapColumnMode::Fixed(expr) | FacetWrapColumnMode::ResponsiveWidth(expr) => {
                    collect_expr_node_placeholders(Some(&expr), ctx, names, all_param_names);
                }
            }
            collect_plot_dependency_placeholders_with_options(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
                options,
            );
        }
    }
}

fn collect_scale_config_placeholders(
    config: &ScaleConfigSpec,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    collect_scale_config_placeholders_with_options(
        config,
        ctx,
        names,
        all_param_names,
        DependencyPlaceholderOptions::ALL,
    );
}

fn collect_scale_config_placeholders_with_options(
    config: &ScaleConfigSpec,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    if let Maybe::Set(domain) = &config.domain {
        collect_scale_domain_placeholders_with_options(
            domain,
            ctx,
            names,
            all_param_names,
            options,
        );
    }
    if let Maybe::Set(ordering) = &config.ordering
        && let Some(order_expr) = &ordering.order_expr
    {
        collect_expr_node_placeholders(Some(order_expr), ctx, names, all_param_names);
    }
    for option_expr in config.options.values() {
        collect_expr_node_placeholders(Some(option_expr), ctx, names, all_param_names);
    }
}

fn collect_scale_domain_placeholders_with_options(
    domain: &ScaleDomain,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
    options: DependencyPlaceholderOptions,
) {
    if options.include_raw_domain {
        collect_expr_node_placeholders(domain.raw_domain.as_ref(), ctx, names, all_param_names);
    }
    match &domain.default_domain {
        ScaleDefaultDomain::Interval(start, end) => {
            collect_expr_node_placeholders(Some(start), ctx, names, all_param_names);
            collect_expr_node_placeholders(Some(end), ctx, names, all_param_names);
        }
        ScaleDefaultDomain::Discrete(values) => {
            for value in values {
                collect_expr_node_placeholders(Some(value), ctx, names, all_param_names);
            }
        }
        ScaleDefaultDomain::DomainExprs(domain_exprs) => {
            for domain_expr in domain_exprs {
                collect_plan_placeholders(
                    Some(domain_expr.dataframe.as_ref()),
                    ctx,
                    names,
                    all_param_names,
                );
                collect_expr_node_placeholders(
                    Some(&domain_expr.expr),
                    ctx,
                    names,
                    all_param_names,
                );
                collect_radius_expr_placeholders(
                    domain_expr.radius.as_ref(),
                    ctx,
                    names,
                    all_param_names,
                );
            }
        }
        ScaleDefaultDomain::NoDefault => {}
    }
}

fn collect_radius_expr_placeholders(
    radius: Option<&RadiusExpression>,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    match radius {
        Some(RadiusExpression::Symmetric(expr)) => {
            collect_expr_node_placeholders(Some(expr), ctx, names, all_param_names);
        }
        Some(RadiusExpression::Asymmetric { lower, upper }) => {
            collect_expr_node_placeholders(Some(lower), ctx, names, all_param_names);
            collect_expr_node_placeholders(Some(upper), ctx, names, all_param_names);
        }
        None => {}
    }
}

fn collect_expr_node_placeholders(
    node: Option<&LogicalExprNode>,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    let Some(node) = node else {
        return;
    };
    match node.to_expr(ctx) {
        Ok(expr) => collect_expr_placeholders(&expr, names),
        Err(_) => names.extend(all_param_names.iter().cloned()),
    }
}

fn collect_plan_placeholders(
    node: Option<&LogicalPlanNode>,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    let Some(node) = node else {
        return;
    };
    match node.to_logical_plan(ctx) {
        Ok(plan) => collect_logical_plan_placeholders(&plan, names),
        Err(_) => names.extend(all_param_names.iter().cloned()),
    }
}

fn collect_logical_plan_placeholders(plan: &LogicalPlan, names: &mut BTreeSet<String>) {
    let _ = plan.apply(|node| {
        for expr in node.expressions() {
            collect_expr_placeholders(&expr, names);
        }
        Ok(TreeNodeRecursion::Continue)
    });
}

fn collect_expr_placeholders(expr: &Expr, names: &mut BTreeSet<String>) {
    let _ = expr.apply(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate {
            names.insert(placeholder.id.trim_start_matches('$').to_string());
        }
        Ok(TreeNodeRecursion::Continue)
    });
}

#[cfg(test)]
mod tests {
    use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
    use datafusion::{prelude::SessionContext, scalar::ScalarValue};

    use crate::prelude::*;

    use super::*;

    fn collect_symbol_positions(scene: &SceneGraph) -> Vec<(f32, f32)> {
        fn collect_from_mark(mark: &SceneMark, origin: [f32; 2], positions: &mut Vec<(f32, f32)>) {
            match mark {
                SceneMark::Group(group) => {
                    let group_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
                    for child in &group.marks {
                        collect_from_mark(child, group_origin, positions);
                    }
                }
                SceneMark::Symbol(symbol) => {
                    for (x, y) in symbol.x_iter().zip(symbol.y_iter()) {
                        positions.push((origin[0] + x, origin[1] + y));
                    }
                }
                _ => {}
            }
        }

        let mut positions = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, scene.origin, &mut positions);
        }
        positions.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.total_cmp(&right.1))
        });
        positions
    }

    fn collect_symbol_sizes(scene: &SceneGraph) -> Vec<f32> {
        fn collect_from_mark(mark: &SceneMark, sizes: &mut Vec<f32>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, sizes);
                    }
                }
                SceneMark::Symbol(symbol) => {
                    sizes.extend(symbol.size_vec());
                }
                _ => {}
            }
        }

        let mut sizes = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut sizes);
        }
        sizes.sort_by(|left, right| left.total_cmp(right));
        sizes
    }

    fn assert_symbol_positions_close(actual: &SceneGraph, expected: &SceneGraph) {
        assert_symbol_positions_close_with_tolerance(actual, expected, 1.5);
    }

    fn assert_symbol_positions_close_with_tolerance(
        actual: &SceneGraph,
        expected: &SceneGraph,
        tolerance: f32,
    ) {
        let actual_positions = collect_symbol_positions(actual);
        let expected_positions = collect_symbol_positions(expected);
        assert_eq!(
            actual_positions.len(),
            expected_positions.len(),
            "symbol count mismatch"
        );
        for (idx, ((actual_x, actual_y), (expected_x, expected_y))) in actual_positions
            .iter()
            .zip(expected_positions.iter())
            .enumerate()
        {
            assert!(
                (actual_x - expected_x).abs() <= tolerance
                    && (actual_y - expected_y).abs() <= tolerance,
                "symbol position mismatch at {idx}: actual=({actual_x:.3}, {actual_y:.3}) expected=({expected_x:.3}, {expected_y:.3}) tolerance={tolerance:.3}"
            );
        }
    }

    fn assert_symbol_sizes_close(actual: &SceneGraph, expected: &SceneGraph) {
        let actual_sizes = collect_symbol_sizes(actual);
        let expected_sizes = collect_symbol_sizes(expected);
        assert_eq!(
            actual_sizes.len(),
            expected_sizes.len(),
            "symbol count mismatch"
        );
        for (idx, (actual_size, expected_size)) in
            actual_sizes.iter().zip(expected_sizes.iter()).enumerate()
        {
            assert!(
                (actual_size - expected_size).abs() <= 0.01,
                "symbol size mismatch at {idx}: actual={actual_size:.3} expected={expected_size:.3}"
            );
        }
    }

    async fn compile_session_test_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
            .compile(ctx)
            .await
    }

    async fn compile_width_param_scale_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(360.0)));
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .add_param(width.clone())
            .canvas_size(width.expr(), 300.0)
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
            .compile(ctx)
            .await
    }

    async fn compile_symbol_size_param_preview_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(360.0)));
        let symbol_size = Param::new("symbol_size", ScalarValue::Float64(Some(20.0)));
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .add_param(width.clone())
            .add_param(symbol_size.clone())
            .canvas_size(width.expr(), 300.0)
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .size_with(symbol_size.expr(), |c| c.no_legend()),
            )
            .compile(ctx)
            .await
    }

    #[tokio::test]
    async fn compiled_plot_resize_policy_classifies_layout_axes() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();

        let canvas = Plot::<Cartesian>::new()
            .canvas_size(640.0, 420.0)
            .compile(&ctx)
            .await?;
        assert_eq!(
            canvas.resize_policy(),
            ChartResizePolicy {
                width: ChartResizeAxisPolicy::CanvasConstrained,
                height: ChartResizeAxisPolicy::CanvasConstrained,
            }
        );

        let plot_area = Plot::<Cartesian>::new()
            .plot_size(320.0, 180.0)
            .compile(&ctx)
            .await?;
        assert_eq!(
            plot_area.resize_policy(),
            ChartResizePolicy {
                width: ChartResizeAxisPolicy::PlotConstrained,
                height: ChartResizeAxisPolicy::PlotConstrained,
            }
        );

        let mixed = Plot::<Cartesian>::new()
            .canvas_constraint(CanvasConstraint::width(700.0))
            .plot_constraint(PlotConstraint::height(160.0))
            .compile(&ctx)
            .await?;
        assert_eq!(
            mixed.resize_policy(),
            ChartResizePolicy {
                width: ChartResizeAxisPolicy::CanvasConstrained,
                height: ChartResizeAxisPolicy::PlotConstrained,
            }
        );

        let auto = Plot::<Cartesian>::new().compile(&ctx).await?;
        assert_eq!(
            auto.resize_policy(),
            ChartResizePolicy {
                width: ChartResizeAxisPolicy::Auto,
                height: ChartResizeAxisPolicy::Auto,
            }
        );

        let conflict = Plot::<Cartesian>::new()
            .canvas_constraint(CanvasConstraint::width(700.0))
            .plot_constraint(PlotConstraint::width(320.0))
            .compile(&ctx)
            .await?;
        assert_eq!(
            conflict.resize_policy(),
            ChartResizePolicy {
                width: ChartResizeAxisPolicy::Conflict,
                height: ChartResizeAxisPolicy::Auto,
            }
        );

        Ok(())
    }

    async fn compile_scale_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let scale_factor = Param::new("scale_factor", ScalarValue::Float64(Some(1.0)));
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .add_param(scale_factor.clone())
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x") * scale_factor.expr())
                    .y(col("y"))
                    .size(20.0),
            )
            .compile(ctx)
            .await
    }

    async fn compile_pan_zoom_param_preview_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let x_min = Param::new("x_min", ScalarValue::Float64(Some(0.0)));
        let x_max = Param::new("x_max", ScalarValue::Float64(Some(10.0)));
        let domain_min = x_min.clone();
        let domain_max = x_max.clone();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (3.0, 3.0), (8.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .add_param(x_min.clone())
            .add_param(x_max.clone())
            .canvas_size(420.0, 320.0)
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), move |c| {
                        c.scale_with::<Linear>(move |s| {
                            s.domain((domain_min.expr(), domain_max.expr()))
                                .nice(false)
                                .zero(false)
                        })
                    })
                    .y(col("y"))
                    .size(20.0),
            )
            .compile(ctx)
            .await
    }

    async fn compile_raw_domain_param_preview_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (3.0, 3.0), (8.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .add_param(x_domain.clone())
            .canvas_size(420.0, 320.0)
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), move |c| {
                        c.scale_with::<Linear>(move |s| {
                            s.raw_domain(raw.clone()).nice(false).zero(false)
                        })
                        .axis(|a| a.visible(false))
                    })
                    .y_with(col("y"), |c| c.axis(|a| a.visible(false)))
                    .size(20.0),
            )
            .compile(ctx)
            .await
    }

    fn count_symbol_scale_adjustments(scene: &SceneGraph) -> usize {
        fn collect_from_mark(mark: &SceneMark, count: &mut usize) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, count);
                    }
                }
                SceneMark::Symbol(symbol) => {
                    if symbol.x_adjustment.is_some() || symbol.y_adjustment.is_some() {
                        *count += 1;
                    }
                }
                _ => {}
            }
        }

        let mut count = 0;
        for mark in &scene.marks {
            collect_from_mark(mark, &mut count);
        }
        count
    }

    async fn compile_legend_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 2.0, 'A'), (2.0, 3.0, 'B'), (3.0, 5.0, 'A')
                ) AS t(x, y, category)",
            )
            .await?;
        Plot::<Cartesian>::new()
            .title("Cached Legend Plot")
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .fill_with(col("category"), |c| {
                        c.legend(|l| l.title("Category").position(LegendPosition::Right))
                    })
                    .size(20.0),
            )
            .compile(ctx)
            .await
    }

    async fn compile_text_measurement_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .title("Cached Session Title")
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
            .compile(ctx)
            .await
    }

    async fn compile_facet_width_param_scale_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(520.0)));
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0), ('A', 2.0, 4.0), ('A', 3.0, 6.0),
                    ('B', 10.0, 3.0), ('B', 12.0, 5.0), ('B', 14.0, 8.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        Plot::<FacetColumn>::new()
            .add_param(width.clone())
            .canvas_size(width.expr(), 320.0)
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(20.0)
                            .fill("#4682b4"),
                    ),
                )
                .column(col("group_name")),
            )
            .compile(ctx)
            .await
    }

    async fn compile_facet_child_scale_param_precompute_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let scale_factor = Param::new("scale_factor", ScalarValue::Float64(Some(1.0)));
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0), ('A', 2.0, 4.0), ('A', 3.0, 6.0),
                    ('B', 10.0, 3.0), ('B', 12.0, 5.0), ('B', 14.0, 8.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        Plot::<FacetColumn>::new()
            .add_param(scale_factor.clone())
            .canvas_size(520.0, 320.0)
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x") * scale_factor.expr())
                            .y(col("y"))
                            .size(20.0)
                            .fill("#4682b4"),
                    ),
                )
                .column(col("group_name")),
            )
            .compile(ctx)
            .await
    }

    async fn compile_responsive_wrap_width_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(420.0)));
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0), ('B', 2.0, 3.0), ('C', 3.0, 4.0),
                    ('D', 4.0, 5.0), ('E', 5.0, 6.0), ('F', 6.0, 7.0)
                ) AS t(facet, x, y)",
            )
            .await?;
        Plot::<FacetWrap>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(120.0))
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(20.0)
                            .fill("#4682b4"),
                    ),
                )
                .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
            )
            .compile(ctx)
            .await
    }

    async fn compile_responsive_wrap_width_and_child_scale_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(420.0)));
        let scale_factor = Param::new("scale_factor", ScalarValue::Float64(Some(1.0)));
        let child_scale_factor = scale_factor.clone();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0), ('B', 2.0, 3.0), ('C', 3.0, 4.0),
                    ('D', 4.0, 5.0), ('E', 5.0, 6.0), ('F', 6.0, 7.0)
                ) AS t(facet, x, y)",
            )
            .await?;
        Plot::<FacetWrap>::new()
            .add_param(width.clone())
            .add_param(scale_factor.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(120.0))
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x") * child_scale_factor.expr())
                            .y(col("y"))
                            .size(20.0)
                            .fill("#4682b4"),
                    ),
                )
                .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
            )
            .compile(ctx)
            .await
    }

    async fn compile_ordered_responsive_wrap_width_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        use datafusion::functions_aggregate::min_max::max;

        let width = Param::new("width", ScalarValue::Float64(Some(420.0)));
        let order_factor = Param::new("order_factor", ScalarValue::Float64(Some(1.0)));
        let order_expr = order_factor.clone();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0, 1.0), ('B', 2.0, 3.0, 2.0),
                    ('C', 3.0, 4.0, 3.0), ('D', 4.0, 5.0, 4.0),
                    ('E', 5.0, 6.0, 5.0), ('F', 6.0, 7.0, 6.0)
                ) AS t(facet, x, y, score)",
            )
            .await?;
        Plot::<FacetWrap>::new()
            .add_param(width.clone())
            .add_param(order_factor.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(120.0))
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(20.0)
                            .fill("#4682b4"),
                    ),
                )
                .wrap_with(col("facet"), move |c| {
                    c.responsive_columns(180.0)
                        .order_by(max(col("score") * order_expr.expr()))
                        .order_desc()
                }),
            )
            .compile(ctx)
            .await
    }

    async fn compile_row_nested_responsive_wrap_width_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(520.0)));
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 'A', 1.0, 2.0), ('North', 'B', 2.0, 3.0),
                    ('North', 'C', 3.0, 4.0), ('North', 'D', 4.0, 5.0),
                    ('North', 'E', 5.0, 6.0),
                    ('South', 'B', 2.5, 3.5), ('South', 'C', 3.5, 4.5),
                    ('South', 'D', 4.5, 5.5), ('South', 'E', 5.5, 6.5),
                    ('South', 'F', 6.5, 7.5)
                ) AS t(region, facet, x, y)",
            )
            .await?;
        let wrap = Plot::<FacetWrap>::new().mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(20.0)
                        .fill("#4682b4"),
                ),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(160.0)),
        );
        Plot::<FacetRow>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(120.0))
            .data(df)
            .mark(Subplot::new(wrap).row(col("region")))
            .compile(ctx)
            .await
    }

    async fn compile_column_nested_responsive_wrap_width_param_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(520.0)));
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 'A', 1.0, 2.0), ('North', 'B', 2.0, 3.0),
                    ('North', 'C', 3.0, 4.0), ('North', 'D', 4.0, 5.0),
                    ('North', 'E', 5.0, 6.0), ('North', 'F', 6.0, 7.0),
                    ('South', 'A', 1.5, 2.5), ('South', 'B', 2.5, 3.5),
                    ('South', 'C', 3.5, 4.5), ('South', 'D', 4.5, 5.5),
                    ('South', 'E', 5.5, 6.5), ('South', 'F', 6.5, 7.5)
                ) AS t(region, facet, x, y)",
            )
            .await?;
        let wrap = Plot::<FacetWrap>::new().mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(20.0)
                        .fill("#4682b4"),
                ),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(160.0)),
        );
        Plot::<FacetColumn>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(120.0))
            .data(df)
            .mark(Subplot::new(wrap).column(col("region")))
            .compile(ctx)
            .await
    }

    async fn compile_positioned_child_width_param_scale_cache_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let width = Param::new("width", ScalarValue::Float64(Some(520.0)));
        let parent_df = ctx
            .sql("SELECT * FROM (VALUES (0.3, 0.5), (0.7, 0.5)) AS t(parent_x, parent_y)")
            .await?;
        let child_df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.5), (3.0, 5.0)) AS t(child_x, child_y)")
            .await?;
        let child = Plot::<Cartesian>::new().data(child_df).mark(
            Symbol::new()
                .x(col("child_x"))
                .y(col("child_y"))
                .size(18.0)
                .fill("#4682b4"),
        );
        Plot::<Cartesian>::new()
            .add_param(width.clone())
            .canvas_size(width.expr(), 320.0)
            .data(parent_df)
            .mark(
                Subplot::<Cartesian>::new(child)
                    .subplot_x_with(col("parent_x"), |c| {
                        c.scale_with::<Linear>(|s| {
                            s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                        })
                    })
                    .subplot_y_with(col("parent_y"), |c| {
                        c.scale_with::<Linear>(|s| {
                            s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                        })
                    })
                    .plot_size(140.0, 100.0),
            )
            .compile(ctx)
            .await
    }

    async fn compile_concat_session_equivalence_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let left = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0)) AS t(x, y)")
            .await?;
        let right = ctx
            .sql("SELECT * FROM (VALUES (10.0, 1.0), (12.0, 4.0)) AS t(x, y)")
            .await?;
        let child = |df| {
            Plot::<Cartesian>::new().data(df).mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .size(18.0)
                    .fill("#4682b4"),
            )
        };
        Plot::<HConcat>::new()
            .canvas_size(620.0, 280.0)
            .mark(Subplot::new(child(left)).key("left"))
            .mark(Subplot::new(child(right)).key("right"))
            .compile(ctx)
            .await
    }

    #[tokio::test]
    async fn plot_session_exact_matches_one_shot() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);

        let one_shot = compiled.evaluate(&ctx, None).await?;
        let mut session = compiled.clone().instantiate(ctx.clone());
        let session_eval = session.evaluate(EvaluationRequest::new().exact()).await?;

        assert_eq!(session_eval.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(session_eval.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            session_eval.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_eq!(
            session.last_metrics().map(|metrics| metrics.mode),
            Some(EvaluationMode::Exact)
        );
        assert!(
            session.layout_profile.is_some(),
            "exact session evaluation should retain the final layout profile"
        );

        Ok(())
    }

    #[tokio::test]
    async fn one_shot_and_session_exact_match_representative_chart_families()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled_plots = vec![
            ("regular", compile_session_test_plot(ctx.as_ref()).await?),
            (
                "facet",
                compile_facet_width_param_scale_cache_plot(ctx.as_ref()).await?,
            ),
            (
                "wrap",
                compile_responsive_wrap_width_param_cache_plot(ctx.as_ref()).await?,
            ),
            (
                "concat",
                compile_concat_session_equivalence_plot(ctx.as_ref()).await?,
            ),
            (
                "positioned_subplot",
                compile_positioned_child_width_param_scale_cache_plot(ctx.as_ref()).await?,
            ),
        ];

        for (label, compiled) in compiled_plots {
            let compiled = Arc::new(compiled);
            let one_shot = compiled.evaluate(ctx.as_ref(), None).await?;
            let mut session = compiled.clone().instantiate(ctx.clone());
            let session_eval = session.evaluate(EvaluationRequest::new().exact()).await?;

            assert_eq!(
                session_eval.scene_graph.width, one_shot.scene_graph.width,
                "{label} width should match"
            );
            assert_eq!(
                session_eval.scene_graph.height, one_shot.scene_graph.height,
                "{label} height should match"
            );
            assert_eq!(
                session_eval.scene_graph.marks.len(),
                one_shot.scene_graph.marks.len(),
                "{label} scenegraph mark count should match"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn one_shot_evaluation_uses_temporary_session_caches_without_persisting()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_width_param_scale_cache_plot(&ctx).await?;

        let (_first, first) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;
        let (_second, second) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;

        assert_eq!(first.pipeline.scale_domain_cache_hits, 0);
        assert_eq!(first.pipeline.scale_domain_cache_misses, 1);
        assert_eq!(second.pipeline.scale_domain_cache_hits, 0);
        assert_eq!(second.pipeline.scale_domain_cache_misses, 1);
        assert!(
            first.pipeline.guide_overflow_cache_misses > 0
                || first.pipeline.text_measurement_cache_misses > 0,
            "one-shot evaluation should use temporary measurement-profile caches"
        );
        assert!(
            second.pipeline.guide_overflow_cache_misses > 0
                || second.pipeline.text_measurement_cache_misses > 0,
            "a second one-shot call should start with fresh temporary caches"
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_instances_keep_independent_params() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);
        let mut left = compiled.clone().instantiate(ctx.clone());
        let mut right = compiled.instantiate(ctx);

        let mut left_patch = IndexMap::new();
        left_patch.insert("zoom".to_string(), ScalarValue::Float64(Some(1.0)));
        let mut right_patch = IndexMap::new();
        right_patch.insert("zoom".to_string(), ScalarValue::Float64(Some(2.0)));

        left.apply_param_patch(left_patch);
        right.apply_param_patch(right_patch);

        assert_eq!(
            left.params().get("zoom"),
            Some(&ScalarValue::Float64(Some(1.0)))
        );
        assert_eq!(
            right.params().get("zoom"),
            Some(&ScalarValue::Float64(Some(2.0)))
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_metrics_record_requested_modes() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        for mode in [
            EvaluationMode::Exact,
            EvaluationMode::Preview,
            EvaluationMode::ForceRemeasure,
        ] {
            let (_evaluated, metrics) = session
                .evaluate_with_metrics(EvaluationRequest::new().mode(mode))
                .await?;
            assert_eq!(metrics.mode, mode);
            assert_eq!(
                session.last_metrics().map(|metrics| metrics.mode),
                Some(mode)
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn scale_domain_cache_reuses_domains_for_width_only_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.scale_domain_cache_hits, 0);
        assert_eq!(first.pipeline.scale_domain_cache_misses, 1);
        assert_eq!(first.pipeline.scale_builder_builds, 1);
        assert!(
            first.pipeline.scale_domain_collects > 0,
            "initial evaluation should infer scale domains"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert_eq!(second.pipeline.scale_domain_cache_hits, 1);
        assert_eq!(second.pipeline.scale_domain_cache_misses, 0);
        assert_eq!(second.pipeline.scale_builder_builds, 0);
        assert_eq!(second.pipeline.scale_domain_collects, 0);

        Ok(())
    }

    #[tokio::test]
    async fn guide_overflow_cache_reuses_profile_for_repeated_exact_evaluation()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.guide_overflow_cache_hits, 0);
        assert_eq!(first.pipeline.guide_overflow_cache_misses, 1);
        assert!(
            first.pipeline.guide_overflow_measure_calls > 0,
            "initial evaluation should measure guide overflow"
        );

        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(second.pipeline.guide_overflow_cache_hits, 1);
        assert_eq!(second.pipeline.guide_overflow_cache_misses, 0);
        assert!(
            second.pipeline.guide_overflow_measure_calls
                < first.pipeline.guide_overflow_measure_calls,
            "warm exact evaluation should skip the cached initial guide-overflow probe"
        );

        Ok(())
    }

    #[tokio::test]
    async fn legend_measurement_cache_reuses_measurements_for_repeated_exact_evaluation()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_legend_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            first.pipeline.legend_measurement_cache_misses > 0,
            "initial evaluation should populate legend measurement cache"
        );
        assert!(
            first.pipeline.legend_measurements > 0,
            "initial evaluation should measure at least one legend"
        );

        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            second.pipeline.legend_measurement_cache_hits > 0,
            "warm exact evaluation should reuse cached legend measurements"
        );
        assert_eq!(second.pipeline.legend_measurement_cache_misses, 0);
        assert_eq!(
            second.pipeline.legend_measurements, 0,
            "warm exact evaluation should avoid uncached legend measurements"
        );

        Ok(())
    }

    #[tokio::test]
    async fn text_measurement_cache_reuses_title_measurements_for_repeated_exact_evaluation()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_text_measurement_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            first.pipeline.text_measurement_cache_misses > 0,
            "initial evaluation should populate the text measurement cache"
        );

        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            second.pipeline.text_measurement_cache_hits > 0,
            "warm exact evaluation should reuse cached title measurements"
        );
        assert_eq!(second.pipeline.text_measurement_cache_misses, 0);

        Ok(())
    }

    #[tokio::test]
    async fn force_remeasure_bypasses_measurement_profile_caches() -> Result<(), AvengerChartError>
    {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_legend_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, warm) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            warm.pipeline.guide_overflow_cache_misses > 0
                || warm.pipeline.legend_measurement_cache_misses > 0
                || warm.pipeline.text_measurement_cache_misses > 0,
            "warm-up exact evaluation should populate at least one measurement-profile cache"
        );

        let (_evaluated, force) = session
            .evaluate_with_metrics(EvaluationRequest::new().force_remeasure())
            .await?;
        assert_eq!(force.mode, EvaluationMode::ForceRemeasure);
        assert_eq!(force.pipeline.guide_overflow_cache_hits, 0);
        assert_eq!(force.pipeline.guide_overflow_cache_misses, 0);
        assert_eq!(force.pipeline.legend_measurement_cache_hits, 0);
        assert_eq!(force.pipeline.legend_measurement_cache_misses, 0);
        assert_eq!(force.pipeline.text_measurement_cache_hits, 0);
        assert_eq!(force.pipeline.text_measurement_cache_misses, 0);
        assert!(
            force.pipeline.guide_overflow_measure_calls > 0,
            "force remeasure should still run guide overflow measurement"
        );
        assert!(
            force.pipeline.legend_measurements > 0,
            "force remeasure should still run legend measurement"
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_reuses_layout_profile_for_width_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial measurement profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_data_mark_reuses, 1);
        assert_eq!(preview.pipeline.preview_data_mark_reuse_misses, 0);
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);
        assert!(
            preview.pipeline.skipped_component_measure_calls > 0,
            "preview should report the skipped recursive measurement profile"
        );
        assert_eq!(evaluated.scene_graph.width, 640.0);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_reuses_data_marks_for_raw_domain_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_raw_domain_param_preview_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial raw-domain measurement profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("x_domain".to_string(), list_domain(2.0, 6.0));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(
            preview.pipeline.preview_data_mark_reuses, 1,
            "simple raw-domain Preview should retarget cached data marks"
        );
        assert_eq!(preview.pipeline.preview_data_mark_reuse_misses, 0);
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);
        assert_eq!(
            preview.pipeline.mark_data_collects, 0,
            "simple raw-domain Preview should not recollect mark data"
        );
        assert_eq!(
            preview.timings.preview_scale_refresh_us, 0,
            "simple raw-domain Preview should patch cached scale domains instead of rebuilding scales"
        );
        assert!(
            count_symbol_scale_adjustments(&evaluated.scene_graph) > 0,
            "retargeted symbol marks should carry scale adjustments for the renderer"
        );

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            6.0,
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_rebuilds_data_marks_for_explicit_domain_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_pan_zoom_param_preview_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial pan/zoom measurement profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("x_min".to_string(), ScalarValue::Float64(Some(2.0)));
        patch.insert("x_max".to_string(), ScalarValue::Float64(Some(6.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_data_mark_reuses, 0);
        assert_eq!(preview.pipeline.preview_data_mark_reuse_misses, 1);
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);
        assert!(
            preview.pipeline.scale_domain_cache_misses > 0,
            "domain-param preview should rebuild scale metadata while keeping measurement padding locked"
        );

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            6.0,
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_rebuilds_data_marks_for_style_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_symbol_size_param_preview_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial measurement profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("symbol_size".to_string(), ScalarValue::Float64(Some(80.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_data_mark_reuses, 0);
        assert_eq!(preview.pipeline.preview_data_mark_reuse_misses, 1);
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            6.0,
        );
        assert_symbol_sizes_close(&evaluated.scene_graph, &one_shot.scene_graph);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_without_prior_measurement_falls_back_to_exact()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 0);
        assert_eq!(preview.pipeline.preview_profile_misses, 1);
        assert_eq!(preview.pipeline.preview_fallbacks, 1);
        assert_eq!(
            preview.pipeline.preview_profile_fallback_reasons,
            vec![PreviewProfileFallbackReason::NoPriorProfile]
        );
        assert!(
            preview.facet_layout.plot_component_measure_calls > 0,
            "preview without a warm measurement should fall back to exact measurement"
        );
        assert_eq!(evaluated.scene_graph.width, 640.0);
        assert!(
            session.layout_profile.is_some(),
            "fallback exact measurement should warm future preview requests"
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_falls_back_when_responsive_wrap_logical_structure_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_ordered_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial ordered wrap profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        patch.insert("order_factor".to_string(), ScalarValue::Float64(Some(-1.0)));
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 0);
        assert_eq!(preview.pipeline.preview_profile_misses, 1);
        assert_eq!(preview.pipeline.preview_fallbacks, 1);
        assert_eq!(preview.pipeline.preview_structure_reflow_misses, 1);
        assert_eq!(
            preview.pipeline.preview_profile_fallback_reasons,
            vec![PreviewProfileFallbackReason::LogicalStructureMismatch]
        );
        assert!(
            preview.facet_layout.plot_component_measure_calls > 0,
            "logical slot/order changes should use exact fallback, not stale cell profiles"
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_facet_cell_measurement_profile_reflows_responsive_wrap_structure()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial responsive-wrap measurement"
        );
        assert_eq!(
            session
                .layout_profile
                .as_ref()
                .expect("warm exact layout profile")
                .facet_cell_profile_count(),
            6,
            "top-level wrap profile should index only terminal child cells"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let (preview_plot, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_structure_reflow_reuses, 1);
        assert_eq!(preview.pipeline.preview_structure_reflow_misses, 0);
        assert!(
            preview.pipeline.facet_cell_measurement_profile_reuses > 0,
            "changed wrap structure should reuse terminal cell measurement profiles"
        );
        assert_eq!(
            preview
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            preview.pipeline.facet_cell_measurement_profile_reuses,
            "each reused cell profile should refresh guide/layout chrome for the new physical wrap grid"
        );
        assert!(
            preview.pipeline.guide_overflow_measure_calls > 0,
            "reused profile preview should recompute guide overflow instead of carrying stale chrome"
        );
        assert!(
            preview.facet_layout.plot_component_measure_calls > 0,
            "changed wrap structure should rebuild container layout"
        );

        let mut preview_exact_params = IndexMap::new();
        preview_exact_params.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let preview_one_shot = compiled
            .evaluate(ctx.as_ref(), Some(preview_exact_params))
            .await?;
        assert_eq!(
            preview_plot.scene_graph.width,
            preview_one_shot.scene_graph.width
        );
        assert_eq!(
            preview_plot.scene_graph.height,
            preview_one_shot.scene_graph.height
        );
        assert_eq!(
            preview_plot.scene_graph.marks.len(),
            preview_one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close(&preview_plot.scene_graph, &preview_one_shot.scene_graph);

        let (settled, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        let mut one_shot_params = IndexMap::new();
        one_shot_params.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let one_shot = compiled
            .evaluate(ctx.as_ref(), Some(one_shot_params))
            .await?;
        assert_eq!(exact.mode, EvaluationMode::Exact);
        assert_eq!(settled.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(settled.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            settled.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close(&settled.scene_graph, &one_shot.scene_graph);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_reflow_refreshes_chrome_when_wrap_column_owner_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let mut initial = IndexMap::new();
        initial.insert("width".to_string(), ScalarValue::Float64(Some(520.0)));
        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(initial))
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the 2-column wrap profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (preview_plot, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_structure_reflow_reuses, 1);
        assert!(
            preview.pipeline.facet_cell_measurement_profile_reuses > 0,
            "2-column to 3-column wrap preview should reuse terminal child measurements"
        );
        assert_eq!(
            preview
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            preview.pipeline.facet_cell_measurement_profile_reuses,
            "reused cells must recompute guide/layout chrome after physical owner changes"
        );
        assert_eq!(
            preview.pipeline.skipped_component_measure_calls,
            preview.pipeline.facet_cell_measurement_profile_reuses,
            "profile reuse should still avoid full terminal component measurements"
        );
        assert!(
            preview.pipeline.guide_overflow_measure_calls > 0,
            "preview reflow should measure current guide ownership"
        );
        assert!(
            preview.timings.preview_attempt_us > 0,
            "preview diagnostics should include attempt timing"
        );
        assert!(
            preview.timings.preview_structure_reflow_us > 0,
            "preview diagnostics should include responsive-wrap reflow timing"
        );
        assert!(
            preview.timings.measure_cells_overflow_probe_us > 0,
            "preview diagnostics should include facet overflow-probe timing"
        );
        assert!(
            preview.timings.refresh_reused_profile_layout_us > 0,
            "preview diagnostics should include reused-cell chrome refresh timing"
        );
        assert!(
            preview.timings.guide_overflow_measure_us > 0,
            "preview diagnostics should include guide measurement timing"
        );
        assert!(
            preview.timings.build_plot_components_us > 0,
            "preview diagnostics should include component build timing"
        );
        assert!(
            preview.timings.components_to_evaluated_plot_us > 0,
            "preview diagnostics should include scene assembly timing"
        );

        let mut exact_params = IndexMap::new();
        exact_params.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let one_shot = compiled.evaluate(ctx.as_ref(), Some(exact_params)).await?;
        assert_eq!(preview_plot.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(preview_plot.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            preview_plot.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close(&preview_plot.scene_graph, &one_shot.scene_graph);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_mixed_canvas_width_preserves_leaf_height_without_wrap_reflow()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let mut initial = IndexMap::new();
        initial.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(initial))
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should populate the mixed-sizing layout profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(693.0)));
        let (preview_plot, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(
            preview.pipeline.preview_structure_reflow_reuses, 0,
            "nearby widths should keep the same responsive-wrap physical structure"
        );

        let mut exact_params = IndexMap::new();
        exact_params.insert("width".to_string(), ScalarValue::Float64(Some(693.0)));
        let one_shot = compiled.evaluate(ctx.as_ref(), Some(exact_params)).await?;
        assert_eq!(preview_plot.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(
            preview_plot.scene_graph.height, one_shot.scene_graph.height,
            "preview must preserve the measured leaf-height-owned extent instead of retargeting to the nominal estimate"
        );
        assert_symbol_positions_close(&preview_plot.scene_graph, &one_shot.scene_graph);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_responsive_wrap_replay_populates_timing_diagnostics()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let mut warm_params = IndexMap::new();
        warm_params.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(warm_params))
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should populate the responsive-wrap profile"
        );

        let mut saw_reflow = false;
        let widths = [650.0, 560.0, 520.0, 440.0, 900.0, 1100.0];
        for width in widths {
            let mut patch = IndexMap::new();
            patch.insert("width".to_string(), ScalarValue::Float64(Some(width)));
            let (evaluated, preview) = session
                .evaluate_with_metrics(
                    EvaluationRequest::new()
                        .preview()
                        .param_patch(patch.clone()),
                )
                .await?;

            assert_eq!(preview.mode, EvaluationMode::Preview);
            assert!(
                preview.timings.preview_attempt_us > 0,
                "preview width {width} should record attempt timing"
            );
            assert!(
                preview.timings.build_plot_components_us > 0,
                "preview width {width} should record component build timing"
            );
            assert!(
                preview.timings.components_to_evaluated_plot_us > 0,
                "preview width {width} should record scene assembly timing"
            );

            if preview.pipeline.preview_structure_reflow_reuses > 0 {
                saw_reflow = true;
                assert!(
                    preview.timings.preview_structure_reflow_us > 0,
                    "reflow preview width {width} should record reflow timing"
                );
                assert!(
                    preview.timings.measure_cells_overflow_probe_us > 0,
                    "reflow preview width {width} should record facet overflow-probe timing"
                );
                assert!(
                    preview.timings.refresh_reused_profile_layout_us > 0,
                    "reflow preview width {width} should record chrome refresh timing"
                );
                assert!(
                    preview.timings.guide_overflow_measure_us > 0,
                    "reflow preview width {width} should record guide measurement timing"
                );
            }

            let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
            assert_eq!(evaluated.scene_graph.width, one_shot.scene_graph.width);
            assert_eq!(evaluated.scene_graph.height, one_shot.scene_graph.height);
            assert_eq!(
                evaluated.scene_graph.marks.len(),
                one_shot.scene_graph.marks.len()
            );
            assert_symbol_positions_close(&evaluated.scene_graph, &one_shot.scene_graph);
        }

        assert!(
            saw_reflow,
            "replay widths should include at least one responsive-wrap structure reflow"
        );

        let (settled, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        let mut final_params = IndexMap::new();
        final_params.insert(
            "width".to_string(),
            ScalarValue::Float64(Some(*widths.last().expect("final width"))),
        );
        let one_shot = compiled.evaluate(ctx.as_ref(), Some(final_params)).await?;
        assert_eq!(exact.mode, EvaluationMode::Exact);
        assert_eq!(settled.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(settled.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            settled.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close(&settled.scene_graph, &one_shot.scene_graph);

        Ok(())
    }

    #[tokio::test]
    async fn facet_cell_measurement_profile_misses_when_child_dependency_param_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_responsive_wrap_width_and_child_scale_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build cell profiles"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        patch.insert("scale_factor".to_string(), ScalarValue::Float64(Some(2.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_structure_reflow_reuses, 1);
        assert_eq!(preview.pipeline.facet_cell_measurement_profile_reuses, 0);
        assert!(
            preview.pipeline.facet_cell_measurement_profile_misses > 0,
            "cell profile keys should miss when child data/scale dependencies change"
        );
        assert_eq!(preview.pipeline.skipped_component_measure_calls, 0);

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
        assert_eq!(evaluated.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(evaluated.scene_graph.height, one_shot.scene_graph.height);
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            3.0,
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_reflows_row_nested_responsive_wrap_with_holes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_row_nested_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build row-local wrap profiles"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_structure_reflow_reuses, 1);
        assert!(
            preview.pipeline.facet_cell_measurement_profile_reuses > 0,
            "row-nested responsive wrap should reuse row-local terminal profiles"
        );
        assert_eq!(
            preview
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            preview.pipeline.facet_cell_measurement_profile_reuses
        );
        assert!(
            preview.facet_layout.plot_component_measure_calls > 0,
            "row-nested reflow should rebuild physical container layout for holes/edges"
        );

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch.clone())).await?;
        assert_eq!(evaluated.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(evaluated.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            evaluated.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            3.0,
        );

        let (settled, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(exact.mode, EvaluationMode::Exact);
        assert_eq!(settled.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(settled.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            settled.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_facet_cell_measurement_profile_reflows_column_nested_responsive_wrap_from_local_width()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_column_nested_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.clone().instantiate(ctx.clone());

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial nested responsive-wrap profile"
        );
        assert_eq!(
            session
                .layout_profile
                .as_ref()
                .expect("warm exact layout profile")
                .facet_cell_profile_count(),
            12,
            "nested wrap profile should not index intermediate facet-band measurements"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let (evaluated, preview) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch.clone()),
            )
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.pipeline.preview_structure_reflow_reuses, 1);
        assert!(
            preview.pipeline.facet_cell_measurement_profile_reuses > 0,
            "nested responsive wrap should reuse terminal cell measurement profiles"
        );
        assert_eq!(
            preview
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            preview.pipeline.facet_cell_measurement_profile_reuses
        );

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(patch)).await?;
        assert_eq!(evaluated.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(evaluated.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            evaluated.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_symbol_positions_close_with_tolerance(
            &evaluated.scene_graph,
            &one_shot.scene_graph,
            6.0,
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_exact_settle_after_preview_uses_current_params_and_remeasures()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, _exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);

        let (settled, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;

        assert_eq!(exact.mode, EvaluationMode::Exact);
        assert_eq!(exact.pipeline.preview_profile_reuses, 0);
        assert_eq!(exact.pipeline.preview_profile_misses, 0);
        assert_eq!(exact.pipeline.preview_fallbacks, 0);
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "exact settle should rebuild an exact measurement profile"
        );
        assert_eq!(settled.scene_graph.width, 640.0);

        Ok(())
    }

    #[tokio::test]
    async fn force_remeasure_ignores_layout_profile_preview_cache() -> Result<(), AvengerChartError>
    {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, exact) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            exact.facet_layout.plot_component_measure_calls > 0,
            "warm exact evaluation should build the initial measurement profile"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
        let (_evaluated, force) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .force_remeasure()
                    .param_patch(patch),
            )
            .await?;

        assert_eq!(force.mode, EvaluationMode::ForceRemeasure);
        assert_eq!(force.pipeline.preview_profile_reuses, 0);
        assert_eq!(force.pipeline.preview_profile_misses, 0);
        assert_eq!(force.pipeline.preview_fallbacks, 0);
        assert!(
            force.facet_layout.plot_component_measure_calls > 0,
            "force remeasure should not retarget the cached preview measurement"
        );

        Ok(())
    }

    #[tokio::test]
    async fn scale_domain_cache_invalidates_when_channel_param_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_scale_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.scale_domain_cache_hits, 0);
        assert_eq!(first.pipeline.scale_domain_cache_misses, 1);
        assert!(
            first.pipeline.scale_domain_collects > 0,
            "initial evaluation should infer scale domains"
        );

        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(second.pipeline.scale_domain_cache_hits, 1);
        assert_eq!(second.pipeline.scale_domain_cache_misses, 0);
        assert_eq!(second.pipeline.scale_builder_builds, 0);
        assert_eq!(second.pipeline.scale_domain_collects, 0);

        let mut patch = IndexMap::new();
        patch.insert("scale_factor".to_string(), ScalarValue::Float64(Some(2.0)));
        let (_evaluated, third) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert_eq!(third.pipeline.scale_domain_cache_hits, 0);
        assert_eq!(third.pipeline.scale_domain_cache_misses, 1);
        assert_eq!(third.pipeline.scale_builder_builds, 1);
        assert!(
            third.pipeline.scale_domain_collects > 0,
            "changed channel param should rebuild scale domains"
        );

        Ok(())
    }

    #[tokio::test]
    async fn scale_domain_cache_reuses_facet_scoped_builders_for_width_only_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_facet_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            first.pipeline.scale_domain_cache_misses > 1,
            "initial faceted evaluation should populate top-level and facet-scope scale-domain caches"
        );
        assert!(
            first.pipeline.scale_domain_collects > 0,
            "initial faceted evaluation should infer scale domains"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert!(
            second.pipeline.scale_domain_cache_hits > 1,
            "width-only reevaluation should hit top-level and facet-scope scale-domain caches"
        );
        assert_eq!(second.pipeline.scale_domain_cache_misses, 0);
        assert_eq!(second.pipeline.scale_builder_builds, 0);
        assert_eq!(second.pipeline.scale_domain_collects, 0);

        Ok(())
    }

    #[tokio::test]
    async fn scale_domain_cache_reuses_child_frame_builders_for_width_only_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_positioned_child_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert!(
            first.pipeline.scale_domain_cache_misses > 1,
            "initial positioned-subplot evaluation should populate top-level and child-frame scale-domain caches"
        );
        assert!(
            first.pipeline.scale_domain_collects > 0,
            "initial positioned-subplot evaluation should infer scale domains"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert!(
            second.pipeline.scale_domain_cache_hits > 1,
            "width-only reevaluation should hit top-level and child-frame scale-domain caches"
        );
        assert_eq!(second.pipeline.scale_domain_cache_misses, 0);
        assert_eq!(second.pipeline.scale_builder_builds, 0);
        assert_eq!(second.pipeline.scale_domain_collects, 0);

        Ok(())
    }

    #[tokio::test]
    async fn facet_semantic_cache_reuses_slots_for_responsive_wrap_width_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.facet_semantic_cache_hits, 0);
        assert!(
            first.pipeline.facet_semantic_cache_misses > 0,
            "initial responsive wrap evaluation should populate the facet semantic cache"
        );

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert!(
            second.pipeline.facet_semantic_cache_hits > 0,
            "width-only responsive wrap reevaluation should reuse semantic partition slots"
        );
        assert_eq!(second.pipeline.facet_semantic_cache_misses, 0);

        Ok(())
    }

    #[tokio::test]
    async fn facet_scale_precompute_cache_reuses_store_for_width_only_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_facet_width_param_scale_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.facet_scale_precompute_cache_hits, 0);
        assert_eq!(first.pipeline.facet_scale_precompute_cache_misses, 1);

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert_eq!(second.pipeline.facet_scale_precompute_cache_hits, 1);
        assert_eq!(second.pipeline.facet_scale_precompute_cache_misses, 0);

        Ok(())
    }

    #[tokio::test]
    async fn facet_scale_precompute_cache_invalidates_when_child_channel_param_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_facet_child_scale_param_precompute_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.facet_scale_precompute_cache_hits, 0);
        assert_eq!(first.pipeline.facet_scale_precompute_cache_misses, 1);

        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(second.pipeline.facet_scale_precompute_cache_hits, 1);
        assert_eq!(second.pipeline.facet_scale_precompute_cache_misses, 0);

        let mut patch = IndexMap::new();
        patch.insert("scale_factor".to_string(), ScalarValue::Float64(Some(2.0)));
        let (_evaluated, third) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert_eq!(third.pipeline.facet_scale_precompute_cache_hits, 0);
        assert_eq!(third.pipeline.facet_scale_precompute_cache_misses, 1);

        Ok(())
    }

    #[tokio::test]
    async fn facet_scale_precompute_cache_respects_responsive_wrap_structure_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        let (_evaluated, first) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        assert_eq!(first.pipeline.facet_scale_precompute_cache_hits, 0);
        assert_eq!(first.pipeline.facet_scale_precompute_cache_misses, 1);

        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
        let (_evaluated, second) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
            .await?;
        assert_eq!(
            second.pipeline.facet_scale_precompute_cache_hits, 0,
            "a changed responsive-wrap physical structure must not reuse path-keyed precompute artifacts"
        );
        assert_eq!(second.pipeline.facet_scale_precompute_cache_misses, 1);

        Ok(())
    }

    #[tokio::test]
    async fn add_param_compiles_to_shared_sharing() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .compile(&ctx)
            .await?;
        let spec = compiled
            .param_specs()
            .get("width")
            .expect("width spec present");
        assert_eq!(spec.sharing, Sharing::Shared);
        Ok(())
    }

    #[tokio::test]
    async fn add_param_with_sharing_round_trips_through_serialization()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let compiled = Plot::<Cartesian>::new()
            .add_param_with_sharing(x_domain, Sharing::Level(1))
            .compile(&ctx)
            .await?;
        assert_eq!(
            compiled.param_specs().get("x_domain").unwrap().sharing,
            Sharing::Level(1)
        );

        let bytes = bincode::serialize(&compiled).expect("serialize compiled plot");
        let restored: CompiledPlot =
            bincode::deserialize(&bytes).expect("deserialize compiled plot");
        assert_eq!(
            restored.param_specs().get("x_domain").unwrap().sharing,
            Sharing::Level(1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_param_names_error_on_compile() {
        let ctx = SessionContext::new();
        let result = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(1.0))))
            .add_param_with_sharing(
                Param::new("width", ScalarValue::Float64(Some(2.0))),
                Sharing::Level(1),
            )
            .compile(&ctx)
            .await;
        let err = match result {
            Ok(_) => panic!("duplicate param name should error"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("Duplicate plot parameter 'width'"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn raw_domain_param_defaults_to_inferred_domain() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw_expr = x_domain.expr();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (3.0, 3.0), (8.0, 5.0)) AS t(x, y)")
            .await?;
        let compiled = Arc::new(
            Plot::<Cartesian>::new()
                .add_param(x_domain.clone())
                .canvas_size(420.0, 320.0)
                .data(df)
                .mark(
                    Symbol::new()
                        .x_with(col("x"), move |c| {
                            c.scale_with::<Linear>(move |s| s.raw_domain(raw_expr.clone()))
                        })
                        .y(col("y"))
                        .size(20.0),
                )
                .compile(&ctx)
                .await?,
        );
        // The raw-domain param default is a typed null list, so the scale falls
        // back to the inferred numeric domain and evaluation succeeds.
        assert!(matches!(
            compiled.get_default_params().get("x_domain"),
            Some(ScalarValue::List(_))
        ));
        let mut session = compiled.instantiate(ctx);
        let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;
        assert!(!evaluated.scene_graph.marks.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn root_cartesian_plot_exports_single_coordinate_scope() -> Result<(), AvengerChartError>
    {
        use crate::render::InteractionScopeKind;
        let ctx = Arc::new(SessionContext::new());
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (3.0, 3.0), (8.0, 5.0)) AS t(x, y)")
            .await?;
        let compiled = Arc::new(
            Plot::<Cartesian>::new()
                .canvas_size(420.0, 320.0)
                .data(df)
                .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);
        let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;

        assert_eq!(
            evaluated.interaction.scopes.len(),
            1,
            "unfaceted Cartesian plot should export exactly one coordinate scope"
        );
        let scope = &evaluated.interaction.scopes[0];
        assert_eq!(scope.kind, InteractionScopeKind::Coordinate);
        assert!(scope.facet_path.is_empty());
        assert!(scope.coord_node_path.is_empty());
        assert!(scope.channels.contains(&"x".to_string()));
        assert!(scope.channels.contains(&"y".to_string()));
        assert!(
            scope.scales.contains_key("x"),
            "scope should carry the x scale"
        );
        assert!(
            scope.scales.contains_key("y"),
            "scope should carry the y scale"
        );
        assert!(scope.plot_area_width > 0.0 && scope.plot_area_height > 0.0);
        assert!((scope.bounds.width - scope.plot_area_width).abs() < 1e-3);
        assert!((scope.bounds.height - scope.plot_area_height).abs() < 1e-3);
        Ok(())
    }

    #[tokio::test]
    async fn faceted_cartesian_plot_exports_one_scope_per_leaf_cell()
    -> Result<(), AvengerChartError> {
        use crate::render::InteractionScopeKind;
        let ctx = Arc::new(SessionContext::new());
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 2.0), ('A', 2.0, 4.0),
                    ('B', 10.0, 3.0), ('B', 12.0, 5.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let compiled = Arc::new(
            Plot::<FacetColumn>::new()
                .canvas_size(520.0, 320.0)
                .data(df)
                .mark(
                    Subplot::new(
                        Plot::<Cartesian>::new()
                            .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0)),
                    )
                    .column(col("group_name")),
                )
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);
        let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;

        // Two columns (A, B) => one Cartesian coordinate scope per leaf cell.
        assert_eq!(
            evaluated.interaction.scopes.len(),
            2,
            "expected one coordinate scope per faceted leaf cell"
        );
        for scope in &evaluated.interaction.scopes {
            assert_eq!(scope.kind, InteractionScopeKind::Coordinate);
            assert_eq!(
                scope.facet_path.len(),
                1,
                "each leaf scope should carry its column facet path"
            );
            assert!(scope.scales.contains_key("x") && scope.scales.contains_key("y"));
            // Free (level 0) resolves to the full cell path; the column owner
            // (level 1) resolves to the empty root path for a single facet level.
            assert_eq!(
                scope.sharing_owner_paths.get(&0),
                Some(&scope.facet_path),
                "Free sharing should own the full cell path"
            );
            assert!(scope.plot_area_width > 0.0 && scope.plot_area_height > 0.0);
        }
        // The two cells occupy distinct horizontal bands (column facet).
        let xs: Vec<f32> = evaluated
            .interaction
            .scopes
            .iter()
            .map(|s| s.bounds.x)
            .collect();
        assert!(
            (xs[0] - xs[1]).abs() > 1.0,
            "column facet cells should have distinct x origins, got {xs:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn faceted_preview_refreshes_per_cell_shared_domain() -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let compiled = Arc::new(
            Plot::<FacetColumn>::new()
                .add_param(x_domain.clone())
                .canvas_size(640.0, 320.0)
                .data(df)
                .mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x"), move |c| {
                                    c.scale_with::<Linear>(move |s| {
                                        s.raw_domain(raw.clone()).nice(false).zero(false)
                                    })
                                    .share_scale()
                                })
                                .y(col("y"))
                                .size(20.0),
                        ),
                    )
                    .column(col("group_name")),
                )
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);

        // Warm exact frame establishes the layout profile.
        session.evaluate(EvaluationRequest::new().exact()).await?;

        // Preview pan: write a concrete shared x domain.
        let mut patch = IndexMap::new();
        patch.insert(
            "x_domain".to_string(),
            ScalarValue::List(ScalarValue::new_list(
                &[
                    ScalarValue::Float64(Some(2.0)),
                    ScalarValue::Float64(Some(8.0)),
                ],
                &DataType::Float64,
                true,
            )),
        );
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        // The preview reused the layout profile (fast path), and every cell scope
        // now reports the panned shared x domain.
        assert_eq!(metrics.pipeline.preview_profile_reuses, 1);
        assert_eq!(metrics.pipeline.preview_fallbacks, 0);
        assert_eq!(evaluated.interaction.scopes.len(), 2);
        for scope in &evaluated.interaction.scopes {
            let (min, max) = scope
                .scales
                .get("x")
                .expect("scope has x scale")
                .numeric_interval_domain()
                .expect("x domain is numeric");
            assert!(
                (min - 2.0).abs() < 1e-3 && (max - 8.0).abs() < 1e-3,
                "preview cell x domain should reflect the pan, got ({min}, {max})"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn faceted_preview_reuses_data_marks_across_repeated_shared_raw_domain_pan()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let compiled = Arc::new(
            Plot::<FacetColumn>::new()
                .add_param(x_domain.clone())
                .canvas_size(640.0, 320.0)
                .data(df)
                .mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x"), move |c| {
                                    c.scale_with::<Linear>(move |s| {
                                        s.raw_domain(raw.clone()).nice(false).zero(false)
                                    })
                                    .share_scale()
                                })
                                .y(col("y"))
                                .size(20.0),
                        ),
                    )
                    .column(col("group_name")),
                )
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.clone().instantiate(ctx.clone());

        session.evaluate(EvaluationRequest::new().exact()).await?;

        let mut first_patch = IndexMap::new();
        first_patch.insert("x_domain".to_string(), list_domain(2.0, 8.0));
        let (_first, first_metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(first_patch))
            .await?;
        assert_eq!(first_metrics.pipeline.preview_profile_reuses, 1);
        assert_eq!(
            first_metrics.pipeline.facet_tree_builds, 0,
            "raw-domain-only Preview should reuse the profiled facet tree"
        );
        assert_eq!(
            first_metrics.pipeline.facet_tree_profile_reuses, 1,
            "raw-domain-only Preview should report facet tree profile reuse"
        );
        assert_eq!(
            first_metrics.pipeline.scale_domain_cache_hits
                + first_metrics.pipeline.scale_domain_cache_misses
                + first_metrics.pipeline.scale_builder_builds,
            0,
            "raw-domain-only facet Preview should reuse profiled root facet scales"
        );
        assert_eq!(
            first_metrics.pipeline.preview_data_mark_reuses, 2,
            "both facet cells should retarget exact data marks for the first raw-domain pan"
        );
        assert_eq!(
            first_metrics.pipeline.preview_data_mark_reuse_misses, 1,
            "only the top-level child-frame container should miss data-mark reuse"
        );
        assert_eq!(
            first_metrics.pipeline.mark_data_collects, 0,
            "facet containers should render from measured facet state without collecting mark data"
        );

        let mut second_patch = IndexMap::new();
        second_patch.insert("x_domain".to_string(), list_domain(3.0, 9.0));
        let (second, second_metrics) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(second_patch.clone()),
            )
            .await?;
        assert_eq!(second_metrics.pipeline.preview_profile_reuses, 1);
        assert_eq!(
            second_metrics.pipeline.facet_tree_builds, 0,
            "steady raw-domain Preview should keep reusing the profiled facet tree"
        );
        assert_eq!(
            second_metrics.pipeline.facet_tree_profile_reuses, 1,
            "steady raw-domain Preview should report facet tree profile reuse"
        );
        assert_eq!(
            second_metrics.pipeline.scale_domain_cache_hits
                + second_metrics.pipeline.scale_domain_cache_misses
                + second_metrics.pipeline.scale_builder_builds,
            0,
            "steady raw-domain facet Preview should reuse profiled root facet scales"
        );
        assert_eq!(
            second_metrics.pipeline.preview_data_mark_reuses, 2,
            "raw-domain params should not invalidate terminal facet-cell data-mark profiles"
        );
        assert_eq!(
            second_metrics.pipeline.preview_data_mark_reuse_misses, 1,
            "steady pan preview should avoid per-cell data rebuilds"
        );
        assert_eq!(
            second_metrics.pipeline.mark_data_collects, 0,
            "steady pan preview should not collect mark data for facet container renderers"
        );

        let one_shot = compiled.evaluate(ctx.as_ref(), Some(second_patch)).await?;
        assert_symbol_positions_close_with_tolerance(
            &second.scene_graph,
            &one_shot.scene_graph,
            6.0,
        );
        Ok(())
    }

    #[tokio::test]
    async fn root_scoped_params_resolve_to_root_owner_path() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let compiled = Arc::new(
            Plot::<Cartesian>::new()
                .add_param_with_sharing(x_domain, Sharing::Level(1))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);

        // Root scope (empty owner paths) resolves the Level(1) param to its default.
        let root = session.effective_params_for_owner_paths(&HashMap::new());
        assert!(matches!(root.get("x_domain"), Some(ScalarValue::List(_))));

        // Write a scoped value for a Level(1) owner path.
        let owner_path = vec![ScalarValue::Utf8(Some("A".to_string()))];
        let domain_value = ScalarValue::List(ScalarValue::new_list(
            &[
                ScalarValue::Float64(Some(2.0)),
                ScalarValue::Float64(Some(8.0)),
            ],
            &datafusion::arrow::datatypes::DataType::Float64,
            true,
        ));
        session.apply_scoped_param_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: owner_path.clone(),
            value: domain_value.clone(),
            replace_scoped_values: false,
        }]);

        // Root remains the default; the scoped owner path sees the written value.
        let root_after = session.effective_params_for_owner_paths(&HashMap::new());
        assert!(matches!(
            root_after.get("x_domain"),
            Some(ScalarValue::List(_))
        ));
        assert_ne!(
            root_after.get("x_domain"),
            Some(&domain_value),
            "root scope must not see the Level(1) scoped write"
        );
        let mut owner_paths = HashMap::new();
        owner_paths.insert(1u8, owner_path);
        let scoped = session.effective_params_for_owner_paths(&owner_paths);
        assert_eq!(scoped.get("x_domain"), Some(&domain_value));

        // Snapshot resolution mirrors live resolution, and fingerprints are non-empty.
        let snapshot = session.snapshot_scoped_params();
        let from_snapshot = session.effective_params_from_snapshot(&snapshot, &owner_paths);
        assert_eq!(from_snapshot.get("x_domain"), Some(&domain_value));
        let mut names = BTreeSet::new();
        names.insert("x_domain".to_string());
        let fingerprint = session.scoped_fingerprint_for_names(&names);
        assert!(!fingerprint.is_empty());
        Ok(())
    }

    /// Read the numeric x domain from each leaf-cell scope, keyed by the cell's
    /// facet value (the first path component).
    fn cell_x_domains(evaluated: &EvaluatedPlot) -> HashMap<String, (f32, f32)> {
        let mut out = HashMap::new();
        for scope in &evaluated.interaction.scopes {
            let cell = match scope.facet_path.first() {
                Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                other => panic!("unexpected facet path head: {other:?}"),
            };
            let domain = scope
                .scales
                .get("x")
                .expect("scope has x scale")
                .numeric_interval_domain()
                .expect("x domain is numeric");
            out.insert(cell, domain);
        }
        out
    }

    fn assert_close_tol(actual: (f32, f32), expected: (f32, f32), tol: f32, context: &str) {
        assert!(
            (actual.0 - expected.0).abs() < tol && (actual.1 - expected.1).abs() < tol,
            "{context}: expected ({}, {}) within {tol}, got ({}, {})",
            expected.0,
            expected.1,
            actual.0,
            actual.1
        );
    }

    /// An applied raw-domain override sets the domain exactly.
    fn assert_override(actual: (f32, f32), expected: (f32, f32), context: &str) {
        assert_close_tol(actual, expected, 1e-3, context);
    }

    /// An inferred (data-derived) domain may carry small scale padding.
    fn assert_inferred(actual: (f32, f32), expected: (f32, f32), context: &str) {
        assert_close_tol(actual, expected, 0.5, context);
    }

    /// Build a single-level column-faceted scatter whose per-cell x scale reads a
    /// raw-domain param with the requested sharing. Each cell spans x in [0, 10].
    async fn build_free_pan_session(
        sharing: Sharing,
        share_scale: bool,
    ) -> Result<(PlotSession, Param), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let compiled = Arc::new(
            Plot::<FacetColumn>::new()
                .add_param_with_sharing(x_domain.clone(), sharing)
                .canvas_size(640.0, 320.0)
                .data(df)
                .mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x"), move |c| {
                                    let c = c.scale_with::<Linear>(move |s| {
                                        s.raw_domain(raw.clone()).nice(false).zero(false)
                                    });
                                    if share_scale { c.share_scale() } else { c }
                                })
                                .y(col("y"))
                                .size(20.0),
                        ),
                    )
                    .column(col("group_name")),
                )
                .compile(&ctx)
                .await?,
        );
        Ok((compiled.instantiate(ctx), x_domain))
    }

    fn list_domain(min: f64, max: f64) -> ScalarValue {
        use datafusion::arrow::datatypes::DataType;
        ScalarValue::List(ScalarValue::new_list(
            &[
                ScalarValue::Float64(Some(min)),
                ScalarValue::Float64(Some(max)),
            ],
            &DataType::Float64,
            true,
        ))
    }

    #[tokio::test]
    async fn faceted_free_pan_updates_only_active_cell() -> Result<(), AvengerChartError> {
        // A `Free` (per-cell) raw-domain param: panning one cell's owner path must
        // move only that cell; the others fall back to their inferred domains.
        let (mut session, _x_domain) = build_free_pan_session(Sharing::Free, false).await?;

        // Warm exact frame (no scoped overrides) establishes the layout profile.
        session.evaluate(EvaluationRequest::new().exact()).await?;

        // Pan only cell "A" by writing a Free-scoped value at owner path ["A"].
        session.apply_scoped_param_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: vec![ScalarValue::Utf8(Some("A".to_string()))],
            value: list_domain(2.0, 8.0),
            replace_scoped_values: false,
        }]);

        // Preview reuse exercises the per-cell override pass (C3).
        let (preview, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview())
            .await?;
        assert_eq!(
            metrics.pipeline.preview_fallbacks, 0,
            "preview should reuse the layout profile"
        );
        assert!(metrics.pipeline.preview_profile_reuses >= 1);
        let preview_domains = cell_x_domains(&preview);
        assert_override(preview_domains["A"], (2.0, 8.0), "preview cell A (panned)");
        assert_inferred(
            preview_domains["B"],
            (0.0, 10.0),
            "preview cell B (inferred)",
        );

        // Exact re-eval exercises the fresh-measure per-cell injection (C2).
        let exact = session.evaluate(EvaluationRequest::new().exact()).await?;
        let exact_domains = cell_x_domains(&exact);
        assert_override(exact_domains["A"], (2.0, 8.0), "exact cell A (panned)");
        assert_inferred(exact_domains["B"], (0.0, 10.0), "exact cell B (inferred)");
        Ok(())
    }

    #[tokio::test]
    async fn effective_params_for_cell_resolves_nested_owner_paths() -> Result<(), AvengerChartError>
    {
        // Build a two-level row > column facet so Level(1)/Level(2) differ.
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North','West', 1.0, 2.0), ('North','East', 3.0, 4.0),
                    ('South','West', 5.0, 6.0), ('South','East', 7.0, 8.0)
                ) AS t(facet_row, facet_col, x, y)",
            )
            .await?;
        let leaf = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")));
        let col_plot =
            Plot::<FacetColumn>::new().mark(Subplot::new(leaf).col_with(col("facet_col"), |c| c));
        let compiled = Plot::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(col_plot).row_with(col("facet_row"), |c| c))
            .compile(&ctx)
            .await?;
        let tree =
            EvaluatedFacetTree::from_compiled_plot_with_params(&compiled, &ctx, &IndexMap::new())
                .await?;

        let utf8 = |s: &str| ScalarValue::Utf8(Some(s.to_string()));
        let north_west = vec![utf8("North"), utf8("West")];
        let north_east = vec![utf8("North"), utf8("East")];
        let south_west = vec![utf8("South"), utf8("West")];
        assert!(
            tree.cell_exists(&north_west),
            "north/west leaf should exist (path order is [row, col])"
        );

        // Owner-path math across sharing levels.
        assert_eq!(
            tree.sharing_owner_path(&north_west, 0),
            north_west,
            "Free → full path"
        );
        let level1_owner = tree.sharing_owner_path(&north_west, 1);
        assert_eq!(level1_owner, vec![utf8("North")], "Level(1) → row owner");
        assert!(
            tree.sharing_owner_path(&north_west, 2).is_empty(),
            "Level(2) → root (no remaining ancestor)"
        );
        assert!(
            tree.sharing_owner_path(&north_west, u8::MAX).is_empty(),
            "Shared → root"
        );

        // Glue: a Level(1) param written at the row owner is shared by every cell
        // in that row, but not by cells in the sibling row.
        let mut specs = IndexMap::new();
        specs.insert(
            "x_domain".to_string(),
            CompiledParamSpec::new(&Param::raw_domain("x_domain"), Sharing::Level(1)),
        );
        let mut store = ScopedParamStore::new(specs);
        let panned = list_domain(2.0, 8.0);
        store.apply_scoped_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: level1_owner.clone(),
            value: panned.clone(),
            replace_scoped_values: false,
        }]);
        assert_eq!(
            store
                .effective_params_for_cell(&tree, &north_west)
                .get("x_domain"),
            Some(&panned),
            "north/west sees the row pan"
        );
        assert_eq!(
            store
                .effective_params_for_cell(&tree, &north_east)
                .get("x_domain"),
            Some(&panned),
            "north/east (same row) sees the row pan"
        );
        assert_ne!(
            store
                .effective_params_for_cell(&tree, &south_west)
                .get("x_domain"),
            Some(&panned),
            "south row must not see north's pan"
        );

        let south_owner = tree.sharing_owner_path(&south_west, 1);
        let south_panned = list_domain(4.0, 9.0);
        store.apply_scoped_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: south_owner,
            value: south_panned.clone(),
            replace_scoped_values: true,
        }]);
        assert_ne!(
            store
                .effective_params_for_cell(&tree, &north_west)
                .get("x_domain"),
            Some(&panned),
            "replace_scoped_values clears the previous owner copy"
        );
        assert_eq!(
            store
                .effective_params_for_cell(&tree, &south_west)
                .get("x_domain"),
            Some(&south_panned),
            "replace_scoped_values keeps the replacement owner copy"
        );
        Ok(())
    }

    #[tokio::test]
    async fn nested_level1_pan_updates_only_active_row() -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;
        // Two-level row > column facet with Level(1) params + Level(1) scales:
        // panning a cell shares the domain with its row (one level up), not the
        // whole chart and not just the single cell.
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Top','Left', 0.0, 0.0),    ('Top','Left', 10.0, 1.0),
                    ('Top','Right', 0.0, 2.0),   ('Top','Right', 10.0, 3.0),
                    ('Bottom','Left', 0.0, 4.0), ('Bottom','Left', 10.0, 5.0),
                    ('Bottom','Right', 0.0, 6.0),('Bottom','Right', 10.0, 7.0)
                ) AS t(row_name, col_name, x, y)",
            )
            .await?;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(raw.clone()).nice(false).zero(false)
                    })
                    .with_scale_sharing(Sharing::Level(1))
                })
                .y(col("y"))
                .size(20.0),
        );
        let columns = Plot::<FacetColumn>::new().mark(Subplot::new(leaf).column(col("col_name")));
        let compiled = Arc::new(
            Plot::<FacetRow>::new()
                .add_param_with_sharing(x_domain.clone(), Sharing::Level(1))
                .canvas_size(640.0, 480.0)
                .data(df)
                .mark(Subplot::new(columns).row(col("row_name")))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);

        // Warm exact frame establishes the layout profile.
        session.evaluate(EvaluationRequest::new().exact()).await?;

        // Pan the "Top" row: write the Level(1) owner path (the row) directly.
        session.apply_scoped_param_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: vec![ScalarValue::Utf8(Some("Top".to_string()))],
            value: ScalarValue::List(ScalarValue::new_list(
                &[
                    ScalarValue::Float64(Some(2.0)),
                    ScalarValue::Float64(Some(8.0)),
                ],
                &DataType::Float64,
                true,
            )),
            replace_scoped_values: false,
        }]);

        // Assert via both the exact (fresh-measure, C2) and preview (reuse, C3)
        // paths that every Top-row cell moved and every Bottom-row cell did not.
        let assert_row_split = |evaluated: &EvaluatedPlot, label: &str| {
            for scope in &evaluated.interaction.scopes {
                let row = match scope.facet_path.first() {
                    Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                    other => panic!("unexpected facet path head: {other:?}"),
                };
                let domain = scope
                    .scales
                    .get("x")
                    .expect("scope has x scale")
                    .numeric_interval_domain()
                    .expect("x domain is numeric");
                match row.as_str() {
                    "Top" => assert_override(domain, (2.0, 8.0), &format!("{label} Top row")),
                    "Bottom" => {
                        assert_inferred(domain, (0.0, 10.0), &format!("{label} Bottom row"))
                    }
                    other => panic!("unexpected row {other}"),
                }
            }
        };

        let preview = session.evaluate(EvaluationRequest::new().preview()).await?;
        assert_eq!(preview.interaction.scopes.len(), 4, "2x2 leaf cells");
        assert_row_split(&preview, "preview");

        let exact = session.evaluate(EvaluationRequest::new().exact()).await?;
        assert_row_split(&exact, "exact");
        Ok(())
    }

    #[tokio::test]
    async fn facet_wrap_free_pans_only_active_cell() -> Result<(), AvengerChartError> {
        // A wrap facet is physically laid out as hidden row bands but has a single
        // logical facet level. `Free` must still target one wrapped cell, and
        // `Level(1)` must collapse to the whole wrap group (root), never an
        // internal physical wrap row.
        use crate::render::EvaluatedInteractionScope;
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 1.0),
                    ('B', 0.0, 2.0), ('B', 10.0, 3.0),
                    ('C', 0.0, 4.0), ('C', 10.0, 5.0),
                    ('D', 0.0, 6.0), ('D', 10.0, 7.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(raw.clone()).nice(false).zero(false)
                    })
                    .free_scale()
                })
                .y(col("y"))
                .size(20.0),
        );
        let compiled = Arc::new(
            Plot::<FacetWrap>::new()
                .add_param_with_sharing(x_domain.clone(), Sharing::Free)
                .canvas_size(640.0, 480.0)
                .data(df)
                .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| c.columns(lit(2))))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);

        // Warm exact frame populates the scopes and layout profile.
        let warm = session.evaluate(EvaluationRequest::new().exact()).await?;
        assert_eq!(warm.interaction.scopes.len(), 4, "four wrapped cells");

        let cell_has = |scope: &EvaluatedInteractionScope, value: &str| {
            scope
                .facet_path
                .iter()
                .any(|component| matches!(component, ScalarValue::Utf8(Some(v)) if v == value))
        };
        let cell_a = warm
            .interaction
            .scopes
            .iter()
            .find(|scope| cell_has(scope, "A"))
            .expect("cell A scope")
            .clone();

        // Level(1) for a wrap cell collapses to the whole group (root), proving it
        // never targets an internal physical wrap row.
        assert_eq!(
            cell_a.sharing_owner_paths.get(&1),
            Some(&Vec::new()),
            "FacetWrap Level(1) owner must be the root wrap group, not a physical row"
        );

        // Pan only cell A by writing at its Free (level 0) owner path — exactly
        // what the app router would compute.
        let free_owner = cell_a
            .sharing_owner_paths
            .get(&0)
            .cloned()
            .expect("level 0 owner path present");
        assert!(
            !free_owner.is_empty(),
            "Free owner path for a wrap cell must be the cell's own path, not root"
        );
        session.apply_scoped_param_patch(vec![ScopedParamAssignment {
            name: "x_domain".to_string(),
            owner_path: free_owner,
            value: list_domain(2.0, 8.0),
            replace_scoped_values: false,
        }]);

        let exact = session.evaluate(EvaluationRequest::new().exact()).await?;
        for scope in &exact.interaction.scopes {
            let domain = scope
                .scales
                .get("x")
                .expect("scope has x scale")
                .numeric_interval_domain()
                .expect("x domain is numeric");
            if cell_has(scope, "A") {
                assert_override(domain, (2.0, 8.0), "wrap Free cell A (panned)");
            } else {
                assert_inferred(domain, (0.0, 10.0), "wrap Free other cell (inferred)");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn facet_wrap_shared_x_free_y_resolve_independently() -> Result<(), AvengerChartError> {
        // Per-channel sharing on a wrap facet: x is Shared (one domain for every
        // cell) while y is Free (per cell). Each param must resolve independently
        // at its own sharing level, so a shared-x write pans all cells' x while a
        // free-y write pans only the active cell's y.
        use crate::render::EvaluatedInteractionScope;
        let ctx = Arc::new(SessionContext::new());
        let x_domain = Param::raw_domain("x_domain");
        let y_domain = Param::raw_domain("y_domain");
        let x_raw = x_domain.expr();
        let y_raw = y_domain.expr();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 0.0), ('B', 10.0, 10.0),
                    ('C', 0.0, 0.0), ('C', 10.0, 10.0),
                    ('D', 0.0, 0.0), ('D', 10.0, 10.0)
                ) AS t(group_name, x, y)",
            )
            .await?;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(x_raw.clone()).nice(false).zero(false)
                    })
                    .share_scale()
                })
                .y_with(col("y"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(y_raw.clone()).nice(false).zero(false)
                    })
                    .free_scale()
                })
                .size(20.0),
        );
        let compiled = Arc::new(
            Plot::<FacetWrap>::new()
                .add_param_with_sharing(x_domain.clone(), Sharing::Shared)
                .add_param_with_sharing(y_domain.clone(), Sharing::Free)
                .canvas_size(640.0, 480.0)
                .data(df)
                .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| c.columns(lit(2))))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);

        let warm = session.evaluate(EvaluationRequest::new().exact()).await?;
        assert_eq!(warm.interaction.scopes.len(), 4, "four wrapped cells");

        let cell_has = |scope: &EvaluatedInteractionScope, value: &str| {
            scope
                .facet_path
                .iter()
                .any(|component| matches!(component, ScalarValue::Utf8(Some(v)) if v == value))
        };
        let cell_a = warm
            .interaction
            .scopes
            .iter()
            .find(|scope| cell_has(scope, "A"))
            .expect("cell A scope")
            .clone();
        let a_free_owner = cell_a
            .sharing_owner_paths
            .get(&0)
            .cloned()
            .expect("level 0 owner path present");

        // Shared x → root owner; Free y → cell A's own owner path.
        session.apply_scoped_param_patch(vec![
            ScopedParamAssignment {
                name: "x_domain".to_string(),
                owner_path: Vec::new(),
                value: list_domain(2.0, 8.0),
                replace_scoped_values: false,
            },
            ScopedParamAssignment {
                name: "y_domain".to_string(),
                owner_path: a_free_owner,
                value: list_domain(1.0, 5.0),
                replace_scoped_values: false,
            },
        ]);

        let exact = session.evaluate(EvaluationRequest::new().exact()).await?;
        for scope in &exact.interaction.scopes {
            let x = scope
                .scales
                .get("x")
                .expect("x scale")
                .numeric_interval_domain()
                .expect("numeric x");
            let y = scope
                .scales
                .get("y")
                .expect("y scale")
                .numeric_interval_domain()
                .expect("numeric y");
            // Shared x panned for every cell.
            assert_override(x, (2.0, 8.0), "shared x (all cells)");
            // Free y panned only for cell A.
            if cell_has(scope, "A") {
                assert_override(y, (1.0, 5.0), "free y cell A (panned)");
            } else {
                assert_inferred(y, (0.0, 10.0), "free y other cell (inferred)");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn incompatible_free_param_for_shared_scale_errors_on_compile() {
        // A `Free` raw-domain param feeding a `Shared` scale is ambiguous and must
        // be rejected at compile time (validation item A).
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let result = Plot::<FacetColumn>::new()
            .add_param_with_sharing(x_domain, Sharing::Free)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), move |c| {
                                c.scale_with::<Linear>(move |s| s.raw_domain(raw.clone()))
                                    .share_scale()
                            })
                            .y(col("y")),
                    ),
                )
                .column(col("group_name")),
            )
            .compile(&ctx)
            .await;
        let err = match result {
            Ok(_) => panic!("Free param feeding a Shared scale should fail validation"),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("x_domain") && err.contains("Free") && err.contains("Shared"),
            "unexpected validation error: {err}"
        );
    }
}
