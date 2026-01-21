//! Shared scale building helpers for faceting
//!
//! This module provides common scale building logic used by all facet types
//! (FacetRow, FacetCol, GridFacet) to ensure consistent behavior.

use crate::channel::config_traits::ScaleSharing;
use crate::error::AvengerChartError;
use crate::facet::coordination::{FacetCoordinationContext, SerializableDataExtents};
use crate::plot::compiled::CompiledPlot;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::builder::ScaleBuilder;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::collections::HashSet;

/// Build scales for a subplot, using shared scales or a ScaleBuilder if provided
///
/// This helper centralizes the scale building logic for faceted layouts:
/// - If `shared_scales_opt` is Some, returns those scales (shared across subplots)
/// - If `shared_scales_opt` is None and `free_scale_builder` is Some, uses the builder for free scales
/// - Otherwise, builds fresh scales from the filtered data (fallback for backward compatibility)
///
/// Using ScaleBuilder enables radius-aware domain inference for both shared and free scales.
///
/// This function is used by all facet types to ensure consistent scale handling.
///
/// # Arguments
/// * `compiled_subplot` - The compiled subplot specification
/// * `shared_scales_opt` - Optional shared scales (if Some, returns these)
/// * `free_scale_builder` - Optional ScaleBuilder for building free scales with radius support
/// * `filter_df` - The filtered dataframe for this specific subplot
/// * `plot_width` - Width of the subplot plot area
/// * `plot_height` - Height of the subplot plot area
/// * `ctx` - DataFusion session context
/// * `params` - Parameters including FacetContext
pub async fn build_scales_helper(
    compiled_subplot: &CompiledPlot,
    shared_scales_opt: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    free_scale_builder: &Option<ScaleBuilder>,
    filter_df: &DataFrame,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    if let Some(shared) = shared_scales_opt {
        Ok(shared.clone())
    } else if let Some(builder) = free_scale_builder {
        // Use ScaleBuilder for radius-aware free scales
        let built = compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await?;

        // Defensive backfill removed.
        // The fallback to build_scales_for_dataframe is not radius-aware and
        // was causing symbol clipping in faceted plots with free scales.
        // Removing this ensures that only the radius-aware ScaleBuilder path is used.
        // Any failures in the builder should now surface as proper errors rather than
        // being silently handled by an incorrect implementation.

        Ok(built)
    } else {
        // Fallback to simple build (no radius support)
        compiled_subplot
            .build_scales_for_dataframe(filter_df, plot_width, plot_height, ctx, params)
            .await
    }
}

/// Build scales for a subplot with fallback support for empty cells
///
/// This is an extended version of `build_scales_helper` that supports a fallback
/// ScaleBuilder for empty cells. When domain propagation creates subplots for
/// domain values that have no data (empty cells), the primary scales will be empty.
/// This function uses the fallback builder to provide scales even for empty cells,
/// ensuring axes and gridlines are rendered correctly.
///
/// # Arguments
/// * `compiled_subplot` - The compiled subplot specification
/// * `shared_scales_opt` - Optional shared scales (if Some, returns these)
/// * `free_scale_builder` - Optional ScaleBuilder for building free scales
/// * `fallback_builder` - Optional ScaleBuilder from full dataset for empty cells
/// * `filter_df` - The filtered dataframe for this specific subplot
/// * `plot_width` - Width of the subplot plot area
/// * `plot_height` - Height of the subplot plot area
/// * `ctx` - DataFusion session context
/// * `params` - Parameters including FacetContext
pub async fn build_scales_helper_with_fallback(
    compiled_subplot: &CompiledPlot,
    shared_scales_opt: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    free_scale_builder: &Option<ScaleBuilder>,
    fallback_builder: &Option<ScaleBuilder>,
    filter_df: &DataFrame,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    // First, try the normal scale building
    let mut scales = build_scales_helper(
        compiled_subplot,
        shared_scales_opt,
        free_scale_builder,
        filter_df,
        plot_width,
        plot_height,
        ctx,
        params,
    )
    .await?;

    // If scales are empty and we have a fallback, use it
    if scales.is_empty() {
        if let Some(builder) = fallback_builder {
            scales = compiled_subplot
                .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
                .await?;
        }
    } else if let Some(builder) = fallback_builder {
        // Even if we got some scales, backfill any missing required scales from fallback
        // This ensures all required positional channels (x, y, x2, y2) are present
        let fallback_scales = compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await?;

        for (scale_name, scale) in fallback_scales {
            // Only insert if not already present (don't overwrite cell-specific scales)
            scales.entry(scale_name).or_insert(scale);
        }
    }

    Ok(scales)
}

