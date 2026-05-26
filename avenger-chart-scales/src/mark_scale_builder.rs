//! Scale building and domain inference from compiled mark channels.

#![allow(clippy::borrowed_box, clippy::too_many_arguments)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_scales::scales::{DomainKind, RangeKind};
use datafusion::{
    arrow::{
        array::AsArray,
        compute::cast,
        datatypes::{DataType as ArrowDataType, Float64Type},
    },
    common::{DFSchema, ScalarValue, tree_node::Transformed},
    dataframe::DataFrame,
    functions_aggregate::min_max::{max, min},
    logical_expr::{Expr, ExprSchemable, LogicalPlan, lit},
    prelude::{SessionContext, col},
};
use datafusion_common::tree_node::{TransformedResult, TreeNode};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use tracing::trace;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledMark, CoordinateSystemTransformCore,
    EvaluationContext as CoreEvaluationContext, RadiusExpression, ScaleRange, Theme,
    array_value_to_f64, default_channel_value_for_eval, params_to_datafusion,
    resolve_all_channel_refs, scalar_total_cmp, strip_trailing_numbers,
};

use crate::{
    Auto, ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec, DomainExpr, Ordinal,
    PlotScaleSpec, Scale, ScaleDefaultDomain, ScaleDomain, ScaleRuntimeExt, ScaleSpec,
    builder::{ChannelScaleData, DataExtents, ScaleBuilder},
    default_range_for_channel, scale_spec_for_preference,
    serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
};

/// Mark data and channels after the chart runtime has resolved any container
/// data scope and runtime aggregate preparation.
#[derive(Clone)]
pub struct PreparedScaleMark {
    pub mark: Arc<dyn CompiledMark>,
    pub dataframe: Option<DataFrame>,
    pub channels: IndexMap<String, ChannelValue>,
}

#[derive(Clone)]
enum PreparedRadiusExpression {
    Symmetric(Expr),
    Asymmetric { lower: Expr, upper: Expr },
}

const RADIUS_CHANNEL_PLACEHOLDER_PREFIX: &str = "__avenger_radius_channel__";

fn prepared_radius_from_serialized(
    radius: &RadiusExpression,
    ctx: &SessionContext,
) -> Result<PreparedRadiusExpression, AvengerChartError> {
    match radius {
        RadiusExpression::Symmetric(node) => {
            Ok(PreparedRadiusExpression::Symmetric(node.to_expr(ctx)?))
        }
        RadiusExpression::Asymmetric { lower, upper } => Ok(PreparedRadiusExpression::Asymmetric {
            lower: lower.to_expr(ctx)?,
            upper: upper.to_expr(ctx)?,
        }),
    }
}

fn substitute_radius_channel_placeholders(
    expr: Expr,
    resolve_channel: &dyn Fn(&str) -> Expr,
) -> Result<Expr, AvengerChartError> {
    expr.transform(&|candidate| {
        if let Expr::Column(column) = &candidate
            && let Some(channel_name) = column.name.strip_prefix(RADIUS_CHANNEL_PLACEHOLDER_PREFIX)
        {
            return Ok(Transformed::yes(resolve_channel(channel_name)));
        }

        Ok(Transformed::no(candidate))
    })
    .data()
    .map_err(AvengerChartError::DataFusionError)
}

fn substitute_radius_expression_placeholders(
    radius: PreparedRadiusExpression,
    resolve_channel: &dyn Fn(&str) -> Expr,
) -> Result<PreparedRadiusExpression, AvengerChartError> {
    match radius {
        PreparedRadiusExpression::Symmetric(expr) => Ok(PreparedRadiusExpression::Symmetric(
            substitute_radius_channel_placeholders(expr, resolve_channel)?,
        )),
        PreparedRadiusExpression::Asymmetric { lower, upper } => {
            Ok(PreparedRadiusExpression::Asymmetric {
                lower: substitute_radius_channel_placeholders(lower, resolve_channel)?,
                upper: substitute_radius_channel_placeholders(upper, resolve_channel)?,
            })
        }
    }
}

impl PreparedScaleMark {
    pub fn new(
        mark: Arc<dyn CompiledMark>,
        dataframe: Option<DataFrame>,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        Self {
            mark,
            dataframe,
            channels,
        }
    }
}

