//! Scale building and domain inference from compiled mark channels.

#![allow(clippy::borrowed_box, clippy::too_many_arguments)]

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_format_number::NumberLocaleRegistry;
use avenger_scales::{
    formatter::{DefaultFormatter, Formatters},
    scales::{ConfiguredScale, DomainKind, RangeKind},
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, StructArray},
        compute::cast,
        datatypes::{DataType as ArrowDataType, Field, Float64Type},
    },
    common::{
        DFSchema, ScalarValue,
        tree_node::{Transformed, TreeNodeRecursion},
    },
    dataframe::DataFrame,
    functions_aggregate::min_max::{max, min},
    logical_expr::{Expr, ExprSchemable, LogicalPlan, lit},
    prelude::{SessionContext, col, get_field},
};
use datafusion_common::tree_node::{TransformedResult, TreeNode};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use tracing::trace;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledMark, CoordinateSystemTransformCore, DerivedScalarMap,
    EvaluationContext as CoreEvaluationContext, MarkDataMode, MarkScaleDomainSource, Maybe,
    NestScope, NestedBandSpec, RadiusExpression, ScalarValueHelpers, ScaleInferenceHint,
    ScaleOrderingSpec, ScaleRange, ScaleTypePreference, Theme, TimeContext, array_value_to_f64,
    collect_derived_scalar_ids, contains_aggregate, default_channel_value_for_eval,
    default_scale_type_for_data_type, eval_to_scalars, params_to_datafusion,
    resolve_all_channel_refs, resolve_derived_scalars, scalar_total_cmp, strip_trailing_numbers,
};

use crate::{
    Auto, ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec, DomainExpr, Ordinal,
    PlotScaleSpec, Scale, ScaleDefaultDomain, ScaleDomain, ScaleRuntimeExt, ScaleSpec,
    builder::resolve_scale_domain_exprs,
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
    pub domain_dataframe: Option<DataFrame>,
    pub domain_channels: IndexMap<String, ChannelValue>,
    pub extra_domain_sources: Vec<MarkScaleDomainSource>,
    pub derived_scalars: DerivedScalarMap,
    pub scale_inference_hints: Vec<ScaleInferenceHint>,
}

#[derive(Clone)]
enum PreparedRadiusExpression {
    Symmetric(Expr),
    Asymmetric { lower: Expr, upper: Expr },
}

const RADIUS_CHANNEL_PLACEHOLDER_PREFIX: &str = "__avenger_radius_channel__";
const SCALE_ORDER_CATEGORY_COL: &str = "__avenger_scale_order_category__";
const SCALE_ORDER_VALUE_COL: &str = "__avenger_scale_order_value__";
const SCALE_ORDER_COLUMN_PREFIX: &str = "__avenger_scale_order_col_";
const NESTED_ORDER_PREFIX_COL_PREFIX: &str = "__avenger_nested_order_prefix_";
const NESTED_ORDER_COMPONENT_COL: &str = "__avenger_nested_order_component__";
const NESTED_ORDER_VALUE_COL: &str = "__avenger_nested_order_value__";
const NESTED_LABEL_PREFIX_COL_PREFIX: &str = "__avenger_nested_label_prefix_";
const NESTED_LABEL_COMPONENT_COL: &str = "__avenger_nested_label_component__";
const NESTED_LABEL_VALUE_COL: &str = "__avenger_nested_label_value__";

fn is_empty_relation(df: &DataFrame) -> bool {
    matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_))
}

fn mark_scale_domain_sources(
    prepared: &PreparedScaleMark,
    ctx: &SessionContext,
) -> Result<Vec<MarkScaleDomainSource>, AvengerChartError> {
    let mut sources = prepared
        .mark
        .scale_domain_sources(prepared.domain_dataframe.as_ref(), ctx)?;
    sources.extend(prepared.extra_domain_sources.clone());
    Ok(sources)
}

fn infer_expr_data_type(
    dataframe: Option<&DataFrame>,
    expr: &Expr,
) -> Result<Option<ArrowDataType>, AvengerChartError> {
    if let Some(df) = dataframe.filter(|df| !is_empty_relation(df)) {
        return match expr {
            Expr::Column(col) => {
                let name = col.name.clone();
                Ok(df
                    .schema()
                    .field_with_unqualified_name(&name)
                    .ok()
                    .map(|field| field.data_type().clone()))
            }
            _ => Ok(df
                .clone()
                .select(vec![expr.clone().alias("__t")])
                .ok()
                .map(|projected| projected.schema().field(0).data_type().clone())),
        };
    }

    if expr.column_refs().is_empty() {
        Ok(expr.get_type(&DFSchema::empty()).ok())
    } else {
        Ok(None)
    }
}

fn scale_input_data_type(data_type: &ArrowDataType) -> ArrowDataType {
    match data_type {
        ArrowDataType::List(field) => field.data_type().clone(),
        _ => data_type.clone(),
    }
}

fn infer_scale_domain_expr_type(
    node: &datafusion_proto::protobuf::LogicalExprNode,
    ctx: &SessionContext,
) -> Option<ArrowDataType> {
    node.to_expr(ctx)
        .ok()
        .and_then(|expr| expr.get_type(&DFSchema::empty()).ok())
        .filter(|data_type| !matches!(data_type, ArrowDataType::Null))
}

fn infer_scale_domain_data_type(
    domain: &ScaleDomain,
    ctx: &SessionContext,
) -> Option<ArrowDataType> {
    match &domain.default_domain {
        ScaleDefaultDomain::Interval(start, end) => infer_scale_domain_expr_type(start, ctx)
            .or_else(|| infer_scale_domain_expr_type(end, ctx))
            .or(Some(ArrowDataType::Float64)),
        ScaleDefaultDomain::Discrete(values) => values
            .iter()
            .find_map(|value| infer_scale_domain_expr_type(value, ctx)),
        ScaleDefaultDomain::DomainExprs(_) | ScaleDefaultDomain::NoDefault => None,
    }
}

fn preferred_scale_spec_for_data_type<C>(
    channel: &str,
    data_type: &ArrowDataType,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
) -> Result<Option<Box<dyn ScaleSpec>>, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    if let Some(preference) = coord_transform.preferred_scale_type(channel, data_type) {
        return Ok(Some(scale_spec_for_preference(preference)));
    }

    for prepared in prepared_marks {
        let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
            .unwrap_or_else(|_| prepared.channels.clone());
        for (channel_name, channel_value) in &resolved {
            if channel_maps_to_scale(coord_transform, channel_name, channel_value, channel)
                && let Some(preference) =
                    prepared.mark.preferred_scale_type(channel_name, data_type)
            {
                return Ok(Some(scale_spec_for_preference(preference)));
            }
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if channel_maps_to_scale(
                coord_transform,
                &source.channel,
                &source.channel_value,
                channel,
            ) && let Some(preference) = prepared
                .mark
                .preferred_scale_type(&source.channel, data_type)
            {
                return Ok(Some(scale_spec_for_preference(preference)));
            }
        }
    }

    Ok(default_scale_type_for_data_type(data_type).map(scale_spec_for_preference))
}

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
        derived_scalars: DerivedScalarMap,
    ) -> Self {
        Self {
            mark,
            domain_dataframe: dataframe.clone(),
            domain_channels: channels.clone(),
            extra_domain_sources: Vec::new(),
            dataframe,
            channels,
            derived_scalars,
            scale_inference_hints: Vec::new(),
        }
    }

    pub fn new_with_domain_source(
        mark: Arc<dyn CompiledMark>,
        dataframe: Option<DataFrame>,
        channels: IndexMap<String, ChannelValue>,
        domain_dataframe: Option<DataFrame>,
        domain_channels: IndexMap<String, ChannelValue>,
        derived_scalars: DerivedScalarMap,
    ) -> Self {
        Self {
            mark,
            dataframe,
            channels,
            domain_dataframe,
            domain_channels,
            extra_domain_sources: Vec::new(),
            derived_scalars,
            scale_inference_hints: Vec::new(),
        }
    }

    pub fn with_extra_domain_sources(mut self, sources: Vec<MarkScaleDomainSource>) -> Self {
        self.extra_domain_sources = sources;
        self
    }

    pub fn with_scale_inference_hints(mut self, hints: Vec<ScaleInferenceHint>) -> Self {
        self.scale_inference_hints = hints;
        self
    }
}

fn collect_channel_derived_scalars<C>(
    channel: &str,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
) -> Result<DerivedScalarMap, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    let mut collected = DerivedScalarMap::new();
    for prepared in prepared_marks {
        let mut collect_from_exprs = |exprs: Vec<Expr>| -> Result<(), AvengerChartError> {
            for expr in exprs {
                for id in collect_derived_scalar_ids(&expr)? {
                    let Some(replacement) = prepared.derived_scalars.get(&id) else {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Derived scalar '{id}' was referenced by channel '{channel}' but was not produced in this data scope"
                        )));
                    };
                    if let Some(existing) = collected.insert(id.clone(), replacement.clone())
                        && existing != *replacement
                    {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Derived scalar '{id}' for channel '{channel}' has conflicting expressions in the same data scope"
                        )));
                    }
                }
            }
            Ok(())
        };

        let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
            .unwrap_or_else(|_| prepared.channels.clone());
        for (channel_name, channel_value) in resolved {
            if !channel_maps_to_scale(coord_transform, &channel_name, &channel_value, channel) {
                continue;
            }

            let mut exprs = channel_value.all_exprs(ctx);
            if let Some(scale_config) = channel_value.get_scale_config() {
                exprs.extend(scale_config.all_exprs(ctx));
            }
            if let Some(axis_config) = channel_value.get_axis_config() {
                exprs.extend(axis_config.all_exprs(ctx));
            }

            collect_from_exprs(exprs)?;
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if !channel_maps_to_scale(
                coord_transform,
                &source.channel,
                &source.channel_value,
                channel,
            ) {
                continue;
            }

            let mut exprs = source.exprs;
            exprs.extend(source.channel_value.all_exprs(ctx));
            if let Some(scale_config) = source.channel_value.get_scale_config() {
                exprs.extend(scale_config.all_exprs(ctx));
            }

            collect_from_exprs(exprs)?;
        }

        if let Some(axis_config) = prepared.mark.state().axis_configs.get(channel) {
            collect_from_exprs(axis_config.all_exprs(ctx))?;
        }
    }
    Ok(collected)
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
            let dataframe = if mark.state().data_mode == MarkDataMode::Unit {
                None
            } else {
                mark.data_context()
                    .dataframe_with_context(ctx)
                    .or_else(|| inherited_df.clone())
            };
            PreparedScaleMark::new(
                mark.clone(),
                dataframe,
                mark.data_context().channels().clone(),
                DerivedScalarMap::new(),
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

fn positional_scale_names_for_prepared_marks<C>(
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
) -> Result<HashSet<String>, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    let required = coord_transform
        .required_channels()
        .iter()
        .filter(|&&channel| coord_transform.channel_uses_scale(channel))
        .copied()
        .collect::<HashSet<_>>();
    let mut names = required
        .iter()
        .map(|channel| (*channel).to_string())
        .collect::<HashSet<_>>();

    for prepared in prepared_marks {
        let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
            .unwrap_or_else(|_| prepared.channels.clone());
        for (channel_name, channel_value) in resolved {
            if !coord_transform.channel_uses_scale(&channel_name) {
                continue;
            }
            let base_channel = strip_trailing_numbers(&channel_name);
            if !coord_transform.is_position_scale_channel(base_channel) {
                continue;
            }
            if let Some(scale_name) = channel_value.get_scale_name(&channel_name) {
                names.insert(scale_name);
            }
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if !coord_transform.channel_uses_scale(&source.channel) {
                continue;
            }
            let base_channel = strip_trailing_numbers(&source.channel);
            if !coord_transform.is_position_scale_channel(base_channel) {
                continue;
            }
            if let Some(scale_name) = source.channel_value.get_scale_name(&source.channel) {
                names.insert(scale_name);
            }
        }
    }

    Ok(names)
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
    let number_locale_registry = eval_ctx
        .formatting_context()
        .number_locale_registry()
        .map_err(AvengerChartError::InvalidArgument)?;
    let mut builder = ScaleBuilder::new()
        .with_number_locale(eval_ctx.formatting_context().resolved_number_locale())
        .with_number_locale_registry(number_locale_registry.clone());

    // Collect channels needing scales from marks
    let mut channels_with_scales = HashSet::new();
    let mut default_range_channels: HashMap<String, String> = HashMap::new();
    for prepared in prepared_marks {
        let encodings = &prepared.channels;
        let resolved =
            resolve_all_channel_refs(encodings, ctx).unwrap_or_else(|_| encodings.clone());
        for (channel_name, channel_value) in resolved {
            if coord_transform.channel_uses_scale(&channel_name)
                && let Some(scale_name) = channel_value.get_scale_name(&channel_name)
            {
                default_range_channels
                    .entry(scale_name.clone())
                    .or_insert(channel_name);
                channels_with_scales.insert(scale_name);
            }
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if coord_transform.channel_uses_scale(&source.channel)
                && let Some(scale_name) = source.channel_value.get_scale_name(&source.channel)
            {
                default_range_channels
                    .entry(scale_name.clone())
                    .or_insert(source.channel);
                channels_with_scales.insert(scale_name);
            }
        }
    }

    // Also include explicit scale specs
    for channel in scale_specs.keys() {
        if coord_transform.channel_uses_scale(channel) {
            channels_with_scales.insert(channel.clone());
            default_range_channels
                .entry(channel.clone())
                .or_insert(channel.clone());
        }
    }

    let positional_scale_names =
        positional_scale_names_for_prepared_marks(prepared_marks, coord_transform, ctx)?;

    // Always ensure base positional scales are present. This handles implicit
    // unnamed scales and guarantees we attempt to build/capture radius-aware
    // domains for x/y even when only a secondary channel is present.
    for channel in coord_transform
        .required_channels()
        .iter()
        .filter(|&&ch| coord_transform.channel_uses_scale(ch))
    {
        channels_with_scales.insert((*channel).to_string());
        default_range_channels
            .entry((*channel).to_string())
            .or_insert((*channel).to_string());
    }

    for (scale_name, range_channel_name) in default_range_channels {
        builder.set_default_range_channel(scale_name, range_channel_name);
    }

    // Determine positional vs non-positional channels
    let non_positional_channels: Vec<String> = channels_with_scales
        .iter()
        .filter(|ch| !positional_scale_names.contains(*ch))
        .cloned()
        .collect();

    let positional_channels: Vec<String> = channels_with_scales
        .iter()
        .filter(|ch| positional_scale_names.contains(*ch))
        .cloned()
        .collect();

    // PHASE 1: Build non-positional scale builders
    // We build non‑positional scales first so their configured scales can be used to construct radius‑aware positional expressions
    for channel in &non_positional_channels {
        if let Some((spec, dt, options, _has_explicit_domain, domain_opt, ordering, range_opt)) =
            Box::pin(build_scale_for_channel(
                channel,
                prepared_marks,
                ctx,
                params,
                false,           // no radius for non-positional
                &HashMap::new(), // no phase1 scales yet
                scale_specs,
                coord_transform,
                eval_ctx.time_context(),
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
                ordering,
                range_opt,
                prepared_marks,
                coord_transform,
                eval_ctx,
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
        let range_channel_name = builder
            .default_range_channels
            .get(channel_name)
            .map(String::as_str)
            .unwrap_or(channel_name);
        if let Some(configured) = Box::pin(build_temp_configured_scale(
            channel_builder,
            range_channel_name,
            400.0, // dummy width
            300.0, // dummy height
            ctx,
            params,
            theme,
            Some(eval_ctx.formatting_context().resolved_number_locale()),
            number_locale_registry.clone(),
        ))
        .await?
        {
            phase1_configured.insert(channel_name.to_string(), configured);
        }
    }

    // PHASE 2: Build positional scales with scale-aware radius expressions
    for channel in &positional_channels {
        if let Some((spec, dt, options, has_explicit_domain, domain_opt, ordering, range_opt)) =
            Box::pin(build_scale_for_channel(
                channel,
                prepared_marks,
                ctx,
                params,
                true, // check radius for positional
                &phase1_configured,
                scale_specs,
                coord_transform,
                eval_ctx.time_context(),
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
                    coord_transform,
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
                ordering,
                range_opt,
                prepared_marks,
                coord_transform,
                eval_ctx,
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
fn channel_maps_to_scale<C>(
    coord_transform: &C,
    channel_name: &str,
    channel_value: &ChannelValue,
    target_channel: &str,
) -> bool
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    if !coord_transform.channel_uses_scale(channel_name) {
        return false;
    }

    channel_value
        .get_scale_name(channel_name)
        .map(|scale_name| scale_name == target_channel)
        .unwrap_or(channel_name == target_channel)
}

fn prepared_mark_maps_to_scale<C>(
    prepared: &PreparedScaleMark,
    coord_transform: &C,
    channel: &str,
    ctx: &SessionContext,
) -> Result<bool, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    if prepared
        .channels
        .iter()
        .any(|(ch_name, ch_val)| channel_maps_to_scale(coord_transform, ch_name, ch_val, channel))
    {
        return Ok(true);
    }

    Ok(mark_scale_domain_sources(prepared, ctx)?
        .iter()
        .any(|source| {
            channel_maps_to_scale(
                coord_transform,
                &source.channel,
                &source.channel_value,
                channel,
            )
        }))
}

fn matching_scale_hint(
    channel: &str,
    prepared_marks: &[PreparedScaleMark],
) -> Result<Option<ScaleTypePreference>, AvengerChartError> {
    let mut preference = None;
    for hint in prepared_marks
        .iter()
        .flat_map(|prepared| prepared.scale_inference_hints.iter())
        .filter(|hint| hint.scale_name == channel)
    {
        match preference {
            Some(existing) if existing != hint.preference => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Conflicting compound mark scale inference hints for scale '{channel}': {existing:?} and {:?}",
                    hint.preference
                )));
            }
            Some(_) => {}
            None => preference = Some(hint.preference),
        }
    }
    Ok(preference)
}

