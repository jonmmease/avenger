//! Generic measurement infrastructure for facet guides.
//!
//! This module contains the `measure_with_coordination_impl` generic function
//! and the `AsyncMeasureOverflowFn` trait that enable shared measurement logic
//! between FacetRowGuide and FacetColGuide.

use crate::scales::DomainExtent;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::facet::guide::shared::{FacetSource, compute_scale_sharing_for_nested_facet};
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::guide::{MeasurementResult, OverflowSpaceRequirement, spacing_keys};
use crate::scales::ConfiguredScaleLegendExt;
use crate::channel::config_traits::ScaleSharing;
use avenger_scales::scales::ConfiguredScale;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Trait for async measure_overflow function to enable generic callback
#[allow(async_fn_in_trait)]
pub trait AsyncMeasureOverflowFn {
    async fn call(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        own_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        other_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError>;
}

/// Generic implementation of two-pass measurement with coordination for facet guides.
///
/// This implements the measure_with_coordination pattern shared by FacetRowGuide and FacetColGuide:
/// 1. Check recursion and nesting guards
/// 2. Pass 1: Measure with zero gap to determine overflow
/// 3. Compute inter-cell gap from Pass 1 overflow
/// 4. Pass 2: Re-measure with computed gap
/// 5. Return aggregated overflow with spacing needs
///
/// # Type Parameters
/// - `D`: The dimension configuration (RowDimensionConfig or ColumnDimensionConfig)
///
/// # Arguments
/// - `facet_sources`: The facet sources to measure
/// - `unified_title`: Optional unified title (for unified_y check in Column)
/// - `scales`: The configured scales (must contain the dimension's channel)
/// - `own_overflow`: Overflow for this dimension (unused in recursion guard pass-through)
/// - `other_overflow`: Overflow for the other dimension (passed to child measure_with_coordination)
/// - `plot_width`, `plot_height`: Plot dimensions
/// - `theme`: The theme for styling
/// - `params`: Parameters including FacetCoordinationContext
/// - `data_override`: Optional data override for nested scenarios
/// - `ctx`: DataFusion session context
#[allow(clippy::too_many_arguments)]
pub async fn measure_with_coordination_impl<D: FacetDimensionConfig>(
    facet_sources: &[FacetSource],
    unified_title: Option<&String>,
    scales: &HashMap<String, ConfiguredScale>,
    _own_overflow: Option<&Vec<OverflowSpaceRequirement>>,
    other_overflow: Option<&Vec<OverflowSpaceRequirement>>,
    plot_width: f32,
    plot_height: f32,
    theme: &crate::theme::Theme,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    data_override: Option<&datafusion::dataframe::DataFrame>,
    ctx: &SessionContext,
    measure_overflow_fn: impl AsyncMeasureOverflowFn,
) -> Result<MeasurementResult, crate::error::AvengerChartError> {
    use crate::facet::coordination::FacetCoordinationContext;
    use crate::facet::guide_measurement::aggregate_overflow;
    use tracing::debug;

    /// Safety margin for gap calculation (accounts for potential measurement variance)
    const GAP_SAFETY_MARGIN: f32 = 1.0;

    let channel_name = D::channel_name();
    let inter_gap_key = D::inter_gap_spacing_key();

    // Guard: Invalid dimensions
    if plot_height <= 0.0 || plot_width <= 0.0 {
        return Ok(MeasurementResult::default());
    }

    // Guard: No facet sources
    if facet_sources.is_empty() {
        return Ok(MeasurementResult::default());
    }

    // Parse coordination context ONCE
    let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();

    // RECURSION GUARD: If already coordinated, use single pass (measure_overflow)
    if coord_ctx.get_coordinated_spacing(inter_gap_key).is_some() {
        debug!(
            "Facet{}Guide: Recursion guard - using pre-coordinated {}",
            if D::is_row_facet() { "Row" } else { "Col" },
            inter_gap_key
        );
        let overflow = measure_overflow_fn
            .call(
                scales,
                _own_overflow,
                other_overflow,
                plot_width,
                plot_height,
                theme,
                params,
                data_override,
                ctx,
            )
            .await?;
        return Ok(MeasurementResult::new(overflow));
    }

    // NESTING GUARD: If we're inside another guide's measure_with_coordination,
    // skip 2-pass to avoid deep async recursion causing stack overflow.
    if coord_ctx
        .get_coordinated_spacing(spacing_keys::NESTED_MEASUREMENT)
        .is_some()
    {
        debug!(
            "Facet{}Guide: Nesting guard - NESTED_MEASUREMENT set, using single-pass",
            if D::is_row_facet() { "Row" } else { "Col" }
        );
        let overflow = measure_overflow_fn
            .call(
                scales,
                _own_overflow,
                other_overflow,
                plot_width,
                plot_height,
                theme,
                params,
                data_override,
                ctx,
            )
            .await?;
        return Ok(MeasurementResult::new(overflow));
    }

    // Get dimension scale
    let dim_scale = scales.get(channel_name).ok_or_else(|| {
        crate::error::AvengerChartError::InternalError(
            format!(
                "Missing '{}' scale for Facet{}Guide",
                channel_name,
                if D::is_row_facet() { "Row" } else { "Col" }
            )
            .into(),
        )
    })?;

    // Extract domain values
    let mut domain_vals = match dim_scale.domain_values()? {
        crate::scales::extensions::DomainValues::Discrete(vals) => vals,
        _ => vec![],
    };
    domain_vals.sort_by(scalar_total_cmp);

    // For uniform Free scaling, use max cell count instead of actual domain length
    let num_cells = coord_ctx
        .get_uniform_cell_count()
        .unwrap_or(domain_vals.len())
        .max(1);

    // === PASS 1: Measure with zero gap ===
    let pass1_band_size = D::compute_band_size(plot_width, plot_height, num_cells, 0.0);
    let (pass1_width, pass1_height) =
        D::subplot_dimensions_exact(pass1_band_size, plot_width, plot_height);

    debug!(
        "Facet{}Guide: Pass 1 - measuring {} cells with zero gap, band_size={:.2}",
        if D::is_row_facet() { "Row" } else { "Col" },
        num_cells,
        pass1_band_size
    );

    let mut cell_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_cells);
    let mut aggregated_spacing: std::collections::HashMap<String, f32> =
        std::collections::HashMap::new();