/// Build ScaleBuilder by executing expensive data queries once
///
/// This function implements a two-phase approach to handle radius-aware positional scales:
///
/// **Phase 1**: Build non-positional scales (e.g., `size`, `color`, `shape`) first.
/// These scales can be fully configured without knowing the plot dimensions.
///
/// **Phase 2**: Build positional scales (e.g., `x`, `y`) that may need radius-awareness.
/// These scales can query the non-positional scales (built in Phase 1) to calculate
/// domain padding based on symbol sizes.
///
/// This two-phase approach prevents type errors (e.g., `sqrt(Utf8)`) that would occur
/// if we tried to apply numeric transformations to categorical size scales during
/// positional scale domain inference.
///
/// The resulting ScaleBuilder caches data extents and can be reused to build scales
/// with different dimensions (e.g., for initial layout pass vs. final render pass)
/// without re-querying the data.
pub async fn build_scale_builder_from_marks<C>(
    compiled_marks: &[Arc<dyn CompiledMark>],
    scale_specs: &HashMap<String, PlotScaleSpec>,
    coord_transform: &C,
    data: &Option<LogicalPlanNode>,
    df_override: Option<DataFrame>,
    eval_ctx: &CoreEvaluationContext,
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    let ctx = eval_ctx.session_context.as_ref();
    let plot_df = data.as_ref().and_then(|data_node| {
        let logical_plan = LogicalPlanNode::to_logical_plan(data_node, ctx).ok()?;
        Some(DataFrame::new(ctx.state().clone(), logical_plan))
    });
    let inherited_df = df_override.or(plot_df);
    let prepared_marks = compiled_marks
        .iter()
        .map(|mark| {
            let dataframe = mark
                .data_context()
                .dataframe_with_context(ctx)
                .or_else(|| inherited_df.clone());
            PreparedScaleMark::new(
                mark.clone(),
                dataframe,
                mark.data_context().channels().clone(),
            )
        })
        .collect::<Vec<_>>();

    build_scale_builder_from_prepared_marks(
        &prepared_marks,
        scale_specs,
        coord_transform,
        eval_ctx,
        theme,
    )
    .await
}

/// Build a ScaleBuilder from mark-specific prepared data and channels.
pub async fn build_scale_builder_from_prepared_marks<C>(
    prepared_marks: &[PreparedScaleMark],
    scale_specs: &HashMap<String, PlotScaleSpec>,
    coord_transform: &C,
    eval_ctx: &CoreEvaluationContext,
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    let ctx = eval_ctx.session_context.as_ref();
    let params = &eval_ctx.params;
    let mut builder = ScaleBuilder::new();

    // Collect channels needing scales from marks
    let mut channels_with_scales = HashSet::new();
    for prepared in prepared_marks {
        let encodings = &prepared.channels;
        let resolved =
            resolve_all_channel_refs(encodings, ctx).unwrap_or_else(|_| encodings.clone());
        for (channel_name, channel_value) in resolved {
            if channel_value.get_scale_name(&channel_name).is_some() {
                channels_with_scales.insert(channel_name.clone());
            }
        }
    }

    // Also include explicit scale specs
    for channel in scale_specs.keys() {
        channels_with_scales.insert(channel.clone());
    }

    // Always ensure positional channels are present (handles implicit/unnamed scales
    // and guarantees we attempt to build/capture radius-aware domains for x/y)
    let positional_channel_set_temp: HashSet<String> = coord_transform
        .required_channels()
        .iter()
        .flat_map(|&ch| vec![ch.to_string(), format!("{}2", ch)])
        .collect();
    for ch in &positional_channel_set_temp {
        channels_with_scales.insert(ch.clone());
    }

    // Determine positional vs non-positional channels
    let positional_channel_set: HashSet<String> = coord_transform
        .required_channels()
        .iter()
        .flat_map(|&ch| vec![ch.to_string(), format!("{}2", ch)])
        .collect();

    let non_positional_channels: Vec<String> = channels_with_scales
        .iter()
        .filter(|ch| !positional_channel_set.contains(*ch))
        .cloned()
        .collect();

    let positional_channels: Vec<String> = channels_with_scales
        .iter()
        .filter(|ch| positional_channel_set.contains(*ch))
        .cloned()
        .collect();

    // PHASE 1: Build non-positional scale builders
    // We build non‑positional scales first so their configured scales can be used to construct radius‑aware positional expressions
    for channel in &non_positional_channels {
        if let Some((spec, dt, options, _has_explicit_domain, domain_opt)) =
            Box::pin(build_scale_for_channel(
                channel,
                prepared_marks,
                ctx,
                params,
                false,           // no radius for non-positional
                &HashMap::new(), // no phase1 scales yet
                scale_specs,
                coord_transform,
            ))
            .await?
        {
            // Cache the domain data under the BASE name (strip trailing numbers)
            // This ensures y2 channel's scale is stored under "y", matching lookup semantics
            let base_name = strip_trailing_numbers(channel);
            Box::pin(cache_domain_data(
                base_name,
                &spec,
                &dt,
                options,
                domain_opt,
                prepared_marks,
                ctx,
                params,
                &HashMap::new(), // no phase1 scales yet
                &mut builder,
                None, // no radius expression
                theme,
            ))
            .await?;
        }
    }

    // Build temporary ConfiguredScaleWithSpec objects from Phase 1 builders for use in Phase 2
    let mut phase1_configured: HashMap<String, ConfiguredScaleWithSpec> = HashMap::new();

    // Extract non‑positional channel builders from the main builder
    for (channel_name, channel_builder) in builder.channel_builders() {
        if let Some(configured) = Box::pin(build_temp_configured_scale(
            channel_builder,
            channel_name,
            400.0, // dummy width
            300.0, // dummy height
            ctx,
            params,
            theme,
        ))
        .await?
        {
            phase1_configured.insert(channel_name.to_string(), configured);
        }
    }

    // PHASE 2: Build positional scales with scale-aware radius expressions
    for channel in &positional_channels {
        if let Some((spec, dt, options, has_explicit_domain, domain_opt)) =
            Box::pin(build_scale_for_channel(
                channel,
                prepared_marks,
                ctx,
                params,
                true, // check radius for positional
                &phase1_configured,
                scale_specs,
                coord_transform,
            ))
            .await?
        {
            // Only apply radius expression if domain is NOT explicitly set by user
            // When user sets explicit domain, they want exact control over the range
            let radius_expr_opt = if has_explicit_domain {
                None
            } else {
                get_radius_expression(
                    channel,
                    &spec,
                    prepared_marks,
                    ctx,
                    &phase1_configured,
                    theme,
                )?
            };

            // Cache the domain data under the BASE name (strip trailing numbers)
            // This ensures y2 channel's scale is stored under "y", matching lookup semantics
            let base_name = strip_trailing_numbers(channel);
            Box::pin(cache_domain_data(
                base_name,
                &spec,
                &dt,
                options,
                domain_opt,
                prepared_marks,
                ctx,
                params,
                &phase1_configured,
                &mut builder,
                radius_expr_opt,
                theme,
            ))
            .await?;
        }
    }

    // Phase 2 builders are also in builder.channel_builders
    Ok(builder)
}