fn has_nested_band_config_for_channel<C>(
    channel: &str,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
) -> bool
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    prepared_marks.iter().any(|prepared| {
        let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
            .unwrap_or_else(|_| prepared.channels.clone());
        resolved.iter().any(|(channel_name, channel_value)| {
            channel_maps_to_scale(coord_transform, channel_name, channel_value, channel)
                && channel_value.get_nested_band_config().is_some()
        })
    })
}

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
    time_context: &TimeContext,
) -> Result<
    Option<(
        Box<dyn ScaleSpec>,
        ArrowDataType,
        HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
        bool,
        Option<ScaleDomain>,
        Option<ScaleOrderingSpec>,
        Option<ScaleRange>,
    )>,
    AvengerChartError,
>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
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
            if !channel_maps_to_scale(coord_transform, channel_name, channel_value, channel) {
                continue;
            }

            // Use scale_input_expr for type inference - we only care about the type
            // of values that actually pass through the scale. For conditionals with
            // literal Value branches, those branches are replaced with NULL so they
            // don't affect type inference (e.g., a numeric scaled branch with a
            // string literal override should infer as numeric, not string).
            let maybe_dt = if let Some(expr) = channel_value.scale_input_expr(ctx) {
                infer_expr_data_type(prepared.dataframe.as_ref(), &expr)?
            } else {
                None
            };

            if let Some(dt) = maybe_dt {
                let scale_dt = scale_input_data_type(&dt);
                data_type = Some(scale_dt.clone());
                chosen_spec = if channel_value.get_nested_band_config().is_some() {
                    Some(scale_spec_for_preference(ScaleTypePreference::NestedBand))
                } else {
                    coord_transform
                        .preferred_scale_type(channel, &scale_dt)
                        .or_else(|| mark.preferred_scale_type(channel_name, &scale_dt))
                        .map(scale_spec_for_preference)
                };
                break 'outer;
            }
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if !channel_maps_to_scale(
                coord_transform,
                &source.channel,
                &source.channel_value,
                channel,
            ) {
                continue;
            }

            for expr in &source.exprs {
                let Some(dt) = infer_expr_data_type(Some(&source.dataframe), expr)? else {
                    continue;
                };
                let scale_dt = scale_input_data_type(&dt);
                data_type = Some(scale_dt.clone());
                chosen_spec = coord_transform
                    .preferred_scale_type(channel, &scale_dt)
                    .or_else(|| mark.preferred_scale_type(&source.channel, &scale_dt))
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
            if !channel_maps_to_scale(coord_transform, channel_name, channel_value, channel) {
                continue;
            }

            if let Some(scale_config) = channel_value.get_scale_config() {
                // Capture any scale config, not just those with explicit domains
                // This ensures options like nice(false) and zero(false) are preserved
                chosen_scale_config = Some(Scale::from_config(scale_config.clone()));
                break 'config_outer;
            }
        }

        for source in mark_scale_domain_sources(prepared, ctx)? {
            if !channel_maps_to_scale(
                coord_transform,
                &source.channel,
                &source.channel_value,
                channel,
            ) {
                continue;
            }

            if let Some(scale_config) = source.channel_value.get_scale_config() {
                chosen_scale_config = Some(Scale::from_config(scale_config.clone()));
                break 'config_outer;
            }
        }
    }

    if data_type.is_none()
        && let Some(domain) = chosen_scale_config
            .as_ref()
            .and_then(|scale| scale.get_domain())
            .filter(|domain| !domain.is_raw_only())
    {
        data_type = infer_scale_domain_data_type(domain, ctx);
    }

    // If user provided an explicit scale type, prefer it over mark/data-type inference
    let mut user_selected_scale_type = false;
    if let Some(user_scale) = &chosen_scale_config {
        if let Some(user_spec) = user_scale.get_scale_spec() {
            chosen_spec = Some(user_spec);
            user_selected_scale_type = true;
        } else if let Some(range) = user_scale.get_range() {
            // If no explicit scale type but discrete range is set, infer ordinal scale
            if matches!(range, ScaleRange::Discrete(_) | ScaleRange::Pattern(_)) {
                chosen_spec = Some(Box::new(Ordinal));
                user_selected_scale_type = true;
            }
        }
    }

    if !user_selected_scale_type {
        if has_nested_band_config_for_channel(channel, prepared_marks, coord_transform, ctx) {
            chosen_spec = Some(scale_spec_for_preference(ScaleTypePreference::NestedBand));
        } else if let Some(preference) = matching_scale_hint(channel, prepared_marks)? {
            chosen_spec = Some(scale_spec_for_preference(preference));
        } else if let Some(dt) = data_type.as_ref() {
            chosen_spec = preferred_scale_spec_for_data_type(
                channel,
                dt,
                prepared_marks,
                coord_transform,
                ctx,
            )?;
        }
    }

    let scale_spec = match chosen_spec {
        Some(spec) => spec,
        None => {
            if matches!(data_type, Some(ArrowDataType::Struct(_))) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Struct-valued chart channel '{channel}' is not supported directly; use nested([...]) for nested-band position scales"
                )));
            }
            return Ok(None);
        }
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
    if let Some(channel_scale) = &chosen_scale_config
        && let Some(ordering) = channel_scale.get_ordering()
    {
        scale.config_mut().ordering = Maybe::Set(ordering.clone());
    }
    if let Some(channel_scale) = &chosen_scale_config
        && let Some(range) = channel_scale.get_range()
    {
        scale = scale.range(range.clone());
    }

    // Check if domain is explicitly set by user on the channel
    let mut has_explicit_domain = chosen_scale_config
        .as_ref()
        .and_then(|c| c.get_domain())
        .map(|domain| !domain.is_raw_only())
        .unwrap_or(false);

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

        if scale_impl.domain_kind() == DomainKind::Temporal {
            scale = scale.option(
                "timezone",
                lit(time_context.resolved_timezone().to_string()),
            );
        }

        // Find mark that has a channel or mark-owned source mapping to this scale.
        for prepared in prepared_marks {
            if !prepared_mark_maps_to_scale(prepared, coord_transform, channel, ctx)? {
                continue;
            }

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
            break;
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
        // If plot-level override set a real fallback domain, treat it as explicit.
        // A raw-only override still uses the inferred/cached fallback domain.
        if scale
            .get_domain()
            .map(|domain| !domain.is_raw_only())
            .unwrap_or(false)
        {
            has_explicit_domain = true;
        }
    }

    if scale_spec.name() == "nested_band" {
        let config = nested_band_config_for_channel(channel, prepared_marks, coord_transform, ctx)?
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "NestedBand position scale '{channel}' must be created with nested([...]); raw struct-valued position columns are not supported"
                ))
            })?;
        let level_count = validate_nested_band_source_columns(&config, &dt)?;
        validate_nested_band_config(&config, level_count)?;
        scale = apply_nested_band_options(scale, &config)?;
    }

    let options = scale.get_options().clone();
    let domain_opt = scale.get_domain().cloned();
    let ordering = scale.get_ordering().cloned();
    let range_opt = scale.get_range().cloned();

    Ok(Some((
        scale_spec,
        dt,
        options,
        has_explicit_domain,
        domain_opt,
        ordering,
        range_opt,
    )))
}

fn nested_band_config_for_channel<C>(
    channel: &str,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
) -> Result<Option<NestedBandSpec>, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    for prepared in prepared_marks {
        validate_nested_band_derivative_configs(&prepared.channels, ctx)?;
        let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
            .unwrap_or_else(|_| prepared.channels.clone());
        for (channel_name, channel_value) in &resolved {
            if channel_maps_to_scale(coord_transform, channel_name, channel_value, channel)
                && let Some(config) = channel_value.get_nested_band_config()
            {
                return Ok(Some(config.clone()));
            }
        }
    }
    Ok(None)
}

fn validate_nested_band_derivative_configs(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    for (channel_name, channel_value) in channels {
        if channel_value.get_nested_band_config().is_none() {
            continue;
        }
        let Some(expr) = channel_value.scale_input_expr(ctx) else {
            continue;
        };
        let references_derivative = expr.column_refs().iter().any(|column| {
            column
                .name
                .strip_prefix(':')
                .is_some_and(|source| !source.is_empty())
        });
        if references_derivative {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Nested-band level configuration must be declared on the source position channel, not derivative channel '{channel_name}'"
            )));
        }
    }
    Ok(())
}

fn validate_nested_band_source_columns(
    config: &NestedBandSpec,
    dt: &ArrowDataType,
) -> Result<usize, AvengerChartError> {
    config.validate_source_columns()?;

    let ArrowDataType::Struct(fields) = dt else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "NestedBand scales require nested([...]) source columns lowered to struct-valued position data, got {dt:?}"
        )));
    };
    let level_count = fields.len();
    if level_count == 0 {
        return Err(AvengerChartError::InvalidArgument(
            "nested([...]) must contain at least one source column".to_string(),
        ));
    }
    if config.source_columns.len() != level_count {
        return Err(AvengerChartError::InvalidArgument(format!(
            "nested([...]) declared {} source columns but lowered position data has {level_count} struct fields",
            config.source_columns.len()
        )));
    }

    for (level, (source_column, field)) in
        config.source_columns.iter().zip(fields.iter()).enumerate()
    {
        if field.name() != source_column {
            return Err(AvengerChartError::InvalidArgument(format!(
                "nested([...]) source column '{source_column}' does not match lowered struct field '{}' at level {level}",
                field.name()
            )));
        }
    }

    Ok(level_count)
}

fn validate_nested_band_config(
    config: &NestedBandSpec,
    level_count: usize,
) -> Result<(), AvengerChartError> {
    for (level, level_config) in &config.levels {
        if *level >= level_count {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} is invalid for struct position data with {level_count} fields"
            )));
        }
        if *level == 0 && level_config.nest_scope.is_some() {
            return Err(AvengerChartError::InvalidArgument(
                "nest_scope(...) is only valid for nested-band child levels, not level 0"
                    .to_string(),
            ));
        }
        for (name, value) in [
            ("padding_inner", level_config.padding_inner),
            ("padding_outer", level_config.padding_outer),
            ("padding_inner_px", level_config.padding_inner_px),
            ("padding_outer_px", level_config.padding_outer_px),
        ] {
            if let Some(value) = value
                && (value < 0.0 || !value.is_finite())
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Nested-band {name} for level {level} must be non-negative and finite"
                )));
            }
        }
    }
    Ok(())
}

fn apply_nested_band_options(
    mut scale: Scale<Auto>,
    config: &NestedBandSpec,
) -> Result<Scale<Auto>, AvengerChartError> {
    let Some(max_level) = config.levels.keys().next_back().copied() else {
        return Ok(scale);
    };
    let len = max_level + 1;
    let mut nest_scopes = vec![String::new(); len];
    let mut padding_inner = vec![String::new(); len];
    let mut padding_outer = vec![String::new(); len];
    let mut padding_inner_px = vec![String::new(); len];
    let mut padding_outer_px = vec![String::new(); len];

    for (level, level_config) in &config.levels {
        if let Some(scope) = level_config.nest_scope {
            nest_scopes[*level] = match scope {
                NestScope::Free => "free".to_string(),
                NestScope::Shared => "shared".to_string(),
            };
        }
        if let Some(value) = level_config.padding_inner {
            padding_inner[*level] = value.to_string();
        }
        if let Some(value) = level_config.padding_outer {
            padding_outer[*level] = value.to_string();
        }
        if let Some(value) = level_config.padding_inner_px {
            padding_inner_px[*level] = value.to_string();
        }
        if let Some(value) = level_config.padding_outer_px {
            padding_outer_px[*level] = value.to_string();
        }
    }

    if nest_scopes.iter().any(|value| !value.is_empty()) {
        scale = scale.option("nest_scopes", lit(nest_scopes.join(",")));
    }
    if padding_inner.iter().any(|value| !value.is_empty()) {
        scale = scale.option("padding_inner_levels", lit(padding_inner.join(",")));
    }
    if padding_outer.iter().any(|value| !value.is_empty()) {
        scale = scale.option("padding_outer_levels", lit(padding_outer.join(",")));
    }
    if padding_inner_px.iter().any(|value| !value.is_empty()) {
        scale = scale.option("padding_inner_px_levels", lit(padding_inner_px.join(",")));
    }
    if padding_outer_px.iter().any(|value| !value.is_empty()) {
        scale = scale.option("padding_outer_px_levels", lit(padding_outer_px.join(",")));
    }

    Ok(scale)
}

