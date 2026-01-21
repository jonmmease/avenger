/// Nested facet detection and coordination logic
///
/// This module extracts the `detect_nested_facet_and_compute_coordination` function
/// from facet_evaluation.rs to reduce file size and improve module organization.
///
/// The primary function detects when a facet contains another facet (nesting) and
/// computes the FacetCoordinationContext needed to coordinate scales, domains, and
/// guide ownership across the nested hierarchy.
use crate::channel::config_traits::ScaleSharing;
use crate::error::AvengerChartError;
use crate::facet::coordination::FacetCoordinationContext;
use crate::facet::marks::facet::determine_facet_band_align;
use crate::facet::marks::facet_extents::{
    ChannelDataKind, DomainSort, classify_channel, compute_extents, normalize_domain_scalar,
    scalar_to_timestamp_ms,
};
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::marks::CompiledMark;
use crate::plot::CompiledPlot;
use avenger_scales::scales::DomainKind;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::sync::Arc;

/// Information about a channel found by recursive search
pub struct FoundChannelInfo {
    pub expr: datafusion::logical_expr::Expr,
    pub share_mode: ScaleSharing,
    pub domain_kind: Option<DomainKind>,
}

/// Recursively search through marks (including nested facets) to find x/y channel info.
/// This is needed for Level(N>1) where the actual channel definitions are in deeply nested subplots.
pub fn find_channel_in_marks(
    marks: &[Arc<dyn CompiledMark>],
    channel_name: &str,
    ctx: &SessionContext,
    max_depth: usize,
) -> Option<FoundChannelInfo> {
    use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};

    if max_depth == 0 {
        return None;
    }

    for mark in marks {
        let channels = mark.data_context().channels();

        // Check if this mark has the channel directly
        if let Some(channel_value) = channels.get(channel_name) {
            if let Some(expr) = channel_value.scale_input_expr(ctx) {
                let share_mode = channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                let domain_kind = channel_value
                    .get_scale_config()
                    .and_then(|s| s.domain_kind());
                return Some(FoundChannelInfo {
                    expr,
                    share_mode,
                    domain_kind,
                });
            }
        }

        // If not found, check if this is a facet mark and recurse into its subplot
        let mark_type = mark.mark_type();
        if mark_type == "facet_row" {
            if let Some(facet) = mark.as_any().downcast_ref::<CompiledFacetRow>() {
                if let Some(info) = find_channel_in_marks(
                    &facet.compiled_subplot.marks,
                    channel_name,
                    ctx,
                    max_depth - 1,
                ) {
                    return Some(info);
                }
            }
        } else if mark_type == "facet_col" {
            if let Some(facet) = mark.as_any().downcast_ref::<CompiledFacetCol>() {
                if let Some(info) = find_channel_in_marks(
                    &facet.compiled_subplot.marks,
                    channel_name,
                    ctx,
                    max_depth - 1,
                ) {
                    return Some(info);
                }
            }
        }
    }

    None
}