/// Build scale specification for a channel
async fn build_scale_for_channel<C>(
    channel: &str,
    prepared_marks: &[PreparedScaleMark],
    ctx: &SessionContext,
    _params: &IndexMap<String, ScalarValue>,
    _check_radius: bool,
    _phase1_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    // Plot-level scale overrides to apply before extracting options/domain
    plot_scale_specs: &HashMap<String, PlotScaleSpec>,
    coord_transform: &C,
) -> Result<
    Option<(
        Box<dyn ScaleSpec>,
        ArrowDataType,
        HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
        bool,
        Option<ScaleDomain>,
    )>,
    AvengerChartError,
>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    // Helper to check if a DataFrame is an EmptyRelation placeholder
    let is_empty_relation =
        |df: &DataFrame| -> bool { matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_)) };

    // Find first mark that uses this channel (or a channel mapping to it) and get its expr and preferred scale type.
    // For positional channels like "y", we also check "y2" since both map to the same scale.
    let mut chosen_spec: Option<Box<dyn ScaleSpec>> = None;
    let mut data_type: Option<ArrowDataType> = None;

    'outer: for prepared in prepared_marks {
        let mark = &prepared.mark;
        let channels = &prepared.channels;
        let resolved = resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

        // Look for any channel whose scale name matches the target channel
        // This allows y2's data to be used when building scale "y"
        for (channel_name, channel_value) in &resolved {
            // Check if this channel maps to our target scale
            let maps_to_target =
                if let Some(scale_name) = channel_value.get_scale_name(channel_name) {
                    scale_name == channel
                } else {
                    // Value channels don't have scales but exact name match counts
                    channel_name == channel
                };

            if !maps_to_target {
                continue;
            }

            // Use scale_input_expr for type inference - we only care about the type
            // of values that actually pass through the scale. For conditionals with
            // literal Value branches, those branches are replaced with NULL so they
            // don't affect type inference (e.g., a numeric scaled branch with a
            // string literal override should infer as numeric, not string).
            let maybe_dt = {
                if let Some(expr) = channel_value.scale_input_expr(ctx) {
                    let df_for_inference = prepared
                        .dataframe
                        .clone()
                        .filter(|df| !is_empty_relation(df));

                    if let Some(df) = df_for_inference {
                        match &expr {
                            Expr::Column(col) => {
                                let name = col.name.clone();
                                df.schema()
                                    .field_with_unqualified_name(&name)
                                    .ok()
                                    .map(|f| f.data_type().clone())
                            }
                            _ => {
                                if let Ok(projected) =
                                    df.clone().select(vec![expr.clone().alias("__t")])
                                {
                                    Some(projected.schema().field(0).data_type().clone())
                                } else {
                                    None
                                }
                            }
                        }
                    } else if expr.column_refs().is_empty() {
                        expr.get_type(&DFSchema::empty()).ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(dt) = maybe_dt {
                data_type = Some(dt.clone());
                chosen_spec = mark
                    .preferred_scale_type(channel_name, &dt)
                    .map(scale_spec_for_preference);
                break 'outer;
            }
        }
    }

    // Check for explicit scale config (type, domain, options like nice, zero, etc.)
    // Look for any channel whose scale maps to the target channel
    let mut chosen_scale_config: Option<Scale<Auto>> = None;
    'config_outer: for prepared in prepared_marks {
        let channels = &prepared.channels;
        let resolved = resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

        for (channel_name, channel_value) in &resolved {
            // Check if this channel maps to our target scale
            let maps_to_target =
                if let Some(scale_name) = channel_value.get_scale_name(channel_name) {
                    scale_name == channel
                } else {
                    channel_name == channel
                };

            if !maps_to_target {
                continue;
            }

            if let Some(scale_config) = channel_value.get_scale_config() {
                // Capture any scale config, not just those with explicit domains
                // This ensures options like nice(false) and zero(false) are preserved
                chosen_scale_config = Some(Scale::from_config(scale_config.clone()));
                break 'config_outer;
            }
        }
    }

    // If user provided an explicit scale type, prefer it over mark/data-type inference
    if let Some(user_scale) = &chosen_scale_config {
        if let Some(user_spec) = user_scale.get_scale_spec() {
            chosen_spec = Some(user_spec);
        } else if let Some(range) = user_scale.get_range() {
            // If no explicit scale type but discrete range is set, infer ordinal scale
            if matches!(range, ScaleRange::Discrete(_)) {
                chosen_spec = Some(Box::new(Ordinal));
            }
        }
    }

    let scale_spec = match chosen_spec {
        Some(spec) => spec,
        None => return Ok(None),
    };

    let dt = match data_type {
        Some(dt) => dt,
        None => return Ok(None),
    };

    // Build scale and collect options
    let mut scale = Scale::<Auto>::from_spec(scale_spec.clone());

    // Apply user's domain if present (before defaults)
    if let Some(channel_scale) = &chosen_scale_config
        && let Some(domain) = channel_scale.get_domain()
    {
        scale = scale.domain(domain.clone());
    }

    // Check if domain is explicitly set by user on the channel
    let mut has_explicit_domain = chosen_scale_config
        .as_ref()
        .and_then(|c| c.get_domain())
        .is_some();

    // Apply coordinate and mark defaults FIRST
    // When explicit domain is set, skip domain-affecting options (nice, zero, padding)
    // but KEEP rendering options (round) as they don't affect the domain
    // User options will be applied after to ensure they take precedence
    if let Ok(scale_impl) = scale.get_scale_impl_or_err() {
        let coord_opts = coord_transform.default_scale_options(channel, scale_impl.as_ref());
        for (k, v) in coord_opts {
            // Skip domain-affecting options if user set explicit domain
            if has_explicit_domain && (k == "nice" || k == "zero" || k == "padding") {
                continue;
            }
            scale = scale.option(&k, lit(v));
        }

        // Find mark that has a channel mapping to this scale
        if let Some(prepared) = prepared_marks.iter().find(|prepared| {
            prepared.channels.iter().any(|(ch_name, ch_val)| {
                if let Some(scale_name) = ch_val.get_scale_name(ch_name) {
                    scale_name == channel
                } else {
                    ch_name == channel
                }
            })
        }) {
            let mark_opts = prepared
                .mark
                .default_scale_options(channel, scale_impl.as_ref(), &dt);
            for (k, v) in mark_opts {
                // Skip domain-affecting options if user set explicit domain
                if has_explicit_domain && (k == "nice" || k == "zero" || k == "padding") {
                    continue;
                }
                scale = scale.option(&k, v);
            }
        }
    }

    // Apply user-specified options LAST (nice, zero, padding, etc.) from the channel
    // This ensures user options override defaults
    if let Some(channel_scale) = &chosen_scale_config {
        for (key, value_node) in channel_scale.get_options() {
            let expr = value_node.to_expr(ctx)?;
            scale = scale.option(key, expr);
        }
    }

    // Apply plot-level overrides AFTER mark defaults and channel options so they take precedence
    if let Some(plot_spec) = plot_scale_specs.get(channel) {
        let PlotScaleSpec::Local(scale_changes) = plot_spec;
        scale = scale.update(Scale::from_config(scale_changes.clone()));
        // If plot-level override set the domain (including DomainExprs), treat as explicit
        if scale.get_domain().is_some() {
            has_explicit_domain = true;
        }
    }

    let options = scale.get_options().clone();
    let domain_opt = scale.get_domain().cloned();

    Ok(Some((
        scale_spec,
        dt,
        options,
        has_explicit_domain,
        domain_opt,
    )))
}

