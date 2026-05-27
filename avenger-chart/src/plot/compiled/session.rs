//! Reusable evaluation session for a compiled plot.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use avenger_chart_core::{
    ChannelInfo, DefaultLogicalExprNodeExt, FacetWrapColumnMode, LegendChannel, LegendPosition,
    LogicalPlanNodeExt, Maybe, RadiusExpression, ScaleConfigSpec, ScaleDefaultDomain, ScaleDomain,
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
    layout::Size2D,
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
    builders: HashMap<ScaleDomainCacheKey, ScaleBuilder>,
}

impl ScaleDomainCache {
    pub(crate) fn get(&self, key: &ScaleDomainCacheKey) -> Option<ScaleBuilder> {
        self.builders.get(key).cloned()
    }

    pub(crate) fn insert(&mut self, key: ScaleDomainCacheKey, builder: ScaleBuilder) {
        self.builders.insert(key, builder);
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
    current_params: IndexMap<String, ScalarValue>,
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
        let current_params = program.get_default_params().clone();
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
            current_params,
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

    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.current_params
    }

    pub fn set_params(&mut self, params: IndexMap<String, ScalarValue>) {
        let mut merged = self.program.get_default_params().clone();
        merged.extend(params);
        self.current_params = merged;
    }

    pub fn apply_param_patch(&mut self, patch: IndexMap<String, ScalarValue>) {
        self.current_params.extend(patch);
    }

    pub fn last_metrics(&self) -> Option<&EvaluationMetrics> {
        self.last_metrics.as_ref()
    }

    pub fn take_metrics(&mut self) -> Option<EvaluationMetrics> {
        self.last_metrics.take()
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
                    )
                    .await?;
                preview_attempt_duration += preview_attempt_start.elapsed();
                if let Some((evaluated, mut metrics, layout_profile)) = attempt.reused {
                    metrics.mode = mode;
                    metrics.record_preview_attempt_duration(preview_attempt_duration);
                    self.current_params = next_params.clone();
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
            self.current_params = next_params.clone();
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
            )
            .await?;
        metrics.mode = mode;
        self.current_params = next_params.clone();
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
                .unwrap_or_else(|| self.current_params.clone()),
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

pub(crate) fn plot_dependency_param_fingerprint(
    plot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Vec<(String, String)> {
    facet_scale_precompute_dependency_params(plot, ctx, params)
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
    collect_plan_placeholders(plot.data.as_ref(), ctx, names, all_param_names);
    collect_marks_dependency_placeholders(&plot.marks, ctx, names, all_param_names);
    for scale_spec in plot.scale_specs.values() {
        match scale_spec {
            PlotScaleSpec::Local(config) => {
                collect_scale_config_placeholders(config, ctx, names, all_param_names);
            }
        }
    }
}

fn collect_marks_dependency_placeholders(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    for mark in compiled_marks {
        collect_mark_dependency_placeholders(mark.as_ref(), ctx, names, all_param_names);
    }
}

fn collect_marks_direct_dependency_placeholders(
    compiled_marks: &[Arc<dyn avenger_chart_core::CompiledMark>],
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    for mark in compiled_marks {
        collect_mark_direct_dependency_placeholders(mark.as_ref(), ctx, names, all_param_names);
    }
}

fn collect_mark_dependency_placeholders(
    mark: &dyn avenger_chart_core::CompiledMark,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    collect_mark_direct_dependency_placeholders(mark, ctx, names, all_param_names);

    if let Some(facet_subplot) = facet_subplot_ref(mark) {
        collect_facet_subplot_dependency_placeholders(facet_subplot, ctx, names, all_param_names);
    }
    if let Some(positioned_subplot) = mark.as_positioned_subplot() {
        if let Some(partition_expr) = positioned_subplot.partition_expr() {
            collect_expr_node_placeholders(Some(partition_expr), ctx, names, all_param_names);
        }
        collect_plot_dependency_placeholders(
            compiled_subplot_payload_child_plot(positioned_subplot.payload()),
            ctx,
            names,
            all_param_names,
        );
    }
    if let Some(concat_subplot) = concat::compiled_subplot(mark) {
        collect_plot_dependency_placeholders(
            concat_subplot.compiled_subplot(),
            ctx,
            names,
            all_param_names,
        );
    }
}

fn collect_mark_direct_dependency_placeholders(
    mark: &dyn avenger_chart_core::CompiledMark,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
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
            collect_scale_config_placeholders(config, ctx, names, all_param_names);
        }
    }
}

