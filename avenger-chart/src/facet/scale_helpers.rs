//! Shared scale building helpers for faceting (STUBBED)
//!
//! This module is stubbed as part of the facet fresh start refactoring.
//! The complex coordination-dependent scale building has been removed.

use crate::channel::config_traits::ScaleSharing;
use crate::error::AvengerChartError;
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
        compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await
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
/// ScaleBuilder for empty cells.
pub async fn build_scales_with_fallback(
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
    let scales = build_scales_helper(
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

    // Check if we need fallback (empty cells)
    let has_all_scales = compiled_subplot
        .coord_transform
        .required_channels()
        .iter()
        .all(|ch| scales.contains_key(*ch));

    if has_all_scales {
        Ok(scales)
    } else if let Some(fallback) = fallback_builder {
        // Use fallback builder for missing scales
        compiled_subplot
            .build_scales_from_builder(fallback, plot_width, plot_height, ctx, params)
            .await
    } else {
        Ok(scales)
    }
}

/// Get the effective scale sharing mode for a channel (STUBBED)
///
/// Returns the sharing mode from the explicit configuration, ignoring
/// any coordination context (which has been removed).
pub fn get_effective_sharing_mode(
    channel: &str,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
) -> ScaleSharing {
    scale_sharing_by_channel
        .get(channel)
        .cloned()
        .unwrap_or(ScaleSharing::Level(0)) // Default to Free
}

/// Build scales directly per-channel based on sharing mode (STUBBED)
///
/// This function is stubbed. The coordination context-based level domain
/// lookup has been removed.
#[allow(clippy::too_many_arguments)]
pub async fn build_scales_per_channel(
    compiled_subplot: &CompiledPlot,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
    shared_scales: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    free_scale_builder: &Option<ScaleBuilder>,
    fallback_builder: &Option<ScaleBuilder>,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    // STUBBED: Just delegate to the simple helper
    // The coordination context-based level domain lookup has been removed

    // Use shared scales for Shared channels, free builder for Free channels
    let mut result = HashMap::new();

    // Collect all channels from available sources
    let mut all_channels: HashSet<String> = HashSet::new();
    if let Some(builder) = free_scale_builder {
        all_channels.extend(builder.channel_builders().keys().cloned());
    }
    if let Some(shared) = shared_scales {
        all_channels.extend(shared.keys().cloned());
    }

    for channel in &all_channels {
        let sharing = get_effective_sharing_mode(channel, scale_sharing_by_channel);

        match sharing {
            ScaleSharing::Shared | ScaleSharing::Level(255) => {
                // Use shared scale
                if let Some(shared) = shared_scales {
                    if let Some(scale) = shared.get(channel) {
                        result.insert(channel.clone(), scale.clone());
                        continue;
                    }
                }
            }
            _ => {
                // Free scale - build from builder
            }
        }
    }

    // Build remaining scales from free_scale_builder or fallback
    if let Some(builder) = free_scale_builder {
        let built = compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await?;
        for (ch, scale) in built {
            result.entry(ch).or_insert(scale);
        }
    } else if let Some(fallback) = fallback_builder {
        let built = compiled_subplot
            .build_scales_from_builder(fallback, plot_width, plot_height, ctx, params)
            .await?;
        for (ch, scale) in built {
            result.entry(ch).or_insert(scale);
        }
    }

    Ok(result)
}
