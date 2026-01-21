//! FacetColGuide implementation for column-based faceting.
//!
//! This module contains the guide implementation for FacetCol coordinate system,
//! which renders facet labels horizontally below (or above) the plot area with one
//! label per column.

use crate::channel::config_traits::ScaleSharing;
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::facet::guide::measurement::{AsyncMeasureOverflowFn, measure_with_coordination_impl};
use crate::facet::guide::shared::{
    FacetSource, aggregate_cached_overflow, apply_shared_overflow_coordination,
    compute_scale_sharing_for_nested_facet, default_overflow_fallback,
};
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::guide::{CompiledGuide, CoordinateGuide, MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::scales::ConfiguredScaleLegendExt;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::measurement::TextMeasurer;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Guide for FacetCol coordinate system
///
/// Renders facet labels horizontally below (or above) the plot area with one label per column.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetColGuide {
    facet_sources: Vec<FacetSource>,
    pub facet_title: Option<String>,
    unified_x_title: Option<String>,
    unified_y_title: Option<String>,
    unifiable_channel: Option<String>,
}

impl FacetColGuide {
    /// Compute maximum subplot overflow across all facet sources
    /// This can be called from both measure_overflow and evaluate without caching
    async fn compute_max_subplot_overflow(
        &self,
        col_scale: &ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        overflow: Option<&Vec<OverflowSpaceRequirement>>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<(f32, f32, f32, f32), crate::error::AvengerChartError> {
        // Extract discrete domain values
        let mut domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        // Sort to ensure deterministic facet ordering
        domain_vals.sort_by(scalar_total_cmp);

        let mut left_max = 0.0_f32;
        let mut right_max = 0.0_f32;
        let mut top_max = 0.0_f32;
        let mut bottom_max = 0.0_f32;

        // Get band width for column facets
        use crate::facet::band_positions::BandPositionIterator;
        let bp_iter = BandPositionIterator::from_configured_scale(col_scale)?;
        let _band_w = bp_iter.bandwidth();

        // Measure edge subplots via the same path as rendering
        for (source_idx, source) in self.facet_sources.iter().enumerate() {
            // Use overflow parameter if provided (passed from rendering pipeline)
            if let Some(per_facet_overflow) = overflow {
                // Aggregate top/bottom across ALL subplots for unified title positioning
                // (matching FacetRowGuide's pattern)
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide: Using per-facet overflow ({} subplots) - aggregating top/bottom across all",
                        per_facet_overflow.len()
                    );
                    for (idx, ov) in per_facet_overflow.iter().enumerate() {
                        eprintln!(
                            "  per_facet_overflow[{}]: top={:.3} bottom={:.3} left={:.3} right={:.3}",
                            idx, ov.top, ov.bottom, ov.left, ov.right
                        );
                    }
                }

                // Use helper to aggregate cached overflow
                (top_max, bottom_max, left_max, right_max) = aggregate_cached_overflow(
                    per_facet_overflow,
                    top_max,
                    bottom_max,
                    left_max,
                    right_max,
                );
            } else if let Some(df) = data_override {
                // No cached overflow but we have data - compute overflow by measuring subplots
                // This is the nested facet case where the inner facet needs to measure its Cartesian subplots
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide [source {}]: Computing overflow from data_override ({} domain values)",
                        source_idx,
                        domain_vals.len()
                    );
                }

                // Get the facet expression from the channel
                let facet_expr = if let Some(cv) = source
                    .data
                    .channels()
                    .get(ColumnDimensionConfig::channel_name())
                {
                    cv.expr(ctx)
                } else {
                    None
                };