/// Get radius expression for a positional channel (returns None for non-positional)
fn get_radius_expression<C>(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    ctx: &SessionContext,
    phase1_configured: &HashMap<String, ConfiguredScaleWithSpec>,
    theme: &Theme,
) -> Result<Option<PreparedRadiusExpression>, AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
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
            let base_channel = strip_trailing_numbers(channel);
            if channel_maps_to_scale(coord_transform, ch, ch_value, base_channel) {
                // Create scale-aware resolve_channel closure
                let resolve_channel = |ch_name: &str| -> Expr {
                    if let Some(channel_value) = resolved.get(ch_name) {
                        match channel_value {
                            ChannelValue::Scaled {
                                expr, scale_name, ..
                            } => {
                                let scale_key = scale_name
                                    .as_ref()
                                    .cloned()
                                    .unwrap_or_else(|| strip_trailing_numbers(ch_name).to_string());

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
                            ChannelValue::Value { expr, .. } => {
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

    Ok(None)
}

/// Cache domain data for a channel in the ScaleBuilder
#[allow(clippy::too_many_arguments)]
async fn cache_domain_data<C>(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    dt: &ArrowDataType,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    domain_opt: Option<ScaleDomain>,
    ordering: Option<ScaleOrderingSpec>,
    range_opt: Option<ScaleRange>,
    prepared_marks: &[PreparedScaleMark],
    coord_transform: &C,
    eval_ctx: &CoreEvaluationContext,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    phase1_configured: &HashMap<String, ConfiguredScaleWithSpec>,
    builder: &mut ScaleBuilder,
    radius_expr_opt: Option<PreparedRadiusExpression>,
    theme: &Theme,
) -> Result<(), AvengerChartError>
where
    C: CoordinateSystemTransformCore + ?Sized,
{
    let derived_scalars =
        collect_channel_derived_scalars(channel, prepared_marks, coord_transform, ctx)?;
    if let Some(range) = range_opt {
        builder.set_explicit_range(channel.to_string(), range);
    }

    // Build a scale with domain and options to inspect domain (including any DomainExprs)
    let mut scale = Scale::<Auto>::from_spec(spec.clone_box());

    // Apply domain if provided (this is the key fix - domain was previously lost)
    if let Some(domain) = &domain_opt {
        scale = scale.domain(domain.clone());
    }

    for (key, value_node) in &options {
        let expr = resolve_derived_scalars(value_node.to_expr(ctx)?, &derived_scalars)?;
        scale = scale.option(key, expr);
    }

    let target_domain_kind = spec.domain_kind();
    if ordering
        .as_ref()
        .and_then(|ordering| ordering.order_expr.as_ref())
        .is_some()
        && !matches!(target_domain_kind, DomainKind::Categorical)
    {
        return Err(AvengerChartError::InvalidArgument(
            "Scale order_by is only supported for categorical-domain scales".to_string(),
        ));
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
                    derived_scalars,
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

    let raw_domain_override = scale
        .get_domain()
        .and_then(|domain| domain.raw_domain.clone());

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
            if prepared.mark.state().exclude_from_scale_domains {
                continue;
            }
            let mark = &prepared.mark;
            let render_df = prepared
                .dataframe
                .clone()
                .filter(|df| !is_empty_relation(df))
                .map(Arc::new);
            let domain_df = prepared
                .domain_dataframe
                .clone()
                .filter(|df| !is_empty_relation(df))
                .map(Arc::new);
            if render_df.is_none() && domain_df.is_none() {
                continue;
            }

            for source in mark_scale_domain_sources(prepared, ctx)? {
                if !channel_maps_to_scale(
                    coord_transform,
                    &source.channel,
                    &source.channel_value,
                    channel,
                ) {
                    continue;
                }
                if is_empty_relation(&source.dataframe) {
                    continue;
                }

                let df = Arc::new(source.dataframe.clone());
                for expr in source.exprs {
                    entries.push((df.clone(), expr, None));
                }
            }

            let resolved = resolve_all_channel_refs(&prepared.channels, ctx)
                .unwrap_or_else(|_| prepared.channels.clone());
            let domain_resolved = resolve_all_channel_refs(&prepared.domain_channels, ctx)
                .unwrap_or_else(|_| prepared.domain_channels.clone());

            for (channel_name, domain_channel_value) in &domain_resolved {
                if channel_maps_to_scale(
                    coord_transform,
                    channel_name,
                    domain_channel_value,
                    channel,
                ) {
                    let domain_scale_input = domain_channel_value.scale_input_expr(ctx);
                    let use_domain_source = domain_scale_input
                        .as_ref()
                        .is_some_and(|expr| !contains_aggregate(expr))
                        && domain_df.is_some();
                    let df = if use_domain_source {
                        domain_df.as_ref().expect("checked domain df").clone()
                    } else if let Some(render_df) = render_df.as_ref() {
                        render_df.clone()
                    } else {
                        domain_df.as_ref().expect("checked domain df").clone()
                    };
                    let channel_value = if use_domain_source {
                        domain_channel_value
                    } else {
                        resolved.get(channel_name).unwrap_or(domain_channel_value)
                    };
                    let source_resolved = if use_domain_source {
                        &domain_resolved
                    } else {
                        &resolved
                    };

                    // Helper to push (df, expr_df, per_mark_radius)
                    let mut push_entry = |expr_df: Expr| -> Result<(), AvengerChartError> {
                        // Compute per-mark radius (if supported by this scale)
                        let per_mark_radius = if let Ok(scale_impl) =
                            Scale::<Auto>::from_spec(spec.clone_box()).to_scale_impl()
                        {
                            if scale_impl.supports_radius_expansion() {
                                // Build resolve_channel as in get_radius_expression
                                let resolve_channel = |ch_name: &str| -> Expr {
                                    if let Some(ch_val) = source_resolved.get(ch_name) {
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
                                            ChannelValue::Value { expr, .. } => {
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
                    let scale_input = if use_domain_source {
                        domain_scale_input
                    } else {
                        channel_value.scale_input_expr(ctx)
                    };
                    if let Some(expr_df) = scale_input {
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
            eval_ctx,
            ctx,
            params,
            builder,
            dt,
            derived_scalars,
        ))
        .await?;
    } else if matches!(
        target_domain_kind,
        DomainKind::Categorical | DomainKind::NestedCategorical
    ) {
        let nested_band_config = if target_domain_kind == DomainKind::NestedCategorical {
            let config =
                nested_band_config_for_channel(channel, prepared_marks, coord_transform, ctx)?;
            if let Some(config) = &config {
                let level_count = validate_nested_band_source_columns(config, dt)?;
                validate_nested_band_config(config, level_count)?;
            }
            config
        } else {
            None
        };
        Box::pin(cache_categorical_data(
            channel,
            spec,
            options,
            &data_expressions,
            if target_domain_kind == DomainKind::Categorical {
                ordering.as_ref()
            } else {
                None
            },
            nested_band_config.as_ref(),
            eval_ctx,
            ctx,
            params,
            builder,
            dt,
            derived_scalars,
        ))
        .await?;
    } else if target_domain_kind == DomainKind::Temporal {
        Box::pin(cache_temporal_data(
            channel,
            spec,
            options,
            &data_expressions,
            eval_ctx,
            ctx,
            params,
            builder,
            dt,
            derived_scalars,
        ))
        .await?;
    } else {
        Box::pin(cache_numeric_data(
            channel,
            spec,
            options,
            &data_expressions,
            eval_ctx,
            ctx,
            params,
            builder,
            dt,
            derived_scalars,
        ))
        .await?;
    }

    builder.apply_raw_domain(channel, raw_domain_override);

    Ok(())
}

/// Build temporary ConfiguredScale used to construct radius‑aware positional expressions
async fn build_temp_configured_scale(
    channel_builder: &ChannelScaleData,
    range_channel_name: &str,
    width: f32,
    height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
    number_locale: Option<&str>,
    number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
) -> Result<Option<ConfiguredScaleWithSpec>, AvengerChartError> {
    match channel_builder {
        ChannelScaleData::Standard {
            scale_spec,
            data_extents,
            options,
            raw_domain,
            derived_scalars,
        } => {
            let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

            // Apply cached options
            for (key, value_node) in options {
                let expr = resolve_derived_scalars(value_node.to_expr(ctx)?, derived_scalars)?;
                scale = scale.option(key, expr);
            }

            // Set domain from cached extents
            let mut domain = data_extents.to_scale_domain()?;
            domain.raw_domain = raw_domain.clone();
            scale = scale.domain(domain);

            // Set range from theme
            let range_kind = match scale.get_scale_impl() {
                Some(impl_) => impl_.range_kind(),
                None => return Ok(None), // Can't build temp scale if impl creation fails
            };
            let range = if let Some(theme_range) =
                theme.get_range_for_channel("mark", range_channel_name, range_kind, None, params)
            {
                theme_range
            } else {
                default_range_for_channel(range_channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;
            let mut configured =
                Box::pin(scale.create_configured_scale(width, height, ctx, params)).await?;
            apply_number_locale_to_temp_configured_scale(
                &mut configured,
                number_locale,
                number_locale_registry.clone(),
            );

            Ok(Some(
                ConfiguredScaleWithSpec::new(scale, configured)
                    .with_derived_scalars(derived_scalars.clone()),
            ))
        }
        ChannelScaleData::ExplicitDomain {
            scale_spec,
            options,
            domain,
            derived_scalars,
        } => {
            // Build a temporary configured scale using the explicit domain and options
            let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

            // Apply the stored explicit domain FIRST
            scale = scale
                .domain(resolve_scale_domain_exprs(domain, ctx, params, derived_scalars).await?);

            // Apply cached options
            for (key, value_node) in options {
                let expr = resolve_derived_scalars(value_node.to_expr(ctx)?, derived_scalars)?;
                scale = scale.option(key, expr);
            }

            // Set range from theme (mirrors Standard arm)
            let range_kind = match scale.get_scale_impl() {
                Some(impl_) => impl_.range_kind(),
                None => return Ok(None),
            };
            let range = if let Some(theme_range) =
                theme.get_range_for_channel("mark", range_channel_name, range_kind, None, params)
            {
                theme_range
            } else {
                default_range_for_channel(range_channel_name, range_kind)
            };
            scale = scale.range(range);

            // Normalize and create configured scale
            scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;
            let mut configured =
                Box::pin(scale.create_configured_scale(width, height, ctx, params)).await?;
            apply_number_locale_to_temp_configured_scale(
                &mut configured,
                number_locale,
                number_locale_registry.clone(),
            );

            Ok(Some(
                ConfiguredScaleWithSpec::new(scale, configured)
                    .with_derived_scalars(derived_scalars.clone()),
            ))
        }
        ChannelScaleData::RadiusAware { .. } => {
            // Radius‑aware scales are not expected here
            Ok(None)
        }
    }
}

fn apply_number_locale_to_temp_configured_scale(
    configured: &mut ConfiguredScale,
    number_locale: Option<&str>,
    number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
) {
    if number_locale.is_some() || number_locale_registry.is_some() {
        let mut formatters: Formatters = configured.config.context.formatters.clone();
        formatters.number = Arc::new(DefaultFormatter {
            number_locale: number_locale.map(str::to_string),
            number_locale_registry,
            ..DefaultFormatter::default()
        });
        configured.config.context.formatters = formatters;
    }
}

// Helper functions for caching different types of data (implementations to follow)
async fn cache_radius_aware_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    entries: &[(Arc<DataFrame>, Expr, Option<PreparedRadiusExpression>)],
    global_radius_opt: Option<&PreparedRadiusExpression>,
    eval_ctx: &CoreEvaluationContext,
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
    derived_scalars: DerivedScalarMap,
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
        eval_ctx.record_scale_domain_collect();
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

        for batch in &batches {
            if batch.num_rows() == 0 {
                continue;
            }
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
            derived_scalars,
        );
        builder.set_channel_data_type(channel.to_string(), dt.clone());
    }

    Ok(())
}

async fn distinct_categorical_values(
    data_expressions: &[(Arc<DataFrame>, Expr)],
    eval_ctx: &CoreEvaluationContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    let mut all_unique_values: Vec<ScalarValue> = Vec::new();

    for (df, expr) in data_expressions {
        let projected = df
            .as_ref()
            .clone()
            .select(vec![expr.clone().alias("value")])?;
        let projected_type = projected.schema().field(0).data_type().clone();
        let collect_df = if matches!(projected_type, ArrowDataType::List(_)) {
            projected
        } else {
            projected.distinct()?
        };

        eval_ctx.record_scale_domain_collect();
        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                collect_df
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                collect_df.collect().await?
            }
        } else {
            collect_df.collect().await?
        };

        for batch in batches.iter() {
            let value_array = batch.column(0);
            append_unique_domain_scalars(value_array, &mut all_unique_values)?;
        }
    }

    all_unique_values.sort_by(scalar_total_cmp);
    Ok(all_unique_values)
}

fn append_unique_domain_scalars(
    array: &ArrayRef,
    values: &mut Vec<ScalarValue>,
) -> Result<(), AvengerChartError> {
    match array.data_type() {
        ArrowDataType::List(_) => {
            let list_array = array.as_list::<i32>();
            for row in 0..list_array.len() {
                if list_array.is_null(row) {
                    continue;
                }
                append_unique_domain_scalars(&list_array.value(row), values)?;
            }
        }
        _ => {
            for row in 0..array.len() {
                if array.is_null(row) {
                    continue;
                }
                let scalar = ScalarValue::try_from_array(array, row)?;
                if !scalar.is_null() && !values.iter().any(|value| value == &scalar) {
                    values.push(scalar);
                }
            }
        }
    }

    Ok(())
}

async fn ordered_categorical_values(
    data_expressions: &[(Arc<DataFrame>, Expr)],
    order_expr: &Expr,
    order_descending: bool,
    eval_ctx: &CoreEvaluationContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<(Vec<ScalarValue>, bool), AvengerChartError> {
    let Some((_, category_expr)) = data_expressions.first() else {
        return Ok((Vec::new(), false));
    };

    validate_scale_order_expr(category_expr, order_expr)?;

    if !contains_aggregate(order_expr) {
        let mut values = distinct_categorical_values(data_expressions, eval_ctx, params).await?;
        if order_expr_matches_category(category_expr, order_expr) && order_descending {
            values.sort_by(|a, b| scalar_total_cmp(b, a));
            return Ok((values, true));
        }
        return Ok((values, false));
    }

    let order_column_names = order_expr_column_names(order_expr);
    let order_column_aliases = order_column_names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name.clone(),
                format!("{SCALE_ORDER_COLUMN_PREFIX}{index}__"),
            )
        })
        .collect::<HashMap<_, _>>();
    let rewritten_order_expr =
        rewrite_order_expr_columns(order_expr.clone(), &order_column_aliases)?;

    let mut projected_sources = Vec::with_capacity(data_expressions.len());
    for (df, category_expr) in data_expressions {
        let mut select_exprs = vec![category_expr.clone().alias(SCALE_ORDER_CATEGORY_COL)];
        for name in &order_column_names {
            let alias = order_column_aliases
                .get(name)
                .expect("order column alias missing");
            select_exprs.push(col(name.clone()).alias(alias));
        }
        let projected = df.as_ref().clone().select(select_exprs).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Scale order_by expression could not be evaluated for all contributing categorical domain data sources: {err}"
            ))
        })?;
        projected_sources.push(projected);
    }

    let mut sources = projected_sources.into_iter();
    let Some(mut ordered_rows) = sources.next() else {
        return Ok((Vec::new(), true));
    };
    for source in sources {
        ordered_rows = ordered_rows.union(source).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Scale order_by data sources must project compatible category and ordering column types: {err}"
            ))
        })?;
    }

    let ordered_df = ordered_rows
        .aggregate(
            vec![col(SCALE_ORDER_CATEGORY_COL)],
            vec![rewritten_order_expr.alias(SCALE_ORDER_VALUE_COL)],
        )
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Scale order_by expression must be a valid aggregate over the contributing domain rows: {err}"
            ))
        })?;

    eval_ctx.record_scale_domain_collect();
    let batches = if !params.is_empty() {
        if let Some(param_values) = params_to_datafusion(params) {
            ordered_df
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            ordered_df.collect().await?
        }
    } else {
        ordered_df.collect().await?
    };

    let mut keyed_values = Vec::new();
    for batch in &batches {
        let category_column = batch
            .column_by_name(SCALE_ORDER_CATEGORY_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Scale order_by category column not found".to_string(),
                )
            })?;
        let order_column = batch.column_by_name(SCALE_ORDER_VALUE_COL).ok_or_else(|| {
            AvengerChartError::InternalError("Scale order_by value column not found".to_string())
        })?;
        for row in 0..batch.num_rows() {
            let category = ScalarValue::try_from_array(category_column, row)?;
            if category.is_null() {
                continue;
            }
            let order_value = ScalarValue::try_from_array(order_column, row)?;
            keyed_values.push((category, order_value));
        }
    }

    keyed_values.sort_by(|(lhs_key, lhs_order), (rhs_key, rhs_order)| {
        let primary = scalar_total_cmp(lhs_order, rhs_order);
        let primary = if order_descending {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| scalar_total_cmp(lhs_key, rhs_key))
    });

    let mut values = Vec::with_capacity(keyed_values.len());
    for (key, _) in keyed_values {
        if !values.iter().any(|existing| existing == &key) {
            values.push(key);
        }
    }

    Ok((values, true))
}