/// Get radius expression for a positional channel (returns None for non-positional)
fn get_radius_expression(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    prepared_marks: &[PreparedScaleMark],
    ctx: &SessionContext,
    phase1_configured: &HashMap<String, ConfiguredScaleWithSpec>,
    theme: &Theme,
) -> Result<Option<PreparedRadiusExpression>, AvengerChartError> {
    // Check if scale supports radius expansion
    let scale_for_check = Scale::<Auto>::from_spec(spec.clone_box());
    let supports_radius = if let Ok(scale_impl) = scale_for_check.to_scale_impl() {
        scale_impl.supports_radius_expansion()
    } else {
        false
    };

    if !supports_radius {
        return Ok(None);
    }

    // Find a mark that uses this channel
    for prepared in prepared_marks {
        let mark = &prepared.mark;
        let channels = &prepared.channels;
        let resolved = resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

        // Check if this mark uses this channel
        for (ch, ch_value) in &resolved {
            if let Some(ch_scale_name) = ch_value.get_scale_name(ch) {
                let base_channel = strip_trailing_numbers(channel);

                if ch_scale_name == base_channel {
                    // Create scale-aware resolve_channel closure
                    let resolve_channel = |ch_name: &str| -> Expr {
                        if let Some(channel_value) = resolved.get(ch_name) {
                            match channel_value {
                                ChannelValue::Scaled {
                                    expr, scale_name, ..
                                } => {
                                    let scale_key =
                                        scale_name.as_ref().cloned().unwrap_or_else(|| {
                                            strip_trailing_numbers(ch_name).to_string()
                                        });

                                    // Check Phase 1 scales
                                    if let Some(configured) = phase1_configured.get(&scale_key)
                                        && let Ok(expr_df) = expr.to_expr(ctx)
                                    {
                                        return ConfiguredScaleDataFusionExt::to_expr(
                                            configured,
                                            expr_df.clone(),
                                        )
                                        .unwrap_or(expr_df);
                                    }

                                    // Fallback to raw expression
                                    if let Ok(expr_df) = expr.to_expr(ctx) {
                                        return expr_df;
                                    }
                                }
                                ChannelValue::Value { expr } => {
                                    if let Ok(expr_df) = expr.to_expr(ctx) {
                                        return expr_df;
                                    }
                                }
                                _ => {}
                            }
                        }

                        let temp_eval_ctx = CoreEvaluationContext::new(
                            Arc::new(theme.clone()),
                            Arc::new(ctx.clone()),
                            IndexMap::new(),
                        );
                        if let Some(default_scalar) =
                            default_channel_value_for_eval(mark.as_ref(), ch_name, &temp_eval_ctx)
                        {
                            return lit(default_scalar);
                        }

                        lit(0.0)
                    };

                    // Get the mark's radius formula using harmless placeholder
                    // columns, then substitute the actual channel expressions.
                    // This avoids serializing scale UDFs through mark crates
                    // that only know about the core/default expression codec.
                    let placeholder_channel = |ch_name: &str| -> Expr {
                        col(format!("{RADIUS_CHANNEL_PLACEHOLDER_PREFIX}{ch_name}"))
                    };
                    let Some(radius) = mark.radius_expression(channel, &placeholder_channel) else {
                        return Ok(None);
                    };
                    let prepared = prepared_radius_from_serialized(&radius, ctx)?;
                    return substitute_radius_expression_placeholders(prepared, &resolve_channel)
                        .map(Some);
                }
            }
        }
    }

    Ok(None)
}