                if let Some(expr) = facet_expr {
                    use crate::facet::coordination::FacetCoordinationContext;
                    use datafusion::logical_expr::lit;

                    // Check if scales are shared via coordination context or facet config
                    let coord_ctx = FacetCoordinationContext::from_params(params);
                    let use_full_domain = if let Some(ctx) = coord_ctx.as_ref() {
                        // Use coordination context's inner_scale_sharing
                        !ctx.inner_scale_sharing.is_free()
                    } else {
                        // No coordination context - check the facet's scale sharing config
                        // This is stored in the FacetSource (from CompiledFacetCol.facet_scale_sharing)
                        source
                            .facet_scale_sharing
                            .map(|mode| !mode.is_free())
                            .unwrap_or(false)
                    };

                    // Build domain_with_data to track which values have data
                    let mut domain_with_data: Vec<(ScalarValue, bool)> = Vec::new();
                    for domain_val in &domain_vals {
                        let filter_df = df
                            .clone()
                            .filter(expr.clone().eq(lit(domain_val.clone())))?;
                        let count = filter_df.clone().count().await?;
                        domain_with_data.push((domain_val.clone(), count > 0));
                    }

                    // Determine which domain values to iterate over
                    // When scales are shared, use full domain (with empty cells for missing data)
                    // When scales are free, only use values present in this row's data
                    let iteration_domain: Vec<(ScalarValue, bool)> = if use_full_domain {
                        domain_with_data.clone()
                    } else {
                        domain_with_data
                            .into_iter()
                            .filter(|(_, has_data)| *has_data)
                            .collect()
                    };

                    let num_present = iteration_domain.iter().filter(|(_, has)| *has).count();

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide [source {}]: Iteration domain has {} values ({} with data, use_full_domain={})",
                            source_idx,
                            iteration_domain.len(),
                            num_present,
                            use_full_domain
                        );
                    }

                    // Compute band width for subplots based on iteration domain size
                    // Account for inter-column gaps when computing band width
                    // For uniform Free scaling, use max cell count instead of actual domain length
                    let num_cols = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_uniform_cell_count())
                        .unwrap_or(iteration_domain.len())
                        .max(1);
                    let inter_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| {
                            ctx.get_coordinated_spacing(ColumnDimensionConfig::inter_gap_key())
                        })
                        .unwrap_or(0.0);
                    let total_gap = inter_gap * (num_cols.saturating_sub(1)) as f32;
                    let band_width = ColumnDimensionConfig::compute_band_size(
                        plot_width,
                        plot_height,
                        num_cols,
                        total_gap,
                    );

                    // Compute scale sharing from marks so subplots know which axes to measure
                    // Use nested facet version to correctly handle FacetRow inside FacetCol
                    let computed_scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

                    // Build scales for subplot measurement using two-step pattern for data override
                    let mut builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, df)
                        .await?;

                    // For nested facet subplots, compute level_domains for Level(N>=1) channels
                    // so they can be passed down via coordination context.
                    // This is needed because level_domains are normally computed during evaluation,
                    // but measurement happens before evaluation.
                    let measurement_coord_ctx = if source.subplot.compiled_guide.is_some() {
                        use crate::facet::coordination::LevelChannelKey;

                        // Find which channels need Level(N>=1) sharing, preserving the level value
                        let level_channels: Vec<(&str, u8)> = computed_scale_sharing
                            .iter()
                            .filter_map(|(channel, mode)| {
                                if let ScaleSharing::Level(n) = mode {
                                    if *n >= 1 {
                                        Some((channel.as_str(), *n))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if !level_channels.is_empty() {
                            use crate::facet::marks::facet_extents::{
                                classify_channel, compute_extents, DomainSort,
                            };
                            use crate::facet::nesting::find_channel_in_marks;

                            // Child nesting depth: one more than current
                            let child_nesting_depth = coord_ctx
                                .as_ref()
                                .map(|ctx| ctx.nesting_depth + 1)
                                .unwrap_or(1);

                            // Build level_domains by recursively finding channel expressions
                            // and computing extents from the full dataframe
                            let mut level_domains = IndexMap::new();
                            for (channel, level) in &level_channels {
                                if let Some(found) =
                                    find_channel_in_marks(&source.subplot.marks, channel, ctx, 5)
                                {
                                    let kind = classify_channel(
                                        &found.expr,
                                        found.domain_kind,
                                        df.schema(),
                                    );
                                    if let Ok(extents) = compute_extents(
                                        kind,
                                        df,
                                        &found.expr,
                                        ctx,
                                        DomainSort::None,
                                    )
                                    .await
                                    {
                                        // Storage depth must align with lookup formula:
                                        // Lookup uses target_depth = nesting_depth - level + 2
                                        // So storage_depth = child_nesting_depth - level + 2
                                        let storage_depth =
                                            (child_nesting_depth as i32 - *level as i32 + 2).max(1)
                                                as usize;
                                        let key = LevelChannelKey::new(storage_depth, *channel);
                                        level_domains.insert(key, extents.into());
                                    }
                                }
                            }

                            if !level_domains.is_empty() {
                                let base_ctx = coord_ctx.clone().unwrap_or_default();
                                Some(
                                    base_ctx
                                        .with_nesting_depth(child_nesting_depth)
                                        .with_level_domains(level_domains),
                                )
                            } else {
                                coord_ctx.clone()
                            }
                        } else {
                            coord_ctx.clone()
                        }
                    } else {
                        coord_ctx.clone()
                    };

                    // Extend with level-based extents from coordination context for Level(N>=1) channels
                    // ONLY for innermost subplots (Cartesian) - not for nested facet guides.
                    if source.subplot.compiled_guide.is_none() {
                        use crate::scales::DomainExtent;
                        let level_extents: HashMap<String, DomainExtent> =
                            computed_scale_sharing
                                .iter()
                                .filter_map(|(channel, mode)| {
                                    if let ScaleSharing::Level(n) = mode {
                                        if *n >= 1 {
                                            coord_ctx.as_ref().and_then(|ctx| {
                                                ctx.get_domain_for_channel_with_level(channel, *n)
                                                    .map(|extents| (channel.clone(), extents.clone()))
                                            })
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
                        .build_scales_from_builder(&builder, band_width, plot_height, ctx, params)
                        .await?;

                    // Measure each subplot with correct per-subplot FacetContext
                    let has_unified_y = self.unified_y_title.is_some();
                    let mut computed_overflow = Vec::new();

                    for (col_idx, (domain_val, has_data)) in iteration_domain.iter().enumerate() {
                        // Get the DataFrame for this cell - either filtered data or empty
                        let cell_df = if *has_data {
                            df.clone()
                                .filter(expr.clone().eq(lit(domain_val.clone())))?
                        } else {
                            // Empty cell: create an empty DataFrame with the same schema
                            df.clone().limit(0, Some(0))?
                        };

                        // Create FacetContext with correct position for this subplot
                        // so that should_show_title/should_show_labels work correctly
                        let subplot_measure_params = {
                            use crate::facet::context::FacetContext;
                            let mut updated_params = params.clone();
                            // Convert static slice to HashSet<String>
                            let mut unified_channels: std::collections::HashSet<String> =
                                ColumnDimensionConfig::unified_channels()
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect();

                            // Merge parent's unified_channels and scale_sharing if present
                            let (parent_row, parent_num_rows, mut merged_scale_sharing) =
                                if let Some(parent_ctx) = FacetContext::from_params(params) {
                                    for ch in &parent_ctx.unified_channels {
                                        unified_channels.insert(ch.clone());
                                    }
                                    (
                                        parent_ctx.position.0,
                                        parent_ctx.grid_dimensions.0,
                                        parent_ctx.scale_sharing.clone(),
                                    )
                                } else {
                                    (0, 1, std::collections::HashMap::new())
                                };

                            // Merge computed scale_sharing (computed takes precedence)
                            for (k, v) in &computed_scale_sharing {
                                merged_scale_sharing.insert(k.clone(), *v);
                            }

                            if has_unified_y {
                                unified_channels.insert("y".to_string());
                            }
                            let facet_ctx = FacetContext {
                                position: (parent_row, col_idx), // Correct row and column position
                                grid_dimensions: (parent_num_rows, num_cols),
                                unified_channels,
                                scale_sharing: merged_scale_sharing,
                            };
                            let ctx_params = facet_ctx.to_params();
                            for (k, v) in ctx_params {
                                updated_params.insert(k, v);
                            }
                            // Add coordination context with level_domains to params
                            if let Some(ref mcoord_ctx) = measurement_coord_ctx {
                                for (k, v) in mcoord_ctx.to_params() {
                                    updated_params.insert(k, v);
                                }
                            }
                            updated_params
                        };

                        let (_guide_only, total_overflow, _legend_info, _legend_positions) = source
                            .subplot
                            .measure_with_scales(
                                band_width,
                                plot_height,
                                ctx,
                                &subplot_measure_params,
                                &subplot_scales,
                                Some(&cell_df),
                            )
                            .await?;

                        computed_overflow.push(total_overflow);
                    }

                    // Aggregate computed overflow using dimension-specific trait method
                    let (t, b, l, r) =
                        ColumnDimensionConfig::aggregate_overflow_edges(&computed_overflow);
                    top_max = top_max.max(t);
                    bottom_max = bottom_max.max(b);
                    left_max = left_max.max(l);
                    right_max = right_max.max(r);
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide [source {}]: No facet expression, using fallback",
                            source_idx
                        );
                    }
                    return Ok(default_overflow_fallback());
                }
            } else if let Some(df) = source.data.dataframe_with_context(ctx) {
                // No cached overflow and no data_override but we have source data - compute overflow
                // This is the outer facet case where we need to measure nested facet subplots
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide [source {}]: Computing overflow from source data ({} domain values)",
                        source_idx,
                        domain_vals.len()
                    );
                }

                // Get the facet expression from the channel
                let facet_expr = if let Some(cv) = source
                    .data
                    .channels()
                    .get(ColumnDimensionConfig::channel_name())
                {
                    cv.expr(ctx)
                } else {
                    None
                };

                if let Some(expr) = facet_expr {
                    use crate::facet::coordination::FacetCoordinationContext;

                    // Compute band width for subplots
                    // Account for inter-column gaps when computing band width
                    let coord_ctx = FacetCoordinationContext::from_params(params);
                    let num_cols = domain_vals.len().max(1);
                    let inter_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| {
                            ctx.get_coordinated_spacing(ColumnDimensionConfig::inter_gap_key())
                        })
                        .unwrap_or(0.0);
                    let total_gap = inter_gap * (num_cols.saturating_sub(1)) as f32;
                    let band_width = ColumnDimensionConfig::compute_band_size(
                        plot_width,
                        plot_height,
                        num_cols,
                        total_gap,
                    );

                    // Compute scale sharing from marks so subplots know which axes to measure
                    // Use nested facet version to correctly handle FacetRow inside FacetCol
                    let scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

                    // Build scales for subplot measurement using two-step pattern for data override
                    let builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, &df)
                        .await?;

                    // For nested facet subplots, compute level_domains for Level(N>=1) channels
                    // so they can be passed down via coordination context.
                    // This is needed because level_domains are normally computed during evaluation,
                    // but measurement happens before evaluation.
                    let measurement_coord_ctx = if source.subplot.compiled_guide.is_some() {
                        use crate::facet::coordination::LevelChannelKey;

                        // Find which channels need Level(N>=1) sharing, preserving the level value
                        let level_channels: Vec<(&str, u8)> = scale_sharing
                            .iter()
                            .filter_map(|(channel, mode)| {
                                if let ScaleSharing::Level(n) = mode {
                                    if *n >= 1 {
                                        Some((channel.as_str(), *n))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if !level_channels.is_empty() {
                            use crate::facet::marks::facet_extents::{
                                classify_channel, compute_extents, DomainSort,
                            };
                            use crate::facet::nesting::find_channel_in_marks;

                            // Child nesting depth: one more than current
                            let child_nesting_depth = coord_ctx
                                .as_ref()
                                .map(|ctx| ctx.nesting_depth + 1)
                                .unwrap_or(1);

                            // Build level_domains by recursively finding channel expressions
                            // and computing extents from the full dataframe
                            let mut level_domains = IndexMap::new();
                            for (channel, level) in &level_channels {
                                if let Some(found) =
                                    find_channel_in_marks(&source.subplot.marks, channel, ctx, 5)
                                {
                                    let kind = classify_channel(
                                        &found.expr,
                                        found.domain_kind,
                                        df.schema(),
                                    );
                                    if let Ok(extents) = compute_extents(
                                        kind,
                                        &df,
                                        &found.expr,
                                        ctx,
                                        DomainSort::None,
                                    )
                                    .await
                                    {
                                        // Storage depth must align with lookup formula:
                                        // Lookup uses target_depth = nesting_depth - level + 2
                                        // So storage_depth = child_nesting_depth - level + 2
                                        let storage_depth =
                                            (child_nesting_depth as i32 - *level as i32 + 2).max(1)
                                                as usize;
                                        let key = LevelChannelKey::new(storage_depth, *channel);
                                        level_domains.insert(key, extents.into());
                                    }
                                }
                            }

                            if !level_domains.is_empty() {
                                let base_ctx = coord_ctx.clone().unwrap_or_default();
                                Some(
                                    base_ctx
                                        .with_nesting_depth(child_nesting_depth)
                                        .with_level_domains(level_domains),
                                )
                            } else {
                                coord_ctx.clone()
                            }
                        } else {
                            coord_ctx.clone()
                        }
                    } else {
                        coord_ctx.clone()
                    };

                    let subplot_scales = source
                        .subplot
                        .build_scales_from_builder(&builder, band_width, plot_height, ctx, params)
                        .await?;

                    // Measure each subplot with correct per-subplot FacetContext
                    let has_unified_y = self.unified_y_title.is_some();
                    let mut computed_overflow = Vec::new();
                    let num_cols = domain_vals.len();

                    for (col_idx, domain_val) in domain_vals.iter().enumerate() {
                        use datafusion::logical_expr::lit;
                        let filter_df = df
                            .clone()
                            .filter(expr.clone().eq(lit(domain_val.clone())))?;

                        // Create FacetContext with correct position for this subplot
                        // so that should_show_title/should_show_labels work correctly
                        let subplot_measure_params = {
                            use crate::facet::context::FacetContext;
                            let mut updated_params = params.clone();
                            // Convert static slice to HashSet<String>
                            let mut unified_channels: std::collections::HashSet<String> =
                                ColumnDimensionConfig::unified_channels()
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect();

                            // Merge parent's unified_channels and scale_sharing if present
                            let (parent_row, parent_num_rows, mut merged_scale_sharing) =
                                if let Some(parent_ctx) = FacetContext::from_params(params) {
                                    for ch in &parent_ctx.unified_channels {
                                        unified_channels.insert(ch.clone());
                                    }
                                    (
                                        parent_ctx.position.0,
                                        parent_ctx.grid_dimensions.0,
                                        parent_ctx.scale_sharing.clone(),
                                    )
                                } else {
                                    (0, 1, std::collections::HashMap::new())
                                };

                            // Merge computed scale_sharing (computed takes precedence)
                            for (k, v) in &scale_sharing {
                                merged_scale_sharing.insert(k.clone(), *v);
                            }

                            // Merge channel_sharing_levels from FacetCoordinationContext
                            // This contains the per-channel scale sharing modes (x, y) computed
                            // from channel configs, ensuring measurement uses same visibility
                            // decisions as rendering.
                            use crate::facet::coordination::FacetCoordinationContext;
                            if let Some(coord_ctx) = FacetCoordinationContext::from_params(params) {
                                for (channel, level) in &coord_ctx.channel_sharing_levels {
                                    merged_scale_sharing
                                        .insert(channel.clone(), ScaleSharing::from_level(*level));
                                }
                            }

                            if has_unified_y {
                                unified_channels.insert("y".to_string());
                            }
                            let facet_ctx = FacetContext {
                                position: (parent_row, col_idx), // Correct row and column position
                                grid_dimensions: (parent_num_rows, num_cols),
                                unified_channels,
                                scale_sharing: merged_scale_sharing,
                            };
                            let ctx_params = facet_ctx.to_params();
                            for (k, v) in ctx_params {
                                updated_params.insert(k, v);
                            }
                            // Add coordination context with level_domains to params
                            if let Some(ref mcoord_ctx) = measurement_coord_ctx {
                                for (k, v) in mcoord_ctx.to_params() {
                                    updated_params.insert(k, v);
                                }
                            }
                            updated_params
                        };

                        let (_guide_only, total_overflow, _legend_info, _legend_positions) = source
                            .subplot
                            .measure_with_scales(
                                band_width,
                                plot_height,
                                ctx,
                                &subplot_measure_params,
                                &subplot_scales,
                                Some(&filter_df),
                            )
                            .await?;

                        computed_overflow.push(total_overflow);
                    }

                    // Aggregate computed overflow using dimension-specific trait method
                    let (t, b, l, r) =
                        ColumnDimensionConfig::aggregate_overflow_edges(&computed_overflow);
                    top_max = top_max.max(t);
                    bottom_max = bottom_max.max(b);
                    left_max = left_max.max(l);
                    right_max = right_max.max(r);
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide [source {}]: No facet expression from source data, using fallback",
                            source_idx
                        );
                    }
                    return Ok(default_overflow_fallback());
                }
            } else {
                // No cached overflow and no data - use reasonable estimates
                // This shouldn't happen in normal operation but provides a safe fallback
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide [source {}]: No overflow data and no data available, using fallback",
                        source_idx
                    );
                }
                return Ok(default_overflow_fallback());
            }
        }
        Ok((top_max, bottom_max, left_max, right_max))
    }
}