fn validate_scale_order_expr(
    category_expr: &Expr,
    order_expr: &Expr,
) -> Result<(), AvengerChartError> {
    if contains_aggregate(order_expr)
        || !order_expr.any_column_refs()
        || order_expr_matches_category(category_expr, order_expr)
    {
        return Ok(());
    }

    Err(AvengerChartError::InvalidArgument(
        "Scale order_by expression must be an aggregate, literal/constant, or the scale category expression"
            .to_string(),
    ))
}

fn order_expr_matches_category(category_expr: &Expr, order_expr: &Expr) -> bool {
    order_expr == category_expr || order_expr.to_string() == category_expr.to_string()
}

fn order_expr_column_names(expr: &Expr) -> Vec<String> {
    let mut names = Vec::new();
    let _ = expr.apply(|candidate| {
        if let Expr::Column(column) = candidate
            && !names.iter().any(|name| name == &column.name)
        {
            names.push(column.name.clone());
        }
        Ok(TreeNodeRecursion::Continue)
    });
    names
}

fn rewrite_order_expr_columns(
    expr: Expr,
    aliases: &HashMap<String, String>,
) -> Result<Expr, AvengerChartError> {
    expr.transform(&|candidate| {
        if let Expr::Column(column) = &candidate
            && let Some(alias) = aliases.get(&column.name)
        {
            return Ok(Transformed::yes(col(alias.clone())));
        }

        Ok(Transformed::no(candidate))
    })
    .data()
    .map_err(AvengerChartError::DataFusionError)
}

struct NestedLevelDomainOrder {
    explicit_domain: Option<Vec<ScalarValue>>,
    ordered_components: Option<HashMap<Vec<ScalarValue>, Vec<ScalarValue>>>,
    descending: Option<bool>,
}

async fn apply_nested_level_domain_ordering(
    mut values: Vec<ScalarValue>,
    config: Option<&NestedBandSpec>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    eval_ctx: &CoreEvaluationContext,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    derived_scalars: &DerivedScalarMap,
) -> Result<(Vec<ScalarValue>, bool), AvengerChartError> {
    let Some(config) = config else {
        return Ok((values, false));
    };

    let field_names = nested_struct_field_names(&values);
    let mut level_orders = HashMap::new();
    for (level, level_config) in &config.levels {
        let explicit_domain = match level_config.domain.as_option() {
            Some(domain) => Some(
                resolve_nested_level_domain_values(*level, domain, ctx, params, derived_scalars)
                    .await?,
            ),
            None => None,
        };

        let mut descending = None;
        let mut ordered_components = None;
        if let Some(ordering) = level_config.ordering.as_option() {
            descending = ordering.order_descending;
            if let Some(order_expr_node) = ordering.order_expr.as_ref() {
                let order_expr = order_expr_node.to_expr(ctx)?;
                let field_names = field_names.as_ref().ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Nested-band level {level} is invalid for struct position data"
                    ))
                })?;
                let level_field_name = field_names.get(*level).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Nested-band level {level} is invalid for struct position data"
                    ))
                })?;
                if contains_aggregate(&order_expr) {
                    ordered_components = Some(
                        ordered_nested_level_components(
                            data_expressions,
                            config,
                            field_names,
                            *level,
                            &order_expr,
                            ordering.order_descending(),
                            eval_ctx,
                            params,
                        )
                        .await?,
                    );
                } else if order_expr.any_column_refs()
                    && !nested_level_order_expr_matches_component(
                        level_field_name,
                        data_expressions.first().map(|(_, expr)| expr),
                        &order_expr,
                    )
                {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Nested-band level {level} order_by expression must be an aggregate, literal/constant, or the level field expression"
                    )));
                }
            }
        }

        if explicit_domain.is_some() || ordered_components.is_some() || descending.is_some() {
            level_orders.insert(
                *level,
                NestedLevelDomainOrder {
                    explicit_domain,
                    ordered_components,
                    descending,
                },
            );
        }
    }

    if level_orders.is_empty() {
        return Ok((values, false));
    }

    values = expand_nested_level_domain_paths(values, config, &level_orders)?;

    let max_depth = values
        .iter()
        .filter_map(nested_struct_depth)
        .max()
        .unwrap_or(0);
    values.sort_by(|lhs, rhs| compare_nested_struct_paths(lhs, rhs, max_depth, &level_orders));
    Ok((values, true))
}

async fn apply_nested_level_labels(
    values: Vec<ScalarValue>,
    config: Option<&NestedBandSpec>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    eval_ctx: &CoreEvaluationContext,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    derived_scalars: &DerivedScalarMap,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    let Some(config) = config else {
        return Ok(values);
    };
    let labeled_levels = config
        .levels
        .iter()
        .filter(|(_, level_config)| level_config.label_expr.is_some())
        .map(|(level, _)| *level)
        .collect::<HashSet<_>>();
    if labeled_levels.is_empty() || values.is_empty() {
        return Ok(values);
    }

    let Some(field_names) = nested_struct_field_names(&values) else {
        return Ok(values);
    };
    let mut labels = HashMap::new();
    for level in &labeled_levels {
        let level_labels = collect_nested_level_labels(
            data_expressions,
            config,
            &field_names,
            *level,
            eval_ctx,
            ctx,
            params,
            derived_scalars,
        )
        .await?;
        labels.extend(level_labels);
    }

    values
        .into_iter()
        .map(|value| {
            label_nested_struct_domain_value(value, config, &field_names, &labeled_levels, &labels)
        })
        .collect()
}

async fn collect_nested_level_labels(
    data_expressions: &[(Arc<DataFrame>, Expr)],
    config: &NestedBandSpec,
    field_names: &[String],
    level: usize,
    eval_ctx: &CoreEvaluationContext,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    derived_scalars: &DerivedScalarMap,
) -> Result<HashMap<(usize, Vec<ScalarValue>, ScalarValue), String>, AvengerChartError> {
    let Some(label_expr_node) = config
        .level(level)
        .and_then(|level| level.label_expr.as_ref())
    else {
        return Ok(HashMap::new());
    };
    let label_expr = resolve_derived_scalars(label_expr_node.to_expr(ctx)?, derived_scalars)?;
    let prefix_len = nested_level_scope_prefix_len(config, level);
    let level_field_name = field_names.get(level).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "Nested-band level {level} is invalid for struct position data"
        ))
    })?;

    let mut projected_sources = Vec::with_capacity(data_expressions.len());
    for (df, nested_expr) in data_expressions {
        let mut select_exprs = Vec::new();
        for prefix_level in 0..prefix_len {
            select_exprs.push(
                get_field(nested_expr.clone(), field_names[prefix_level].clone())
                    .alias(nested_label_prefix_col(prefix_level)),
            );
        }
        select_exprs.push(
            get_field(nested_expr.clone(), level_field_name.clone())
                .alias(NESTED_LABEL_COMPONENT_COL),
        );
        select_exprs.push(label_expr.clone().alias(NESTED_LABEL_VALUE_COL));

        let projected = df.as_ref().clone().select(select_exprs).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} label_with expression could not be evaluated for all contributing domain data sources: {err}"
            ))
        })?;
        projected_sources.push(projected);
    }

    let mut sources = projected_sources.into_iter();
    let Some(mut label_rows) = sources.next() else {
        return Ok(HashMap::new());
    };
    for source in sources {
        label_rows = label_rows.union(source).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} label_with data sources must project compatible component and label column types: {err}"
            ))
        })?;
    }

    eval_ctx.record_scale_domain_collect();
    let batches = if !params.is_empty() {
        if let Some(param_values) = params_to_datafusion(params) {
            label_rows
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            label_rows.collect().await?
        }
    } else {
        label_rows.collect().await?
    };

    let mut labels = HashMap::new();
    for batch in &batches {
        let prefix_columns = (0..prefix_len)
            .map(|prefix_level| {
                batch
                    .column_by_name(&nested_label_prefix_col(prefix_level))
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Nested-band level {level} label prefix column {prefix_level} not found"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let component_column = batch
            .column_by_name(NESTED_LABEL_COMPONENT_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Nested-band level {level} label component column not found"
                ))
            })?;
        let label_column = batch
            .column_by_name(NESTED_LABEL_VALUE_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Nested-band level {level} label value column not found"
                ))
            })?;

        for row in 0..batch.num_rows() {
            let prefix = prefix_columns
                .iter()
                .map(|column| ScalarValue::try_from_array(*column, row))
                .collect::<Result<Vec<_>, _>>()?;
            let component = ScalarValue::try_from_array(component_column, row)?;
            let label_value = ScalarValue::try_from_array(label_column, row)?;
            if label_value.is_null() {
                continue;
            }
            let label = label_value.as_scalar_string()?;
            let key = (level, prefix, component);
            if let Some(existing) = labels.get(&key) {
                if existing != &label {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Nested-band level {level} label_with expression produced multiple labels for the same domain key"
                    )));
                }
            } else {
                labels.insert(key, label);
            }
        }
    }

    Ok(labels)
}

fn label_nested_struct_domain_value(
    value: ScalarValue,
    config: &NestedBandSpec,
    field_names: &[String],
    labeled_levels: &HashSet<usize>,
    labels: &HashMap<(usize, Vec<ScalarValue>, ScalarValue), String>,
) -> Result<ScalarValue, AvengerChartError> {
    let Some(components) = nested_struct_components(&value) else {
        return Ok(value);
    };
    let labeled_components = components
        .iter()
        .enumerate()
        .map(|(level, component)| {
            if !labeled_levels.contains(&level) {
                return Ok(component.clone());
            }
            let prefix_len = nested_level_scope_prefix_len(config, level);
            let prefix = components[..prefix_len].to_vec();
            let label = if let Some(label) = labels.get(&(level, prefix, component.clone())) {
                label.clone()
            } else {
                nested_component_default_label(component)?
            };
            nested_labeled_component_scalar(component.clone(), label)
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    nested_struct_scalar(field_names, &labeled_components)
}

fn nested_level_scope_prefix_len(config: &NestedBandSpec, level: usize) -> usize {
    let scope = config
        .level(level)
        .and_then(|level_config| level_config.nest_scope)
        .unwrap_or(NestScope::Free);
    if level > 0 && scope == NestScope::Shared {
        0
    } else {
        level
    }
}

fn nested_label_prefix_col(level: usize) -> String {
    format!("{NESTED_LABEL_PREFIX_COL_PREFIX}{level}__")
}

fn nested_component_default_label(component: &ScalarValue) -> Result<String, AvengerChartError> {
    if component.is_null() {
        Ok("null".to_string())
    } else {
        component
            .as_scalar_string()
            .map_err(AvengerChartError::DataFusionError)
    }
}

fn nested_labeled_component_scalar(
    key: ScalarValue,
    label: String,
) -> Result<ScalarValue, AvengerChartError> {
    let key_array = ScalarValue::iter_to_array(std::iter::once(key)).map_err(|err| {
        AvengerChartError::InternalError(format!(
            "Failed to build nested-band label key component: {err}"
        ))
    })?;
    let label_array = ScalarValue::iter_to_array(std::iter::once(ScalarValue::Utf8(Some(label))))
        .map_err(|err| {
        AvengerChartError::InternalError(format!(
            "Failed to build nested-band label component: {err}"
        ))
    })?;
    Ok(ScalarValue::Struct(Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("key", key_array.data_type().clone(), true)),
            key_array,
        ),
        (
            Arc::new(Field::new("label", label_array.data_type().clone(), true)),
            label_array,
        ),
    ]))))
}

async fn resolve_nested_level_domain_values(
    level: usize,
    domain: &ScaleDomain,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    derived_scalars: &DerivedScalarMap,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    match &domain.default_domain {
        ScaleDefaultDomain::Discrete(values) => {
            let exprs = values
                .iter()
                .map(|node| resolve_derived_scalars(node.to_expr(ctx)?, derived_scalars))
                .collect::<Result<Vec<_>, _>>()?;
            let datafusion_params = params_to_datafusion(params);
            eval_to_scalars(exprs, Some(ctx), datafusion_params.as_ref())
                .await
                .map_err(AvengerChartError::DataFusionError)
        }
        ScaleDefaultDomain::NoDefault if domain.raw_domain.is_none() => Ok(Vec::new()),
        ScaleDefaultDomain::NoDefault => Err(AvengerChartError::InvalidArgument(format!(
            "Nested-band level {level} domain must be discrete values"
        ))),
        ScaleDefaultDomain::Interval(_, _) | ScaleDefaultDomain::DomainExprs(_) => {
            Err(AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} domain must be discrete values"
            )))
        }
    }
}

async fn ordered_nested_level_components(
    data_expressions: &[(Arc<DataFrame>, Expr)],
    config: &NestedBandSpec,
    field_names: &[String],
    level: usize,
    order_expr: &Expr,
    order_descending: bool,
    eval_ctx: &CoreEvaluationContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<Vec<ScalarValue>, Vec<ScalarValue>>, AvengerChartError> {
    if data_expressions.is_empty() {
        return Ok(HashMap::new());
    }

    let scope = config
        .level(level)
        .and_then(|level_config| level_config.nest_scope)
        .unwrap_or(NestScope::Free);
    let prefix_len = if level > 0 && scope == NestScope::Shared {
        0
    } else {
        level
    };

    let order_column_names = order_expr_column_names(order_expr);
    let order_column_aliases = order_column_names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name.clone(),
                format!("{SCALE_ORDER_COLUMN_PREFIX}{index}__"),
            )
        })
        .collect::<HashMap<_, _>>();
    let rewritten_order_expr =
        rewrite_order_expr_columns(order_expr.clone(), &order_column_aliases)?;

    let mut projected_sources = Vec::with_capacity(data_expressions.len());
    for (df, nested_expr) in data_expressions {
        let mut select_exprs = Vec::new();
        for prefix_level in 0..prefix_len {
            select_exprs.push(
                get_field(nested_expr.clone(), field_names[prefix_level].clone())
                    .alias(nested_order_prefix_col(prefix_level)),
            );
        }
        select_exprs.push(
            get_field(nested_expr.clone(), field_names[level].clone())
                .alias(NESTED_ORDER_COMPONENT_COL),
        );
        for name in &order_column_names {
            let alias = order_column_aliases
                .get(name)
                .expect("order column alias missing");
            select_exprs.push(col(name.clone()).alias(alias));
        }

        let projected = df.as_ref().clone().select(select_exprs).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} order_by expression could not be evaluated for all contributing domain data sources: {err}"
            ))
        })?;
        projected_sources.push(projected);
    }

    let mut sources = projected_sources.into_iter();
    let Some(mut ordered_rows) = sources.next() else {
        return Ok(HashMap::new());
    };
    for source in sources {
        ordered_rows = ordered_rows.union(source).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} order_by data sources must project compatible component and ordering column types: {err}"
            ))
        })?;
    }

    let mut group_exprs = (0..prefix_len)
        .map(|prefix_level| col(nested_order_prefix_col(prefix_level)))
        .collect::<Vec<_>>();
    group_exprs.push(col(NESTED_ORDER_COMPONENT_COL));

    let ordered_df = ordered_rows
        .aggregate(
            group_exprs,
            vec![rewritten_order_expr.alias(NESTED_ORDER_VALUE_COL)],
        )
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Nested-band level {level} order_by expression must be a valid aggregate over the contributing domain rows: {err}"
            ))
        })?;

    eval_ctx.record_scale_domain_collect();
    let batches = if !params.is_empty() {
        if let Some(param_values) = params_to_datafusion(params) {
            ordered_df
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            ordered_df.collect().await?
        }
    } else {
        ordered_df.collect().await?
    };

    let mut grouped_values: HashMap<Vec<ScalarValue>, Vec<(ScalarValue, ScalarValue)>> =
        HashMap::new();
    for batch in &batches {
        let prefix_columns = (0..prefix_len)
            .map(|prefix_level| {
                batch
                    .column_by_name(&nested_order_prefix_col(prefix_level))
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Nested-band level {level} order_by prefix column {prefix_level} not found"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let component_column = batch
            .column_by_name(NESTED_ORDER_COMPONENT_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Nested-band level {level} order_by component column not found"
                ))
            })?;
        let order_column = batch
            .column_by_name(NESTED_ORDER_VALUE_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Nested-band level {level} order_by value column not found"
                ))
            })?;

        for row in 0..batch.num_rows() {
            let prefix = prefix_columns
                .iter()
                .map(|column| ScalarValue::try_from_array(*column, row))
                .collect::<Result<Vec<_>, _>>()?;
            let component = ScalarValue::try_from_array(component_column, row)?;
            let order_value = ScalarValue::try_from_array(order_column, row)?;
            grouped_values
                .entry(prefix)
                .or_default()
                .push((component, order_value));
        }
    }

    let mut ordered_components = HashMap::new();
    for (prefix, mut keyed_components) in grouped_values {
        keyed_components.sort_by(|(lhs_component, lhs_order), (rhs_component, rhs_order)| {
            let primary = scalar_total_cmp(lhs_order, rhs_order);
            let primary = if order_descending {
                primary.reverse()
            } else {
                primary
            };
            primary.then_with(|| scalar_total_cmp(lhs_component, rhs_component))
        });

        let mut components = Vec::with_capacity(keyed_components.len());
        append_unique_scalars(
            &mut components,
            keyed_components.into_iter().map(|(component, _)| component),
        );
        ordered_components.insert(prefix, components);
    }

    Ok(ordered_components)
}