/// Collect all channel names from available builders
///
/// This function gathers channels from all available sources (free builder,
/// shared scales, fallback builder) to ensure non-positional scales like
/// color, size, and shape are included alongside positional axes.
///
/// # Arguments
/// * `free_scale_builder` - Per-cell builder for Free/Level(0) channels
/// * `shared_scales` - Pre-built scales for Shared/Level(255) channels
/// * `fallback_builder` - Fallback builder for empty cells
///
/// # Returns
/// A set of all channel names from any available source
pub fn get_all_channels(
    free_scale_builder: &Option<ScaleBuilder>,
    shared_scales: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    fallback_builder: &Option<ScaleBuilder>,
) -> HashSet<String> {
    let mut channels = HashSet::new();

    // Collect from free_scale_builder
    if let Some(builder) = free_scale_builder {
        channels.extend(builder.channel_builders().keys().cloned());
    }

    // Collect from shared_scales
    if let Some(scales) = shared_scales {
        channels.extend(scales.keys().cloned());
    }

    // Collect from fallback_builder
    if let Some(builder) = fallback_builder {
        channels.extend(builder.channel_builders().keys().cloned());
    }

    channels
}

/// Get the effective sharing mode for a channel
///
/// This function merges the explicit `scale_sharing_by_channel` with the
/// inherited sharing levels from `coord_ctx.channel_sharing_levels`.
/// The coordination context takes precedence for Level(N > 0) sharing,
/// as it represents inherited sharing from parent facets.
///
/// # Arguments
/// * `channel` - Channel name (e.g., "x", "y", "color")
/// * `scale_sharing_by_channel` - Explicit sharing modes from the facet spec
/// * `coord_ctx` - Optional coordination context with inherited sharing levels
///
/// # Returns
/// The effective ScaleSharing mode for this channel
pub fn get_effective_sharing_mode(
    channel: &str,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
    coord_ctx: Option<&FacetCoordinationContext>,
) -> ScaleSharing {
    // First check coord_ctx for inherited Level(N) sharing
    // The coordination context represents inherited sharing from parent facets
    if let Some(ctx) = coord_ctx {
        let level = ctx.get_channel_level(channel);
        if level > 0 {
            // Inherited sharing level takes precedence
            return ScaleSharing::Level(level);
        }
    }

    // Fall back to explicit sharing mode from the facet spec
    scale_sharing_by_channel
        .get(channel)
        .cloned()
        .unwrap_or(ScaleSharing::Level(0)) // Default to Free
}