impl CoordinateGuide for FacetColGuide {
    type Axis = crate::cartesian::axis::CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        self.facet_sources.clear();
        for m in compiled_marks {
            if let Some(facet) = m
                .as_any()
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetCol>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: facet.facet_title.clone(),
                    facet_scale_sharing: facet.facet_scale_sharing,
                });
            }
        }
        // Derive default facet title if not explicitly set
        if self.facet_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src
                    .data
                    .channels()
                    .get(ColumnDimensionConfig::channel_name())
                {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.facet_title = Some(name);
                    }
                }
                if let Some(title) = &src.user_title {
                    self.facet_title = Some(title.clone());
                }
            }
        }
        // Derive unified x-axis title from subplot guide
        if self.unified_x_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        ColumnDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_channel = Some(info.channel);
                    self.unified_x_title = info.title;
                }
            }
        }
        // Derive unified y-axis title from subplot guide (for row dimension)
        // This allows FacetCol to show ONE y-axis title on the left for nested FacetRow subplots
        // IMPORTANT: Only do this if the subplot guide actually unifies y (i.e., is a FacetRowGuide).
        // CartesianGuide reports y as unifiable but renders its own y-axis title, so we shouldn't
        // extract unified_y_title from it (would cause duplicate y-axis titles).
        if self.unified_y_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(guide) = src.subplot.compiled_guide.as_ref() {
                    // Only extract unified_y_title if the subplot guide suppresses y-axis titles
                    if guide.unifies_channel("y") {
                        if let Some(info) = guide.facet_unifiable_channel(
                            RowDimensionConfig::facet_direction(),
                            src.subplot.marks(),
                            _session_context,
                        ) {
                            self.unified_y_title = info.title;
                        }
                    }
                }
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