/// Detect nested facet and compute coordination context
///
/// This function examines a compiled subplot to detect if it contains a nested facet
/// (either FacetRow or FacetCol). If found, it:
///
/// 1. Extracts the inner facet's channel expression and scale sharing configuration
/// 2. Computes domain values for coordinating scales across nested levels
/// 3. For Free scaling mode, computes max cell count for uniform sizing
/// 4. Computes shared data extents for x/y channels when using Shared mode
/// 5. Builds level_domains for Level(N) scale sharing
/// 6. Creates a FacetCoordinationContext with all coordination parameters
///
/// # Arguments
/// * `compiled_subplot` - The compiled plot that may contain a nested facet
/// * `df` - The full DataFrame (before per-cell filtering)
/// * `ctx` - DataFusion session context
/// * `_outer_is_row_facet` - Whether the outer facet is a row facet (currently unused)
/// * `outer_facet_expr` - Expression for the outer facet's grouping channel
/// * `params` - Parameter map for expression evaluation
/// * `incoming_coord_ctx` - Coordination context from parent facet (for deep nesting)
///
/// # Returns
/// Some(FacetCoordinationContext) if a nested facet is found, None otherwise
pub(crate) async fn detect_nested_facet_and_compute_coordination(
    compiled_subplot: &Arc<CompiledPlot>,
    df: &DataFrame,
    ctx: &SessionContext,
    _outer_is_row_facet: bool,
    outer_facet_expr: &datafusion::logical_expr::Expr,
    params: &indexmap::IndexMap<String, ScalarValue>,
    incoming_coord_ctx: Option<&FacetCoordinationContext>,
) -> Result<Option<FacetCoordinationContext>, AvengerChartError> {
    use crate::facet::coordination::{GuideOwnership, LevelChannelKey};
    use crate::facet::keys::FacetKeyExtractor;
    use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};
    use std::collections::HashMap;

    // Find nested facet mark
    let mut inner_facet_info: Option<(&Arc<dyn crate::marks::CompiledMark>, &str, bool)> = None;
    for mark in &compiled_subplot.marks {
        let mark_type = mark.mark_type();
        if mark_type == "facet_row" {
            inner_facet_info = Some((mark, "row", true));
            break;
        } else if mark_type == "facet_col" {
            inner_facet_info = Some((mark, "column", false));
            break;
        }
    }

    let Some((inner_mark, inner_channel, inner_is_row_facet)) = inner_facet_info else {
        return Ok(None);
    };

    // Get inner facet's channel expression
    let inner_expr = inner_mark
        .data_context()
        .channels()
        .get(inner_channel)
        .and_then(|cv| cv.expr(ctx));

    let Some(expr) = inner_expr else {
        // Inner facet doesn't have the expected channel expression - unexpected but handle gracefully
        return Ok(None);
    };

    // Extract scale sharing configuration from inner facet mark
    // Default is Free (each outer cell computes its own domain from filtered data)
    let (configured_scale_sharing, inner_facet_spacing) = if inner_is_row_facet {
        let (sharing, spacing) = inner_mark
            .as_any()
            .downcast_ref::<CompiledFacetRow>()
            .map(|f| (f.facet_scale_sharing, f.facet_spacing))
            .unwrap_or((None, None));
        (sharing.unwrap_or(ScaleSharing::Free), spacing)
    } else {
        let (sharing, spacing) = inner_mark
            .as_any()
            .downcast_ref::<CompiledFacetCol>()
            .map(|f| (f.facet_scale_sharing, f.facet_spacing))
            .unwrap_or((None, None));
        (sharing.unwrap_or(ScaleSharing::Free), spacing)
    };

    // Normalize scale sharing mode based on facet orientation
    // For inner FacetRow (row variable inside FacetColumn):
    //   - Shared/Level(1+) → should_share_domain = true (share rows across columns)
    //   - Free/Level(0) → should_share_domain = false (each column has own rows)
    // For inner FacetColumn (column variable inside FacetRow):
    //   - Shared/Level(1+) → should_share_domain = true (share columns across rows)
    //   - Free/Level(0) → should_share_domain = false (each row has own columns)
    //
    // Use is_free() helper: Level(0) is semantically equivalent to Free
    let should_share_domain = !configured_scale_sharing.is_free();

    // For guide ownership coordination, we still use Shared mode so axis labels
    // only appear on edges, regardless of domain sharing configuration
    let inner_scale_sharing = ScaleSharing::Shared;

    // Compute domain from full dataset
    let mut domain_vals = FacetKeyExtractor::extract_keys(df, &expr).await?;
    domain_vals.sort_by(scalar_total_cmp);

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "  Detected nested facet: channel={}, is_row={}, domain_len={}",
            inner_channel,
            inner_is_row_facet,
            domain_vals.len()
        );
    }

    // ========== COMPUTE UNIFORM FREE SCALING MAX CELL COUNT ==========
    // When inner facet uses Free scaling, compute max inner cell count across all outer cells
    // This enables uniform subplot sizing with empty space where data is missing.
    let (max_inner_cell_count, enable_uniform_free_scaling) = if !should_share_domain {
        // Extract outer domain values to iterate over parent cells
        let outer_domain_vals = FacetKeyExtractor::extract_keys(df, outer_facet_expr).await?;

        let mut max_count: usize = 1; // Floor at 1 to prevent divide-by-zero

        for outer_val in &outer_domain_vals {
            // Filter dataset by outer facet value
            let filter_df = df.clone().filter(
                outer_facet_expr
                    .clone()
                    .eq(datafusion::logical_expr::lit(outer_val.clone())),
            );

            let inner_count = if let Ok(filter_df) = filter_df {
                // Extract inner domain from filtered data
                if let Ok(inner_domain_for_cell) =
                    FacetKeyExtractor::extract_keys(&filter_df, &expr).await
                {
                    inner_domain_for_cell.len().max(1)
                } else {
                    1
                }
            } else {
                1
            };

            max_count = max_count.max(inner_count);
        }

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Computed max_inner_cell_count for uniform Free scaling: {} (across {} outer cells)",
                max_count,
                outer_domain_vals.len()
            );
        }

        (Some(max_count), true)
    } else {
        // Domain is shared, don't need uniform Free scaling
        (None, false)
    };

    // ========== COMPUTE SHARED DATA EXTENTS ==========
    // Get the inner facet's compiled subplot to extract x/y channel expressions
    let inner_subplot: Option<&Arc<CompiledPlot>> = if inner_is_row_facet {
        inner_mark
            .as_any()
            .downcast_ref::<CompiledFacetRow>()
            .map(|f| &f.compiled_subplot)
    } else {
        inner_mark
            .as_any()
            .downcast_ref::<CompiledFacetCol>()
            .map(|f| &f.compiled_subplot)
    };

    let mut shared_data_extents: HashMap<String, crate::scales::DomainExtent> = HashMap::new();

    // First, collect channel expressions, share modes, and scale-derived domain kinds
    struct ChannelInfo {
        name: String,
        expr: datafusion::logical_expr::Expr,
        share_mode: ScaleSharing,
        domain_kind: Option<DomainKind>,
        explicit_domain: Option<crate::scales::DomainExtent>,
        domain_sort: Option<DomainSort>,
    }

    impl ChannelInfo {
        fn sort_order(&self) -> DomainSort {
            self.domain_sort.unwrap_or(DomainSort::Ascending)
        }

        fn data_kind(&self, schema: &datafusion::common::DFSchema) -> ChannelDataKind {
            classify_channel(&self.expr, self.domain_kind, schema)
        }
    }

    let mut channel_info: Vec<ChannelInfo> = Vec::new();

    if let Some(inner_subplot) = inner_subplot {
        for channel_name in ["x", "y"] {
            let mut channel_expr: Option<datafusion::logical_expr::Expr> = None;
            let mut share_mode = ScaleSharing::Free;
            let mut domain_kind: Option<DomainKind> = None;
            let mut explicit_domain: Option<crate::scales::DomainExtent> = None;
            let mut domain_sort: Option<DomainSort> = None;

            // First, try to find channel directly in inner_subplot marks
            for mark in &inner_subplot.marks {
                let channels = mark.data_context().channels();
                if let Some(channel_value) = channels.get(channel_name) {
                    if channel_expr.is_none() {
                        channel_expr = channel_value.scale_input_expr(ctx);
                        share_mode = channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                    }

                    // Prefer categorical if ANY mark explicitly configures a categorical scale.
                    let scale_config = channel_value.get_scale_config();
                    let this_kind = scale_config.and_then(|s| s.domain_kind());
                    if this_kind == Some(DomainKind::Categorical) {
                        domain_kind = Some(DomainKind::Categorical);
                    } else if domain_kind.is_none() {
                        domain_kind = this_kind;
                    }

                    if explicit_domain.is_none() {
                        if let Some(scale) = scale_config {
                            explicit_domain =
                                explicit_domain_extents_from_scale(scale, ctx, params).await?;
                        }
                    }

                    if domain_sort.is_none() {
                        if let Some(scale) = scale_config {
                            domain_sort = scale_sort_order(scale, ctx, params).await?;
                        }
                    }
                }
            }

            // If not found directly, recursively search through nested facets
            // This is needed for Level(N>1) where channels are defined in deeply nested subplots
            if channel_expr.is_none() {
                if let Some(found) =
                    find_channel_in_marks(&inner_subplot.marks, channel_name, ctx, 5)
                {
                    channel_expr = Some(found.expr);
                    share_mode = found.share_mode;
                    domain_kind = found.domain_kind;

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "  Found {} channel via recursive search: share_mode={:?}",
                            channel_name, share_mode
                        );
                    }
                }
            }

            if let Some(channel_expr) = channel_expr {
                // Fall back to compiled scale spec if no explicit config was found.
                let domain_kind = domain_kind.or_else(|| {
                    inner_subplot
                        .scale_specs
                        .get(channel_name)
                        .and_then(|spec| match spec {
                            crate::plot::ScaleSpec::Local(scale) => scale.domain_kind(),
                        })
                });

                if explicit_domain.is_none() {
                    if let Some(spec) = inner_subplot.scale_specs.get(channel_name) {
                        match spec {
                            crate::plot::ScaleSpec::Local(scale) => {
                                explicit_domain =
                                    explicit_domain_extents_from_scale(scale, ctx, params).await?;
                            }
                        }
                    }
                }

                if domain_sort.is_none() {
                    if let Some(spec) = inner_subplot.scale_specs.get(channel_name) {
                        match spec {
                            crate::plot::ScaleSpec::Local(scale) => {
                                domain_sort = scale_sort_order(scale, ctx, params).await?;
                            }
                        }
                    }
                }

                channel_info.push(ChannelInfo {
                    name: channel_name.to_string(),
                    expr: channel_expr,
                    share_mode,
                    domain_kind,
                    explicit_domain,
                    domain_sort,
                });
            }
        }

        // Now compute extents based on share mode
        for channel in &channel_info {
            if channel.share_mode.is_fully_shared() {
                if let Some(explicit_extents) = &channel.explicit_domain {
                    shared_data_extents.insert(channel.name.clone(), explicit_extents.clone());
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "  Using explicit domain for shared {}: {:?}",
                            channel.name,
                            shared_data_extents.get(&channel.name)
                        );
                    }
                    continue;
                }

                // Inherit from incoming coordination context if available
                // This is critical for deep nesting (3+ levels) where the middle facet
                // receives filtered data but should use the global domain from the outer facet
                if let Some(inherited_extents) = incoming_coord_ctx
                    .and_then(|ctx| ctx.shared_data_extents.as_ref())
                    .and_then(|extents| extents.get(&channel.name))
                {
                    shared_data_extents.insert(channel.name.clone(), inherited_extents.clone());
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "  Inherited shared {} extents from outer facet: {:?}",
                            channel.name, inherited_extents
                        );
                    }
                    continue;
                }

                let kind = channel.data_kind(df.schema());
                let extents_result =
                    compute_extents(kind, df, &channel.expr, ctx, channel.sort_order()).await;

                if let Ok(extents) = extents_result {
                    shared_data_extents.insert(channel.name.clone(), extents.into());
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        let kind_label = match kind {
                            ChannelDataKind::Categorical => "categorical",
                            ChannelDataKind::Temporal => "temporal",
                            ChannelDataKind::Numeric => "numeric",
                        };
                        eprintln!(
                            "  Computed shared {} extents for {}: {:?}",
                            kind_label,
                            channel.name,
                            shared_data_extents.get(&channel.name)
                        );
                    }
                }
            }
            // For Free and Level(N) modes, no pre-computation needed
        }
    }

    // Create coordination context for guide ownership
    // Domain propagation (for grid-like behavior) is controlled by should_share_domain:
    // - When true (Shared mode): propagate full domain so all outer cells show same inner values
    // - When false (Free mode): let each outer cell compute its own domain from filtered data
    // The actual outer_position and outer_count will be updated per-subplot in work_items.
    let inner_domain_count = if should_share_domain {
        domain_vals.len()
    } else {
        0 // Use 0 to indicate filtered data domain
    };

    let mut coord_ctx = FacetCoordinationContext::new(
        inner_channel,        // Channel name ("row" or "column") for the inner facet
        inner_scale_sharing,  // Scale sharing mode
        GuideOwnership::Full, // Will be refined per-subplot
        0,                    // outer_position - will be set per-subplot in work_items
        0,                    // outer_count - will be set per-subplot in work_items
        inner_domain_count,   // Domain count: >0 for shared domain, 0 for filtered
    );

    // Conditionally propagate domain for grid-like behavior
    if should_share_domain {
        coord_ctx = coord_ctx
            .with_inner_domain(domain_vals.clone())
            .with_empty_cell_fallback(true);

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Nested facet domain sharing ENABLED: propagating {} domain values",
                domain_vals.len()
            );
        }
    } else if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!("  Nested facet domain sharing DISABLED: each cell will compute own domain");
    }

    // Add uniform free scaling fields if computed
    // This enables uniform subplot sizes when inner facet uses Free scaling
    if enable_uniform_free_scaling {
        if let Some(count) = max_inner_cell_count {
            // Compute inner_band_align based on the inner facet's axis position
            // This is needed for guides to compute phantom_prepend locally
            let inner_band_align = if let Some(subplot) = inner_subplot.as_ref() {
                determine_facet_band_align(inner_is_row_facet, subplot)
            } else {
                0.0 // Default to start alignment if subplot not available
            };

            coord_ctx = coord_ctx
                .with_max_inner_cell_count(count)
                .with_uniform_free_scaling(true)
                .with_inner_band_align(inner_band_align);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Enabled uniform Free scaling: max_inner_cell_count={} inner_band_align={:.1}",
                    count, inner_band_align
                );
            }
        }
    }

    // Add shared data extents if we computed any
    // This still helps with scale ranges for shared scales
    if !shared_data_extents.is_empty() {
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Adding shared_data_extents to coord_ctx: {:?}",
                shared_data_extents.keys().collect::<Vec<_>>()
            );
            for (ch, ext) in &shared_data_extents {
                eprintln!("    {}: {:?}", ch, ext);
            }
        }
        coord_ctx = coord_ctx.with_shared_data_extents(shared_data_extents);
    }

    // ========== EXTRACT CHANNEL SHARING LEVELS (Level-based scale sharing) ==========
    // Convert ScaleSharing modes to level values for the new hierarchical system.
    // This enables get_channel_level() and get_domain_for_channel() lookups.
    let channel_sharing_levels: HashMap<String, u8> = {
        let mut levels = HashMap::new();
        for channel in &channel_info {
            levels.insert(channel.name.clone(), channel.share_mode.to_level());
        }
        levels
    };
    if !channel_sharing_levels.is_empty() {
        coord_ctx = coord_ctx.with_channel_sharing_levels(channel_sharing_levels.clone());
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Added channel_sharing_levels to coord context: {:?}",
                coord_ctx.channel_sharing_levels
            );
        }
    }

    // ========== BUILD LEVEL_DOMAINS IndexMap (Level-based domain propagation) ==========
    // For channels with Level(N) where N >= 1, compute domain extents and store
    // at the appropriate level.
    //
    // For deep nesting (3+ levels), we inherit domains from the incoming coordination
    // context and add domains at the current level. This enables Level(2), Level(3), etc.
    // to share domains with grandparent facets.
    //
    // current_depth = incoming_depth + 1 (or 1 if no incoming context)
    // - For 2-level nesting (A > B): current_depth = 1
    // - For 3-level nesting (A > B > C): B has current_depth = 1, C has current_depth = 2
    //
    // Note: This function is called from the OUTER facet while processing nested facets.
    // The df parameter is the OUTER facet's full dataset (before per-cell filtering).
    // Uses IndexMap for deterministic iteration order during serialization.

    // Determine current nesting depth based on incoming context
    let current_depth = incoming_coord_ctx
        .map(|ctx| ctx.nesting_depth + 1)
        .unwrap_or(1);

    // Start with level_domains from incoming context (preserves ancestor domains)
    let mut level_domains: IndexMap<LevelChannelKey, crate::scales::DomainExtent> = incoming_coord_ctx
        .map(|ctx| ctx.level_domains.clone())
        .unwrap_or_default();

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "  Level domain computation: current_depth={}, inherited {} domains from parent",
            current_depth,
            level_domains.len()
        );
    }

    for channel in &channel_info {
        let level = channel.share_mode.to_level();
        if level >= 1 {
            if let Some(explicit_extents) = &channel.explicit_domain {
                let key = LevelChannelKey::new(current_depth, &channel.name);
                level_domains.insert(key, explicit_extents.clone());
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Added explicit level_domain for level={}, channel={}: {:?}",
                        current_depth,
                        channel.name,
                        level_domains.get(&LevelChannelKey::new(current_depth, &channel.name))
                    );
                }
                continue;
            }

            let kind = channel.data_kind(df.schema());
            // For Level(1+) channels, compute domain from the outer's full dataset
            // This ensures all inner facets share the same scale domain
            let extents_result =
                compute_extents(kind, df, &channel.expr, ctx, channel.sort_order()).await;

            if let Ok(extents) = extents_result {
                let key = LevelChannelKey::new(current_depth, &channel.name);
                level_domains.insert(key, extents.into());
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    let kind_label = match kind {
                        ChannelDataKind::Categorical => "categorical",
                        ChannelDataKind::Temporal => "temporal",
                        ChannelDataKind::Numeric => "numeric",
                    };
                    eprintln!(
                        "  Added level_domain for level={}, channel={} ({}): {:?}",
                        current_depth,
                        channel.name,
                        kind_label,
                        level_domains.get(&LevelChannelKey::new(current_depth, &channel.name))
                    );
                }
            }
        }
    }

    if !level_domains.is_empty() {
        // Set nesting depth to current level
        coord_ctx = coord_ctx
            .with_nesting_depth(current_depth)
            .with_level_domains(level_domains);
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Added level_domains to coord context: {} entries, nesting_depth={}",
                coord_ctx.level_domains.len(),
                current_depth
            );
        }
    }

    // Add inner facet's explicit spacing if configured
    // This enables outer facet to account for inner spacing when computing band sizes
    if let Some(spacing) = inner_facet_spacing {
        coord_ctx = coord_ctx.with_inner_facet_spacing(spacing);
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Added inner_facet_spacing to coord context: {} (inner_domain_count={})",
                spacing, coord_ctx.inner_domain_count
            );
        }
    }

    Ok(Some(coord_ctx))
}