/// Cache domain data for a channel in the ScaleBuilder
#[allow(clippy::too_many_arguments)]
async fn cache_domain_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    dt: &ArrowDataType,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    domain_opt: Option<ScaleDomain>,
    prepared_marks: &[PreparedScaleMark],
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    phase1_configured: &HashMap<String, ConfiguredScaleWithSpec>,
    builder: &mut ScaleBuilder,
    radius_expr_opt: Option<PreparedRadiusExpression>,
    theme: &Theme,
) -> Result<(), AvengerChartError> {
    // Build a scale with domain and options to inspect domain (including any DomainExprs)
    let mut scale = Scale::<Auto>::from_spec(spec.clone_box());

    // Apply domain if provided (this is the key fix - domain was previously lost)
    if let Some(domain) = &domain_opt {
        scale = scale.domain(domain.clone());
    }

    for (key, value_node) in &options {
        let expr = value_node.to_expr(ctx)?;
        scale = scale.option(key, expr);
    }

    // If the domain is set to an explicit interval/discrete, cache as explicit and return.
    // If it's DomainExprs, we will use those expressions to cache data extents.
    if let Some(domain) = scale.get_domain() {
        match &domain.default_domain {
            ScaleDefaultDomain::Interval(_, _) | ScaleDefaultDomain::Discrete(_) => {
                builder.add_explicit_domain(
                    channel.to_string(),
                    spec.clone_box(),
                    options,
                    domain.clone(),
                );
                builder.set_channel_data_type(channel.to_string(), dt.clone());
                return Ok(());
            }
            ScaleDefaultDomain::DomainExprs(_exprs) => {
                // Fall through: we will build entries from these expressions
            }
            ScaleDefaultDomain::NoDefault => {
                // Nothing to do; proceed to infer from mark channels
            }
        }
    }

    // Collect data expressions (and per-mark/per-override radius) for this scale
    let mut entries: Vec<(Arc<DataFrame>, Expr, Option<PreparedRadiusExpression>)> = Vec::new();

    // First, if the scale domain is DomainExprs from overrides, use those directly
    if let Some(domain) = scale.get_domain()
        && let ScaleDefaultDomain::DomainExprs(exprs) = &domain.default_domain
    {
        for DomainExpr {
            dataframe,
            expr,
            radius,
        } in exprs.iter()
        {
            let logical_plan = dataframe.to_logical_plan(ctx)?;
            let df = Arc::new(DataFrame::new(ctx.state().clone(), logical_plan));
            let expr_df = expr.to_expr(ctx)?;
            let prepared_radius = radius
                .as_ref()
                .map(|radius| prepared_radius_from_serialized(radius, ctx))
                .transpose()?;
            entries.push((df, expr_df, prepared_radius));
        }
    }

    // Helper to check if a DataFrame is an EmptyRelation placeholder
    let is_empty_relation =
        |df: &DataFrame| -> bool { matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_)) };

    // If overrides didn't provide DomainExprs, fall back to collecting from marks
    if entries.is_empty() {
        for prepared in prepared_marks {
            let mark = &prepared.mark;
            let df = if let Some(mark_df) = prepared
                .dataframe
                .clone()
                .filter(|df| !is_empty_relation(df))
            {
                Arc::new(mark_df)
            } else {
                continue;
            };

            let channels = &prepared.channels;
            let resolved =
                resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

            for (channel_name, channel_value) in &resolved {
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel_name)
                    && channel_scale_name == channel
                {
                    // Helper to push (df, expr_df, per_mark_radius)
                    let mut push_entry = |expr_df: Expr| -> Result<(), AvengerChartError> {
                        // Compute per-mark radius (if supported by this scale)
                        let per_mark_radius = if let Ok(scale_impl) =
                            Scale::<Auto>::from_spec(spec.clone_box()).to_scale_impl()
                        {
                            if scale_impl.supports_radius_expansion() {
                                // Build resolve_channel as in get_radius_expression
                                let resolve_channel = |ch_name: &str| -> Expr {
                                    if let Some(ch_val) = resolved.get(ch_name) {
                                        match ch_val {
                                            ChannelValue::Scaled {
                                                expr, scale_name, ..
                                            } => {
                                                let scale_key = scale_name
                                                    .as_ref()
                                                    .cloned()
                                                    .unwrap_or_else(|| {
                                                        strip_trailing_numbers(ch_name).to_string()
                                                    });
                                                if let Some(configured) =
                                                    phase1_configured.get(&scale_key)
                                                    && let Ok(expr_df2) = expr.to_expr(ctx)
                                                {
                                                    return ConfiguredScaleDataFusionExt::to_expr(
                                                        configured,
                                                        expr_df2.clone(),
                                                    )
                                                    .unwrap_or(expr_df2);
                                                }
                                                expr.to_expr(ctx).unwrap_or(lit(0.0))
                                            }
                                            ChannelValue::Value { expr } => {
                                                expr.to_expr(ctx).unwrap_or(lit(0.0))
                                            }
                                            _ => lit(0.0),
                                        }
                                    } else {
                                        let temp_eval_ctx = CoreEvaluationContext::new(
                                            Arc::new(theme.clone()),
                                            Arc::new(ctx.clone()),
                                            IndexMap::new(),
                                        );
                                        if let Some(default_scalar) = default_channel_value_for_eval(
                                            mark.as_ref(),
                                            ch_name,
                                            &temp_eval_ctx,
                                        ) {
                                            lit(default_scalar)
                                        } else {
                                            lit(0.0)
                                        }
                                    }
                                };
                                let placeholder_channel = |ch_name: &str| -> Expr {
                                    col(format!("{RADIUS_CHANNEL_PLACEHOLDER_PREFIX}{ch_name}"))
                                };
                                mark.radius_expression(channel, &placeholder_channel)
                                    .map(|radius| {
                                        let prepared =
                                            prepared_radius_from_serialized(&radius, ctx)?;
                                        substitute_radius_expression_placeholders(
                                            prepared,
                                            &resolve_channel,
                                        )
                                    })
                                    .transpose()?
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        entries.push((df.clone(), expr_df, per_mark_radius));
                        Ok(())
                    };

                    // Use scale_input_expr to get an expression for domain collection.
                    // For conditionals, this returns a CASE expression with NULL for
                    // literal Value branches (they bypass the scale and shouldn't
                    // affect domain computation like min/max or distinct values).
                    if let Some(expr_df) = channel_value.scale_input_expr(ctx) {
                        push_entry(expr_df)?;
                    }
                }
            }
        }
    }

    if entries.is_empty() {
        return Ok(());
    }

    // For non-radius paths, we still need position expressions only
    let data_expressions: Vec<(Arc<DataFrame>, Expr)> = entries
        .iter()
        .map(|(df, expr, _)| (df.clone(), expr.clone()))
        .collect();

    // Prefer the scale's declared domain kind when choosing inference path
    let target_domain_kind = spec.domain_kind();

    // Determine if any entry carries radius or caller provided one
    let has_any_radius = radius_expr_opt.is_some() || entries.iter().any(|(_, _, r)| r.is_some());

    // Cache domain data based on scale domain kind and radius requirement
    if has_any_radius {
        // Radius-aware scale - cache raw vectors
        Box::pin(cache_radius_aware_data(
            channel,
            spec,
            options,
            &entries,
            radius_expr_opt.as_ref(),
            ctx,
            params,
            builder,
            dt,
        ))
        .await?;
    } else if target_domain_kind == DomainKind::Categorical {
        Box::pin(cache_categorical_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        ))
        .await?;
    } else if target_domain_kind == DomainKind::Temporal {
        Box::pin(cache_temporal_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        ))
        .await?;
    } else {
        Box::pin(cache_numeric_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        ))
        .await?;
    }

    Ok(())
}