    // Get the first facet source (primary source for measurement)
    let source = &facet_sources[0];

    // Get facet expression
    let facet_expr = source
        .data
        .channels()
        .get(channel_name)
        .and_then(|cv| cv.expr(ctx));

    // Get the dataframe for measurement
    let df = data_override
        .cloned()
        .or_else(|| source.data.dataframe_with_context(ctx));

    if let (Some(expr), Some(df)) = (facet_expr, df) {
        use datafusion::logical_expr::lit;

        // Compute scale sharing from marks BEFORE building scales
        // (needed for Level(N) domain extension)
        let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

        // Build scales for subplot measurement
        let mut builder = source
            .subplot
            .build_scale_builder_from_dataframe(ctx, params, &df)
            .await?;

        // Extend with level-based extents from coordination context for Level(N>=1) channels
        // ONLY for innermost subplots (Cartesian) - not for nested facet guides.
        // When the subplot has a compiled_guide, it's a nested facet that will handle
        // its own scale extension during its measurement. We only extend here when
        // measuring Cartesian subplots directly.
        if source.subplot.compiled_guide.is_none() {
            let level_extents: HashMap<String, DomainExtent> = scale_sharing
                .iter()
                .filter_map(|(channel, mode)| {
                    if let ScaleSharing::Level(n) = mode {
                        if *n >= 1 {
                            coord_ctx
                                .get_domain_for_channel_with_level(channel, *n)
                                .map(|extents| (channel.clone(), extents.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            if !level_extents.is_empty() {
                builder.extend_with_domain_extents(&level_extents);
            }
        }

        let subplot_scales = source
            .subplot
            .build_scales_from_builder(&builder, pass1_width, pass1_height, ctx, params)
            .await?;

        // Convert to ConfiguredScale for measure_with_coordination trait method
        let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        // Measure each cell in Pass 1
        for (cell_idx, domain_val) in domain_vals.iter().enumerate() {
            use crate::facet::context::FacetContext;

            let filter_df = df
                .clone()
                .filter(expr.clone().eq(lit(domain_val.clone())))?;

            // Create FacetContext for this cell with NESTED_MEASUREMENT marker
            let measure_params = {
                let mut updated_params = params.clone();
                let position = D::index_to_position(cell_idx);
                let grid_dimensions = D::count_to_grid_dimensions(num_cells);

                // Compute global_edge_tracked_channels and global_edge_channels
                // For Row (default="y"), orthogonal channel is "x" - track if parent tracks or Level(2+)
                // For Col (default="x"), orthogonal channel is "y" - track if parent tracks or Level(2+)
                let default_unified: std::collections::HashSet<&str> =
                    D::unified_channels().iter().copied().collect();
                let orthogonal_channel = if default_unified.contains(&"y") { "x" } else { "y" };
                let orthogonal_has_multilevel_sharing = scale_sharing
                    .get(orthogonal_channel)
                    .map(|s| matches!(s, ScaleSharing::Level(n) if *n >= 2))
                    .unwrap_or(false);

                let (global_edge_tracked_channels, global_edge_channels) = if let Some(parent_ctx) = FacetContext::from_params(params) {
                    // Check if parent is tracking orthogonal channel OR if it has Level(2+) sharing
                    let parent_tracking = parent_ctx.global_edge_tracked_channels.contains(orthogonal_channel);
                    let should_track = parent_tracking || orthogonal_has_multilevel_sharing;

                    if should_track {
                        let (row, col) = position;
                        let (num_rows, _num_cols) = grid_dimensions;

                        // Start with parent's tracked channels
                        let mut tracked = parent_ctx.global_edge_tracked_channels.clone();
                        if orthogonal_has_multilevel_sharing {
                            tracked.insert(orthogonal_channel.to_string());
                        }

                        // Inherit parent's global_edge_channels, or start fresh if parent wasn't tracking
                        let parent_edge = if parent_tracking {
                            parent_ctx.global_edge_channels.clone()
                        } else {
                            tracked.clone()
                        };

                        let at_edge = parent_edge
                            .into_iter()
                            .filter(|ch| tracked.contains(ch))
                            .filter(|ch| match ch.as_str() {
                                "x" => row == num_rows - 1, // Bottom edge
                                "y" => col == 0,           // Left edge
                                _ => true,
                            })
                            .collect();
                        (tracked, at_edge)
                    } else {
                        (std::collections::HashSet::new(), std::collections::HashSet::new())
                    }
                } else if orthogonal_has_multilevel_sharing {
                    // No parent but orthogonal has Level(2+) - start tracking at top level
                    let mut tracked = std::collections::HashSet::new();
                    tracked.insert(orthogonal_channel.to_string());
                    // Only mark as at-edge if at appropriate edge for the channel
                    let (row, col) = position;
                    let (num_rows, _num_cols) = grid_dimensions;
                    let is_at_edge = match orthogonal_channel {
                        "x" => row == num_rows - 1, // Bottom edge
                        "y" => col == 0,            // Left edge
                        _ => true,
                    };
                    let at_edge: std::collections::HashSet<String> = if is_at_edge {
                        tracked.clone()
                    } else {
                        std::collections::HashSet::new()
                    };
                    (tracked, at_edge)
                } else {
                    (std::collections::HashSet::new(), std::collections::HashSet::new())
                };

                let facet_ctx = FacetContext {
                    position,
                    grid_dimensions,
                    unified_channels: D::unified_channels()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    scale_sharing: scale_sharing.clone(),
                    global_edge_tracked_channels,
                    global_edge_channels,
                };
                for (k, v) in facet_ctx.to_params() {
                    updated_params.insert(k, v);
                }
                // Add NESTED_MEASUREMENT marker to coordination context
                let mut nested_ctx = coord_ctx.clone();
                nested_ctx
                    .coordinated_spacing
                    .insert(spacing_keys::NESTED_MEASUREMENT.to_string(), 1.0);
                for (k, v) in nested_ctx.to_params() {
                    updated_params.insert(k, v);
                }
                updated_params
            };

            // Call measure_with_coordination on child guide to get MeasurementResult
            // Pass other_overflow to child based on dimension
            let (child_row_overflow, child_col_overflow) = if D::is_row_facet() {
                (None, other_overflow)
            } else {
                (other_overflow, None)
            };

            let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                guide
                    .measure_with_coordination(
                        &subplot_scales_configured,
                        child_row_overflow,
                        child_col_overflow,
                        pass1_width,
                        pass1_height,
                        theme,
                        &measure_params,
                        Some(&filter_df),
                        ctx,
                    )
                    .await?
            } else {
                // Fallback: measure directly
                let (_, total_overflow, _, _) = source
                    .subplot
                    .measure_with_scales(
                        pass1_width,
                        pass1_height,
                        ctx,
                        &measure_params,
                        &subplot_scales,
                        Some(&filter_df),
                    )
                    .await?;
                MeasurementResult::new(total_overflow)
            };

            cell_overflows.push(child_result.overflow);

            // Aggregate child spacing_needs
            for (key, value) in child_result.spacing_needs {
                aggregated_spacing
                    .entry(key)
                    .and_modify(|v| *v = v.max(value))
                    .or_insert(value);
            }
        }
    } else {
        // No data or expression - use measure_overflow fallback
        let overflow = measure_overflow_fn
            .call(
                scales,
                _own_overflow,
                other_overflow,
                plot_width,
                plot_height,
                theme,
                params,
                data_override,
                ctx,
            )
            .await?;
        return Ok(MeasurementResult::new(overflow));
    }

    // === Compute gap from Pass 1 ===
    let computed_gap = D::calculate_inter_gap(&cell_overflows, GAP_SAFETY_MARGIN);

    debug!(
        "Facet{}Guide: Computed {}={:.2} (with {:.1}px safety margin)",
        if D::is_row_facet() { "Row" } else { "Col" },
        inter_gap_key,
        computed_gap,
        GAP_SAFETY_MARGIN
    );

    // === PASS 2: Re-measure with computed gap ===
    let total_gap = computed_gap * (num_cells.saturating_sub(1)) as f32;
    let pass2_band_size = D::compute_band_size(plot_width, plot_height, num_cells, total_gap);
    let (pass2_width, pass2_height) =
        D::subplot_dimensions_exact(pass2_band_size, plot_width, plot_height);

    debug!(
        "Facet{}Guide: Pass 2 - re-measuring with gap={:.2}, band_size={:.2}",
        if D::is_row_facet() { "Row" } else { "Col" },
        computed_gap,
        pass2_band_size
    );

    // Create params with computed gap for nested guides
    let pass2_params = {
        let mut new_ctx = coord_ctx.clone();
        // Add our own computed gap
        new_ctx
            .coordinated_spacing
            .insert(inter_gap_key.to_string(), computed_gap);
        // Add all aggregated spacing from children
        for (key, value) in &aggregated_spacing {
            new_ctx.coordinated_spacing.insert(key.clone(), *value);
        }
        // Add NESTED_MEASUREMENT marker for child guides
        new_ctx
            .coordinated_spacing
            .insert(spacing_keys::NESTED_MEASUREMENT.to_string(), 1.0);
        let mut new_params = params.clone();
        for (k, v) in new_ctx.to_params() {
            new_params.insert(k, v);
        }
        new_params
    };

    let source = &facet_sources[0];
    let facet_expr = source
        .data
        .channels()
        .get(channel_name)
        .and_then(|cv| cv.expr(ctx));

    let df = data_override
        .cloned()
        .or_else(|| source.data.dataframe_with_context(ctx));

    let mut final_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_cells);
    aggregated_spacing.clear(); // Reset for Pass 2 aggregation

    if let (Some(expr), Some(df)) = (facet_expr, df) {
        use datafusion::logical_expr::lit;

        // Rebuild scales with Pass 2 band size
        let builder = source
            .subplot
            .build_scale_builder_from_dataframe(ctx, &pass2_params, &df)
            .await?;
        let subplot_scales = source
            .subplot
            .build_scales_from_builder(&builder, pass2_width, pass2_height, ctx, &pass2_params)
            .await?;

        let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

        for (cell_idx, domain_val) in domain_vals.iter().enumerate() {
            use crate::facet::context::FacetContext;

            let filter_df = df
                .clone()
                .filter(expr.clone().eq(lit(domain_val.clone())))?;

            let measure_params = {
                let mut updated_params = pass2_params.clone();
                let position = D::index_to_position(cell_idx);
                let grid_dimensions = D::count_to_grid_dimensions(num_cells);
                let unified_channels: std::collections::HashSet<String> = D::unified_channels()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();

                // Compute global_edge_tracked_channels and global_edge_channels
                // For Row (default="y"), orthogonal channel is "x" - track if parent tracks or Level(2+)
                // For Col (default="x"), orthogonal channel is "y" - track if parent tracks or Level(2+)
                let default_unified: std::collections::HashSet<&str> =
                    D::unified_channels().iter().copied().collect();
                let orthogonal_channel = if default_unified.contains(&"y") { "x" } else { "y" };
                let orthogonal_has_multilevel_sharing = scale_sharing
                    .get(orthogonal_channel)
                    .map(|s| matches!(s, ScaleSharing::Level(n) if *n >= 2))
                    .unwrap_or(false);

                let (global_edge_tracked_channels, global_edge_channels) = if let Some(parent_ctx) = FacetContext::from_params(&pass2_params) {
                    // Check if parent is tracking orthogonal channel OR if it has Level(2+) sharing
                    let parent_tracking = parent_ctx.global_edge_tracked_channels.contains(orthogonal_channel);
                    let should_track = parent_tracking || orthogonal_has_multilevel_sharing;

                    if should_track {
                        let (row, col) = position;
                        let (num_rows, _num_cols) = grid_dimensions;

                        // Start with parent's tracked channels
                        let mut tracked = parent_ctx.global_edge_tracked_channels.clone();
                        if orthogonal_has_multilevel_sharing {
                            tracked.insert(orthogonal_channel.to_string());
                        }

                        // Inherit parent's global_edge_channels, or start fresh if parent wasn't tracking
                        let parent_edge = if parent_tracking {
                            parent_ctx.global_edge_channels.clone()
                        } else {
                            tracked.clone()
                        };

                        let at_edge = parent_edge
                            .into_iter()
                            .filter(|ch| tracked.contains(ch))
                            .filter(|ch| match ch.as_str() {
                                "x" => row == num_rows - 1, // Bottom edge
                                "y" => col == 0,           // Left edge
                                _ => true,
                            })
                            .collect();
                        (tracked, at_edge)
                    } else {
                        (std::collections::HashSet::new(), std::collections::HashSet::new())
                    }
                } else if orthogonal_has_multilevel_sharing {
                    // No parent but orthogonal has Level(2+) - start tracking at top level
                    let mut tracked = std::collections::HashSet::new();
                    tracked.insert(orthogonal_channel.to_string());
                    // Only mark as at-edge if at appropriate edge for the channel
                    let (row, col) = position;
                    let (num_rows, _num_cols) = grid_dimensions;
                    let is_at_edge = match orthogonal_channel {
                        "x" => row == num_rows - 1, // Bottom edge
                        "y" => col == 0,            // Left edge
                        _ => true,
                    };
                    let at_edge: std::collections::HashSet<String> = if is_at_edge {
                        tracked.clone()
                    } else {
                        std::collections::HashSet::new()
                    };
                    (tracked, at_edge)
                } else {
                    (std::collections::HashSet::new(), std::collections::HashSet::new())
                };

                let facet_ctx = FacetContext {
                    position,
                    grid_dimensions,
                    unified_channels,
                    scale_sharing: scale_sharing.clone(),
                    global_edge_tracked_channels,
                    global_edge_channels,
                };
                for (k, v) in facet_ctx.to_params() {
                    updated_params.insert(k, v);
                }
                updated_params
            };

            let (child_row_overflow, child_col_overflow) = if D::is_row_facet() {
                (None, other_overflow)
            } else {
                (other_overflow, None)
            };

            let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                guide
                    .measure_with_coordination(
                        &subplot_scales_configured,
                        child_row_overflow,
                        child_col_overflow,
                        pass2_width,
                        pass2_height,
                        theme,
                        &measure_params,
                        Some(&filter_df),
                        ctx,
                    )
                    .await?
            } else {
                let (_, total_overflow, _, _) = source
                    .subplot
                    .measure_with_scales(
                        pass2_width,
                        pass2_height,
                        ctx,
                        &measure_params,
                        &subplot_scales,
                        Some(&filter_df),
                    )
                    .await?;
                MeasurementResult::new(total_overflow)
            };

            final_overflows.push(child_result.overflow);

            for (key, value) in child_result.spacing_needs {
                aggregated_spacing
                    .entry(key)
                    .and_modify(|v| *v = v.max(value))
                    .or_insert(value);
            }
        }
    }

    // Aggregate final overflow
    let final_overflow = aggregate_overflow(&final_overflows);

    // Build result with our computed gap and child spacing
    // Ignore unified_title parameter - it's only relevant for measure_overflow/evaluate
    let _ = unified_title;

    Ok(MeasurementResult::new(final_overflow)
        .with_spacing(inter_gap_key, computed_gap)
        .merge_spacing_needs(aggregated_spacing))
}