/// Build scales directly per-channel based on sharing mode
///
/// This replaces the two-step (initial build + overlay) approach with
/// a single pass that builds each channel from the correct domain source.
///
/// # Algorithm
///
/// 1. Collect all channels from available builders (including non-positional like color)
/// 2. For each channel, determine effective sharing mode (merging coord_ctx)
/// 3. Build scales based on sharing level:
///    - Level(0): Build from free_scale_builder (per-cell data)
///    - Level(255): Use pre-built shared_scales
///    - Level(1..254): Extend free_scale_builder with level domain, then build
///
/// # Arguments
///
/// * `compiled_subplot` - The subplot specification
/// * `scale_sharing_by_channel` - Sharing mode for each channel
/// * `shared_scales` - Pre-built scales for Shared/Level(255) channels
/// * `free_scale_builder` - Per-cell builder for Free/Level(0) channels
/// * `coord_ctx` - Coordination context for Level(1..254) domain lookup
/// * `fallback_builder` - Fallback for empty cells
/// * `plot_width`, `plot_height` - Subplot dimensions
/// * `ctx` - DataFusion session context
/// * `params` - Parameters including FacetContext
#[allow(clippy::too_many_arguments)]
pub async fn build_scales_per_channel(
    compiled_subplot: &CompiledPlot,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
    shared_scales: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    free_scale_builder: &Option<ScaleBuilder>,
    coord_ctx: Option<&FacetCoordinationContext>,
    fallback_builder: &Option<ScaleBuilder>,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    use crate::error::AvengerChartError::InternalError;

    // Debug logging
    let debug = std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok();
    if debug {
        eprintln!("build_scales_per_channel: starting");
        eprintln!("  scale_sharing_by_channel: {:?}", scale_sharing_by_channel);
        eprintln!("  shared_scales present: {}", shared_scales.is_some());
        eprintln!(
            "  free_scale_builder present: {}",
            free_scale_builder.is_some()
        );
        eprintln!("  coord_ctx present: {}", coord_ctx.is_some());
        eprintln!("  fallback_builder present: {}", fallback_builder.is_some());
    }

    let mut result_scales: HashMap<String, ConfiguredScaleWithSpec> = HashMap::new();

    // Collect all channels from any available source
    let all_channels = get_all_channels(free_scale_builder, shared_scales, fallback_builder);

    if debug {
        eprintln!("  all_channels: {:?}", all_channels);
    }

    // If no channels, return empty (valid for empty cells)
    if all_channels.is_empty() {
        return Ok(result_scales);
    }

    // Categorize channels by their effective sharing level
    let mut free_channels: Vec<String> = Vec::new();
    let mut shared_channels: Vec<String> = Vec::new();
    let mut level_n_channels: Vec<(String, u8)> = Vec::new();

    for channel in &all_channels {
        let sharing_mode = get_effective_sharing_mode(channel, scale_sharing_by_channel, coord_ctx);
        let level = sharing_mode.to_level();

        match level {
            0 => free_channels.push(channel.clone()),
            255 => shared_channels.push(channel.clone()),
            n => level_n_channels.push((channel.clone(), n)),
        }
    }

    if debug {
        eprintln!("  free_channels: {:?}", free_channels);
        eprintln!("  shared_channels: {:?}", shared_channels);
        eprintln!("  level_n_channels: {:?}", level_n_channels);
    }

    // Step 1: Handle Level(255)/Shared channels - copy from pre-built shared_scales
    // If not in shared_scales, fall back to extending free_scale_builder with shared_data_extents
    let mut shared_fallback_channels: Vec<String> = Vec::new();
    for channel in &shared_channels {
        if let Some(scales) = shared_scales {
            if let Some(scale) = scales.get(channel) {
                result_scales.insert(channel.clone(), scale.clone());
                if debug {
                    eprintln!("  Shared channel '{}': using pre-built scale", channel);
                }
                continue;
            }
        }
        // Channel not in shared_scales - need fallback
        shared_fallback_channels.push(channel.clone());
        if debug {
            eprintln!(
                "  Shared channel '{}' not in shared_scales - will try shared_data_extents fallback",
                channel
            );
        }
    }

    // Handle Shared channels that weren't in shared_scales by extending free_scale_builder
    // with inherited shared_data_extents from coordination context
    if !shared_fallback_channels.is_empty() {
        if let Some(coord) = coord_ctx {
            if let Some(shared_extents) = &coord.shared_data_extents {
                // Filter to only channels we need
                let needed_extents: std::collections::HashMap<String, _> = shared_extents
                    .iter()
                    .filter(|(ch, _)| shared_fallback_channels.contains(ch))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                if !needed_extents.is_empty() {
                    if let Some(builder) = free_scale_builder.as_ref().or(fallback_builder.as_ref())
                    {
                        // Clone and extend the builder with inherited shared extents
                        let mut extended_builder = builder.clone();
                        extended_builder.extend_with_shared_extents(&needed_extents);

                        if debug {
                            eprintln!(
                                "  Extended builder with inherited shared_data_extents for channels: {:?}",
                                needed_extents.keys().collect::<Vec<_>>()
                            );
                        }

                        // Build scales from extended builder
                        let built_scales = compiled_subplot
                            .build_scales_from_builder(
                                &extended_builder,
                                plot_width,
                                plot_height,
                                ctx,
                                params,
                            )
                            .await?;

                        // Extract the Shared channels
                        for channel in &shared_fallback_channels {
                            if let Some(scale) = built_scales.get(channel) {
                                result_scales.insert(channel.clone(), scale.clone());
                                if debug {
                                    eprintln!(
                                        "  Shared channel '{}': built from extended free_scale_builder with inherited extents",
                                        channel
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 2: Handle Level(0)/Free channels - build from free_scale_builder
    if !free_channels.is_empty() {
        // Determine which builder to use (free or fallback)
        let builder_to_use = free_scale_builder.as_ref().or(fallback_builder.as_ref());

        if let Some(builder) = builder_to_use {
            if debug {
                eprintln!(
                    "  Building Free scales from builder (channels: {:?})",
                    builder.channel_builders().keys().collect::<Vec<_>>()
                );
            }

            // Build all scales from the builder
            let built_scales = compiled_subplot
                .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
                .await?;

            // Extract only the Free channels
            for channel in &free_channels {
                if let Some(scale) = built_scales.get(channel) {
                    result_scales.insert(channel.clone(), scale.clone());
                    if debug {
                        eprintln!("  Free channel '{}': built from per-cell data", channel);
                    }
                } else if debug {
                    eprintln!(
                        "  WARNING: Free channel '{}' not in built_scales (keys: {:?})",
                        channel,
                        built_scales.keys().collect::<Vec<_>>()
                    );
                }
            }
        } else if debug {
            eprintln!("  No builder available for Free channels - skipping (empty cell case)");
        }
    }

    // Step 3: Handle Level(1..254) channels - extend builder with level domain, then build
    if !level_n_channels.is_empty() {
        // For Level(N), we need the coordination context to look up domains.
        // If coord_ctx is None, fall back to shared_scales (full dataset domain)
        // to ensure consistent behavior between measurement and render passes.
        let coord_ctx = match coord_ctx {
            Some(ctx) => ctx,
            None => {
                // No coord_ctx - try to use shared_scales as fallback
                // This ensures Level(N) channels use full dataset domain even without coord_ctx
                if debug {
                    let channels: Vec<_> = level_n_channels
                        .iter()
                        .map(|(c, l)| format!("{}(L{})", c, l))
                        .collect();
                    eprintln!(
                        "  Level(N) channels {:?} have no coord_ctx - trying shared_scales fallback",
                        channels
                    );
                }

                // First, try to use shared_scales for Level(N) channels
                let mut remaining_level_n: Vec<(String, u8)> = Vec::new();
                for (channel, level) in level_n_channels.drain(..) {
                    if let Some(scales) = shared_scales {
                        if let Some(scale) = scales.get(&channel) {
                            result_scales.insert(channel.clone(), scale.clone());
                            if debug {
                                eprintln!(
                                    "  Level({}) channel '{}': using shared_scales (no coord_ctx)",
                                    level, channel
                                );
                            }
                            continue;
                        }
                    }
                    // Not in shared_scales, treat as Free
                    remaining_level_n.push((channel, level));
                }

                // For Level(N) channels not in shared_scales, treat as Free
                if !remaining_level_n.is_empty() {
                    if debug {
                        let channels: Vec<_> = remaining_level_n
                            .iter()
                            .map(|(c, l)| format!("{}(L{})", c, l))
                            .collect();
                        eprintln!(
                            "  Level(N) channels {:?} not in shared_scales - treating as Free",
                            channels
                        );
                    }
                    for (channel, _level) in remaining_level_n {
                        free_channels.push(channel);
                    }
                }

                // Re-process Free channels if we just added some
                if !free_channels.is_empty() {
                    let builder_to_use = free_scale_builder.as_ref().or(fallback_builder.as_ref());
                    if let Some(builder) = builder_to_use {
                        let built_scales = compiled_subplot
                            .build_scales_from_builder(
                                builder,
                                plot_width,
                                plot_height,
                                ctx,
                                params,
                            )
                            .await?;
                        for channel in &free_channels {
                            if let Some(scale) = built_scales.get(channel) {
                                result_scales.insert(channel.clone(), scale.clone());
                            }
                        }
                    }
                }
                // Early return since we've handled all channels
                // Backfill from fallback still needs to happen
                if let Some(builder) = fallback_builder {
                    let fallback_scales = compiled_subplot
                        .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
                        .await?;
                    for (scale_name, scale) in fallback_scales {
                        result_scales.entry(scale_name).or_insert(scale);
                    }
                }
                return Ok(result_scales);
            }
        };

        // Get the base builder
        let base_builder = free_scale_builder
            .as_ref()
            .or(fallback_builder.as_ref())
            .ok_or_else(|| {
                let channels: Vec<_> = level_n_channels
                    .iter()
                    .map(|(c, l)| format!("{}(L{})", c, l))
                    .collect();
                InternalError(format!(
                    "No builder for Level(N) channels {:?}. free={}, fallback={}",
                    channels,
                    free_scale_builder.is_some(),
                    fallback_builder.is_some()
                ))
            })?;

        // Clone the builder and extend with level domains for all Level(N) channels
        let mut extended_builder = base_builder.clone();

        // Collect all domain extensions
        // Use get_domain_for_channel_with_level which accepts the level explicitly,
        // since the level comes from scale_sharing_by_channel and may not be in
        // coord_ctx.channel_sharing_levels
        let mut extents: HashMap<String, SerializableDataExtents> = HashMap::new();
        let mut channels_with_no_domain: Vec<(String, u8)> = Vec::new();

        for (channel, level) in &level_n_channels {
            if let Some(domain) = coord_ctx.get_domain_for_channel_with_level(channel, *level) {
                extents.insert(channel.clone(), domain.clone());
                if debug {
                    eprintln!(
                        "  Level({}) channel '{}': extending with domain {:?}",
                        level, channel, domain
                    );
                }
            } else {
                // Domain not found in coord_ctx - will fall back to shared_scales
                channels_with_no_domain.push((channel.clone(), *level));
                if debug {
                    eprintln!(
                        "  Level({}) channel '{}': no domain in coord_ctx (nesting_depth={}), will use shared_scales",
                        level, channel, coord_ctx.nesting_depth
                    );
                }
            }
        }

        // Apply all extensions at once (UNION semantics)
        if !extents.is_empty() {
            extended_builder.extend_with_shared_extents(&extents);
        }

        // Build scales from extended builder for channels that had domains
        let extended_scales = compiled_subplot
            .build_scales_from_builder(&extended_builder, plot_width, plot_height, ctx, params)
            .await?;

        // Extract Level(N) channels that had domains
        for (channel, level) in &level_n_channels {
            // Skip channels that didn't have domains (handled below)
            if channels_with_no_domain.iter().any(|(c, _)| c == channel) {
                continue;
            }

            if let Some(scale) = extended_scales.get(channel) {
                result_scales.insert(channel.clone(), scale.clone());
                if debug {
                    eprintln!(
                        "  Level({}) channel '{}': built from extended builder",
                        level, channel
                    );
                }
            } else if debug {
                eprintln!(
                    "  WARNING: Level({}) channel '{}' not in extended_scales (keys: {:?})",
                    level,
                    channel,
                    extended_scales.keys().collect::<Vec<_>>()
                );
            }
        }

        // For Level(N) channels without domains, fall back to shared_scales
        // This handles single-level facets where Level(N) should use the full dataset domain
        for (channel, level) in channels_with_no_domain {
            if let Some(scales) = shared_scales {
                if let Some(scale) = scales.get(&channel) {
                    result_scales.insert(channel.clone(), scale.clone());
                    if debug {
                        eprintln!(
                            "  Level({}) channel '{}': using shared_scales (fallback for no coord_ctx domain)",
                            level, channel
                        );
                    }
                    continue;
                }
            }
            // If not in shared_scales, use the extended builder result anyway
            if let Some(scale) = extended_scales.get(&channel) {
                result_scales.insert(channel.clone(), scale.clone());
                if debug {
                    eprintln!(
                        "  Level({}) channel '{}': built from extended builder (no shared_scale fallback)",
                        level, channel
                    );
                }
            }
        }
    }

    // Backfill any missing required scales from fallback (for x2/y2 support)
    if let Some(builder) = fallback_builder {
        let fallback_scales = compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await?;

        for (scale_name, scale) in fallback_scales {
            // Only insert if not already present (don't overwrite cell-specific scales)
            result_scales.entry(scale_name).or_insert(scale);
        }
    }

    if debug {
        eprintln!(
            "build_scales_per_channel: complete, result keys: {:?}",
            result_scales.keys().collect::<Vec<_>>()
        );
    }

    Ok(result_scales)
}