/// Build temporary ConfiguredScale used to construct radius‑aware positional expressions
async fn build_temp_configured_scale(
    channel_builder: &ChannelScaleData,
    channel_name: &str,
    width: f32,
    height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
) -> Result<Option<ConfiguredScaleWithSpec>, AvengerChartError> {
    match channel_builder {
        ChannelScaleData::Standard {
            scale_spec,
            data_extents,
            options,
        } => {
            let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

            // Apply cached options
            for (key, value_node) in options {
                let expr = value_node.to_expr(ctx)?;
                scale = scale.option(key, expr);
            }

            // Set domain from cached extents
            let domain = data_extents.to_scale_domain()?;
            scale = scale.domain(domain);

            // Set range from theme
            let range_kind = match scale.get_scale_impl() {
                Some(impl_) => impl_.range_kind(),
                None => return Ok(None), // Can't build temp scale if impl creation fails
            };
            let range = if let Some(theme_range) =
                theme.get_range_for_channel("mark", channel_name, range_kind, None, params)
            {
                theme_range
            } else {
                default_range_for_channel(channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;
            let configured =
                Box::pin(scale.create_configured_scale(width, height, ctx, params)).await?;

            // Wrap in ConfiguredScaleWithSpec
            Ok(Some(ConfiguredScaleWithSpec::new(scale, configured)))
        }
        ChannelScaleData::ExplicitDomain {
            scale_spec,
            options,
            domain,
            ..
        } => {
            // Build a temporary configured scale using the explicit domain and options
            let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

            // Apply the stored explicit domain FIRST
            scale = scale.domain(domain.clone());

            // Apply cached options
            for (key, value_node) in options {
                let expr = value_node.to_expr(ctx)?;
                scale = scale.option(key, expr);
            }

            // Set range from theme (mirrors Standard arm)
            let range_kind = match scale.get_scale_impl() {
                Some(impl_) => impl_.range_kind(),
                None => return Ok(None),
            };
            let range = if let Some(theme_range) =
                theme.get_range_for_channel("mark", channel_name, range_kind, None, params)
            {
                theme_range
            } else {
                default_range_for_channel(channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;
            let configured =
                Box::pin(scale.create_configured_scale(width, height, ctx, params)).await?;

            Ok(Some(ConfiguredScaleWithSpec::new(scale, configured)))
        }
        ChannelScaleData::RadiusAware { .. } => {
            // Radius‑aware scales are not expected here
            Ok(None)
        }
    }
}

// Helper functions for caching different types of data (implementations to follow)
async fn cache_radius_aware_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    entries: &[(Arc<DataFrame>, Expr, Option<PreparedRadiusExpression>)],
    global_radius_opt: Option<&PreparedRadiusExpression>,
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
) -> Result<(), AvengerChartError> {
    let mut all_positions = Vec::new();
    let mut all_radius_lower = Vec::new();
    let mut all_radius_upper = Vec::new();

    for (df, expr, mark_radius_opt) in entries {
        let select_exprs = if let Some(mark_radius) = mark_radius_opt.as_ref().or(global_radius_opt)
        {
            match mark_radius {
                PreparedRadiusExpression::Symmetric(radius_expr_df) => {
                    vec![
                        expr.clone().alias("__position__"),
                        radius_expr_df.clone().alias("__radius_lower__"),
                        radius_expr_df.clone().alias("__radius_upper__"),
                    ]
                }
                PreparedRadiusExpression::Asymmetric { lower, upper } => {
                    vec![
                        expr.clone().alias("__position__"),
                        lower.clone().alias("__radius_lower__"),
                        upper.clone().alias("__radius_upper__"),
                    ]
                }
            }
        } else {
            // No radius for this mark on this channel — use zeros
            vec![
                expr.clone().alias("__position__"),
                lit(0.0f32).alias("__radius_lower__"),
                lit(0.0f32).alias("__radius_upper__"),
            ]
        };

        let df_with_exprs = df.as_ref().clone().select(select_exprs)?;
        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                df_with_exprs
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                df_with_exprs.collect().await?
            }
        } else {
            df_with_exprs.collect().await?
        };

        if !batches.is_empty() && batches[0].num_rows() > 0 {
            let batch = &batches[0];
            let position_array = batch.column_by_name("__position__").ok_or_else(|| {
                AvengerChartError::InternalError("Position column not found".to_string())
            })?;
            let radius_lower_array = batch.column_by_name("__radius_lower__").ok_or_else(|| {
                AvengerChartError::InternalError("Radius lower column not found".to_string())
            })?;
            let radius_upper_array = batch.column_by_name("__radius_upper__").ok_or_else(|| {
                AvengerChartError::InternalError("Radius upper column not found".to_string())
            })?;
            trace!(
                pos_type = ?position_array.data_type(),
                lower_type = ?radius_lower_array.data_type(),
                upper_type = ?radius_upper_array.data_type(),
                "RadiusAware cast input types"
            );
            let position_f64 = cast(position_array, &ArrowDataType::Float64)?;
            let radius_lower_f64 = cast(radius_lower_array, &ArrowDataType::Float64)?;
            let radius_upper_f64 = cast(radius_upper_array, &ArrowDataType::Float64)?;
            trace!(
                pos_type = ?position_f64.data_type(),
                lower_type = ?radius_lower_f64.data_type(),
                upper_type = ?radius_upper_f64.data_type(),
                "RadiusAware cast output types"
            );
            let positions = position_f64.as_primitive::<Float64Type>();
            let radius_lower = radius_lower_f64.as_primitive::<Float64Type>();
            let radius_upper = radius_upper_f64.as_primitive::<Float64Type>();

            // Collect as vectors and normalize lengths to avoid padding errors
            let pos_vals: Vec<f64> = positions.iter().flatten().collect();
            let lower_vals: Vec<f64> = radius_lower.iter().flatten().collect();
            let upper_vals: Vec<f64> = radius_upper.iter().flatten().collect();
            let min_len = pos_vals.len().min(lower_vals.len()).min(upper_vals.len());
            if min_len > 0 {
                all_positions.extend(&pos_vals[..min_len]);
                all_radius_lower.extend(&lower_vals[..min_len]);
                all_radius_upper.extend(&upper_vals[..min_len]);
            }
        }
    }

    if !all_positions.is_empty() {
        builder.add_radius_aware(
            channel.to_string(),
            spec.clone_box(),
            all_positions,
            all_radius_lower,
            all_radius_upper,
            options,
        );
        builder.set_channel_data_type(channel.to_string(), dt.clone());
    }

    Ok(())
}

async fn cache_categorical_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
) -> Result<(), AvengerChartError> {
    let mut all_unique_values: Vec<ScalarValue> = Vec::new();

    for (df, expr) in data_expressions {
        let distinct_df = df
            .as_ref()
            .clone()
            .select(vec![expr.clone().alias("value")])?
            .distinct()?;

        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                distinct_df
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                distinct_df.collect().await?
            }
        } else {
            distinct_df.collect().await?
        };

        for batch in batches.iter() {
            let value_array = batch.column(0);
            for i in 0..value_array.len() {
                let scalar = ScalarValue::try_from_array(value_array, i)?;
                // Skip NULL values - these come from conditional literal branches
                // that use NULL placeholders and shouldn't affect the domain
                if scalar.is_null() {
                    continue;
                }
                if !all_unique_values.iter().any(|v| v == &scalar) {
                    all_unique_values.push(scalar);
                }
            }
        }
    }

    if !all_unique_values.is_empty() {
        all_unique_values.sort_by(scalar_total_cmp);

        let extents = DataExtents::Discrete(all_unique_values);
        builder.add_standard(channel.to_string(), spec.clone_box(), extents, options);

        // For ordinal scales with continuous range, store Float64 type
        let stored_type = if spec.range_kind() == RangeKind::Continuous {
            ArrowDataType::Float64
        } else {
            dt.clone()
        };
        builder.set_channel_data_type(channel.to_string(), stored_type);
    }

    Ok(())
}

