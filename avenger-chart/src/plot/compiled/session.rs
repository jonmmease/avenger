//! Reusable evaluation session for a compiled plot.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    DefaultLogicalExprNodeExt, FacetWrapColumnMode, LogicalPlanNodeExt, Maybe, RadiusExpression,
    ScaleConfigSpec, ScaleDefaultDomain, ScaleDomain,
};
use avenger_chart_scales::{PlotScaleSpec, ScaleBuilder};
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
    partition::PartitionSlotCache,
    render::{EvaluatedPlot, EvaluationMetrics, EvaluationMode, EvaluationOptions},
};

use super::{CompiledPlot, compiled_subplot_payload_child_plot};

pub(crate) type ScaleDomainCacheHandle = Arc<Mutex<ScaleDomainCache>>;
pub(crate) type FacetSemanticCacheHandle = Arc<Mutex<PartitionSlotCache>>;
pub(crate) type FacetScalePrecomputeCacheHandle = Arc<Mutex<FacetScalePrecomputeSessionCache>>;

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
    last_measurement: Option<()>,
    last_metrics: Option<EvaluationMetrics>,
    scale_domain_cache: ScaleDomainCacheHandle,
    facet_semantic_cache: FacetSemanticCacheHandle,
    facet_scale_precompute_cache: FacetScalePrecomputeCacheHandle,
}

impl PlotSession {
    pub(crate) fn new(program: Arc<CompiledPlot>, ctx: Arc<SessionContext>) -> Self {
        let current_params = program.get_default_params().clone();
        Self {
            program,
            ctx,
            current_params,
            last_request: None,
            last_measurement: None,
            last_metrics: None,
            scale_domain_cache: Arc::new(Mutex::new(ScaleDomainCache::default())),
            facet_semantic_cache: Arc::new(Mutex::new(PartitionSlotCache::new())),
            facet_scale_precompute_cache: Arc::new(Mutex::new(
                FacetScalePrecomputeSessionCache::default(),
            )),
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
        let (evaluated, mut metrics) = self
            .program
            .evaluate_with_options_and_metrics_with_scale_domain_cache(
                self.ctx.as_ref(),
                Some(next_params.clone()),
                request.options,
                self.scale_domain_cache.clone(),
                self.facet_semantic_cache.clone(),
                self.facet_scale_precompute_cache.clone(),
            )
            .await?;
        metrics.mode = mode;
        self.current_params = next_params.clone();
        self.last_request = Some(EvaluationRequestSummary {
            mode,
            params: next_params,
        });
        self.last_measurement = Some(());
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