fn nested_order_prefix_col(level: usize) -> String {
    format!("{NESTED_ORDER_PREFIX_COL_PREFIX}{level}__")
}

fn nested_level_order_expr_matches_component(
    level_field_name: &str,
    nested_expr: Option<&Expr>,
    order_expr: &Expr,
) -> bool {
    if let Expr::Column(column) = order_expr
        && column.name == level_field_name
    {
        return true;
    }

    nested_expr
        .map(|nested_expr| {
            let component_expr = get_field(nested_expr.clone(), level_field_name.to_string());
            order_expr_matches_category(&component_expr, order_expr)
        })
        .unwrap_or(false)
}

fn expand_nested_level_domain_paths(
    values: Vec<ScalarValue>,
    config: &NestedBandSpec,
    level_orders: &HashMap<usize, NestedLevelDomainOrder>,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    if !level_orders
        .values()
        .any(|order| order.explicit_domain.is_some())
    {
        return Ok(values);
    }
    let Some(field_names) = nested_struct_field_names(&values) else {
        return Ok(values);
    };
    let depth = field_names.len();
    if depth == 0 {
        return Ok(values);
    }

    let existing_paths = values
        .iter()
        .filter_map(nested_struct_components)
        .filter(|components| components.len() == depth)
        .collect::<Vec<_>>();
    if existing_paths.is_empty() {
        return Ok(values);
    }

    let mut component_paths = Vec::new();
    collect_expanded_nested_component_paths(
        &mut Vec::new(),
        0,
        depth,
        config,
        level_orders,
        &existing_paths,
        &mut component_paths,
    );
    if component_paths.is_empty() {
        return Ok(values);
    }

    let mut expanded = component_paths
        .iter()
        .map(|components| nested_struct_scalar(&field_names, components))
        .collect::<Result<Vec<_>, _>>()?;
    for value in values {
        if !expanded.iter().any(|existing| existing == &value) {
            expanded.push(value);
        }
    }
    Ok(expanded)
}

fn collect_expanded_nested_component_paths(
    prefix: &mut Vec<ScalarValue>,
    level: usize,
    depth: usize,
    config: &NestedBandSpec,
    level_orders: &HashMap<usize, NestedLevelDomainOrder>,
    existing_paths: &[Vec<ScalarValue>],
    output: &mut Vec<Vec<ScalarValue>>,
) {
    if level == depth {
        output.push(prefix.clone());
        return;
    }

    for component in
        nested_level_component_candidates(prefix, level, config, level_orders, existing_paths)
    {
        prefix.push(component);
        collect_expanded_nested_component_paths(
            prefix,
            level + 1,
            depth,
            config,
            level_orders,
            existing_paths,
            output,
        );
        prefix.pop();
    }
}

fn nested_level_component_candidates(
    prefix: &[ScalarValue],
    level: usize,
    config: &NestedBandSpec,
    level_orders: &HashMap<usize, NestedLevelDomainOrder>,
    existing_paths: &[Vec<ScalarValue>],
) -> Vec<ScalarValue> {
    let order = level_orders.get(&level);
    let explicit_domain = order.and_then(|order| order.explicit_domain.as_ref());
    let scope = config
        .level(level)
        .and_then(|level_config| level_config.nest_scope)
        .unwrap_or(NestScope::Free);
    let expands_explicit_domain =
        explicit_domain.is_some() && (level == 0 || scope == NestScope::Shared);
    let collect_existing_globally =
        expands_explicit_domain && level > 0 && scope == NestScope::Shared;

    let existing = collect_existing_nested_components(
        existing_paths,
        prefix,
        level,
        collect_existing_globally,
    );

    let mut candidates = Vec::new();
    if expands_explicit_domain && let Some(explicit_domain) = explicit_domain {
        append_unique_scalars(&mut candidates, explicit_domain.iter().cloned());
    }
    append_unique_scalars(&mut candidates, existing);

    candidates.sort_by(|lhs, rhs| {
        if let Some(order) = order {
            order.compare(prefix, Some(lhs), Some(rhs))
        } else {
            scalar_total_cmp(lhs, rhs)
        }
    });
    dedup_scalars(candidates)
}

fn collect_existing_nested_components(
    existing_paths: &[Vec<ScalarValue>],
    prefix: &[ScalarValue],
    level: usize,
    collect_globally: bool,
) -> Vec<ScalarValue> {
    let mut components = Vec::new();
    for path in existing_paths {
        if path.len() <= level {
            continue;
        }
        if !collect_globally && !path_prefix_matches(path, prefix) {
            continue;
        }
        append_unique_scalars(&mut components, std::iter::once(path[level].clone()));
    }
    components
}

fn path_prefix_matches(path: &[ScalarValue], prefix: &[ScalarValue]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix)
            .all(|(component, expected)| component == expected)
}

fn append_unique_scalars(
    target: &mut Vec<ScalarValue>,
    values: impl IntoIterator<Item = ScalarValue>,
) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

fn dedup_scalars(values: Vec<ScalarValue>) -> Vec<ScalarValue> {
    let mut unique = Vec::with_capacity(values.len());
    append_unique_scalars(&mut unique, values);
    unique
}

fn nested_struct_field_names(values: &[ScalarValue]) -> Option<Vec<String>> {
    values.iter().find_map(|value| {
        let ScalarValue::Struct(array) = value else {
            return None;
        };
        Some(
            array
                .fields()
                .iter()
                .map(|field| field.name().to_string())
                .collect(),
        )
    })
}

fn nested_struct_depth(value: &ScalarValue) -> Option<usize> {
    match value {
        ScalarValue::Struct(array) if array.len() > 0 && !array.is_null(0) => {
            Some(array.num_columns())
        }
        ScalarValue::Struct(_) => Some(0),
        _ => None,
    }
}

fn nested_struct_components(value: &ScalarValue) -> Option<Vec<ScalarValue>> {
    let ScalarValue::Struct(array) = value else {
        return None;
    };
    if array.len() == 0 || array.is_null(0) {
        return None;
    }
    (0..array.num_columns())
        .map(|level| ScalarValue::try_from_array(array.column(level), 0).ok())
        .collect()
}