async fn cache_temporal_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
) -> Result<(), AvengerChartError> {
    let mut global_min_ts: Option<i64> = None;
    let mut global_max_ts: Option<i64> = None;

    for (df, expr) in data_expressions {
        let agg_df = df.as_ref().clone().aggregate(
            vec![],
            vec![
                min(expr.clone()).alias("min"),
                max(expr.clone()).alias("max"),
            ],
        )?;

        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                agg_df.with_param_values(param_values)?.collect().await?
            } else {
                agg_df.collect().await?
            }
        } else {
            agg_df.collect().await?
        };

        if !batches.is_empty() && batches[0].num_rows() > 0 {
            let batch = &batches[0];
            let min_scalar = ScalarValue::try_from_array(batch.column(0), 0)?;
            let max_scalar = ScalarValue::try_from_array(batch.column(1), 0)?;

            let min_ts = match min_scalar {
                ScalarValue::TimestampNanosecond(Some(ts), _) => ts,
                ScalarValue::TimestampMicrosecond(Some(ts), _) => ts,
                ScalarValue::TimestampMillisecond(Some(ts), _) => ts,
                ScalarValue::TimestampSecond(Some(ts), _) => ts,
                ScalarValue::Date32(Some(days)) => days as i64 * 86400000,
                ScalarValue::Date64(Some(ms)) => ms,
                _ => continue,
            };

            let max_ts = match max_scalar {
                ScalarValue::TimestampNanosecond(Some(ts), _) => ts,
                ScalarValue::TimestampMicrosecond(Some(ts), _) => ts,
                ScalarValue::TimestampMillisecond(Some(ts), _) => ts,
                ScalarValue::TimestampSecond(Some(ts), _) => ts,
                ScalarValue::Date32(Some(days)) => days as i64 * 86400000,
                ScalarValue::Date64(Some(ms)) => ms,
                _ => continue,
            };

            global_min_ts = Some(global_min_ts.map_or(min_ts, |current| current.min(min_ts)));
            global_max_ts = Some(global_max_ts.map_or(max_ts, |current| current.max(max_ts)));
        }
    }

    if let (Some(min_ts), Some(max_ts)) = (global_min_ts, global_max_ts) {
        let extents = DataExtents::Temporal(min_ts, max_ts);
        builder.add_standard(channel.to_string(), spec.clone_box(), extents, options);
        builder.set_channel_data_type(channel.to_string(), dt.clone());
    }

    Ok(())
}