fn collect_facet_subplot_dependency_placeholders(
    facet_subplot: FacetSubplotRef<'_>,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    match facet_subplot {
        FacetSubplotRef::Row(mark) => {
            collect_expr_node_placeholders(mark.facet_order_expr(), ctx, names, all_param_names);
            collect_plot_dependency_placeholders(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
            );
        }
        FacetSubplotRef::Col(mark) => {
            collect_expr_node_placeholders(mark.facet_order_expr(), ctx, names, all_param_names);
            collect_plot_dependency_placeholders(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
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
            collect_plot_dependency_placeholders(
                mark.compiled_subplot(),
                ctx,
                names,
                all_param_names,
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
    if let Maybe::Set(domain) = &config.domain {
        collect_scale_domain_placeholders(domain, ctx, names, all_param_names);
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

fn collect_scale_domain_placeholders(
    domain: &ScaleDomain,
    ctx: &SessionContext,
    names: &mut BTreeSet<String>,
    all_param_names: &BTreeSet<String>,
) {
    collect_expr_node_placeholders(domain.raw_domain.as_ref(), ctx, names, all_param_names);
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
    use datafusion::{prelude::SessionContext, scalar::ScalarValue};

    use crate::prelude::*;

    use super::*;

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
        let mut session = compiled.instantiate(ctx);

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
        let mut session = compiled.instantiate(ctx);

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
        let mut session = compiled.instantiate(ctx);

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
        let mut session = compiled.instantiate(ctx);

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
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);
        assert!(
            preview.pipeline.skipped_component_measure_calls > 0,
            "preview should report the skipped recursive measurement profile"
        );
        assert_eq!(evaluated.scene_graph.width, 640.0);

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_reuses_layout_profile_for_domain_param_change()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_pan_zoom_param_preview_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

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
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;

        assert_eq!(preview.mode, EvaluationMode::Preview);
        assert_eq!(preview.pipeline.preview_profile_reuses, 1);
        assert_eq!(preview.pipeline.preview_profile_misses, 0);
        assert_eq!(preview.pipeline.preview_fallbacks, 0);
        assert_eq!(preview.facet_layout.plot_component_measure_calls, 0);
        assert!(
            preview.pipeline.scale_domain_cache_misses > 0,
            "domain-param preview should rebuild scale metadata while keeping measurement padding locked"
        );

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

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_responsive_wrap_replay_populates_timing_diagnostics()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

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
        for width in [650.0, 560.0, 520.0, 440.0, 900.0, 1100.0] {
            let mut patch = IndexMap::new();
            patch.insert("width".to_string(), ScalarValue::Float64(Some(width)));
            let (_evaluated, preview) = session
                .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
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
        }

        assert!(
            saw_reflow,
            "replay widths should include at least one responsive-wrap structure reflow"
        );

        Ok(())
    }

    #[tokio::test]
    async fn facet_cell_measurement_profile_misses_when_child_dependency_param_changes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_responsive_wrap_width_and_child_scale_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

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
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
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
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
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

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_preview_facet_cell_measurement_profile_reflows_column_nested_responsive_wrap_from_local_width()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled =
            Arc::new(compile_column_nested_responsive_wrap_width_param_cache_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

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
        let (_evaluated, preview) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
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
}
