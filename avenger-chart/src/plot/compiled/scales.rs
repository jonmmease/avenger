//! Scale building and domain inference for CompiledPlot

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::channel::value::strip_trailing_numbers;
use crate::error::AvengerChartError;
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::scales::Scale;
use crate::serialization::LogicalPlanNodeExt;

use super::CompiledPlot;

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
pub(crate) async fn build_scale_builder_from_marks(
    compiled_marks: &[Arc<dyn crate::marks::CompiledMark>],
    scale_specs: &HashMap<String, crate::plot::ScaleSpec>,
    coord_transform: &Box<dyn crate::coords::CoordinateSystemTransform>,
    data: &Option<datafusion_proto::protobuf::LogicalPlanNode>,
    df_override: Option<DataFrame>,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    theme: &crate::theme::Theme,
) -> Result<crate::scales::ScaleBuilder, AvengerChartError> {
    use crate::channel::resolution::resolve_all_channel_refs;
    use crate::scales::builder::ScaleBuilder;

    let mut builder = ScaleBuilder::new();

    // Get DataFrame if available - mark-level data takes precedence over plot-level data
    // If df_override is provided, use that instead of plot-level data
    let df_opt = if let Some(df) = df_override {
        Some(df)
    } else {
        compiled_marks
            .iter()
            .find_map(|mark| mark.data_context().dataframe_with_context(ctx))
            .or_else(|| {
                // Fall back to plot-level data if no mark has data
                if let Some(data_node) = data {
                    use datafusion_proto::protobuf::LogicalPlanNode as ProtoNode;
                    let logical_plan = ProtoNode::to_logical_plan(data_node, ctx).ok()?;
                    Some(DataFrame::new(ctx.state().clone(), logical_plan))
                } else {
                    None
                }
            })
    };

    // 1. Collect channels needing scales from marks
    let mut channels_with_scales = HashSet::new();
    for mark in compiled_marks {
        let encodings = mark.data_context().channels();
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

    // 2. Determine positional vs non-positional channels
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

    // 3. PHASE 1: Build non-positional scale builders
    // We build non‑positional scales first so their configured scales can be used to construct radius‑aware positional expressions
    for channel in &non_positional_channels {
        if let Some((spec, dt, options, _has_explicit_domain, domain_opt)) =
            build_scale_for_channel(
                channel,
                compiled_marks,
                &df_opt,
                ctx,
                params,
                false,           // no radius for non-positional
                &HashMap::new(), // no phase1 scales yet
                scale_specs,
                coord_transform,
            )
            .await?
        {
            // Cache the domain data under the BASE name (strip trailing numbers)
            // This ensures y2 channel's scale is stored under "y", matching lookup semantics
            let base_name = strip_trailing_numbers(channel);
            cache_domain_data(
                base_name,
                &spec,
                &dt,
                options,
                domain_opt,
                compiled_marks,
                &df_opt,
                ctx,
                params,
                &HashMap::new(), // no phase1 scales yet
                &mut builder,
                None, // no radius expression
                theme,
            )
            .await?;
        }
    }

    // 4. Build temporary ConfiguredScaleWithSpec objects from Phase 1 builders for use in Phase 2
    let mut phase1_configured: HashMap<String, crate::scales::ConfiguredScaleWithSpec> =
        HashMap::new();

    // Extract non‑positional channel builders from the main builder
    for (channel_name, channel_builder) in &builder.channel_builders {
        if let Some(configured) = build_temp_configured_scale(
            channel_builder,
            channel_name,
            400.0, // dummy width
            300.0, // dummy height
            ctx,
            params,
            theme,
        )
        .await?
        {
            phase1_configured.insert(channel_name.clone(), configured);
        }
    }

    // 5. PHASE 2: Build positional scales with scale-aware radius expressions
    for channel in &positional_channels {
        if let Some((spec, dt, options, has_explicit_domain, domain_opt)) = build_scale_for_channel(
            channel,
            compiled_marks,
            &df_opt,
            ctx,
            params,
            true, // check radius for positional
            &phase1_configured,
            scale_specs,
            coord_transform,
        )
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
                    compiled_marks,
                    ctx,
                    &phase1_configured,
                    theme,
                )
            };

            // Cache the domain data under the BASE name (strip trailing numbers)
            // This ensures y2 channel's scale is stored under "y", matching lookup semantics
            let base_name = strip_trailing_numbers(channel);
            cache_domain_data(
                base_name,
                &spec,
                &dt,
                options,
                domain_opt,
                compiled_marks,
                &df_opt,
                ctx,
                params,
                &phase1_configured,
                &mut builder,
                radius_expr_opt,
                theme,
            )
            .await?;
        }
    }

    //     // Phase 2 builders are also in builder.channel_builders
    // All done!
    Ok(builder)
}