async fn cache_numeric_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
) -> Result<(), AvengerChartError> {
    let mut global_min_val: Option<f64> = None;
    let mut global_max_val: Option<f64> = None;

    for (df, expr) in data_expressions {
        let agg_df = df.as_ref().clone().aggregate(
            vec![],
            vec![
                min(expr.clone()).alias("min"),
                max(expr.clone()).alias("max"),
            ],
        )?;

        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                agg_df.with_param_values(param_values)?.collect().await?
            } else {
                agg_df.collect().await?
            }
        } else {
            agg_df.collect().await?
        };

        if !batches.is_empty() && batches[0].num_rows() > 0 {
            let batch = &batches[0];
            let min_col = batch.column(0);
            let max_col = batch.column(1);

            // Skip if min/max are NULL (all values were NULL from conditional literals)
            if min_col.is_null(0) || max_col.is_null(0) {
                continue;
            }

            let min_val = array_value_to_f64(min_col, 0, dt)?;
            let max_val = array_value_to_f64(max_col, 0, dt)?;

            global_min_val = Some(global_min_val.map_or(min_val, |current| current.min(min_val)));
            global_max_val = Some(global_max_val.map_or(max_val, |current| current.max(max_val)));
        }
    }

    if let (Some(min_val), Some(max_val)) = (global_min_val, global_max_val) {
        let extents = DataExtents::Interval(min_val, max_val);
        builder.add_standard(channel.to_string(), spec.clone_box(), extents, options);
        builder.set_channel_data_type(channel.to_string(), dt.clone());
    }

    Ok(())
}