/// Wrapper to call FacetColGuide::measure_overflow from generic function
pub(crate) struct ColMeasureOverflowWrapper<'a> {
    pub guide: &'a FacetColGuide,
}

impl AsyncMeasureOverflowFn for ColMeasureOverflowWrapper<'_> {
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
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        self.guide
            .measure_overflow(
                scales,
                own_overflow,
                other_overflow,
                plot_width,
                plot_height,
                theme,
                params,
                data_override,
                ctx,
            )
            .await
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetColGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::scalar::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;

        // Get col scale
        let col_scale = scales
            .get(ColumnDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetColGuide",
                        ColumnDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Compute subplot overflow using shared helper
        let (top_max, bottom_max, left_max, right_max) = self
            .compute_max_subplot_overflow(
                col_scale,
                plot_width,
                plot_height,
                _theme,
                params,
                ctx,
                _col_overflow,
                data_override,
            )
            .await?;

        // Apply shared overflow from coordination context for uniform padding across rows
        let mut overflow = (top_max, bottom_max, left_max, right_max);
        apply_shared_overflow_coordination(&mut overflow, params);
        let (top_max, bottom_max, left_max, right_max) = overflow;

        // Add facet guide space (labels/titles) on top of child overflows
        // Measure facet label slab (same theme contexts used elsewhere)
        let labels = col_scale.domain_labels().unwrap_or_default();
        let measurer = avenger_text::measurement::default_text_measurer();
        let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = _theme.font_size(&label_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family_owned = _theme
            .font_family(&label_ctx)
            .or_else(|| _theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let label_font_family = label_font_family_owned.as_str();

        let mut max_label_height = 0.0_f32;
        for label in &labels {
            let config = avenger_text::measurement::TextMeasurementConfig {
                text: label,
                font: label_font_family,
                font_size: label_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let bounds = measurer.measure_text_bounds(&config);
            max_label_height = max_label_height.max(bounds.height);
        }

        // Facet title (per column) space
        let title_height = if let Some(title_text) = &self.facet_title {
            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = _theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_family_owned = _theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg = avenger_text::measurement::TextMeasurementConfig {
                text: title_text,
                font: title_family_owned.as_str(),
                font_size: title_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b = measurer.measure_text_bounds(&cfg);
            b.height
        } else {
            0.0
        };

        // Unified x-axis title space (if present)
        let unified_title_height = if let Some(unified_text) = &self.unified_x_title {
            let unified_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let unified_font_px = _theme.font_size(&unified_ctx).unwrap_or(12.0_f32);
            let unified_family_owned = _theme
                .font_family(&unified_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg = avenger_text::measurement::TextMeasurementConfig {
                text: unified_text,
                font: unified_family_owned.as_str(),
                font_size: unified_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b = measurer.measure_text_bounds(&cfg);
            b.height
        } else {
            0.0
        };

        let gap_title = if self.facet_title.is_some() {
            10.0
        } else {
            0.0
        };
        // Space for rule + ticks when there are multiple labels (used in labels-only case)
        let rule_tick_space = if labels.len() > 1 {
            let gap = 10.0_f32;
            let rule_stroke = 1.0_f32;
            let tick_size = 4.0_f32;
            gap / 2.0 + rule_stroke + tick_size // = 10.0
        } else {
            0.0
        };
        // When title IS present, gap_title already accounts for rule/tick spacing
        // rule_tick_space is only used in the nested case (labels without title)
        let facet_label_space = if self.facet_title.is_some() {
            max_label_height + gap_title + title_height + 1.0 // Original - gap_title handles spacing
        } else {
            max_label_height + rule_tick_space + 1.0 // Add rule_tick_space when no title
        };
        let gap_axis = if self.unified_x_title.is_some() {
            6.0
        } else {
            0.0
        };
        let x_axis_title_space = if self.unified_x_title.is_some() {
            gap_axis + unified_title_height + 1.0
        } else {
            0.0
        };

        // Measure unified y-axis title height (rotated, so use text height)
        let unified_y_title_height = if let Some(unified_y_text) = &self.unified_y_title {
            let unified_y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let unified_y_font_px = _theme.font_size(&unified_y_ctx).unwrap_or(12.0_f32);
            let unified_y_family_owned = _theme
                .font_family(&unified_y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());
            let cfg_y = avenger_text::measurement::TextMeasurementConfig {
                text: unified_y_text,
                font: unified_y_family_owned.as_str(),
                font_size: unified_y_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let b_y = measurer.measure_text_bounds(&cfg_y);
            b_y.height
        } else {
            0.0
        };
        let gap_y_axis = if self.unified_y_title.is_some() {
            6.0
        } else {
            0.0
        };
        let y_axis_title_space = if self.unified_y_title.is_some() {
            gap_y_axis + unified_y_title_height + 1.0
        } else {
            0.0
        };

        // ====================================================================
        // Resolve visibility using centralized FacetColVisibility
        // ====================================================================
        use crate::facet::context::FacetContext;
        use crate::facet::visibility::{FacetColVisibility, FacetColVisibilityInput};

        // Extract axis positions from subplot guide
        let x_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("x"));

        let y_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("y"));

        let facet_scale_sharing = self
            .facet_sources
            .first()
            .and_then(|source| source.facet_scale_sharing);
        let visibility_input = FacetColVisibilityInput {
            x_axis_position,
            y_axis_position,
            max_top: top_max,
            max_bottom: bottom_max,
            has_unified_x_title: self.unified_x_title.is_some(),
            has_unified_y_title: self.unified_y_title.is_some(),
            facet_scale_sharing,
        };
        let parent_ctx = FacetContext::from_params(params);
        let visibility = FacetColVisibility::resolve(&visibility_input, parent_ctx.as_ref());

        // Use visibility struct for placement decisions
        let place_below = visibility.place_below;
        let y_axis_on_right = visibility.y_axis_on_right;

        // Conditionally include facet_label_space based on visibility
        // When render_facet_labels=true but render_facet_title=false (nested case),
        // only include label space + rule/ticks, not title space
        let adjusted_facet_label_space = if visibility.render_facet_labels {
            if visibility.render_facet_title {
                facet_label_space // includes labels + rule/ticks + title
            } else {
                // Nested: include label space + rule/ticks, not title
                max_label_height + rule_tick_space + 1.0
            }
        } else {
            0.0
        };

        // Only include x_axis_title_space when visibility allows rendering unified x title
        let adjusted_x_axis_title_space = if visibility.render_unified_x_title {
            x_axis_title_space
        } else {
            0.0
        };

        let mut top_final = top_max;
        let mut bottom_final = bottom_max;
        if place_below {
            top_final += adjusted_x_axis_title_space;
            bottom_final += adjusted_facet_label_space;
        } else {
            top_final += adjusted_facet_label_space;
            bottom_final += adjusted_x_axis_title_space;
        }

        let mut left_final = left_max;
        let mut right_final = right_max;
        if y_axis_on_right {
            right_final += y_axis_title_space;
        } else {
            left_final += y_axis_title_space;
        }

        let result = OverflowSpaceRequirement {
            left: left_final,
            right: right_final,
            top: top_final,
            bottom: bottom_final,
        };
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FACET_COL (edge-measured): place_below={} top_max={:.3} bottom_max={:.3} facet_label_space={:.3} adj_facet_label_space={:.3} x_axis_title_space={:.3} adj_x_axis_title_space={:.3} y_axis_title_space={:.3} render_facet_labels={} render_unified_x_title={} is_top={} is_bottom={}",
                place_below,
                top_max,
                bottom_max,
                facet_label_space,
                adjusted_facet_label_space,
                x_axis_title_space,
                adjusted_x_axis_title_space,
                y_axis_title_space,
                visibility.render_facet_labels,
                visibility.render_unified_x_title,
                visibility.is_top_edge,
                visibility.is_bottom_edge,
            );
            eprintln!(
                "FACET_COL (edge-measured) result: left={:.3} right={:.3} top={:.3} bottom={:.3}",
                result.left, result.right, result.top, result.bottom
            );
        }
        return Ok(result);
    }

    async fn measure_intrinsic_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::scalar::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        // Get col scale
        let col_scale = scales
            .get(ColumnDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetColGuide",
                        ColumnDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Return ONLY the intrinsic subplot overflow - no facet labels, titles, or unified axis titles
        let (top, bottom, left, right) = self
            .compute_max_subplot_overflow(
                col_scale,
                plot_width,
                plot_height,
                _theme,
                params,
                ctx,
                _col_overflow,
                data_override,
            )
            .await?;

        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        })
    }

    async fn measure_with_coordination(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<MeasurementResult, crate::error::AvengerChartError> {
        // Use generic implementation with ColumnDimensionConfig
        measure_with_coordination_impl::<ColumnDimensionConfig>(
            &self.facet_sources,
            self.unified_x_title.as_ref(),
            scales,
            _col_overflow,
            row_overflow,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            ColMeasureOverflowWrapper { guide: self },
        )
        .await
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::scalar::ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        let col_scale = match scales.get(ColumnDimensionConfig::channel_name()) {
            Some(s) => s,
            None => return Ok(marks),
        };

        let labels = col_scale.domain_labels()?;

        // Compute subplot overflow using shared helper (no caching needed between calls)
        // Pass data_override so nested facets use filtered data (matching measure_overflow behavior)
        let (subplot_max_top, subplot_max_bottom, _left, _right) = self
            .compute_max_subplot_overflow(
                col_scale,
                plot_width,
                plot_height,
                theme,
                params,
                ctx,
                _col_overflow,
                data_override,
            )
            .await?;

        // Get band positions from the scale
        // For uniform Free scaling: if uniform_cell_count > actual labels, we need to compute
        // band positions as if there were uniform_cell_count values. This ensures labels are
        // positioned at the correct center (e.g., half-width for 2 cells when only 1 exists).
        use crate::facet::band_positions::BandPositionIterator;
        use crate::facet::coordination::FacetCoordinationContext;
        let uniform_cell_count = FacetCoordinationContext::from_params(params)
            .and_then(|ctx| ctx.get_uniform_cell_count());

        let band_positions: Vec<_> = if let Some(uniform_count) = uniform_cell_count {
            if labels.len() < uniform_count {
                // Create temporary scale with padded domain for correct band sizing
                use datafusion::arrow::array::StringArray;
                use std::sync::Arc as StdArc;

                // Create padded domain values
                let mut padded_labels: Vec<String> = labels.clone();
                for i in labels.len()..uniform_count {
                    padded_labels.push(format!("__placeholder_{}", i));
                }

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide evaluate: uniform sizing - creating temp scale with {} values (actual={}) for band positioning",
                        uniform_count,
                        labels.len()
                    );
                }

                // Create temporary scale with padded domain
                let padded_array = StdArc::new(StringArray::from(padded_labels))
                    as datafusion::arrow::array::ArrayRef;
                let temp_scale = col_scale.clone().with_domain(padded_array);

                // Get positions from the padded scale (only use first labels.len() positions)
                let all_positions: Vec<_> =
                    BandPositionIterator::from_configured_scale(&temp_scale)?.collect();

                // Return only the positions for actual labels (not placeholders)
                all_positions.into_iter().take(labels.len()).collect()
            } else {
                // No padding needed
                BandPositionIterator::from_configured_scale(col_scale)?.collect()
            }
        } else {
            // No uniform sizing
            BandPositionIterator::from_configured_scale(col_scale)?.collect()
        };

        // Theme-based font for labels (use facet label theme context matching RowFacet)
        let label_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&label_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family_owned = theme
            .font_family(&label_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let label_font_family = label_font_family_owned.as_str();

        // ====================================================================
        // Resolve visibility using centralized FacetColVisibility
        // ====================================================================
        use crate::facet::context::FacetContext;
        use crate::facet::visibility::{FacetColVisibility, FacetColVisibilityInput};

        // Extract axis positions from subplot guide
        let x_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("x"));

        let y_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("y"));

        let facet_scale_sharing = self
            .facet_sources
            .first()
            .and_then(|source| source.facet_scale_sharing);
        let visibility_input = FacetColVisibilityInput {
            x_axis_position,
            y_axis_position,
            max_top: subplot_max_top,
            max_bottom: subplot_max_bottom,
            has_unified_x_title: self.unified_x_title.is_some(),
            has_unified_y_title: self.unified_y_title.is_some(),
            facet_scale_sharing,
        };
        let parent_ctx = FacetContext::from_params(params);
        let visibility = FacetColVisibility::resolve(&visibility_input, parent_ctx.as_ref());

        // Use visibility struct for placement and rendering decisions
        let place_below = visibility.place_below;

        // Resolve title font properties for rendering
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Use guide_utils to render facet label slab (labels + rule + title)
        use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

        // Extend plot bounds to include subplot overflow so facet labels are positioned
        // outside of the subplot axes
        let render_plot_bounds = if place_below {
            // Labels below: extend downward by bottom overflow
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y,
                width: plot_width,
                height: plot_height + subplot_max_bottom,
            }
        } else {
            // Labels above: extend upward by top overflow
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y - subplot_max_top,
                width: plot_width,
                height: plot_height + subplot_max_top,
            }
        };

        let render_config = FacetLabelRenderConfig {
            labels: labels.clone(),
            band_positions: band_positions.clone(),
            plot_bounds: render_plot_bounds,
            is_rotated: false,         // Col labels are horizontal
            place_at_end: place_below, // place_at_end=true means bottom
            font_family: label_font_family.to_string(),
            font_size_px: label_font_px,
            title: self.facet_title.clone(),
            title_font_family: title_font_family.clone(),
            title_font_size_px: title_font_px,
            render_title: visibility.render_facet_title,
        };

        // Only render facet labels when visibility allows (edge row only when nested)
        if visibility.render_facet_labels {
            marks.extend(render_facet_label_slab(&render_config, theme, params));
        }

        // Render unified x-axis title (use facet title theme context matching RowFacet)
        // The unified x-axis title should always be positioned near the x-axes,
        // not move with facet labels. It goes below plot when x-axis is at bottom,
        // above plot when x-axis is at top.
        // Only render when visibility allows (not nested in row facet, parent hasn't unified)
        if let Some(unified_title) = &self.unified_x_title {
            if !visibility.render_unified_x_title {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide SKIP unified_x_title='{}' (parent already unified x or nested in row facet)",
                        unified_title
                    );
                }
            } else {
                use crate::facet::guide_utils::{
                    UnifiedTitleRenderConfig, render_unified_axis_title,
                };

                let x_axis_at_top = visibility.x_axis_at_top;
                let gap_axis = 6.0_f32;

                let y_unified = if x_axis_at_top {
                    plot_bounds.y - subplot_max_top - gap_axis
                } else {
                    plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
                };

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide RENDER unified_x_title='{}' at y={:.3} (plot_y={:.3} plot_height={:.3} subplot_max_bottom={:.3} gap_axis={:.3})",
                        unified_title,
                        y_unified,
                        plot_bounds.y,
                        plot_height,
                        subplot_max_bottom,
                        gap_axis
                    );
                }

                let title_config = UnifiedTitleRenderConfig {
                    title: unified_title.clone(),
                    font_family: title_font_family.clone(),
                    font_size_px: title_font_px,
                    x: plot_bounds.x + plot_width / 2.0,
                    y: y_unified,
                    angle: 0.0,
                    axis_at_far_edge: !x_axis_at_top, // bottom is "far edge" for x-axis
                };
                let unified_mark = render_unified_axis_title(&title_config, theme, params);
                marks.push(SceneMark::Text(StdArc::new(unified_mark)));
            }
        }

        // Render unified y-axis title if available (rotated 90 degrees, on the left side)
        // This is only used for FacetCol with nested FacetRow - the outer col renders the unified y-title
        if let Some(unified_y_title) = &self.unified_y_title {
            use crate::facet::guide_utils::{UnifiedTitleRenderConfig, render_unified_axis_title};

            let y_axis_on_right = visibility.y_axis_on_right;
            let gap_y_axis = 6.0_f32;

            // Measure the title height (becomes width when rotated) for positioning
            let title_measurer = avenger_text::measurement::default_text_measurer();
            let title_cfg = avenger_text::measurement::TextMeasurementConfig {
                text: unified_y_title,
                font: title_font_family.as_str(),
                font_size: title_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let title_bounds = title_measurer.measure_text_bounds(&title_cfg);
            let title_height = title_bounds.height;
            let y_axis_title_space = gap_y_axis + title_height + 1.0;

            // Re-measure subplot overflow with None to get true Cartesian subplot values
            // (not including nested facet labels)
            let (_, _, subplot_left_actual, subplot_right_actual) = self
                .compute_max_subplot_overflow(
                    col_scale,
                    plot_width,
                    plot_height,
                    theme,
                    params,
                    ctx,
                    None,
                    data_override,
                )
                .await?;

            // Calculate outer overflow by adding title space to actual subplot overflow
            let outer_left = if !y_axis_on_right {
                subplot_left_actual + y_axis_title_space
            } else {
                subplot_left_actual
            };
            let outer_right = if y_axis_on_right {
                subplot_right_actual + y_axis_title_space
            } else {
                subplot_right_actual
            };

            let x_unified_y = if y_axis_on_right {
                plot_bounds.x + plot_width + outer_right - gap_y_axis - title_height / 2.0
            } else {
                plot_bounds.x - outer_left + gap_y_axis + title_height / 2.0
            };
            let y_center = plot_bounds.y + plot_height / 2.0;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetColGuide RENDER unified_y_title='{}' at x={:.3} y={:.3} (y_axis_on_right={} subplot_left={:.3} outer_left={:.3} subplot_right={:.3} outer_right={:.3})",
                    unified_y_title,
                    x_unified_y,
                    y_center,
                    y_axis_on_right,
                    subplot_left_actual,
                    outer_left,
                    subplot_right_actual,
                    outer_right
                );
            }

            let title_config = UnifiedTitleRenderConfig {
                title: unified_y_title.clone(),
                font_family: title_font_family.clone(),
                font_size_px: title_font_px,
                x: x_unified_y,
                y: y_center,
                angle: if y_axis_on_right { 90.0 } else { -90.0 },
                axis_at_far_edge: y_axis_on_right,
            };
            let unified_y_mark = render_unified_axis_title(&title_config, theme, params);
            marks.push(SceneMark::Text(StdArc::new(unified_y_mark)));
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        // Don't clip faceted plots - legends may extend beyond plot area
        avenger_scenegraph::marks::group::Clip::None
    }

    fn axis_position(&self, channel: &str) -> Option<crate::cartesian::axis::AxisPosition> {
        // Delegate to inner subplot's guide to get actual axis position
        // This allows outer code to correctly determine where axes are positioned in nested facets
        if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                return guide.axis_position(channel);
            }
        }
        None
    }

    fn facet_unifiable_channel(
        &self,
        facet_direction: crate::guide::FacetDirection,
        _marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
        session_context: &datafusion::prelude::SessionContext,
    ) -> Option<crate::guide::UnifiableChannelInfo> {
        use crate::guide::FacetDirection;

        // For FacetColGuide, delegate to inner subplot's guide
        // Column faceting unifies x-channel, so if asked for Column, return our own unified_x_title
        // For Row faceting, drill down to inner subplot
        match facet_direction {
            FacetDirection::Column => {
                // This FacetCol already unifies x, return our stored title
                Some(crate::guide::UnifiableChannelInfo {
                    channel: ColumnDimensionConfig::unified_title_channel().to_string(),
                    title: self.unified_x_title.clone(),
                })
            }
            FacetDirection::Row => {
                // Drill down to inner subplot to get y-axis title
                if let Some(src) = self.facet_sources.first() {
                    if let Some(guide) = src.subplot.compiled_guide.as_ref() {
                        // Ask the inner guide for row-unifiable channel (y-axis)
                        return guide.facet_unifiable_channel(
                            facet_direction,
                            src.subplot.marks(),
                            session_context,
                        );
                    }
                }
                None
            }
        }
    }

    fn unifies_channel(&self, channel: &str) -> bool {
        // FacetColGuide unifies the x-channel (suppresses x-axis titles in subplots)
        channel == ColumnDimensionConfig::unified_title_channel()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
