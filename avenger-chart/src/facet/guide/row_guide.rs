//! FacetRowGuide implementation for row-based faceting.
//!
//! This module contains the guide implementation for FacetRow coordinate system,
//! which renders facet labels vertically along the side of the plot area with one
//! label per row.

use crate::channel::config_traits::ScaleSharing;
use crate::facet::dimension_config::{FacetDimensionConfig, RowDimensionConfig};
use crate::facet::guide::measurement::{AsyncMeasureOverflowFn, measure_with_coordination_impl};
use crate::facet::guide::shared::{
    FacetSource, aggregate_cached_overflow, apply_shared_overflow_coordination,
    compute_scale_sharing_for_nested_facet, default_overflow_fallback,
};
use crate::facet::phantom_cells::PhantomPlacement;
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

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    // Collected facet sources from compiled marks (populated via set_compiled_marks)
    facet_sources: Vec<FacetSource>,
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
    unified_y_title: Option<String>,
    /// Unified x-axis title for nested FacetRow cases (rendered at bottom)
    unified_x_title: Option<String>,
    /// The channel that can be unified (from subplot guide declaration)
    unifiable_channel: Option<String>,
}

impl FacetRowGuide {
    /// Compute maximum subplot overflow across all facet sources
    /// This can be called from both measure_overflow and evaluate without caching
    async fn compute_max_subplot_overflow(
        &self,
        row_scale: &ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        overflow: Option<&Vec<OverflowSpaceRequirement>>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<(f32, f32, f32, f32), crate::error::AvengerChartError> {
        // Extract discrete domain values
        let mut domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        // Sort to ensure deterministic facet ordering
        domain_vals.sort_by(scalar_total_cmp);

        let mut max_left: f32 = 0.0;
        let mut max_right: f32 = 0.0;
        let mut top: f32 = 0.0;
        let mut bottom: f32 = 0.0;

        // For each facet source (there could be more than one Facet mark)
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetRowGuide: Checking {} facet sources",
                self.facet_sources.len()
            );
        }
        for (source_idx, source) in self.facet_sources.iter().enumerate() {
            // Priority: overflow parameter takes precedence over data_override
            // When overflow parameter is provided (from measurement phase), use it for
            // consistent unified title positioning. This ensures rendering uses the same
            // global overflow values as measurement, preventing title/label overlap.
            if let Some(per_facet_overflow) = overflow {
                // Use overflow parameter if provided (passed from rendering pipeline)
                // This ensures unified titles are positioned based on global measurement,
                // not local column-specific overflow.
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide [source {}]: Using per-facet overflow ({} subplots) - takes priority over data_override",
                        source_idx,
                        per_facet_overflow.len()
                    );
                }