fn nested_struct_scalar(
    field_names: &[String],
    components: &[ScalarValue],
) -> Result<ScalarValue, AvengerChartError> {
    let fields = field_names
        .iter()
        .zip(components)
        .map(|(name, component)| {
            let array =
                ScalarValue::iter_to_array(std::iter::once(component.clone())).map_err(|err| {
                    AvengerChartError::InternalError(format!(
                        "Failed to build nested-band domain component: {err}"
                    ))
                })?;
            Ok((
                Arc::new(Field::new(name, array.data_type().clone(), true)),
                array,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    Ok(ScalarValue::Struct(Arc::new(StructArray::from(fields))))
}

fn compare_nested_struct_paths(
    lhs: &ScalarValue,
    rhs: &ScalarValue,
    max_depth: usize,
    level_orders: &HashMap<usize, NestedLevelDomainOrder>,
) -> Ordering {
    let lhs_components = nested_struct_components(lhs).unwrap_or_default();
    let rhs_components = nested_struct_components(rhs).unwrap_or_default();

    for level in 0..max_depth {
        let lhs_component = lhs_components.get(level);
        let rhs_component = rhs_components.get(level);
        let common_prefix = level <= lhs_components.len()
            && level <= rhs_components.len()
            && lhs_components[..level] == rhs_components[..level];
        let prefix = if common_prefix {
            &lhs_components[..level]
        } else {
            &[] as &[ScalarValue]
        };
        let cmp = if let Some(order) = level_orders.get(&level) {
            order.compare(prefix, lhs_component, rhs_component)
        } else {
            compare_optional_scalars(lhs_component, rhs_component)
        };
        if !cmp.is_eq() {
            return cmp;
        }
    }
    scalar_total_cmp(lhs, rhs)
}

impl NestedLevelDomainOrder {
    fn compare(
        &self,
        prefix: &[ScalarValue],
        lhs: Option<&ScalarValue>,
        rhs: Option<&ScalarValue>,
    ) -> Ordering {
        if let Some(domain) = &self.explicit_domain {
            let lhs_rank = lhs.and_then(|value| domain.iter().position(|known| known == value));
            let rhs_rank = rhs.and_then(|value| domain.iter().position(|known| known == value));
            match (lhs_rank, rhs_rank) {
                (Some(lhs_rank), Some(rhs_rank)) => {
                    let cmp = lhs_rank.cmp(&rhs_rank);
                    if !cmp.is_eq() {
                        return cmp;
                    }
                }
                (Some(_), None) => return Ordering::Less,
                (None, Some(_)) => return Ordering::Greater,
                (None, None) => {}
            }
        }

        if let Some(domain) = self.ordered_components_for_prefix(prefix) {
            let lhs_rank = lhs.and_then(|value| domain.iter().position(|known| known == value));
            let rhs_rank = rhs.and_then(|value| domain.iter().position(|known| known == value));
            match (lhs_rank, rhs_rank) {
                (Some(lhs_rank), Some(rhs_rank)) => {
                    let cmp = lhs_rank.cmp(&rhs_rank);
                    if !cmp.is_eq() {
                        return cmp;
                    }
                }
                (Some(_), None) => return Ordering::Less,
                (None, Some(_)) => return Ordering::Greater,
                (None, None) => {}
            }
        }

        let cmp = compare_optional_scalars(lhs, rhs);
        if self.descending.unwrap_or(false) {
            cmp.reverse()
        } else {
            cmp
        }
    }

    fn ordered_components_for_prefix(&self, prefix: &[ScalarValue]) -> Option<&Vec<ScalarValue>> {
        let domains = self.ordered_components.as_ref()?;
        domains.get(prefix).or_else(|| {
            (!prefix.is_empty())
                .then(|| domains.get(&[] as &[ScalarValue]))
                .flatten()
        })
    }
}

fn compare_optional_scalars(lhs: Option<&ScalarValue>, rhs: Option<&ScalarValue>) -> Ordering {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => scalar_total_cmp(lhs, rhs),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

async fn cache_categorical_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    ordering: Option<&ScaleOrderingSpec>,
    nested_band_config: Option<&NestedBandSpec>,
    eval_ctx: &CoreEvaluationContext,
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
    derived_scalars: DerivedScalarMap,
) -> Result<(), AvengerChartError> {
    let ordering_expr = ordering.and_then(|ordering| ordering.order_expr.as_ref());
    let (all_unique_values, ordered) = if let Some(order_expr_node) = ordering_expr {
        let order_expr = order_expr_node.to_expr(_ctx)?;
        ordered_categorical_values(
            data_expressions,
            &order_expr,
            ordering
                .map(ScaleOrderingSpec::order_descending)
                .unwrap_or(false),
            eval_ctx,
            params,
        )
        .await?
    } else {
        (
            distinct_categorical_values(data_expressions, eval_ctx, params).await?,
            false,
        )
    };
    let (all_unique_values, nested_ordered) = apply_nested_level_domain_ordering(
        all_unique_values,
        nested_band_config,
        data_expressions,
        eval_ctx,
        _ctx,
        params,
        &derived_scalars,
    )
    .await?;
    let ordered = ordered || nested_ordered;
    let all_unique_values = apply_nested_level_labels(
        all_unique_values,
        nested_band_config,
        data_expressions,
        eval_ctx,
        _ctx,
        params,
        &derived_scalars,
    )
    .await?;

    if !all_unique_values.is_empty() {
        if let Some(config) = nested_band_config {
            trace!(
                channel,
                configured_levels = config.levels.len(),
                path_count = all_unique_values.len(),
                ordered,
                "cached nested-band categorical domain"
            );
        }

        let extents = if ordered {
            DataExtents::OrderedDiscrete(all_unique_values)
        } else {
            DataExtents::Discrete(all_unique_values)
        };
        builder.add_standard(
            channel.to_string(),
            spec.clone_box(),
            extents,
            options,
            derived_scalars,
        );

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
    eval_ctx: &CoreEvaluationContext,
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
    derived_scalars: DerivedScalarMap,
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

        eval_ctx.record_scale_domain_collect();
        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                agg_df.with_param_values(param_values)?.collect().await?
            } else {
                agg_df.collect().await?
            }
        } else {
            agg_df.collect().await?
        };

        for batch in &batches {
            for row in 0..batch.num_rows() {
                let min_scalar = ScalarValue::try_from_array(batch.column(0), row)?;
                let max_scalar = ScalarValue::try_from_array(batch.column(1), row)?;

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
    }

    if let (Some(min_ts), Some(max_ts)) = (global_min_ts, global_max_ts) {
        let extents = DataExtents::Temporal(min_ts, max_ts);
        builder.add_standard(
            channel.to_string(),
            spec.clone_box(),
            extents,
            options,
            derived_scalars,
        );
        builder.set_channel_data_type(channel.to_string(), dt.clone());
    }

    Ok(())
}

fn update_numeric_extent_from_array(
    array: &ArrayRef,
    min_val: &mut Option<f64>,
    max_val: &mut Option<f64>,
) -> Result<(), AvengerChartError> {
    match array.data_type() {
        ArrowDataType::List(_) => {
            let list_array = array.as_list::<i32>();
            for row in 0..list_array.len() {
                if list_array.is_null(row) {
                    continue;
                }
                update_numeric_extent_from_array(&list_array.value(row), min_val, max_val)?;
            }
        }
        data_type => {
            for row in 0..array.len() {
                if array.is_null(row) {
                    continue;
                }
                let value = array_value_to_f64(array, row, data_type)?;
                if !value.is_finite() {
                    continue;
                }
                *min_val = Some(min_val.map_or(value, |current| current.min(value)));
                *max_val = Some(max_val.map_or(value, |current| current.max(value)));
            }
        }
    }

    Ok(())
}

async fn cache_numeric_data(
    channel: &str,
    spec: &Box<dyn ScaleSpec>,
    options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    data_expressions: &[(Arc<DataFrame>, Expr)],
    eval_ctx: &CoreEvaluationContext,
    _ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    builder: &mut ScaleBuilder,
    dt: &ArrowDataType,
    derived_scalars: DerivedScalarMap,
) -> Result<(), AvengerChartError> {
    let mut global_min_val: Option<f64> = None;
    let mut global_max_val: Option<f64> = None;

    for (df, expr) in data_expressions {
        let projected = df
            .as_ref()
            .clone()
            .select(vec![expr.clone().alias("value")])?;
        let projected_type = projected.schema().field(0).data_type().clone();

        if matches!(projected_type, ArrowDataType::List(_)) {
            eval_ctx.record_scale_domain_collect();
            let batches = if !params.is_empty() {
                if let Some(param_values) = params_to_datafusion(params) {
                    projected.with_param_values(param_values)?.collect().await?
                } else {
                    projected.collect().await?
                }
            } else {
                projected.collect().await?
            };

            for batch in &batches {
                update_numeric_extent_from_array(
                    batch.column(0),
                    &mut global_min_val,
                    &mut global_max_val,
                )?;
            }
            continue;
        }

        let agg_df = projected.aggregate(
            vec![],
            vec![
                min(col("value")).alias("min"),
                max(col("value")).alias("max"),
            ],
        )?;

        eval_ctx.record_scale_domain_collect();
        let batches = if !params.is_empty() {
            if let Some(param_values) = params_to_datafusion(params) {
                agg_df.with_param_values(param_values)?.collect().await?
            } else {
                agg_df.collect().await?
            }
        } else {
            agg_df.collect().await?
        };

        for batch in &batches {
            let min_col = batch.column(0);
            let max_col = batch.column(1);

            for row in 0..batch.num_rows() {
                // Skip if min/max are NULL (all values were NULL from conditional literals)
                if min_col.is_null(row) || max_col.is_null(row) {
                    continue;
                }

                let scalar_type = scale_input_data_type(&projected_type);
                let min_val = array_value_to_f64(min_col, row, &scalar_type)?;
                let max_val = array_value_to_f64(max_col, row, &scalar_type)?;
                if !min_val.is_finite() || !max_val.is_finite() {
                    continue;
                }

                global_min_val =
                    Some(global_min_val.map_or(min_val, |current| current.min(min_val)));
                global_max_val =
                    Some(global_max_val.map_or(max_val, |current| current.max(max_val)));
            }
        }
    }

    let extents = match (global_min_val, global_max_val) {
        (Some(min_val), Some(max_val)) => DataExtents::Interval(min_val, max_val),
        _ if !data_expressions.is_empty() => DataExtents::Interval(0.0, 1.0),
        _ => return Ok(()),
    };
    builder.add_standard(
        channel.to_string(),
        spec.clone_box(),
        extents,
        options,
        derived_scalars,
    );
    builder.set_channel_data_type(channel.to_string(), dt.clone());

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use datafusion::{
        arrow::{
            array::{
                Array, ArrayRef, Float32Array, Float64Array, ListArray, StringArray, StructArray,
            },
            datatypes::{DataType, Field, Float64Type, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::{expr_fn::sum, min_max::max},
        prelude::{SessionContext, col, lit, named_struct},
    };
    use indexmap::IndexMap;

    use super::*;
    use crate::{Band, Linear, NestedBand, Point};
    use avenger_chart_core::{
        ChannelDescriptor, ChannelExpr, CompiledDataContext, CompiledMarkCore, CompiledMarkState,
        ConfiguredScaleLegendExt, CoordinationScope, DomainCoordinationGroup, DomainValues,
        MarkDataMode, MarkRuntimeContext, NestedBandLevelSpec, PatternChannelValue, PlotGeometry,
        RepeatContext, ResolvedDomain, ResolvedRepeatVariable, ScaleChannelValue,
        ScaleInferenceHint, ScaleRange, ScaleRangeBinding, ScaleTypePreference, SubplotGeometry,
        default_scale_type_for_data_type, nested, repeat::column_name, resolve_repeat_channel_expr,
    };
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::marks::mark::SceneMark;
    use avenger_scenegraph::marks::pattern::PatternFill;

    struct TestCoordTransform;

    impl CoordinateSystemTransformCore for TestCoordTransform {
        fn required_channels(&self) -> &'static [&'static str] {
            &["x", "y"]
        }

        fn transform(
            &self,
            _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
            _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
            _plot_width: f32,
            _plot_height: f32,
        ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
            Ok(Box::new(SubplotGeometry::default()))
        }

        fn default_scale_options(
            &self,
            _channel: &str,
            _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        ) -> HashMap<String, ScalarValue> {
            HashMap::new()
        }
    }

    struct DynamicPositionCoordTransform;

    impl CoordinateSystemTransformCore for DynamicPositionCoordTransform {
        fn required_channels(&self) -> &'static [&'static str] {
            &[]
        }

        fn channel_uses_scale(&self, channel: &str) -> bool {
            channel.starts_with("dim_")
        }

        fn is_position_scale_channel(&self, channel: &str) -> bool {
            channel.starts_with("dim_")
        }

        fn transform(
            &self,
            _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
            _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
            _plot_width: f32,
            _plot_height: f32,
        ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
            Ok(Box::new(SubplotGeometry::default()))
        }

        fn default_scale_options(
            &self,
            _channel: &str,
            _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        ) -> HashMap<String, ScalarValue> {
            HashMap::new()
        }
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct TestCompiledMark {
        state: CompiledMarkState,
        position_preference: Option<ScaleTypePreference>,
    }

    impl TestCompiledMark {
        fn new(dataframe: DataFrame, channels: IndexMap<String, ChannelValue>) -> Self {
            Self::new_with_position_preference(dataframe, channels, None)
        }

        fn new_with_position_preference(
            dataframe: DataFrame,
            channels: IndexMap<String, ChannelValue>,
            position_preference: Option<ScaleTypePreference>,
        ) -> Self {
            Self {
                state: CompiledMarkState {
                    id: None,
                    data: CompiledDataContext::new(Some(dataframe), Vec::new(), channels),
                    view: None,
                    data_mode: MarkDataMode::Inherit,
                    mark_index: 0,
                    public_target_path: None,
                    facet_data_scope: Default::default(),
                    exclude_from_scale_domains: false,
                    visible: None,
                    details: None,
                    zindex: None,
                    geometry_space: None,
                    axis_configs: HashMap::new(),
                },
                position_preference,
            }
        }
    }

    impl CompiledMarkCore for TestCompiledMark {
        fn state(&self) -> &CompiledMarkState {
            &self.state
        }

        fn state_mut(&mut self) -> &mut CompiledMarkState {
            &mut self.state
        }

        fn data_context(&self) -> &CompiledDataContext {
            &self.state.data
        }

        fn mark_type(&self) -> &str {
            "test"
        }

        fn supported_channels(&self) -> Vec<ChannelDescriptor> {
            ["x", "x2", "y", "y2"]
                .into_iter()
                .map(|name| ChannelDescriptor {
                    name,
                    required: false,
                    default_value: None,
                    allow_column_ref: true,
                })
                .collect()
        }

        fn preferred_scale_type(
            &self,
            channel: &str,
            data_type: &DataType,
        ) -> Option<ScaleTypePreference> {
            if matches!(channel, "x" | "x2" | "y" | "y2")
                && let Some(preference) = self.position_preference
            {
                return Some(preference);
            }
            if matches!(channel, "x" | "x2" | "y" | "y2")
                && matches!(data_type, DataType::Struct(_))
            {
                Some(ScaleTypePreference::NestedBand)
            } else {
                default_scale_type_for_data_type(data_type)
            }
        }
    }

    #[typetag::serde]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    impl CompiledMark for TestCompiledMark {
        async fn render_from_data(
            &self,
            _data: Option<&RecordBatch>,
            _scalars: &RecordBatch,
            _context: &dyn MarkRuntimeContext,
            _coord: &dyn CoordinateSystemTransformCore,
        ) -> Result<Vec<SceneMark>, AvengerChartError> {
            Ok(Vec::new())
        }
    }

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn nested_path(group: &str, member: &str) -> ScalarValue {
        ScalarValue::Struct(Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("group", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![group])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("member", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![member])) as ArrayRef,
            ),
        ])))
    }

    fn nested_path_labels(values: &[ScalarValue]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|value| {
                let components = nested_struct_components(value)
                    .unwrap_or_else(|| panic!("unexpected nested path: {value:?}"));
                let group = match components.first().cloned() {
                    Some(ScalarValue::Utf8(Some(value))) => value,
                    other => panic!("unexpected group component: {other:?}"),
                };
                let member = match components.get(1).cloned() {
                    Some(ScalarValue::Utf8(Some(value))) => value,
                    other => panic!("unexpected member component: {other:?}"),
                };
                (group, member)
            })
            .collect()
    }

    fn nested_expr() -> Expr {
        named_struct(vec![
            lit("group"),
            col("group"),
            lit("member"),
            col("member"),
        ])
    }

    fn nested_domain_array(values: Vec<ScalarValue>) -> ArrayRef {
        ScalarValue::iter_to_array(values.into_iter()).expect("nested domain array")
    }

    fn scaled_positions(
        scale: &avenger_scales::scales::ConfiguredScale,
        values: Vec<ScalarValue>,
    ) -> Vec<Option<f32>> {
        let values = nested_domain_array(values);
        let scaled = scale.scale(&values).expect("scale");
        let scaled = scaled.as_any().downcast_ref::<Float32Array>().unwrap();
        (0..scaled.len())
            .map(|index| {
                if scaled.is_null(index) {
                    None
                } else {
                    Some(scaled.value(index))
                }
            })
            .collect()
    }

    fn discrete_domain(scale: &ConfiguredScaleWithSpec) -> Vec<ScalarValue> {
        match scale.domain_values().expect("domain values") {
            DomainValues::Discrete(values) => values,
            other => panic!("expected discrete domain, got {other:?}"),
        }
    }

    fn nested_component_display_label(value: &ScalarValue, field_name: &str) -> String {
        let ScalarValue::Struct(path) = value else {
            panic!("expected nested struct path");
        };
        let component_column = path.column_by_name(field_name).expect("component field");
        let component = ScalarValue::try_from_array(component_column, 0).expect("component scalar");
        let ScalarValue::Struct(component) = component else {
            panic!("expected labeled component struct");
        };
        let label = component.column_by_name("label").expect("label field");
        match ScalarValue::try_from_array(label, 0).expect("label scalar") {
            ScalarValue::Utf8(Some(label)) => label,
            other => panic!("unexpected label scalar: {other:?}"),
        }
    }

    fn df(
        ctx: &SessionContext,
        categories: Vec<&str>,
        values: Vec<f64>,
    ) -> datafusion::error::Result<DataFrame> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(categories)),
                Arc::new(Float64Array::from(values)),
            ],
        )?;
        ctx.read_batch(batch)
    }

    fn list_df(ctx: &SessionContext) -> datafusion::error::Result<DataFrame> {
        let values = Arc::new(ListArray::from_iter_primitive::<Float64Type, _, _>(vec![
            Some(vec![Some(3.0), None, Some(f64::NAN)]),
            None,
            Some(vec![Some(-2.0), Some(10.0), Some(f64::INFINITY)]),
        ])) as ArrayRef;
        let schema = Arc::new(Schema::new(vec![Field::new(
            "values",
            values.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema, vec![values])?;
        ctx.read_batch(batch)
    }

    fn nested_df(
        ctx: &SessionContext,
        groups: Vec<&str>,
        members: Vec<&str>,
        values: Vec<f64>,
    ) -> datafusion::error::Result<DataFrame> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("member", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(groups)),
                Arc::new(StringArray::from(members)),
                Arc::new(Float64Array::from(values)),
            ],
        )?;
        ctx.read_batch(batch)
    }

    fn nested_label_df(
        ctx: &SessionContext,
        groups: Vec<&str>,
        members: Vec<&str>,
        labels: Vec<Option<&str>>,
    ) -> datafusion::error::Result<DataFrame> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("member", DataType::Utf8, false),
            Field::new("member_label", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(groups)),
                Arc::new(StringArray::from(members)),
                Arc::new(StringArray::from(labels)),
            ],
        )?;
        ctx.read_batch(batch)
    }

    fn nested_struct_df(ctx: &SessionContext) -> datafusion::error::Result<DataFrame> {
        let nested_key = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("group", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["A", "A", "B"])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("member", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["a", "b", "a"])) as ArrayRef,
            ),
        ])) as ArrayRef;
        let schema = Arc::new(Schema::new(vec![
            Field::new("nested_key", nested_key.data_type().clone(), false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                nested_key,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
            ],
        )?;
        ctx.read_batch(batch)
    }

    fn eval_ctx(ctx: &SessionContext) -> CoreEvaluationContext {
        CoreEvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(ctx.clone()),
            IndexMap::new(),
        )
    }

    fn no_default_range(
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &DataType,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }

    async fn configured_scales_for_prepared(
        ctx: &SessionContext,
        prepared_marks: Vec<PreparedScaleMark>,
        scale_specs: HashMap<String, PlotScaleSpec>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let coord_transform = TestCoordTransform;
        let builder = build_scale_builder_from_prepared_marks(
            &prepared_marks,
            &scale_specs,
            &coord_transform,
            &eval_ctx(ctx),
            &Theme::light(),
        )
        .await?;
        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 300.0),
        );
        coord_ranges.insert(
            "y".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 200.0),
        );
        builder
            .build_scales(
                300.0,
                200.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                ctx,
                &IndexMap::new(),
            )
            .await
    }

    #[tokio::test]
    async fn cartesian_positional_scale_discovery_is_unchanged() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("value")));
        channels.insert(
            "x2".to_string(),
            ChannelValue::from(col("value")).with_scale_name("x"),
        );
        channels.insert("y".to_string(), ChannelValue::from(col("category")));
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let names =
            positional_scale_names_for_prepared_marks(&[prepared], &TestCoordTransform, &ctx)
                .unwrap();

        assert!(names.contains("x"));
        assert!(names.contains("y"));
        assert_eq!(names.len(), 2);
    }

    #[tokio::test]
    async fn list_array_numeric_domain_inference_flattens_finite_values() {
        let ctx = SessionContext::new();
        let data = list_df(&ctx).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("values")));
        channels.insert("y".to_string(), ChannelValue::from(lit(0.0)));
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new())
            .await
            .unwrap();
        let x_domain = scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("x numeric domain");
        assert_eq!(x_domain, (-2.0, 10.0));
    }

    #[tokio::test]
    async fn explicit_domain_scale_builds_without_dataframe() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec![], vec![]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert(
            "fill".to_string(),
            ChannelValue::from(col("density"))
                .scale_with::<Linear>(|scale| scale.domain((0.0, 10.0))),
        );
        let mark = Arc::new(TestCompiledMark::new(data, IndexMap::new())) as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new_with_domain_source(
            mark,
            None,
            channels.clone(),
            None,
            channels,
            DerivedScalarMap::new(),
        );

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new())
            .await
            .unwrap();
        let fill_domain = scales
            .get("fill")
            .expect("fill scale")
            .configured()
            .numeric_interval_domain()
            .expect("fill numeric domain");
        assert_eq!(fill_domain, (0.0, 10.0));
    }

    #[tokio::test]
    async fn redundant_fill_and_fill_pattern_domains_share_order() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let data = df(
            &ctx,
            vec!["b", "a", "c", "a", "b"],
            vec![1.0, 2.0, 3.0, 4.0, 5.0],
        )?;
        let mut channels = IndexMap::new();
        channels.insert("fill".to_string(), ChannelValue::from(col("category")));
        let pattern_channel = PatternChannelValue::from(col("category"));
        channels.insert(
            "fill_pattern".to_string(),
            pattern_channel
                .scaled_channel_surrogate()
                .expect("scaled pattern channel surrogate"),
        );
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new()).await?;
        let fill_domain = discrete_domain(scales.get("fill").expect("fill scale"));
        let pattern_domain =
            discrete_domain(scales.get("fill_pattern").expect("fill_pattern scale"));

        assert_eq!(fill_domain, vec![s("a"), s("b"), s("c")]);
        assert_eq!(pattern_domain, fill_domain);
        assert!(
            scales
                .get("fill_pattern")
                .and_then(ConfiguredScaleWithSpec::pattern_range)
                .is_some(),
            "fill_pattern should keep its structured pattern range"
        );
        Ok(())
    }

    #[tokio::test]
    async fn separate_fill_and_fill_pattern_expressions_keep_separate_domains()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("pattern_category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["b", "a", "c", "a", "b"])) as ArrayRef,
                Arc::new(StringArray::from(vec![
                    "stripe", "dot", "dash", "dot", "stripe",
                ])) as ArrayRef,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as ArrayRef,
            ],
        )?;
        let data = ctx.read_batch(batch)?;
        let mut channels = IndexMap::new();
        channels.insert("fill".to_string(), ChannelValue::from(col("category")));
        let pattern_channel = PatternChannelValue::from(col("pattern_category"));
        channels.insert(
            "fill_pattern".to_string(),
            pattern_channel
                .scaled_channel_surrogate()
                .expect("scaled pattern channel surrogate"),
        );
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new()).await?;
        let fill_domain = discrete_domain(scales.get("fill").expect("fill scale"));
        let pattern_domain =
            discrete_domain(scales.get("fill_pattern").expect("fill_pattern scale"));

        assert_eq!(fill_domain, vec![s("a"), s("b"), s("c")]);
        assert_eq!(pattern_domain, vec![s("dash"), s("dot"), s("stripe")]);
        assert_ne!(pattern_domain, fill_domain);
        Ok(())
    }

    #[tokio::test]
    async fn explicit_pattern_range_infers_ordinal_scale_without_scale_type()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0])?;
        let pattern_range = ScaleRange::new_pattern(vec![Some(PatternFill::default()), None]);
        let mut channels = IndexMap::new();
        channels.insert(
            "fill_pattern".to_string(),
            ChannelValue::from(col("category"))
                .scale(move |scale| scale.range(pattern_range.clone())),
        );
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new()).await?;
        let scale = scales
            .get("fill_pattern")
            .expect("fill_pattern scale should be configured");

        assert_eq!(scale.configured().scale_impl.scale_type(), "ordinal");
        assert_eq!(scale.pattern_range().expect("pattern range").len(), 2);
        assert_eq!(discrete_domain(scale), vec![s("A"), s("B")]);
        Ok(())
    }

    #[test]
    fn pattern_channel_surrogate_preserves_domain_group() {
        let fill_channel = ChannelValue::from(col("category")).with_domain_group("semantic-style");
        let pattern_channel =
            PatternChannelValue::from(col("category")).with_domain_group("semantic-style");
        let surrogate = pattern_channel
            .scaled_channel_surrogate()
            .expect("scaled pattern channel surrogate");
        let coordination = surrogate
            .get_domain_coordination()
            .expect("domain coordination");

        assert_eq!(coordination.scope, CoordinationScope::Free.to_normalized());
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("semantic-style".to_string())
        );
        assert_eq!(fill_channel.get_domain_coordination(), Some(coordination));
    }

    #[tokio::test]
    async fn dynamic_position_scale_hook_marks_non_required_channel_positional() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert(
            "dim_height".to_string(),
            ChannelValue::from(col("value")).with_scale_name("height"),
        );
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new());

        let names = positional_scale_names_for_prepared_marks(
            &[prepared],
            &DynamicPositionCoordTransform,
            &ctx,
        )
        .unwrap();

        assert_eq!(names, HashSet::from(["height".to_string()]));
    }

    #[tokio::test]
    async fn scale_hint_forces_band_over_point_preference() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("y".to_string(), ChannelValue::from(col("category")));
        channels.insert("x".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new_with_position_preference(
            data.clone(),
            channels.clone(),
            Some(ScaleTypePreference::Point),
        )) as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new())
            .with_scale_inference_hints(vec![ScaleInferenceHint::new(
                "y",
                ScaleTypePreference::Band,
            )]);

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            scales
                .get("y")
                .expect("y scale")
                .configured()
                .scale_impl
                .scale_type(),
            "band"
        );
    }

    #[tokio::test]
    async fn conflicting_scale_hints_error_without_explicit_scale_type() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("y".to_string(), ChannelValue::from(col("category")));
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new())
            .with_scale_inference_hints(vec![
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
                ScaleInferenceHint::new("y", ScaleTypePreference::Point),
            ]);

        let err = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new())
            .await
            .expect_err("conflicting hints should fail");
        assert!(
            err.to_string()
                .contains("Conflicting compound mark scale inference hints"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn explicit_scale_type_wins_over_conflicting_scale_hints() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert(
            "y".to_string(),
            ChannelValue::from(col("category")).scale_with::<Point>(|scale| scale),
        );
        let mark = Arc::new(TestCompiledMark::new(data.clone(), channels.clone()))
            as Arc<dyn CompiledMark>;
        let prepared = PreparedScaleMark::new(mark, Some(data), channels, DerivedScalarMap::new())
            .with_scale_inference_hints(vec![
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
                ScaleInferenceHint::new("y", ScaleTypePreference::Point),
            ]);

        let scales = configured_scales_for_prepared(&ctx, vec![prepared], HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            scales
                .get("y")
                .expect("y scale")
                .configured()
                .scale_impl
                .scale_type(),
            "point"
        );
    }

    #[tokio::test]
    async fn categorical_order_by_max_descends_and_ties_by_category() {
        let ctx = SessionContext::new();
        let data = df(
            &ctx,
            vec!["A", "B", "C", "A", "B", "C"],
            vec![2.0, 9.0, 9.0, 5.0, 1.0, 4.0],
        )
        .unwrap();

        let (values, ordered) = ordered_categorical_values(
            &[(Arc::new(data), col("category"))],
            &max(col("value")),
            true,
            &eval_ctx(&ctx),
            &IndexMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(values, vec![s("B"), s("C"), s("A")]);
    }

    #[tokio::test]
    async fn categorical_order_by_sum_combines_multiple_sources() {
        let ctx = SessionContext::new();
        let left = df(&ctx, vec!["A", "B"], vec![5.0, 9.0]).unwrap();
        let right = df(&ctx, vec!["A", "B"], vec![10.0, 1.0]).unwrap();

        let (values, ordered) = ordered_categorical_values(
            &[
                (Arc::new(left), col("category")),
                (Arc::new(right), col("category")),
            ],
            &sum(col("value")),
            true,
            &eval_ctx(&ctx),
            &IndexMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(values, vec![s("A"), s("B")]);
    }

    #[tokio::test]
    async fn categorical_order_by_category_descends_without_aggregate() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["B", "A", "C"], vec![1.0, 1.0, 1.0]).unwrap();

        let (values, ordered) = ordered_categorical_values(
            &[(Arc::new(data), col("category"))],
            &col("category"),
            true,
            &eval_ctx(&ctx),
            &IndexMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(values, vec![s("C"), s("B"), s("A")]);
    }

    #[tokio::test]
    async fn categorical_order_by_literal_keeps_default_category_order() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["B", "A", "C"], vec![1.0, 1.0, 1.0]).unwrap();

        let (values, ordered) = ordered_categorical_values(
            &[(Arc::new(data), col("category"))],
            &lit(1.0),
            true,
            &eval_ctx(&ctx),
            &IndexMap::new(),
        )
        .await
        .unwrap();

        assert!(!ordered);
        assert_eq!(values, vec![s("A"), s("B"), s("C")]);
    }

    #[tokio::test]
    async fn categorical_order_by_rejects_non_aggregate_non_category_column() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "B"], vec![1.0, 2.0]).unwrap();

        let err = ordered_categorical_values(
            &[(Arc::new(data), col("category"))],
            &col("value"),
            false,
            &eval_ctx(&ctx),
            &IndexMap::new(),
        )
        .await
        .expect_err("expected invalid ordering expression");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("Scale order_by expression"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn nested_level_domain_values_order_struct_paths_by_level() {
        let ctx = SessionContext::new();
        let values = vec![
            nested_path("A", "north"),
            nested_path("A", "south"),
            nested_path("B", "north"),
            nested_path("B", "south"),
        ];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            0,
            NestedBandLevelSpec {
                domain: Maybe::Set(ScaleDomain::new_discrete(vec![lit("B"), lit("A")])),
                ..Default::default()
            },
        );
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                domain: Maybe::Set(ScaleDomain::new_discrete(vec![lit("south"), lit("north")])),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("B".to_string(), "south".to_string()),
                ("B".to_string(), "north".to_string()),
                ("A".to_string(), "south".to_string()),
                ("A".to_string(), "north".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_shared_level_domain_values_synthesize_missing_paths() {
        let ctx = SessionContext::new();
        let values = vec![nested_path("A", "north"), nested_path("B", "east")];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            0,
            NestedBandLevelSpec {
                domain: Maybe::Set(ScaleDomain::new_discrete(vec![lit("B"), lit("A")])),
                ..Default::default()
            },
        );
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                domain: Maybe::Set(ScaleDomain::new_discrete(vec![
                    lit("south"),
                    lit("north"),
                    lit("east"),
                ])),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("B".to_string(), "south".to_string()),
                ("B".to_string(), "north".to_string()),
                ("B".to_string(), "east".to_string()),
                ("A".to_string(), "south".to_string()),
                ("A".to_string(), "north".to_string()),
                ("A".to_string(), "east".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_changes_axis_labels_not_positions() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "A"],
            vec!["a", "b"],
            vec![Some("Alpha"), Some("Beta")],
        )
        .unwrap();
        let values = vec![nested_path("A", "a"), nested_path("A", "b")];
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let labeled_values = apply_nested_level_labels(
            values.clone(),
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();
        let domain = nested_domain_array(labeled_values);
        let scale =
            avenger_scales::scales::nested_band::NestedBandScale::configured(domain, (0.0, 200.0));
        let bands = avenger_scales::scales::nested_band::nested_axis_bands(&scale.config, 1)
            .expect("axis bands");

        assert_eq!(
            bands
                .iter()
                .map(|band| band.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"]
        );
        assert_eq!(
            scaled_positions(&scale, values),
            vec![Some(0.0), Some(100.0)]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_conflicting_labels_error() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "A"],
            vec!["a", "a"],
            vec![Some("Alpha"), Some("Different")],
        )
        .unwrap();
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let err = apply_nested_level_labels(
            vec![nested_path("A", "a")],
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .expect_err("conflicting labels");

        assert!(
            err.to_string().contains("multiple labels"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_free_level_scopes_labels_by_parent() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "B"],
            vec!["x", "x"],
            vec![Some("A-X"), Some("B-X")],
        )
        .unwrap();
        let values = vec![nested_path("A", "x"), nested_path("B", "x")];
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let labeled_values = apply_nested_level_labels(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            labeled_values
                .iter()
                .map(|value| nested_component_display_label(value, "member"))
                .collect::<Vec<_>>(),
            vec!["A-X", "B-X"]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_shared_level_collects_global_labels() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "B"],
            vec!["x", "x"],
            vec![Some("Shared X"), Some("Shared X")],
        )
        .unwrap();
        let values = vec![nested_path("A", "x"), nested_path("B", "x")];
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).nest_scope = Some(NestScope::Shared);
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let labeled_values = apply_nested_level_labels(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            labeled_values
                .iter()
                .map(|value| nested_component_display_label(value, "member"))
                .collect::<Vec<_>>(),
            vec!["Shared X", "Shared X"]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_explicit_domain_falls_back_for_invented_keys() {
        let ctx = SessionContext::new();
        let data = nested_label_df(&ctx, vec!["A"], vec!["x"], vec![Some("Label X")]).unwrap();
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).nest_scope = Some(NestScope::Shared);
        config.level_mut(1).domain =
            Maybe::Set(ScaleDomain::new_discrete(vec![lit("x"), lit("y")]));
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let (values, _) = apply_nested_level_domain_ordering(
            vec![nested_path("A", "x")],
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();
        let labeled_values = apply_nested_level_labels(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            labeled_values
                .iter()
                .map(|value| nested_component_display_label(value, "member"))
                .collect::<Vec<_>>(),
            vec!["Label X", "y"]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_ordering_still_uses_key_domain() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "A"],
            vec!["a", "b"],
            vec![Some("Zulu"), Some("Alpha")],
        )
        .unwrap();
        let mut config =
            NestedBandSpec::from_source_columns(vec!["group".to_string(), "member".to_string()]);
        config.level_mut(1).domain =
            Maybe::Set(ScaleDomain::new_discrete(vec![lit("b"), lit("a")]));
        config.level_mut(1).label_expr = Some(
            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("member_label"))
                .expect("serialize label expr"),
        );

        let (values, _) = apply_nested_level_domain_ordering(
            vec![nested_path("A", "a"), nested_path("A", "b")],
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();
        let labeled_values = apply_nested_level_labels(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            labeled_values
                .iter()
                .map(|value| nested_component_display_label(value, "member"))
                .collect::<Vec<_>>(),
            vec!["Alpha", "Zulu"]
        );
    }

    #[tokio::test]
    async fn nested_band_label_with_repeat_placeholder_resolves_before_collection() {
        let ctx = SessionContext::new();
        let data = nested_label_df(
            &ctx,
            vec!["A", "A"],
            vec!["a", "b"],
            vec![Some("Alias A"), Some("Alias B")],
        )
        .unwrap();
        let repeat_ctx = RepeatContext::new().with_column(
            ResolvedRepeatVariable {
                id: "member".to_string(),
                expr: col("source_member"),
                title: "Member".to_string(),
                type_hint: None,
            },
            0,
            1,
        );
        let value = nested(["group".to_string(), column_name()]).map_channel_value(|value| {
            let mut config = value
                .get_nested_band_config()
                .cloned()
                .expect("nested config");
            config.level_mut(1).label_expr = Some(
                datafusion_proto::protobuf::LogicalExprNode::from_expr(col(format!(
                    "{}_label",
                    column_name()
                )))
                .expect("serialize label expr"),
            );
            value.with_nested_band_config(config)
        });
        let resolved =
            resolve_repeat_channel_expr(value, &repeat_ctx).expect("resolve repeat placeholders");
        let config = resolved
            .channel_value()
            .get_nested_band_config()
            .cloned()
            .expect("resolved nested config");

        assert_eq!(
            config.source_columns,
            vec!["group".to_string(), "member".to_string()]
        );

        let labeled_values = apply_nested_level_labels(
            vec![nested_path("A", "a"), nested_path("A", "b")],
            Some(&config),
            &[(Arc::new(data), resolved.data_expr().clone())],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            labeled_values
                .iter()
                .map(|value| nested_component_display_label(value, "member"))
                .collect::<Vec<_>>(),
            vec!["Alias A", "Alias B"]
        );
    }

    #[tokio::test]
    async fn nested_free_level_domain_values_do_not_synthesize_missing_paths() {
        let ctx = SessionContext::new();
        let values = vec![nested_path("A", "north"), nested_path("B", "east")];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                domain: Maybe::Set(ScaleDomain::new_discrete(vec![
                    lit("south"),
                    lit("north"),
                    lit("east"),
                ])),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("A".to_string(), "north".to_string()),
                ("B".to_string(), "east".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_level_order_desc_preserves_parent_grouping() {
        let ctx = SessionContext::new();
        let values = vec![
            nested_path("A", "a"),
            nested_path("A", "c"),
            nested_path("B", "b"),
            nested_path("B", "a"),
            nested_path("A", "b"),
            nested_path("B", "c"),
        ];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                ordering: Maybe::Set(ScaleOrderingSpec {
                    order_expr: None,
                    order_descending: Some(true),
                }),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("A".to_string(), "c".to_string()),
                ("A".to_string(), "b".to_string()),
                ("A".to_string(), "a".to_string()),
                ("B".to_string(), "c".to_string()),
                ("B".to_string(), "b".to_string()),
                ("B".to_string(), "a".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_level_order_by_aggregate_preserves_parent_grouping() {
        let ctx = SessionContext::new();
        let data = nested_df(
            &ctx,
            vec!["A", "A", "A", "B", "B", "B", "A"],
            vec!["a", "b", "c", "a", "b", "c", "a"],
            vec![2.0, 9.0, 5.0, 7.0, 1.0, 8.0, 11.0],
        )
        .unwrap();
        let nested_expr = named_struct(vec![
            lit("group"),
            col("group"),
            lit("member"),
            col("member"),
        ]);
        let values = vec![
            nested_path("A", "a"),
            nested_path("A", "b"),
            nested_path("A", "c"),
            nested_path("B", "a"),
            nested_path("B", "b"),
            nested_path("B", "c"),
        ];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                ordering: Maybe::Set(
                    Scale::<Band>::new()
                        .order_by(max(col("value")))
                        .order_desc()
                        .into_config()
                        .ordering
                        .into_option()
                        .expect("ordering config"),
                ),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr)],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("A".to_string(), "a".to_string()),
                ("A".to_string(), "b".to_string()),
                ("A".to_string(), "c".to_string()),
                ("B".to_string(), "c".to_string()),
                ("B".to_string(), "a".to_string()),
                ("B".to_string(), "b".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_shared_level_order_by_aggregate_uses_global_child_order() {
        let ctx = SessionContext::new();
        let data = nested_df(
            &ctx,
            vec!["A", "A", "B", "B", "B"],
            vec!["a", "b", "a", "b", "c"],
            vec![2.0, 9.0, 7.0, 1.0, 8.0],
        )
        .unwrap();
        let nested_expr = named_struct(vec![
            lit("group"),
            col("group"),
            lit("member"),
            col("member"),
        ]);
        let values = vec![
            nested_path("A", "a"),
            nested_path("A", "b"),
            nested_path("B", "a"),
            nested_path("B", "b"),
            nested_path("B", "c"),
        ];
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                ordering: Maybe::Set(
                    Scale::<Band>::new()
                        .order_by(max(col("value")))
                        .order_desc()
                        .into_config()
                        .ordering
                        .into_option()
                        .expect("ordering config"),
                ),
                ..Default::default()
            },
        );

        let (values, ordered) = apply_nested_level_domain_ordering(
            values,
            Some(&config),
            &[(Arc::new(data), nested_expr)],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .unwrap();

        assert!(ordered);
        assert_eq!(
            nested_path_labels(&values),
            vec![
                ("A".to_string(), "b".to_string()),
                ("A".to_string(), "a".to_string()),
                ("B".to_string(), "b".to_string()),
                ("B".to_string(), "c".to_string()),
                ("B".to_string(), "a".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_level_order_by_rejects_non_aggregate_non_level_field() {
        let ctx = SessionContext::new();
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                ordering: Maybe::Set(
                    Scale::<Band>::new()
                        .order_by(col("value"))
                        .into_config()
                        .ordering
                        .into_option()
                        .expect("ordering config"),
                ),
                ..Default::default()
            },
        );

        let err = apply_nested_level_domain_ordering(
            vec![nested_path("A", "a")],
            Some(&config),
            &[],
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &DerivedScalarMap::new(),
        )
        .await
        .expect_err("non-aggregate non-level order_by should be rejected");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("Nested-band level 1 order_by expression"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn categorical_order_by_ignores_explicit_discrete_domain() {
        let ctx = SessionContext::new();
        let spec: Box<dyn ScaleSpec> = Box::new(Band);
        let ordering = Scale::<Band>::new()
            .order_by(max(col("value")))
            .order_desc()
            .into_config()
            .ordering
            .into_option()
            .expect("ordering config");
        let mut builder = ScaleBuilder::new();
        let coord_transform = TestCoordTransform;

        cache_domain_data(
            "x",
            &spec,
            &ArrowDataType::Utf8,
            HashMap::new(),
            Some(ScaleDomain::new_discrete(vec![lit("B"), lit("A")])),
            Some(ordering),
            None,
            &[],
            &coord_transform,
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &HashMap::new(),
            &mut builder,
            None,
            &Theme::light(),
        )
        .await
        .unwrap();

        let domain = match builder.channel_builders().get("x") {
            Some(ChannelScaleData::ExplicitDomain { domain, .. }) => domain,
            other => panic!("expected explicit domain builder, got {other:?}"),
        };
        let values = match &domain.default_domain {
            ScaleDefaultDomain::Discrete(values) => values,
            other => panic!("expected discrete domain, got {other:?}"),
        };
        let decoded = values
            .iter()
            .map(|node| match node.to_expr(&ctx).unwrap() {
                Expr::Literal(ScalarValue::Utf8(Some(value)), _) => value,
                other => panic!("unexpected domain literal: {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(decoded, vec!["B", "A"]);
    }

    #[tokio::test]
    async fn nested_band_domain_cache_preserves_struct_paths() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "A", "B"], vec![1.0, 2.0, 1.0]).unwrap();
        let nested_expr = named_struct(vec![
            lit("group"),
            col("category"),
            lit("series"),
            col("value"),
        ]);
        let projected = data
            .clone()
            .select(vec![nested_expr.clone().alias("nested")])
            .unwrap();
        let nested_type = projected.schema().field(0).data_type().clone();

        let spec: Box<dyn ScaleSpec> = Box::new(NestedBand);
        let coord_transform = TestCoordTransform;
        let mut builder = ScaleBuilder::new();
        cache_domain_data(
            "x",
            &spec,
            &nested_type,
            HashMap::new(),
            Some(ScaleDomain::new_data_fields(vec![(
                Arc::new(data),
                nested_expr,
            )])),
            None,
            None,
            &[],
            &coord_transform,
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &HashMap::new(),
            &mut builder,
            None,
            &Theme::light(),
        )
        .await
        .unwrap();

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 300.0),
        );
        let scales = builder
            .build_scales(
                300.0,
                200.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &IndexMap::new(),
            )
            .await
            .unwrap();

        let x_scale = scales.get("x").expect("x scale");
        assert_eq!(x_scale.configured().scale_impl.scale_type(), "nested_band");
        assert!(matches!(
            x_scale.configured().domain().data_type(),
            DataType::Struct(_)
        ));

        let scaled = x_scale
            .configured()
            .scale(x_scale.configured().domain())
            .unwrap();
        let scaled = scaled.as_any().downcast_ref::<Float32Array>().unwrap();
        assert_eq!(scaled.len(), 3);
        assert_eq!(scaled.value(0), 0.0);
        assert_eq!(scaled.value(1), 100.0);
        assert_eq!(scaled.value(2), 200.0);
    }

    #[tokio::test]
    async fn nested_band_uses_custom_position_scale_name() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "A", "B"], vec![1.0, 2.0, 1.0]).unwrap();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            nested(["category", "value"])
                .with_scale_name("grouped_x")
                .into(),
        );
        channels.insert(
            "x2".to_string(),
            ChannelExpr::scaled(col(":x")).band(1.0).into(),
        );
        channels.insert("y".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;
        let builder = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .unwrap();

        assert!(
            builder.channel_builders().contains_key("grouped_x"),
            "nested position domain should be cached under the custom scale name"
        );
        assert!(
            !builder.channel_builders().contains_key("x"),
            "custom named x scale should not also create an unused default x scale"
        );

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "grouped_x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 300.0),
        );
        let scales = builder
            .build_scales(
                300.0,
                200.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &IndexMap::new(),
            )
            .await
            .unwrap();

        let x_scale = scales.get("grouped_x").expect("custom nested x scale");
        assert_eq!(x_scale.configured().scale_impl.scale_type(), "nested_band");
        assert!(matches!(
            x_scale.configured().domain().data_type(),
            DataType::Struct(_)
        ));
    }

    #[tokio::test]
    async fn raw_named_struct_nested_band_level_config_requires_nested_api() {
        let ctx = SessionContext::new();
        let data = nested_df(
            &ctx,
            vec!["A", "A", "B"],
            vec!["a", "b", "a"],
            vec![1.0, 2.0, 3.0],
        )
        .unwrap();
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                ..Default::default()
            },
        );
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::from(named_struct(vec![
                lit("group"),
                col("group"),
                lit("member"),
                col("member"),
            ]))
            .with_nested_band_config(config),
        );
        channels.insert(
            "x2".to_string(),
            ChannelExpr::scaled(col(":x")).band(1.0).into(),
        );
        channels.insert("y".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;

        let err = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .expect_err("raw named_struct nested position should be rejected");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("must be created with nested"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn physical_struct_nested_band_position_column_requires_nested_api() {
        let ctx = SessionContext::new();
        let data = nested_struct_df(&ctx).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("nested_key")));
        channels.insert("y".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;

        let err = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .expect_err("raw physical struct position should be rejected");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("must be created with nested"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn explicit_nested_band_scale_on_raw_struct_requires_nested_api() {
        let ctx = SessionContext::new();
        let data = nested_struct_df(&ctx).unwrap();
        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            ChannelValue::from(col("nested_key")).scale_with::<NestedBand>(|scale| scale),
        );
        channels.insert("y".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;

        let err = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .expect_err("explicit nested band scale without nested metadata should be rejected");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("must be created with nested"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn nested_band_zero_field_struct_reports_invalid_argument() {
        let ctx = SessionContext::new();
        let empty_struct = Arc::new(StructArray::new_empty_fields(1, None)) as ArrayRef;
        let schema = Arc::new(Schema::new(vec![
            Field::new("empty", empty_struct.data_type().clone(), false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                empty_struct,
                Arc::new(Float64Array::from(vec![1.0])) as ArrayRef,
            ],
        )
        .unwrap();
        let data = ctx.read_batch(batch).unwrap();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("empty")));
        channels.insert("y".to_string(), ChannelValue::from(col("value")));
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;

        let err = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .expect_err("zero-field struct position should be rejected");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("must be created with nested"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn nested_band_preserves_independent_x_y_level_options() {
        let ctx = SessionContext::new();
        let data = df(&ctx, vec!["A", "A", "B"], vec![1.0, 2.0, 1.0]).unwrap();
        let mut x_config =
            NestedBandSpec::from_source_columns(vec!["category".to_string(), "value".to_string()]);
        x_config.levels.insert(
            1,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                padding_inner: Some(0.2),
                ..Default::default()
            },
        );
        let mut y_config =
            NestedBandSpec::from_source_columns(vec!["category".to_string(), "value".to_string()]);
        y_config.levels.insert(
            1,
            NestedBandLevelSpec {
                padding_inner_px: Some(7.0),
                ..Default::default()
            },
        );

        let mut channels = IndexMap::new();
        channels.insert(
            "x".to_string(),
            nested(["category", "value"])
                .map_channel_value(|value| value.with_nested_band_config(x_config))
                .into(),
        );
        channels.insert(
            "x2".to_string(),
            ChannelExpr::scaled(col(":x")).band(1.0).into(),
        );
        channels.insert(
            "y".to_string(),
            nested(["category", "value"])
                .map_channel_value(|value| value.with_nested_band_config(y_config))
                .into(),
        );
        channels.insert(
            "y2".to_string(),
            ChannelExpr::scaled(col(":y")).band(1.0).into(),
        );
        let mark = Arc::new(TestCompiledMark::new(data, channels)) as Arc<dyn CompiledMark>;
        let coord_transform = TestCoordTransform;
        let builder = build_scale_builder_from_marks(
            &[mark],
            &HashMap::new(),
            &coord_transform,
            &None,
            None,
            &eval_ctx(&ctx),
            &Theme::light(),
        )
        .await
        .unwrap();

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 300.0),
        );
        coord_ranges.insert(
            "y".to_string(),
            ScaleRangeBinding::fixed_interval(200.0, 0.0),
        );
        let scales = builder
            .build_scales(
                300.0,
                200.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &IndexMap::new(),
            )
            .await
            .unwrap();

        let x_options = &scales
            .get("x")
            .expect("x nested scale")
            .configured()
            .config
            .options;
        let y_options = &scales
            .get("y")
            .expect("y nested scale")
            .configured()
            .config
            .options;
        assert_eq!(
            x_options
                .get("nest_scopes")
                .expect("x nest scopes")
                .as_string()
                .unwrap(),
            ",shared"
        );
        assert_eq!(
            x_options
                .get("padding_inner_levels")
                .expect("x padding levels")
                .as_string()
                .unwrap(),
            ",0.2"
        );
        assert!(
            !x_options.contains_key("padding_inner_px_levels"),
            "x scale should not inherit y pixel padding"
        );
        assert_eq!(
            y_options
                .get("padding_inner_px_levels")
                .expect("y pixel padding levels")
                .as_string()
                .unwrap(),
            ",7"
        );
        assert!(
            !y_options.contains_key("nest_scopes"),
            "y scale should not inherit x nest scope"
        );
    }

    #[test]
    fn nested_band_level_config_encodes_scale_options() {
        let ctx = SessionContext::new();
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            1,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                padding_inner: Some(0.25),
                padding_outer: Some(0.5),
                padding_inner_px: Some(4.0),
                padding_outer_px: Some(8.0),
                ..Default::default()
            },
        );

        validate_nested_band_config(&config, 2).unwrap();
        let scale =
            apply_nested_band_options(Scale::<Auto>::from_spec(Box::new(NestedBand)), &config)
                .unwrap();

        let option_string = |key: &str| match scale.get_options()[key].to_expr(&ctx).unwrap() {
            Expr::Literal(ScalarValue::Utf8(Some(value)), _) => value,
            other => panic!("unexpected option expr for {key}: {other:?}"),
        };

        assert_eq!(option_string("nest_scopes"), ",shared");
        assert_eq!(option_string("padding_inner_levels"), ",0.25");
        assert_eq!(option_string("padding_outer_levels"), ",0.5");
        assert_eq!(option_string("padding_inner_px_levels"), ",4");
        assert_eq!(option_string("padding_outer_px_levels"), ",8");
    }

    #[test]
    fn nested_band_level_zero_nest_scope_is_invalid() {
        let mut config = NestedBandSpec::default();
        config.levels.insert(
            0,
            NestedBandLevelSpec {
                nest_scope: Some(NestScope::Shared),
                ..Default::default()
            },
        );

        let err = validate_nested_band_config(&config, 2).expect_err("level 0 nest scope");
        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("level 0"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn nested_band_level_beyond_struct_fields_is_invalid() {
        let mut config = NestedBandSpec::default();
        config.levels.insert(2, NestedBandLevelSpec::default());

        let err = validate_nested_band_config(&config, 2).expect_err("invalid level");
        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("level 2"));
                assert!(message.contains("2 fields"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn nested_band_derivative_channel_rejects_level_config() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("x")));
        channels.insert(
            "x2".to_string(),
            ChannelValue::from(col(":x")).with_nested_band_config({
                let mut config = NestedBandSpec::default();
                config.levels.insert(
                    1,
                    NestedBandLevelSpec {
                        nest_scope: Some(NestScope::Shared),
                        ..Default::default()
                    },
                );
                config
            }),
        );

        let err = validate_nested_band_derivative_configs(&channels, &ctx)
            .expect_err("derivative config");
        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("derivative channel 'x2'"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        channels.insert(
            "x2".to_string(),
            ChannelExpr::scaled(col(":x")).level_band(0, 1.0).into(),
        );
        validate_nested_band_derivative_configs(&channels, &ctx).unwrap();
    }

    #[test]
    fn nested_band_derivative_channel_rejects_label_with() {
        let ctx = SessionContext::new();
        let mut channels = IndexMap::new();
        channels.insert("x".to_string(), ChannelValue::from(col("x")));
        channels.insert(
            "x2".to_string(),
            ChannelValue::from(col(":x")).with_nested_band_config({
                let mut config = NestedBandSpec::default();
                config.levels.insert(
                    0,
                    NestedBandLevelSpec {
                        label_expr: Some(
                            datafusion_proto::protobuf::LogicalExprNode::from_expr(col("label"))
                                .expect("serialize label expr"),
                        ),
                        ..Default::default()
                    },
                );
                config
            }),
        );

        let err = validate_nested_band_derivative_configs(&channels, &ctx)
            .expect_err("derivative label config");
        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("derivative channel 'x2'"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn scale_order_by_rejects_non_categorical_scale() {
        let ctx = SessionContext::new();
        let spec: Box<dyn ScaleSpec> = Box::new(Linear);
        let ordering = Scale::<Linear>::new()
            .order_by(col("value"))
            .into_config()
            .ordering
            .into_option()
            .expect("ordering config");
        let mut builder = ScaleBuilder::new();
        let coord_transform = TestCoordTransform;

        let err = cache_domain_data(
            "x",
            &spec,
            &ArrowDataType::Float64,
            HashMap::new(),
            None,
            Some(ordering),
            None,
            &[],
            &coord_transform,
            &eval_ctx(&ctx),
            &ctx,
            &IndexMap::new(),
            &HashMap::new(),
            &mut builder,
            None,
            &Theme::light(),
        )
        .await
        .expect_err("expected non-categorical order_by rejection");

        match err {
            AvengerChartError::InvalidArgument(message) => {
                assert!(message.contains("only supported for categorical-domain scales"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