/// Extract explicit domain extents from a scale configuration
///
/// Handles both discrete (categorical) and interval (numeric/temporal) domains
async fn explicit_domain_extents_from_scale(
    scale: &crate::scales::Scale<crate::scales::spec::Auto>,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
) -> Result<Option<crate::scales::DomainExtent>, AvengerChartError> {
    use crate::scales::ScaleDefaultDomain;

    let Some(domain) = scale.get_domain() else {
        return Ok(None);
    };

    match &domain.default_domain {
        ScaleDefaultDomain::Discrete(values) => {
            use crate::serialization::LogicalExprNodeExt;
            let exprs = values
                .iter()
                .map(|value| value.to_expr(ctx))
                .collect::<Result<Vec<_>, _>>()?;
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let scalars =
                crate::utils::eval_to_scalars(exprs, Some(ctx), datafusion_params.as_ref()).await?;
            let normalized: Vec<ScalarValue> =
                scalars.into_iter().map(normalize_domain_scalar).collect();
            Ok(Some(
                crate::facet::coordination::SerializableDataExtents::discrete(normalized).into(),
            ))
        }
        ScaleDefaultDomain::Interval(start, end) => {
            use crate::serialization::LogicalExprNodeExt;
            use crate::utils::ScalarValueHelpers;
            let start_expr = start.to_expr(ctx)?;
            let end_expr = end.to_expr(ctx)?;
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let scalars = crate::utils::eval_to_scalars(
                vec![start_expr, end_expr],
                Some(ctx),
                datafusion_params.as_ref(),
            )
            .await?;
            let [start_val, end_val] = scalars.as_slice() else {
                return Err(AvengerChartError::InternalError(
                    "Expected two scalar values for interval domain".to_string(),
                ));
            };

            let extents = if scale.domain_kind() == Some(DomainKind::Temporal) {
                let start_ts =
                    scalar_to_timestamp_ms(start_val).unwrap_or(start_val.as_f64()? as i64);
                let end_ts = scalar_to_timestamp_ms(end_val).unwrap_or(end_val.as_f64()? as i64);
                crate::facet::coordination::SerializableDataExtents::temporal(start_ts, end_ts)
            } else {
                crate::facet::coordination::SerializableDataExtents::interval(
                    start_val.as_f64()?,
                    end_val.as_f64()?,
                )
            };

            Ok(Some(extents.into()))
        }
        ScaleDefaultDomain::DomainExprs(_) | ScaleDefaultDomain::NoDefault => Ok(None),
    }
}