/// Build scale specification for a channel
async fn build_scale_for_channel(
    channel: &str,
    compiled_marks: &[Arc<dyn crate::marks::CompiledMark>],
    df_opt: &Option<DataFrame>,
    ctx: &SessionContext,
    _params: &IndexMap<String, datafusion::common::ScalarValue>,
    _check_radius: bool,
    _phase1_scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
    // Plot-level scale overrides to apply before extracting options/domain
    plot_scale_specs: &HashMap<String, crate::plot::ScaleSpec>,
    coord_transform: &Box<dyn crate::coords::CoordinateSystemTransform>,
) -> Result<
    Option<(
        Box<dyn crate::scales::ScaleSpec>,
        datafusion::arrow::datatypes::DataType,
        HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
        bool,
        Option<crate::scales::ScaleDomain>,
    )>,
    AvengerChartError,
> {
    use crate::channel::resolution::resolve_all_channel_refs;
    use crate::scales::spec::Auto;
    use datafusion::arrow::datatypes::DataType;
    use datafusion::logical_expr::Expr;

    // Helper to check if a DataFrame is an EmptyRelation placeholder
    let is_empty_relation = |df: &DataFrame| -> bool {
        matches!(
            df.logical_plan(),
            datafusion::logical_expr::LogicalPlan::EmptyRelation(_)
        )
    };

    // Find first mark that uses this channel (or a channel mapping to it) and get its expr and preferred scale type.
    // For positional channels like "y", we also check "y2" since both map to the same scale.
    let mut chosen_spec: Option<Box<dyn crate::scales::ScaleSpec>> = None;
    let mut data_type: Option<DataType> = None;

    'outer: for mark in compiled_marks {
        let channels = mark.data_context().channels();
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
                    // Try to infer type using the mark's own DataFrame first,
                    // then fall back to plot-level DataFrame if available.
                    let mark_df = mark.data_context().dataframe_with_context(ctx);
                    // Skip EmptyRelation placeholders and use plot-level DataFrame instead
                    let df_for_inference = mark_df
                        .filter(|df| !is_empty_relation(df))
                        .or_else(|| df_opt.clone());
                    let inferred_dt = if let Some(df) = df_for_inference {
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
                    } else {
                        None
                    };
                    inferred_dt
                } else {
                    None
                }
            };

            if let Some(dt) = maybe_dt {
                data_type = Some(dt.clone());
                chosen_spec = mark.preferred_scale_type(channel_name, &dt);
                break 'outer;
            }
        }
    }

    // Check for explicit scale config (type, domain, options like nice, zero, etc.)
    // Look for any channel whose scale maps to the target channel
    let mut chosen_scale_config: Option<Scale<Auto>> = None;
    'config_outer: for mark in compiled_marks {
        let channels = mark.data_context().channels();
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
                chosen_scale_config = Some(scale_config.clone());
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
            use crate::scales::ScaleRange;
            if matches!(range, ScaleRange::Discrete(_)) {
                use crate::scales::spec::Ordinal;
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
    if let Some(channel_scale) = &chosen_scale_config {
        if let Some(domain) = channel_scale.get_domain() {
            scale = scale.domain(domain.clone());
        }
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
        use datafusion::logical_expr::lit;
        let coord_opts = coord_transform.default_scale_options(channel, scale_impl.as_ref());
        for (k, v) in coord_opts {
            // Skip domain-affecting options if user set explicit domain
            if has_explicit_domain && (k == "nice" || k == "zero" || k == "padding") {
                continue;
            }
            scale = scale.option(&k, lit(v));
        }

        // Find mark that has a channel mapping to this scale
        if let Some(mark) = compiled_marks.iter().find(|m| {
            let channels = m.data_context().channels();
            channels.iter().any(|(ch_name, ch_val)| {
                if let Some(scale_name) = ch_val.get_scale_name(ch_name) {
                    scale_name == channel
                } else {
                    ch_name == channel
                }
            })
        }) {
            let mark_opts = mark.default_scale_options(channel, scale_impl.as_ref(), &dt);
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
        use crate::serialization::LogicalExprNodeExt;
        for (key, value_node) in channel_scale.get_options() {
            let expr = value_node.to_expr(ctx)?;
            scale = scale.option(key, expr);
        }
    }

    // Apply plot-level overrides AFTER mark defaults and channel options so they take precedence
    if let Some(plot_spec) = plot_scale_specs.get(channel) {
        let crate::plot::ScaleSpec::Local(scale_changes) = plot_spec;
        scale = scale.update(scale_changes.clone());
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
    spec: &Box<dyn crate::scales::ScaleSpec>,
    compiled_marks: &[Arc<dyn crate::marks::CompiledMark>],
    ctx: &SessionContext,
    phase1_configured: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
    theme: &crate::theme::Theme,
) -> Option<crate::marks::RadiusExpression> {
    use crate::channel::resolution::resolve_all_channel_refs;
    use crate::scales::{Scale, spec::Auto};
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::logical_expr::{Expr, lit};

    // Check if scale supports radius expansion
    let scale_for_check = Scale::<Auto>::from_spec(spec.clone_box());
    let supports_radius = if let Ok(scale_impl) = scale_for_check.to_scale_impl() {
        scale_impl.supports_radius_expansion()
    } else {
        false
    };

    if !supports_radius {
        return None;
    }

    // Find a mark that uses this channel
    for mark in compiled_marks {
        let channels = mark.data_context().channels();
        let resolved = resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

        // Check if this mark uses this channel
        for (ch, ch_value) in &resolved {
            if let Some(ch_scale_name) = ch_value.get_scale_name(ch) {
                use crate::channel::value::strip_trailing_numbers;
                use crate::scales::ConfiguredScaleDataFusionExt;
                let base_channel = strip_trailing_numbers(channel);

                if ch_scale_name == base_channel {
                    // Create scale-aware resolve_channel closure
                    let resolve_channel = |ch_name: &str| -> Expr {
                        if let Some(channel_value) = resolved.get(ch_name) {
                            match channel_value {
                                crate::marks::ChannelValue::Scaled {
                                    expr, scale_name, ..
                                } => {
                                    let scale_key =
                                        scale_name.as_ref().cloned().unwrap_or_else(|| {
                                            strip_trailing_numbers(ch_name).to_string()
                                        });

                                    // Check Phase 1 scales
                                    if let Some(configured) = phase1_configured.get(&scale_key) {
                                        if let Ok(expr_df) = expr.to_expr(ctx) {
                                            return ConfiguredScaleDataFusionExt::to_expr(
                                                configured,
                                                expr_df.clone(),
                                            )
                                            .unwrap_or(expr_df);
                                        }
                                    }

                                    // Fallback to raw expression
                                    if let Ok(expr_df) = expr.to_expr(ctx) {
                                        return expr_df;
                                    }
                                }
                                crate::marks::ChannelValue::Value { expr } => {
                                    if let Ok(expr_df) = expr.to_expr(ctx) {
                                        return expr_df;
                                    }
                                }
                                _ => {}
                            }
                        }

                        // Channel not found in mark - use default value from mark
                        // Create a minimal RenderContext for querying defaults
                        let temp_eval_ctx = crate::render::EvaluationContext::new(
                            Arc::new(theme.clone()),
                            Arc::new(ctx.clone()),
                            indexmap::IndexMap::new(),
                            Arc::new(
                                crate::facet::evaluated_facet_tree::EvaluatedFacetTree::empty(),
                            ),
                        );
                        let temp_state = crate::render::RenderState::new(
                            400.0, // dummy width
                            300.0, // dummy height
                            HashMap::new(),
                        );
                        let temp_ctx =
                            crate::render::RenderContext::new(&temp_eval_ctx, &temp_state, None);

                        if let Some(default_scalar) = mark.default_channel_value(ch_name, &temp_ctx)
                        {
                            return lit(default_scalar);
                        }

                        lit(0.0)
                    };

                    // Get radius expression from mark
                    return mark.radius_expression(channel, &resolve_channel);
                }
            }
        }
    }

    None
}

/// Cache domain data for a channel in the ScaleBuilder
#[allow(clippy::too_many_arguments)]
async fn cache_domain_data(
    channel: &str,
    spec: &Box<dyn crate::scales::ScaleSpec>,
    dt: &datafusion::arrow::datatypes::DataType,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    domain_opt: Option<crate::scales::ScaleDomain>,
    compiled_marks: &[Arc<dyn crate::marks::CompiledMark>],
    df_opt: &Option<DataFrame>,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    phase1_configured: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
    builder: &mut crate::scales::ScaleBuilder,
    radius_expr_opt: Option<crate::marks::RadiusExpression>,
    theme: &crate::theme::Theme,
) -> Result<(), AvengerChartError> {
    use crate::channel::resolution::resolve_all_channel_refs;
    use crate::scales::{Scale, spec::Auto};
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::logical_expr::Expr;

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
        use crate::scales::ScaleDefaultDomain;
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
    let mut entries: Vec<(Arc<DataFrame>, Expr, Option<crate::marks::RadiusExpression>)> =
        Vec::new();

    // First, if the scale domain is DomainExprs from overrides, use those directly
    if let Some(domain) = scale.get_domain() {
        use crate::scales::{DomainExpr, ScaleDefaultDomain};
        use crate::serialization::LogicalPlanNodeExt;
        if let ScaleDefaultDomain::DomainExprs(exprs) = &domain.default_domain {
            for DomainExpr {
                dataframe,
                expr,
                radius,
            } in exprs.iter()
            {
                let logical_plan = dataframe.to_logical_plan(ctx)?;
                let df = Arc::new(DataFrame::new(ctx.state().clone(), logical_plan));
                let expr_df = expr.to_expr(ctx)?;
                entries.push((df, expr_df, radius.clone()));
            }
        }
    }

    // Helper to check if a DataFrame is an EmptyRelation placeholder
    let is_empty_relation = |df: &DataFrame| -> bool {
        matches!(
            df.logical_plan(),
            datafusion::logical_expr::LogicalPlan::EmptyRelation(_)
        )
    };

    // If overrides didn't provide DomainExprs, fall back to collecting from marks
    if entries.is_empty() {
        for mark in compiled_marks {
            let mark_df = mark.data_context().dataframe_with_context(ctx);
            // Skip EmptyRelation placeholders and use plot-level DataFrame instead
            let df = if let Some(mark_df) = mark_df.filter(|df| !is_empty_relation(df)) {
                Arc::new(mark_df)
            } else if let Some(plot_df) = df_opt.as_ref() {
                Arc::new(plot_df.clone())
            } else {
                continue;
            };

            let channels = mark.data_context().channels();
            let resolved =
                resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());

            for (channel_name, channel_value) in &resolved {
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel_name) {
                    if channel_scale_name == channel {
                        // Helper to push (df, expr_df, per_mark_radius)
                        let mut push_entry = |expr_df: Expr| {
                            // Compute per-mark radius (if supported by this scale)
                            let per_mark_radius = if let Ok(scale_impl) =
                                Scale::<Auto>::from_spec(spec.clone_box()).to_scale_impl()
                            {
                                if scale_impl.supports_radius_expansion() {
                                    // Build resolve_channel as in get_radius_expression
                                    use crate::channel::value::strip_trailing_numbers;
                                    use crate::scales::ConfiguredScaleDataFusionExt;
                                    use datafusion::logical_expr::lit;
                                    let resolve_channel = |ch_name: &str| -> Expr {
                                        if let Some(ch_val) = resolved.get(ch_name) {
                                            match ch_val {
                                                crate::marks::ChannelValue::Scaled {
                                                    expr,
                                                    scale_name,
                                                    ..
                                                } => {
                                                    let scale_key = scale_name
                                                        .as_ref()
                                                        .cloned()
                                                        .unwrap_or_else(|| {
                                                            strip_trailing_numbers(ch_name)
                                                                .to_string()
                                                        });
                                                    if let Some(configured) =
                                                        phase1_configured.get(&scale_key)
                                                    {
                                                        if let Ok(expr_df2) = expr.to_expr(ctx) {
                                                            return ConfiguredScaleDataFusionExt::to_expr(configured, expr_df2.clone()).unwrap_or(expr_df2);
                                                        }
                                                    }
                                                    expr.to_expr(ctx).unwrap_or(lit(0.0))
                                                }
                                                crate::marks::ChannelValue::Value { expr } => {
                                                    expr.to_expr(ctx).unwrap_or(lit(0.0))
                                                }
                                                _ => lit(0.0),
                                            }
                                        } else {
                                            // Default or zero
                                            let temp_eval_ctx = crate::render::EvaluationContext::new(
                                                Arc::new(theme.clone()),
                                                Arc::new(ctx.clone()),
                                                indexmap::IndexMap::new(),
                                                Arc::new(crate::facet::evaluated_facet_tree::EvaluatedFacetTree::empty()),
                                            );
                                            let temp_state = crate::render::RenderState::new(
                                                400.0, 300.0, HashMap::new(),
                                            );
                                            let temp_ctx = crate::render::RenderContext::new(
                                                &temp_eval_ctx, &temp_state, None,
                                            );
                                            if let Some(default_scalar) =
                                                mark.default_channel_value(ch_name, &temp_ctx)
                                            {
                                                lit(default_scalar)
                                            } else {
                                                lit(0.0)
                                            }
                                        }
                                    };
                                    mark.radius_expression(channel, &resolve_channel)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            entries.push((df.clone(), expr_df, per_mark_radius));
                        };

                        // Use scale_input_expr to get an expression for domain collection.
                        // For conditionals, this returns a CASE expression with NULL for
                        // literal Value branches (they bypass the scale and shouldn't
                        // affect domain computation like min/max or distinct values).
                        if let Some(expr_df) = channel_value.scale_input_expr(ctx) {
                            push_entry(expr_df);
                        }
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
        cache_radius_aware_data(channel, spec, options, &entries, ctx, params, builder, dt).await?;
    } else if target_domain_kind == avenger_scales::scales::DomainKind::Categorical {
        cache_categorical_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        )
        .await?;
    } else if target_domain_kind == avenger_scales::scales::DomainKind::Temporal {
        cache_temporal_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        )
        .await?;
    } else {
        cache_numeric_data(
            channel,
            spec,
            options,
            &data_expressions,
            ctx,
            params,
            builder,
            dt,
        )
        .await?;
    }

    Ok(())
}

/// Build temporary ConfiguredScale used to construct radius‑aware positional expressions
async fn build_temp_configured_scale(
    channel_builder: &crate::scales::ChannelScaleBuilder,
    channel_name: &str,
    width: f32,
    height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    theme: &crate::theme::Theme,
) -> Result<Option<crate::scales::ConfiguredScaleWithSpec>, AvengerChartError> {
    use crate::scales::{ConfiguredScaleWithSpec, Scale, builder::ChannelScaleBuilder, spec::Auto};
    use crate::serialization::LogicalExprNodeExt;

    // Debug logging removed

    match channel_builder {
        ChannelScaleBuilder::Standard {
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
                crate::scales::default_range_for_channel(channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = scale.normalize_domain(width, height, ctx, params).await?;
            let configured = scale
                .create_configured_scale(width, height, ctx, params)
                .await?;

            // Wrap in ConfiguredScaleWithSpec
            Ok(Some(ConfiguredScaleWithSpec::new(scale, configured)))
        }
        ChannelScaleBuilder::ExplicitDomain {
            scale_spec,
            options,
            domain,
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
                crate::scales::default_range_for_channel(channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = scale.normalize_domain(width, height, ctx, params).await?;
            let configured = scale
                .create_configured_scale(width, height, ctx, params)
                .await?;

            Ok(Some(ConfiguredScaleWithSpec::new(scale, configured)))
        }
        ChannelScaleBuilder::RadiusAware { .. } => {
            // Radius‑aware scales are not expected here
            Ok(None)
        }
    }
}

// Helper functions for caching different types of data (implementations to follow)
async fn cache_radius_aware_data(
    channel: &str,
    spec: &Box<dyn crate::scales::ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    entries: &[(
        Arc<DataFrame>,
        datafusion::logical_expr::Expr,
        Option<crate::marks::RadiusExpression>,
    )],
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    builder: &mut crate::scales::ScaleBuilder,
    dt: &datafusion::arrow::datatypes::DataType,
) -> Result<(), AvengerChartError> {
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::arrow::array::AsArray;
    use datafusion::arrow::compute::cast;
    use datafusion::arrow::datatypes::DataType as ArrowDataType;
    use datafusion::arrow::datatypes::Float64Type;

    let mut all_positions = Vec::new();
    let mut all_radius_lower = Vec::new();
    let mut all_radius_upper = Vec::new();

    for (df, expr, mark_radius_opt) in entries {
        use datafusion::logical_expr::lit;
        let select_exprs = if let Some(mark_radius) = mark_radius_opt {
            match mark_radius {
                crate::marks::RadiusExpression::Symmetric(radius_node) => {
                    let radius_expr_df = radius_node.to_expr(ctx)?;
                    vec![
                        expr.clone().alias("__position__"),
                        radius_expr_df.clone().alias("__radius_lower__"),
                        radius_expr_df.alias("__radius_upper__"),
                    ]
                }
                crate::marks::RadiusExpression::Asymmetric { lower, upper } => {
                    let lower_expr_df = lower.to_expr(ctx)?;
                    let upper_expr_df = upper.to_expr(ctx)?;
                    vec![
                        expr.clone().alias("__position__"),
                        lower_expr_df.alias("__radius_lower__"),
                        upper_expr_df.alias("__radius_upper__"),
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
            if let Some(param_values) = crate::utils::params_to_datafusion(params) {
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
            if std::env::var("AVENGER_DEBUG_CAST").is_ok() {
                eprintln!(
                    "RadiusAware cast types: pos={:?}, lower={:?}, upper={:?}",
                    position_array.data_type(),
                    radius_lower_array.data_type(),
                    radius_upper_array.data_type()
                );
            }
            let position_f64 = cast(position_array, &ArrowDataType::Float64)?;
            let radius_lower_f64 = cast(radius_lower_array, &ArrowDataType::Float64)?;
            let radius_upper_f64 = cast(radius_upper_array, &ArrowDataType::Float64)?;
            if std::env::var("AVENGER_DEBUG_CAST").is_ok() {
                eprintln!(
                    "After cast types: pos={:?}, lower={:?}, upper={:?}",
                    position_f64.data_type(),
                    radius_lower_f64.data_type(),
                    radius_upper_f64.data_type()
                );
            }
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
    spec: &Box<dyn crate::scales::ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, datafusion::logical_expr::Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    builder: &mut crate::scales::ScaleBuilder,
    dt: &datafusion::arrow::datatypes::DataType,
) -> Result<(), AvengerChartError> {
    use crate::scales::builder::DataExtents;
    use datafusion::common::ScalarValue;

    let mut all_unique_values: Vec<ScalarValue> = Vec::new();

    for (df, expr) in data_expressions {
        let distinct_df = df
            .as_ref()
            .clone()
            .select(vec![expr.clone().alias("value")])?
            .distinct()?;

        let batches = if !params.is_empty() {
            if let Some(param_values) = crate::utils::params_to_datafusion(params) {
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
        let stored_type = if spec.range_kind() == avenger_scales::scales::RangeKind::Continuous {
            datafusion::arrow::datatypes::DataType::Float64
        } else {
            dt.clone()
        };
        builder.set_channel_data_type(channel.to_string(), stored_type);
    }

    Ok(())
}

async fn cache_temporal_data(
    channel: &str,
    spec: &Box<dyn crate::scales::ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, datafusion::logical_expr::Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    builder: &mut crate::scales::ScaleBuilder,
    dt: &datafusion::arrow::datatypes::DataType,
) -> Result<(), AvengerChartError> {
    use crate::scales::builder::DataExtents;
    use datafusion::common::ScalarValue;
    use datafusion::functions_aggregate::min_max::{max, min};

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
            if let Some(param_values) = crate::utils::params_to_datafusion(params) {
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
    spec: &Box<dyn crate::scales::ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, datafusion::logical_expr::Expr)],
    _ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    builder: &mut crate::scales::ScaleBuilder,
    dt: &datafusion::arrow::datatypes::DataType,
) -> Result<(), AvengerChartError> {
    use crate::scales::builder::DataExtents;
    use datafusion::functions_aggregate::min_max::{max, min};

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
            if let Some(param_values) = crate::utils::params_to_datafusion(params) {
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

            let min_val = crate::utils::array_value_to_f64(min_col, 0, dt)?;
            let max_val = crate::utils::array_value_to_f64(max_col, 0, dt)?;

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

impl CompiledPlot {
    /// Collect all channels that need scales from marks
    pub(crate) fn collect_channels_needing_scales(&self, ctx: &SessionContext) -> HashSet<String> {
        use crate::channel::resolution::resolve_all_channel_refs;

        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            // Try to resolve, but use original channels if resolution fails
            let resolved_encodings =
                resolve_all_channel_refs(encodings, ctx).unwrap_or_else(|_| encodings.clone());
            for (channel_name, channel_value) in resolved_encodings {
                if channel_value.get_scale_name(&channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }

    /// Check if a data type is numeric
    pub(super) fn is_numeric_type(dtype: &datafusion::arrow::datatypes::DataType) -> bool {
        use datafusion::arrow::datatypes::DataType;
        match dtype {
            // Standard numeric types
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64 => true,
            // Dictionary types are allowed if their value type is numeric
            // (This happens when categorical data goes through an ordinal scale)
            DataType::Dictionary(_, value_type) => Self::is_numeric_type(value_type),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_scale_builder_from_marks;
    use crate::prelude::*;
    use crate::render::RenderContext;
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::sync::Arc;

    async fn two_phase_build_scales(
        compiled: &super::CompiledPlot,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        super::AvengerChartError,
    > {
        use crate::channel::value::strip_trailing_numbers;
        use std::collections::HashMap;

        let builder = build_scale_builder_from_marks(
            &compiled.marks,
            &compiled.scale_specs,
            &compiled.coord_transform,
            &compiled.data,
            None,
            ctx,
            params,
            compiled.get_theme().as_ref(),
        )
        .await?;

        let mut coord_system_ranges = HashMap::new();
        for ch in builder.channel_builders().keys() {
            let base = strip_trailing_numbers(ch);
            if let Some((min, max)) =
                compiled
                    .coord_transform
                    .default_range(base, width as f64, height as f64)
            {
                coord_system_ranges.insert(ch.clone(), (min, max));
            }
        }

        let theme = compiled.get_theme();
        builder
            .build_scales(
                width,
                height,
                &coord_system_ranges,
                &compiled.scale_specs,
                &compiled.marks,
                theme.as_ref(),
                ctx,
                params,
            )
            .await
    }

    #[tokio::test]
    async fn symbol_constant_size_expands_domain() {
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));

        let x_values = Float64Array::from(vec![0.0, 2.0, 10.0]);
        let y_values = Float64Array::from(vec![1.0, 3.0, 5.0]);

        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
        let df = ctx.read_batch(batch).unwrap();

        let plot = Plot::<Cartesian>::new().data(df).mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(400.0)
                .fill("#4682b4"),
        );

        let compiled = plot.compile(&ctx).await.expect("compile plot");

        let params = IndexMap::new();
        let positional_scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &params)
            .await
            .expect("build scales two-phase");

        let (domain_min, domain_max) = positional_scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");

        assert!(
            domain_min < 0.0,
            "domain_min should be less than data minimum (0.0), got {}",
            domain_min
        );
        assert!(
            domain_max > 10.0,
            "domain_max should be greater than data maximum (10.0), got {}",
            domain_max
        );

        let (y_min, y_max) = positional_scales
            .get("y")
            .expect("y scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");

        assert!(
            y_min < 1.0,
            "y_min should be less than data minimum (1.0), got {}",
            y_min
        );
        assert!(
            y_max > 5.0,
            "y_max should be greater than data maximum (5.0), got {}",
            y_max
        );
    }

    #[tokio::test]
    async fn legend_titles_radius_padding_matches_data() {
        use crate::utils::ScalarValueHelpers;
        use datafusion::arrow::array::Float64Array;
        use datafusion::prelude::*;

        let ctx = SessionContext::new();
        let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
        let df = ctx
            .read_parquet(iris_path, ParquetReadOptions::default())
            .await
            .expect("load iris dataset");

        let projected = df
            .clone()
            .select(vec![col("sepal_length"), col("sepal_width")])
            .expect("project columns")
            .collect()
            .await
            .expect("collect samples");

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for batch in &projected {
            let x_array = batch
                .column(0)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("x array");
            let y_array = batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("y array");

            for value in x_array.iter().flatten() {
                min_x = min_x.min(value);
                max_x = max_x.max(value);
            }
            for value in y_array.iter().flatten() {
                min_y = min_y.min(value);
                max_y = max_y.max(value);
            }
        }

        let min_x = min_x as f32;
        let max_x = max_x as f32;
        let min_y = min_y as f32;
        let max_y = max_y as f32;

        let plot_df = df;

        let plot = Plot::<Cartesian>::new()
            .data(plot_df)
            .title("Custom Legend Titles")
            .mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(150.0)
                    .fill_with(col("species"), |c| {
                        c.scale_with::<Ordinal>(|s| s)
                            .legend(|l| l.title("Iris Species"))
                    }),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");

        let params = IndexMap::new();
        let final_scales = two_phase_build_scales(&compiled, 220.0, 300.0, &ctx, &params)
            .await
            .expect("build scales two-phase");

        let (x_domain_min, x_domain_max) = final_scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");
        let y_scale = final_scales.get("y").expect("y scale").configured();
        let (y_domain_min, y_domain_max) = y_scale.numeric_interval_domain().expect("numeric");

        assert!(
            x_domain_min < min_x,
            "x domain minimum ({x_domain_min}) should be less than data minimum ({min_x})"
        );
        assert!(
            x_domain_max > max_x,
            "x domain maximum ({x_domain_max}) should be greater than data maximum ({max_x})"
        );
        assert!(
            y_domain_min < min_y,
            "y domain minimum ({y_domain_min}) should be less than data minimum ({min_y})"
        );
        assert!(
            y_domain_max > max_y,
            "y domain maximum ({y_domain_max}) should be greater than data maximum ({max_y})"
        );

        let theme = compiled.get_theme();
        let eval_ctx = crate::render::EvaluationContext::new(
            theme.clone(),
            Arc::new(ctx.clone()),
            IndexMap::new(),
            Arc::new(crate::facet::evaluated_facet_tree::EvaluatedFacetTree::empty()),
        );
        let render_state =
            crate::render::RenderState::new(220.0, 300.0, std::collections::HashMap::new());
        let final_context = RenderContext::new(&eval_ctx, &render_state, None);
        let first_mark = compiled.marks().first().expect("compiled mark");
        let stroke_width = first_mark
            .default_channel_value("stroke_width", &final_context)
            .and_then(|scalar| scalar.as_f32().ok())
            .unwrap_or(1.0);
        let radius_px = 150.0_f32.sqrt() * 0.5 + stroke_width / 2.0;

        let x_scale_span = x_domain_max - x_domain_min;
        let y_scale_span = y_domain_max - y_domain_min;
        let padding_left = (min_x - x_domain_min) * final_context.plot_width() / x_scale_span;
        let padding_right = (x_domain_max - max_x) * final_context.plot_width() / x_scale_span;
        let padding_bottom = (min_y - y_domain_min) * final_context.plot_height() / y_scale_span;
        let padding_top = (y_domain_max - max_y) * final_context.plot_height() / y_scale_span;

        let tolerance = 0.5;

        assert!(
            padding_left + tolerance >= radius_px,
            "x left padding {padding_left} smaller than radius {radius_px}"
        );
        assert!(
            padding_right + tolerance >= radius_px,
            "x right padding {padding_right} smaller than radius {radius_px}"
        );
        assert!(
            padding_bottom + tolerance >= radius_px,
            "y bottom padding {padding_bottom} smaller than radius {radius_px}"
        );
        assert!(
            padding_top + tolerance >= radius_px,
            "y top padding {padding_top} smaller than radius {radius_px}"
        );
    }
}