                // Use helper to aggregate cached overflow
                (top, bottom, max_left, max_right) =
                    aggregate_cached_overflow(per_facet_overflow, top, bottom, max_left, max_right);
            } else if let Some(df) = data_override {
                // No cached overflow but we have data - compute overflow by measuring subplots
                // This is the nested facet case where the inner facet needs to measure its Cartesian subplots

                // Get the facet expression from the channel
                let facet_expr = if let Some(cv) = source
                    .data
                    .channels()
                    .get(RowDimensionConfig::channel_name())
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
                        // This is stored in the FacetSource (from CompiledFacetRow.facet_scale_sharing)
                        source
                            .facet_scale_sharing
                            .map(|mode| !mode.is_free())
                            .unwrap_or(false)
                    };

                    // Build filtered_domain_vals to track which values have data
                    // Also collect (domain_val, has_data) pairs for iteration
                    let mut domain_with_data: Vec<(ScalarValue, bool)> = Vec::new();
                    for domain_val in &domain_vals {
                        let filter_df = df
                            .clone()
                            .filter(expr.clone().eq(lit(domain_val.clone())))?;
                        // Check if there's any data for this domain value
                        let count = filter_df.clone().count().await?;
                        domain_with_data.push((domain_val.clone(), count > 0));
                    }

                    // Determine which domain values to iterate over
                    // When scales are shared, use full domain (with empty cells for missing data)
                    // When scales are free, only use values present in this column's data
                    let mut iteration_domain: Vec<(ScalarValue, bool)> = if use_full_domain {
                        // Shared: use all domain values, track which have data
                        domain_with_data.clone()
                    } else {
                        // Free: filter to only values with data
                        domain_with_data
                            .into_iter()
                            .filter(|(_, has_data)| *has_data)
                            .collect()
                    };

                    let num_present = iteration_domain.iter().filter(|(_, has)| *has).count();

                    // IMPORTANT: For nested facets with shared scales, the row scale may be built
                    // from column-filtered data, causing domain_vals to have fewer values than the
                    // actual shared domain. Use inner_domain_count to ensure we iterate over all
                    // positions for correct edge detection (bottom subplot shows x-axis labels).
                    if let Some(ctx) = coord_ctx.as_ref() {
                        if ctx.inner_domain_count > 0
                            && iteration_domain.len() < ctx.inner_domain_count
                            && use_full_domain
                        {
                            // Expand iteration_domain to include all positions with placeholder values
                            // Mark extra positions as has_data=false (empty cells)
                            let current_len = iteration_domain.len();
                            for _i in current_len..ctx.inner_domain_count {
                                // Use Null placeholder for missing domain values
                                iteration_domain.push((ScalarValue::Null, false));
                            }
                            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "FacetRowGuide [source {}]: Expanded iteration_domain from {} to {} using inner_domain_count",
                                    source_idx, current_len, ctx.inner_domain_count
                                );
                            }
                        }
                    }

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetRowGuide [source {}]: Computing overflow from data_override ({} of {} domain values present, use_full_domain={})",
                            source_idx,
                            num_present,
                            domain_vals.len(),
                            use_full_domain
                        );
                    }

                    // Compute band height for subplots based on iteration domain size
                    // Account for inter-row gaps when computing band height
                    // For uniform Free scaling, use max cell count instead of actual domain length
                    let num_rows = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_uniform_cell_count())
                        .unwrap_or(iteration_domain.len())
                        .max(1);
                    let inter_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| {
                            ctx.get_coordinated_spacing(RowDimensionConfig::inter_gap_key())
                        })
                        .unwrap_or(0.0);
                    let total_gap = inter_gap * (num_rows.saturating_sub(1)) as f32;
                    let band_height = RowDimensionConfig::compute_band_size(
                        plot_width,
                        plot_height,
                        num_rows,
                        total_gap,
                    );

                    // Compute scale sharing from marks for Level(N) domain extension
                    let computed_scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

                    // Build scales for subplot measurement using two-step pattern for data override
                    let mut builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, df)
                        .await?;

                    // Extend with level-based extents from coordination context for Level(N>=1) channels
                    {
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
                        .build_scales_from_builder(&builder, plot_width, band_height, ctx, params)
                        .await?;

                    // Measure each subplot using the iteration domain
                    // Create per-row FacetContext with correct position so Cartesian
                    // subplots correctly determine which axes to show/measure
                    let mut computed_overflow = Vec::new();
                    for (row_idx, (domain_val, has_data)) in iteration_domain.iter().enumerate() {
                        use crate::facet::context::FacetContext;
                        // Get the DataFrame for this cell - either filtered data or empty
                        let cell_df = if *has_data {
                            df.clone()
                                .filter(expr.clone().eq(lit(domain_val.clone())))?
                        } else {
                            // Empty cell: create an empty DataFrame with the same schema
                            df.clone().limit(0, Some(0))?
                        };

                        // Create FacetContext with correct position for THIS row
                        let measure_params = {
                            let mut updated_params = params.clone();
                            // Convert static slice to HashSet<String>
                            let mut unified_channels: std::collections::HashSet<String> =
                                RowDimensionConfig::unified_channels()
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect();

                            // Also unify "x" if we have a unified x-title (same-type nesting case)
                            if self.unified_x_title.is_some() {
                                unified_channels.insert("x".to_string());
                            }

                            // Merge parent's unified_channels and scale_sharing if present
                            let (parent_col, parent_num_cols, mut merged_scale_sharing) =
                                if let Some(parent_ctx) = FacetContext::from_params(params) {
                                    for ch in &parent_ctx.unified_channels {
                                        unified_channels.insert(ch.clone());
                                    }
                                    (
                                        parent_ctx.position.1,
                                        parent_ctx.grid_dimensions.1,
                                        parent_ctx.scale_sharing.clone(),
                                    )
                                } else {
                                    (0, 1, std::collections::HashMap::new())
                                };

                            // Merge channel_sharing_levels from FacetCoordinationContext
                            // This contains the per-channel scale sharing modes (x, y) computed
                            // from channel configs, ensuring measurement uses same visibility
                            // decisions as rendering.
                            // Also extract inner_domain_count or uniform_cell_count for proper grid dimensions.
                            // Also extract phantom_prepend_count for correct row position adjustment.
                            let (grid_num_rows, phantom_prepend) = if let Some(coord_ctx) =
                                FacetCoordinationContext::from_params(params)
                            {
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "FacetRowGuide FacetContext: coord_ctx enable_uniform={} max_inner={:?} phantom_prepend={}",
                                        coord_ctx.enable_uniform_free_scaling,
                                        coord_ctx.max_inner_cell_count,
                                        coord_ctx.phantom_prepend_count
                                    );
                                }
                                for (channel, level) in &coord_ctx.channel_sharing_levels {
                                    merged_scale_sharing
                                        .insert(channel.clone(), ScaleSharing::from_level(*level));
                                }
                                // Use uniform_cell_count (for Free scaling) or inner_domain_count (for Shared) for grid dimensions
                                // This ensures correct edge detection for axis visibility and uniform band sizing
                                let grid_rows = if let Some(uniform_count) =
                                    coord_ctx.get_uniform_cell_count()
                                {
                                    uniform_count
                                } else if coord_ctx.inner_domain_count > 0 {
                                    coord_ctx.inner_domain_count
                                } else {
                                    num_rows
                                };

                                // Compute phantom_prepend using PhantomPlacement helper
                                let actual_rows = iteration_domain.len();
                                let phantom_placement = PhantomPlacement::compute(
                                    coord_ctx.inner_band_align,
                                    actual_rows,
                                    grid_rows,
                                );
                                let computed_phantom_prepend = phantom_placement.prepend_count();
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
                                    && phantom_placement.phantom_count > 0
                                {
                                    eprintln!(
                                        "FacetRowGuide: computed phantom_prepend={} (grid_rows={} actual={} band_align={})",
                                        computed_phantom_prepend,
                                        grid_rows,
                                        actual_rows,
                                        coord_ctx.inner_band_align
                                    );
                                }
                                (grid_rows, computed_phantom_prepend)
                            } else {
                                (num_rows, 0)
                            };

                            // Adjust row index by phantom_prepend_count for correct position
                            // When phantoms are prepended, actual data starts at position phantom_prepend
                            let adjusted_row_idx = row_idx + phantom_prepend;

                            // Compute global_edge_channels: inherit from parent ONLY if same-type nesting
                            // Row's additional unified channel is "x" (when unified_x_title is set).
                            // Only inherit if parent's global_edge_channels contains "x" (same-type: Row>Row).
                            let global_edge_channels = if let Some(parent_ctx) = FacetContext::from_params(params) {
                                // Check if parent's global_edge_channels has "x" (Row>Row same-type nesting)
                                let is_same_type = parent_ctx.global_edge_channels.contains("x");
                                if is_same_type {
                                    let (row, col) = (adjusted_row_idx, parent_col);
                                    let (num_rows, _num_cols) = (grid_num_rows, parent_num_cols);
                                    parent_ctx.global_edge_channels
                                        .into_iter()
                                        .filter(|ch| match ch.as_str() {
                                            "x" => row == num_rows - 1, // Bottom edge
                                            "y" => col == 0,           // Left edge
                                            _ => true,
                                        })
                                        .collect()
                                } else {
                                    std::collections::HashSet::new()
                                }
                            } else {
                                std::collections::HashSet::new()
                            };

                            let facet_ctx = FacetContext {
                                position: (adjusted_row_idx, parent_col),
                                grid_dimensions: (grid_num_rows, parent_num_cols),
                                unified_channels,
                                scale_sharing: merged_scale_sharing,
                                global_edge_channels,
                            };
                            let ctx_params = facet_ctx.to_params();
                            for (k, v) in ctx_params {
                                updated_params.insert(k, v);
                            }
                            updated_params
                        };

                        let (_guide_only, total_overflow, _legend_info, _legend_positions) = source
                            .subplot
                            .measure_with_scales(
                                plot_width,
                                band_height,
                                ctx,
                                &measure_params,
                                &subplot_scales,
                                Some(&cell_df),
                            )
                            .await?;

                        computed_overflow.push(total_overflow);
                    }

                    // Aggregate computed overflow using dimension-specific trait method
                    let (t, b, l, r) =
                        RowDimensionConfig::aggregate_overflow_edges(&computed_overflow);
                    top = top.max(t);
                    bottom = bottom.max(b);
                    max_left = max_left.max(l);
                    max_right = max_right.max(r);
                } else {
                    // No facet expression available - use fallback (data_override path)
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetRowGuide [source {}]: No facet expression in data_override path, using fallback",
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
                        "FacetRowGuide [source {}]: Computing overflow from source data ({} domain values)",
                        source_idx,
                        domain_vals.len()
                    );
                }

                // Get the facet expression from the channel
                let facet_expr = if let Some(cv) = source
                    .data
                    .channels()
                    .get(RowDimensionConfig::channel_name())
                {
                    cv.expr(ctx)
                } else {
                    None
                };

                if let Some(expr) = facet_expr {
                    use crate::facet::coordination::FacetCoordinationContext;

                    // Compute band height for subplots
                    // Account for inter-row gaps when computing band height
                    let coord_ctx = FacetCoordinationContext::from_params(params);
                    let num_rows = domain_vals.len().max(1);
                    let inter_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| {
                            ctx.get_coordinated_spacing(RowDimensionConfig::inter_gap_key())
                        })
                        .unwrap_or(0.0);
                    let total_gap = inter_gap * (num_rows.saturating_sub(1)) as f32;
                    let band_height = RowDimensionConfig::compute_band_size(
                        plot_width,
                        plot_height,
                        num_rows,
                        total_gap,
                    );

                    // Build scales for subplot measurement using two-step pattern for data override
                    let builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, &df)
                        .await?;
                    let subplot_scales = source
                        .subplot
                        .build_scales_from_builder(&builder, plot_width, band_height, ctx, params)
                        .await?;

                    // Measure each subplot with correct per-subplot FacetContext
                    let mut computed_overflow = Vec::new();
                    let num_rows = domain_vals.len();

                    // Compute scale sharing from marks so subplots know which axes to measure
                    // Use nested facet version to correctly handle FacetCol inside FacetRow
                    let mut scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

                    // Merge channel_sharing_levels from FacetCoordinationContext
                    // This contains the per-channel scale sharing modes (x, y) computed
                    // from channel configs, ensuring measurement uses same visibility
                    // decisions as rendering.
                    if let Some(coord_ctx) = FacetCoordinationContext::from_params(params) {
                        for (channel, level) in &coord_ctx.channel_sharing_levels {
                            scale_sharing.insert(channel.clone(), ScaleSharing::from_level(*level));
                        }
                    }

                    for (row_idx, domain_val) in domain_vals.iter().enumerate() {
                        use datafusion::logical_expr::lit;
                        let filter_df = df
                            .clone()
                            .filter(expr.clone().eq(lit(domain_val.clone())))?;

                        // Create FacetContext with correct position for this subplot
                        // so that should_show_title/should_show_labels work correctly
                        let subplot_measure_params = {
                            use crate::facet::context::FacetContext;
                            let mut updated_params = params.clone();
                            let mut unified_channels: std::collections::HashSet<String> =
                                RowDimensionConfig::unified_channels()
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect();
                            // Also unify "x" if we have a unified x-title (same-type nesting case)
                            if self.unified_x_title.is_some() {
                                unified_channels.insert("x".to_string());
                            }
                            // Merge parent's unified_channels to propagate x unification through nesting
                            if let Some(parent_ctx) = FacetContext::from_params(params) {
                                for ch in &parent_ctx.unified_channels {
                                    unified_channels.insert(ch.clone());
                                }
                            }
                            // Compute global_edge_channels: inherit from parent ONLY if same-type nesting
                            // Row's additional unified channel is "x" (when unified_x_title is set).
                            // Only inherit if parent's global_edge_channels contains "x" (same-type: Row>Row).
                            let global_edge_channels = if let Some(parent_ctx) = FacetContext::from_params(params) {
                                // Check if parent's global_edge_channels has "x" (Row>Row same-type nesting)
                                let is_same_type = parent_ctx.global_edge_channels.contains("x");
                                if is_same_type {
                                    let (row, col) = (row_idx, 0);
                                    parent_ctx.global_edge_channels
                                        .into_iter()
                                        .filter(|ch| match ch.as_str() {
                                            "x" => row == num_rows - 1, // Bottom edge
                                            "y" => col == 0,           // Left edge
                                            _ => true,
                                        })
                                        .collect()
                                } else {
                                    std::collections::HashSet::new()
                                }
                            } else {
                                std::collections::HashSet::new()
                            };

                            let facet_ctx = FacetContext {
                                position: (row_idx, 0), // Correct row position
                                grid_dimensions: (num_rows, 1),
                                unified_channels,
                                scale_sharing: scale_sharing.clone(),
                                global_edge_channels,
                            };
                            let ctx_params = facet_ctx.to_params();
                            for (k, v) in ctx_params {
                                updated_params.insert(k, v);
                            }
                            updated_params
                        };

                        let (_guide_only, total_overflow, _legend_info, _legend_positions) = source
                            .subplot
                            .measure_with_scales(
                                plot_width,
                                band_height,
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
                        RowDimensionConfig::aggregate_overflow_edges(&computed_overflow);
                    top = top.max(t);
                    bottom = bottom.max(b);
                    max_left = max_left.max(l);
                    max_right = max_right.max(r);
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetRowGuide [source {}]: No facet expression from source data, using fallback",
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
                        "FacetRowGuide [source {}]: No overflow data and no data available, using fallback",
                        source_idx
                    );
                }
                return Ok(default_overflow_fallback());
            }
        }
        Ok((top, bottom, max_left, max_right))
    }
}