/// Extract sort order from scale configuration
async fn scale_sort_order(
    scale: &crate::scales::Scale<crate::scales::spec::Auto>,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
) -> Result<Option<DomainSort>, AvengerChartError> {
    use crate::serialization::LogicalExprNodeExt;

    let Some(value_node) = scale.get_options().get("sort") else {
        return Ok(None);
    };

    let expr = value_node.to_expr(ctx)?;
    let datafusion_params = crate::utils::params_to_datafusion(params);
    let mut scalars =
        crate::utils::eval_to_scalars(vec![expr], Some(ctx), datafusion_params.as_ref()).await?;
    let scalar = scalars
        .pop()
        .ok_or_else(|| AvengerChartError::InternalError("Missing sort option value".to_string()))?;

    Ok(parse_sort_scalar(&scalar))
}

/// Parse a scalar value as a sort order
fn parse_sort_scalar(value: &ScalarValue) -> Option<DomainSort> {
    match value {
        ScalarValue::Boolean(Some(true)) => Some(DomainSort::Ascending),
        ScalarValue::Boolean(Some(false)) => Some(DomainSort::None),
        ScalarValue::Int8(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int16(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int32(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int64(Some(v)) => Some(parse_sort_int(*v)),
        ScalarValue::UInt8(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt16(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt32(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt64(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => parse_sort_string(s),
        _ => None,
    }
}

/// Parse an integer value as a sort order
fn parse_sort_int(value: i64) -> DomainSort {
    if value == 0 {
        DomainSort::None
    } else if value < 0 {
        DomainSort::Descending
    } else {
        DomainSort::Ascending
    }
}

/// Parse a string value as a sort order
fn parse_sort_string(value: &str) -> Option<DomainSort> {
    match value.trim().to_lowercase().as_str() {
        "asc" | "ascending" => Some(DomainSort::Ascending),
        "desc" | "descending" => Some(DomainSort::Descending),
        "none" | "false" | "natural" | "data" => Some(DomainSort::None),
        "true" => Some(DomainSort::Ascending),
        _ => None,
    }
}