impl CoordinateGuide for FacetRowGuide {
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
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetRow>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: facet.facet_title.clone(),
                    facet_scale_sharing: facet.facet_scale_sharing,
                });
            }
        }
        // Derive default facet title if not explicitly set on any facet mark
        if self.facet_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                // Try to extract a column name from channel
                if let Some(cv) = src.data.channels().get(RowDimensionConfig::channel_name()) {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.facet_title = Some(name);
                    }
                }
                // Prefer user-specified title if available
                if let Some(title) = &src.user_title {
                    self.facet_title = Some(title.clone());
                }
            }
        }
        // Derive unified title from subplot's guide declaration
        // The subplot guide declares what channel can be unified for row faceting
        if self.unified_y_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        RowDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_channel = Some(info.channel);
                    self.unified_y_title = info.title;
                }
            }
        }

        // Derive unified x-axis title from subplot guide (for nested FacetRow case)
        // This allows FacetRow to show ONE x-axis title at the bottom for nested FacetRow subplots.
        //
        // For same-type nesting (Row > Row > Row), we extract the x-axis title from the innermost
        // subplot to display once at the bottom. The outer FacetRow unifies "x" to suppress
        // intermediate x-axis titles.
        if self.unified_x_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(guide) = src.subplot.compiled_guide.as_ref() {
                    // Check if this is same-type nesting (FacetRow wrapping FacetRow)
                    // For same-type nesting, we always extract the x-axis title to unify it
                    let is_same_type_nesting = guide
                        .as_any()
                        .downcast_ref::<FacetRowGuide>()
                        .is_some();

                    // Also extract if the nested guide already unifies x (e.g., deeply nested)
                    let should_extract = is_same_type_nesting || guide.unifies_channel("x");

                    if should_extract {
                        use crate::facet::dimension_config::ColumnDimensionConfig;
                        if let Some(info) = guide.facet_unifiable_channel(
                            ColumnDimensionConfig::facet_direction(),
                            src.subplot.marks(),
                            _session_context,
                        ) {
                            self.unified_x_title = info.title;
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

/// Wrapper to call FacetRowGuide::measure_overflow from generic function
pub(crate) struct RowMeasureOverflowWrapper<'a> {
    pub guide: &'a FacetRowGuide,
}

impl AsyncMeasureOverflowFn for RowMeasureOverflowWrapper<'_> {
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
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        // Need row scale
        let row_scale = scales
            .get(RowDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetRowGuide",
                        RowDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Compute subplot overflow using shared helper
        let (top, bottom, max_left, max_right) = self
            .compute_max_subplot_overflow(
                row_scale,
                plot_width,
                plot_height,
                theme,
                params,
                ctx,
                _row_overflow,
                data_override,
            )
            .await?;

        // Apply shared overflow from coordination context for uniform padding across columns
        let mut overflow = (top, bottom, max_left, max_right);
        apply_shared_overflow_coordination(&mut overflow, params);
        let (top, bottom, max_left, max_right) = overflow;

        // Add space for facet labels by measuring text bounds
        // For 90° rotation, horizontal footprint ≈ text height
        let labels = row_scale.domain_labels().unwrap_or_default();

        // Resolve facet-label theme (fallbacks kept for now)
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());

        // Resolve facet title theme
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Extract y-axis position from subplot guide for visibility resolution
        let y_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("y"));

        // Extract x-axis position for unified x-title placement (top vs bottom)
        let x_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("x"));

        // Resolve visibility decisions using single source of truth
        // (moved before measurement so we can use render_facet_title)
        use crate::facet::context::FacetContext;
        use crate::facet::visibility::{FacetRowVisibility, FacetRowVisibilityInput};
        let facet_scale_sharing = self
            .facet_sources
            .first()
            .and_then(|source| source.facet_scale_sharing);
        let visibility_input = FacetRowVisibilityInput {
            y_axis_position,
            max_left,
            max_right,
            has_unified_y_title: self.unified_y_title.is_some(),
            has_unified_x_title: self.unified_x_title.is_some(),
            facet_scale_sharing,
        };
        let parent_ctx = FacetContext::from_params(params);
        let visibility = FacetRowVisibility::resolve(&visibility_input, parent_ctx.as_ref());

        // Use guide_utils to measure facet label slab
        use crate::facet::guide_utils::{FacetLabelMeasurementConfig, measure_facet_label_slab};

        let measurement_config = FacetLabelMeasurementConfig {
            labels: labels.clone(),
            is_rotated: true, // Row labels are vertical
            font_family: label_font_family.clone(),
            font_size_px: label_font_px,
            title: self.facet_title.clone(),
            title_font_family: title_font_family.clone(),
            title_font_size_px: title_font_px,
            render_title: visibility.render_facet_title,
        };
        let estimated_right = measure_facet_label_slab(&measurement_config);

        // Use visibility struct for all derived values
        let axis_on_right = visibility.axis_on_right;
        let is_left_edge = visibility.is_left_edge;
        let is_right_edge = visibility.is_right_edge;
        let parent_unified_y = visibility.parent_unified_y;

        // FacetContext now correctly propagates position to subplots, so they
        // already produce correct overflow values. Use measured values directly.
        let adjusted_max_left = max_left;
        let adjusted_max_right = max_right;

        // Measure unified y title height (rotated width) - but only if parent hasn't unified y
        let unified_y_height = if !parent_unified_y {
            if let Some(y_title) = &self.unified_y_title {
                let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
                let y_family_owned = theme
                    .font_family(&y_ctx)
                    .unwrap_or_else(|| "sans-serif".to_string());
                let cfg_y = avenger_text::measurement::TextMeasurementConfig {
                    text: y_title,
                    font: y_family_owned.as_str(),
                    font_size: y_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let b_y =
                    avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg_y);
                b_y.height + 1.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        let gap_y_axis = if self.unified_y_title.is_some() && !parent_unified_y {
            10.0
        } else {
            0.0
        };

        // Measure unified x-title height (for top or bottom overflow based on x-axis position)
        let unified_x_height = if visibility.render_unified_x_title {
            if let Some(x_title) = &self.unified_x_title {
                let x_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let x_font_px = theme.font_size(&x_ctx).unwrap_or(12.0_f32);
                let x_family_owned = theme
                    .font_family(&x_ctx)
                    .unwrap_or_else(|| "sans-serif".to_string());
                let cfg_x = avenger_text::measurement::TextMeasurementConfig {
                    text: x_title,
                    font: x_family_owned.as_str(),
                    font_size: x_font_px,
                    font_weight: &avenger_text::types::FontWeight::Name(
                        avenger_text::types::FontWeightNameSpec::Normal,
                    ),
                    font_style: &avenger_text::types::FontStyle::Normal,
                };
                let b_x =
                    avenger_text::measurement::default_text_measurer().measure_text_bounds(&cfg_x);
                b_x.height + 1.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        let gap_x_axis = if unified_x_height > 0.0 { 10.0 } else { 0.0 };

        // Add unified x-title space to top or bottom based on x-axis position
        use crate::cartesian::axis::AxisPosition as CartesianAxisPosition;
        let x_axis_at_top = matches!(x_axis_position, Some(CartesianAxisPosition::Top));
        let (top_final, bottom_final) = if x_axis_at_top {
            (top + gap_x_axis + unified_x_height, bottom)
        } else {
            (top, bottom + gap_x_axis + unified_x_height)
        };

        // Unified y-title goes on the SAME side as the y-axis (it labels all rows together)
        // Facet labels go on the OPPOSITE side from the y-axis
        // Only include facet label space when labels will actually be rendered
        // (This is the key fix: measurement now matches rendering visibility)
        let adjusted_estimated_right = if visibility.render_facet_labels {
            estimated_right
        } else {
            0.0
        };

        let left_final = if axis_on_right {
            // Axis on right: left side has facet labels (no unified y-title here)
            adjusted_max_left + adjusted_estimated_right
        } else {
            // Axis on left: left side has axis overflow + unified y-title
            adjusted_max_left
                + if unified_y_height > 0.0 && is_left_edge {
                    gap_y_axis + unified_y_height
                } else {
                    0.0
                }
        };
        let right_final = if axis_on_right {
            // Axis on right: right side has axis overflow + unified y-title
            adjusted_max_right
                + if unified_y_height > 0.0 && is_right_edge {
                    gap_y_axis + unified_y_height
                } else {
                    0.0
                }
        } else {
            // Axis on left: right side has facet labels (no unified y-title here)
            adjusted_max_right + adjusted_estimated_right
        };
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetRowGuide measure_overflow: left_final={:.3} right_final={:.3} (max_left={:.3} adj_max_left={:.3} max_right={:.3} adj_max_right={:.3} estimated_right={:.3} adj_estimated_right={:.3} unified_y_height={:.3} gap_y_axis={:.3} axis_on_right={} is_left_edge={} is_right_edge={} render_facet_labels={})",
                left_final,
                right_final,
                max_left,
                adjusted_max_left,
                max_right,
                adjusted_max_right,
                estimated_right,
                adjusted_estimated_right,
                unified_y_height,
                gap_y_axis,
                axis_on_right,
                is_left_edge,
                is_right_edge,
                visibility.render_facet_labels
            );
        }
        Ok(OverflowSpaceRequirement {
            top: top_final,
            bottom: bottom_final,
            left: left_final,
            right: right_final,
        })
    }

    async fn measure_intrinsic_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        // Get row scale
        let row_scale = scales
            .get(RowDimensionConfig::channel_name())
            .ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(
                    format!(
                        "Missing '{}' scale for FacetRowGuide",
                        RowDimensionConfig::channel_name()
                    )
                    .into(),
                )
            })?;

        // Return ONLY the intrinsic subplot overflow - no facet labels, titles, or unified axis titles
        let (top, bottom, left, right) = self
            .compute_max_subplot_overflow(
                row_scale,
                plot_width,
                plot_height,
                theme,
                params,
                ctx,
                _row_overflow,
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
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<MeasurementResult, crate::error::AvengerChartError> {
        // Use generic implementation with RowDimensionConfig
        measure_with_coordination_impl::<RowDimensionConfig>(
            &self.facet_sources,
            self.unified_y_title.as_ref(),
            scales,
            _row_overflow,
            col_overflow,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            RowMeasureOverflowWrapper { guide: self },
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
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        // Row scale
        let row_scale = match scales.get(RowDimensionConfig::channel_name()) {
            Some(s) => s,
            None => {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!("FacetRowGuide evaluate: NO ROW SCALE - returning early");
                }
                return Ok(marks);
            }
        };

        // Domain labels
        let labels = row_scale.domain_labels()?;
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetRowGuide evaluate: {} domain labels, unified_y_title={:?}",
                labels.len(),
                self.unified_y_title
            );
        }

        // Get band positions from the scale
        // The scale now has the correct padding_inner_px from the facet mark (via scale updates).
        // We'll use .center() on each BandPosition for label/tick positioning.
        //
        // For uniform Free scaling: if uniform_cell_count > actual labels, we need to compute
        // band positions as if there were uniform_cell_count values. This ensures labels are
        // positioned at the correct center (e.g., half-height for 2 cells when only 1 exists).
        use crate::facet::band_positions::BandPositionIterator;
        use crate::facet::coordination::FacetCoordinationContext;
        let uniform_cell_count = FacetCoordinationContext::from_params(params)
            .and_then(|ctx| ctx.get_uniform_cell_count());

        // Get inner_band_align from coordination context for facet label positioning
        let inner_band_align = FacetCoordinationContext::from_params(params)
            .map(|ctx| ctx.inner_band_align)
            .unwrap_or(0.0);

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
                        "FacetRowGuide evaluate: uniform sizing - creating temp scale with {} values (actual={}) for band positioning, inner_band_align={}",
                        uniform_count,
                        labels.len(),
                        inner_band_align
                    );
                }

                // Create temporary scale with padded domain
                let padded_array = StdArc::new(StringArray::from(padded_labels))
                    as datafusion::arrow::array::ArrayRef;
                let temp_scale = row_scale.clone().with_domain(padded_array);

                // Get positions from the padded scale
                let all_positions: Vec<_> =
                    BandPositionIterator::from_configured_scale(&temp_scale)?.collect();

                // Extract only the positions for actual labels (not placeholders)
                let phantom_placement =
                    PhantomPlacement::compute(inner_band_align, labels.len(), uniform_count);
                phantom_placement.extract_actual(all_positions)
            } else {
                // No padding needed
                BandPositionIterator::from_configured_scale(row_scale)?.collect()
            }
        } else {
            // No uniform sizing
            BandPositionIterator::from_configured_scale(row_scale)?.collect()
        };

        // Theme-based font for rendering (match measurement)
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let font_family_owned = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());
        let font_family = font_family_owned.as_str();

        // Decide side based on child overflow (prefer left if right child overflow > left)
        let domain_labels_eval = row_scale.domain_labels().unwrap_or_default();

        // Convert domain labels to ScalarValues for SubplotIterator
        let _domain_vals_eval: Vec<datafusion::common::ScalarValue> = domain_labels_eval
            .iter()
            .map(|s| datafusion::common::ScalarValue::Utf8(Some(s.clone())))
            .collect();

        // Compute INTRINSIC subplot overflow for label positioning.
        //
        // The cached _row_overflow may contain INFLATED values if our subplot contains
        // a nested FacetRow. This is because:
        // - FacetRow adds labels on LEFT/RIGHT
        // - FacetRowGuide uses LEFT/RIGHT overflow to position its labels
        // - If subplot has nested FacetRow, its LEFT/RIGHT overflow includes those labels
        //
        // We need to re-compute intrinsic overflow (ignoring nested facet labels) when:
        // - The subplot contains a nested FacetRow (same-type nesting causes dimension overlap)
        //
        // For alternating nesting (FacetRow > FacetCol), the cached values are correct
        // because FacetCol adds labels on TOP/BOTTOM, which don't affect LEFT/RIGHT.
        use crate::facet::guide::shared::marks_contain_nested_facet_type;
        let has_same_type_nested_facet = self
            .facet_sources
            .iter()
            .any(|source| marks_contain_nested_facet_type(&source.subplot.marks, "facet_row"));

        let overflow_for_computation = if has_same_type_nested_facet {
            None // Same-type nesting: force re-computation to get intrinsic overflow
        } else {
            _row_overflow // Different-type or no nesting: cached values are correct
        };
        let (max_top_child, max_bottom_child, max_left_child, max_right_child) = self
            .compute_max_subplot_overflow(
                row_scale,
                plot_width,
                plot_height,
                theme,
                params,
                ctx,
                overflow_for_computation,
                data_override,
            )
            .await?;
        // Extract y-axis position from subplot guide for visibility resolution
        let y_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("y"));

        // Extract x-axis position for unified x-title placement (top vs bottom)
        let x_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("x"));

        // Resolve visibility decisions using single source of truth (same as measure_overflow)
        use crate::facet::context::FacetContext;
        use crate::facet::visibility::{FacetRowVisibility, FacetRowVisibilityInput};
        let facet_scale_sharing = self
            .facet_sources
            .first()
            .and_then(|source| source.facet_scale_sharing);
        let visibility_input = FacetRowVisibilityInput {
            y_axis_position,
            max_left: max_left_child,
            max_right: max_right_child,
            has_unified_y_title: self.unified_y_title.is_some(),
            has_unified_x_title: self.unified_x_title.is_some(),
            facet_scale_sharing,
        };
        let parent_ctx = FacetContext::from_params(params);
        let visibility = FacetRowVisibility::resolve(&visibility_input, parent_ctx.as_ref());

        // Use visibility struct for rendering decisions
        let place_on_left = visibility.facet_labels_on_left;
        let should_render_facet_labels = visibility.render_facet_labels;

        // Resolve title font properties for rendering
        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Use guide_utils to render facet label slab (labels + rule + title)
        // Only render on the edge column where facet labels should appear
        if should_render_facet_labels {
            use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

            // Extend plot bounds to include subplot overflow so facet labels are positioned
            // outside of the subplot axes and legends
            let render_plot_bounds = if place_on_left {
                // Labels on left: extend leftward by left overflow
                LayoutBounds {
                    x: plot_bounds.x - max_left_child,
                    y: plot_bounds.y,
                    width: plot_width + max_left_child,
                    height: plot_height,
                }
            } else {
                // Labels on right: extend rightward by right overflow (includes legends)
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_width + max_right_child,
                    height: plot_height,
                }
            };

            let render_config = FacetLabelRenderConfig {
                labels: labels.clone(),
                band_positions: band_positions.clone(),
                plot_bounds: render_plot_bounds,
                is_rotated: true,             // Row labels are vertical
                place_at_end: !place_on_left, // place_at_end=true means right side
                font_family: font_family.to_string(),
                font_size_px: font_px,
                title: self.facet_title.clone(),
                title_font_family: title_font_family.clone(),
                title_font_size_px: title_font_px,
                render_title: visibility.render_facet_title,
            };

            marks.extend(render_facet_label_slab(&render_config, theme, params));
        }

        // Render unified y-axis title if visibility allows
        // (visibility struct already checked parent_unified_y and nested_in_col_facet)
        if let Some(y_title) = &self.unified_y_title {
            if !visibility.render_unified_y_title {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide SKIP unified_y_title='{}' (parent already unified y or nested in col facet)",
                        y_title
                    );
                }
            } else {
                use crate::facet::guide_utils::{
                    UnifiedTitleRenderConfig, render_unified_axis_title,
                };

                let axis_on_right = visibility.axis_on_right;
                let gap = 6.0_f32;
                let x_pos = if axis_on_right {
                    plot_bounds.x + plot_width + max_right_child + gap
                } else {
                    plot_bounds.x - max_left_child - gap
                };
                let y_center = plot_bounds.y + 0.5 * plot_height;

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide RENDER unified_y_title='{}' at x={:.3} y={:.3} (plot_bounds.x={:.3} max_left_child={:.3} gap={:.3} axis_on_right={})",
                        y_title, x_pos, y_center, plot_bounds.x, max_left_child, gap, axis_on_right
                    );
                }

                let title_config = UnifiedTitleRenderConfig {
                    title: y_title.clone(),
                    font_family: title_font_family.clone(),
                    font_size_px: title_font_px,
                    x: x_pos,
                    y: y_center,
                    angle: if axis_on_right { 90.0 } else { -90.0 },
                    axis_at_far_edge: axis_on_right,
                };
                let y_mark = render_unified_axis_title(&title_config, theme, params);
                marks.push(SceneMark::Text(StdArc::new(y_mark)));
            }
        }

        // Render unified x-axis title if visibility allows
        // (visibility struct already checked parent_unified_x and is_bottom_edge)
        if let Some(x_title) = &self.unified_x_title {
            if !visibility.render_unified_x_title {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide SKIP unified_x_title='{}' (parent already unified x or not bottom edge)",
                        x_title
                    );
                }
            } else {
                use crate::facet::guide_utils::{
                    UnifiedTitleRenderConfig, render_unified_axis_title,
                };
                use crate::cartesian::axis::AxisPosition as CartesianAxisPosition;

                let gap = 6.0_f32;
                let x_center = plot_bounds.x + 0.5 * plot_width;
                let x_axis_at_top = matches!(x_axis_position, Some(CartesianAxisPosition::Top));

                // Position at top or bottom based on x-axis position
                let (y_pos, axis_at_far_edge) = if x_axis_at_top {
                    // Title at top: position above the plot and top overflow
                    (plot_bounds.y - max_top_child - gap, false)
                } else {
                    // Title at bottom: position below the plot and bottom overflow
                    (plot_bounds.y + plot_height + max_bottom_child + gap, true)
                };

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide RENDER unified_x_title='{}' at x={:.3} y={:.3} (x_axis_at_top={} plot_bounds.y={:.3} plot_height={:.3} max_top={:.3} max_bottom={:.3} gap={:.3})",
                        x_title, x_center, y_pos, x_axis_at_top, plot_bounds.y, plot_height, max_top_child, max_bottom_child, gap
                    );
                }

                let title_config = UnifiedTitleRenderConfig {
                    title: x_title.clone(),
                    font_family: title_font_family.clone(),
                    font_size_px: title_font_px,
                    x: x_center,
                    y: y_pos,
                    angle: 0.0, // Horizontal, not rotated
                    axis_at_far_edge,
                };
                let x_mark = render_unified_axis_title(&title_config, theme, params);
                marks.push(SceneMark::Text(StdArc::new(x_mark)));
            }
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

    fn facet_unifiable_channel(
        &self,
        facet_direction: crate::guide::FacetDirection,
        _marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
        session_context: &datafusion::prelude::SessionContext,
    ) -> Option<crate::guide::UnifiableChannelInfo> {
        use crate::guide::FacetDirection;

        // For FacetRowGuide, delegate to inner subplot's guide
        // Row faceting unifies y-channel, so if asked for Row, return our own unified_y_title
        // For Column faceting, drill down to inner subplot
        match facet_direction {
            FacetDirection::Row => {
                // This FacetRow already unifies y, return our stored title
                Some(crate::guide::UnifiableChannelInfo {
                    channel: RowDimensionConfig::unified_title_channel().to_string(),
                    title: self.unified_y_title.clone(),
                })
            }
            FacetDirection::Column => {
                // Drill down to inner subplot to get x-axis title
                if let Some(src) = self.facet_sources.first() {
                    if let Some(guide) = src.subplot.compiled_guide.as_ref() {
                        // Ask the inner guide for column-unifiable channel (x-axis)
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

    fn axis_position(&self, channel: &str) -> Option<crate::cartesian::axis::AxisPosition> {
        // Delegate to inner subplot's guide to get actual axis position
        // This allows outer facet guides (FacetColGuide) to correctly determine
        // where axes are positioned in nested facets
        if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                return guide.axis_position(channel);
            }
        }
        None
    }

    fn unifies_channel(&self, channel: &str) -> bool {
        // FacetRowGuide unifies the y-channel (suppresses y-axis titles in subplots)
        // It also unifies x-channel if unified_x_title is set (from nested FacetRow case)
        if channel == RowDimensionConfig::unified_title_channel() {
            return true;
        }
        // Unify x if we have a unified_x_title (nested FacetRow case)
        if channel == "x" && self.unified_x_title.is_some() {
            return true;
        }
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
