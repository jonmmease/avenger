use crate::channel::config_traits::ScaleSharing;
use crate::facet::dimension_config::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig,
};
use crate::guide::{spacing_keys, CompiledGuide, CoordinateGuide, MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::scales::ConfiguredScaleLegendExt;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::measurement::TextMeasurer;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Compute scale sharing mode for each channel from marks.
/// This extracts the sharing configuration from channel definitions.
fn compute_scale_sharing_from_marks(
    marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
) -> HashMap<String, ScaleSharing> {
    let mut scale_sharing_by_channel = HashMap::new();

    // Collect all unique channel names from all marks
    let mut all_channels = std::collections::HashSet::new();
    for m in marks {
        for ch in m.data_context().channels().keys() {
            all_channels.insert(ch.clone());
        }
    }

    // Determine sharing mode for each channel
    for ch in all_channels {
        let mut mode = ScaleSharing::Free;
        for m in marks {
            if let Some(cv) = m.data_context().channels().get(&ch) {
                if let Some(share_mode) = cv.get_share_mode() {
                    mode = match (mode, share_mode) {
                        (ScaleSharing::Free, new_mode) => new_mode,
                        (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                        (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                        (existing, _) => existing,
                    };
                }
            }
        }
        scale_sharing_by_channel.insert(ch, mode);
    }

    scale_sharing_by_channel
}

/// Compute scale sharing mode for nested facet scenarios.
/// When the marks contain a nested facet (e.g., FacetRow inside FacetCol),
/// we need to look at the INNERMOST subplot's marks to get the x/y channel
/// scale sharing from the actual channel configurations (e.g., `.x_with(..., |c| c.with_scale_sharing(...))`).
fn compute_scale_sharing_for_nested_facet(
    marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
) -> HashMap<String, ScaleSharing> {
    use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};

    // First try the standard approach - this won't find x/y sharing for nested facets
    // since the outer marks don't have x/y channels
    let mut scale_sharing = compute_scale_sharing_from_marks(marks);

    // Check if any mark is a nested facet and extract x/y sharing from its subplot
    for m in marks {
        let mark_type = m.mark_type();

        // Check for nested FacetRow (inside FacetCol)
        if mark_type == "facet_row" {
            if let Some(facet_row) = m.as_any().downcast_ref::<CompiledFacetRow>() {
                // Found a nested FacetRow - extract x/y sharing from its subplot's marks
                let inner_marks = &facet_row.compiled_subplot.marks;
                for inner_mark in inner_marks {
                    let channels = inner_mark.data_context().channels();
                    for channel_name in ["x", "y"] {
                        if let Some(channel_value) = channels.get(channel_name) {
                            let share_mode =
                                channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                            scale_sharing.insert(channel_name.to_string(), share_mode);
                        }
                    }
                }
                break;
            }
        }

        // Check for nested FacetCol (inside FacetRow)
        if mark_type == "facet_col" {
            if let Some(facet_col) = m.as_any().downcast_ref::<CompiledFacetCol>() {
                // Found a nested FacetCol - extract x/y sharing from its subplot's marks
                let inner_marks = &facet_col.compiled_subplot.marks;
                for inner_mark in inner_marks {
                    let channels = inner_mark.data_context().channels();
                    for channel_name in ["x", "y"] {
                        if let Some(channel_value) = channels.get(channel_name) {
                            let share_mode =
                                channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                            scale_sharing.insert(channel_name.to_string(), share_mode);
                        }
                    }
                }
                break;
            }
        }
    }

    scale_sharing
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    // Collected facet sources from compiled marks (populated via set_compiled_marks)
    facet_sources: Vec<FacetSource>,
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
    unified_y_title: Option<String>,
    /// The channel that can be unified (from subplot guide declaration)
    unifiable_channel: Option<String>,
}

/// A facet source represents a compiled facet mark and its associated data context.
/// Used during overflow measurement to determine how many cells should be measured.
#[derive(Clone, Serialize, Deserialize)]
struct FacetSource {
    subplot: std::sync::Arc<crate::plot::CompiledPlot>,
    data: crate::marks::CompiledDataContext,
    user_title: Option<String>,
    /// Scale sharing mode for this facet dimension.
    /// When `Shared`, measurement should use the full domain (including empty cells).
    /// When `Free` or None, measurement should only use values present in the current data slice.
    facet_scale_sharing: Option<ScaleSharing>,
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
        domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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

                // Aggregate top/bottom across all subplots
                for overflow_item in per_facet_overflow {
                    top = top.max(overflow_item.top);
                    bottom = bottom.max(overflow_item.bottom);
                }

                // Use first subplot's left overflow
                if let Some(first) = per_facet_overflow.first() {
                    max_left = max_left.max(first.left);
                }

                // Use last subplot's right overflow
                if let Some(last) = per_facet_overflow.last() {
                    max_right = max_right.max(last.right);
                }
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
                        !matches!(ctx.inner_scale_sharing, ScaleSharing::Free)
                    } else {
                        // No coordination context - check the facet's scale sharing config
                        // This is stored in the FacetSource (from CompiledFacetRow.facet_scale_sharing)
                        source
                            .facet_scale_sharing
                            .map(|mode| !matches!(mode, ScaleSharing::Free))
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
                    let inter_row_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_coordinated_spacing("inter_row_gap"))
                        .unwrap_or(0.0);
                    let total_gap = inter_row_gap * (num_rows.saturating_sub(1)) as f32;
                    let band_height = (plot_height - total_gap) / num_rows as f32;

                    // Build scales for subplot measurement using two-step pattern for data override
                    let builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, df)
                        .await?;
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
                            df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?
                        } else {
                            // Empty cell: create an empty DataFrame with the same schema
                            df.clone().limit(0, Some(0))?
                        };

                        // Create FacetContext with correct position for THIS row
                        let measure_params = {
                            let mut updated_params = params.clone();
                            let mut unified_channels = RowDimensionConfig::unified_channels();

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

                            // Merge axis_scale_sharing from FacetCoordinationContext
                            // This contains the per-channel scale sharing modes (x, y) computed
                            // from channel configs, ensuring measurement uses same visibility
                            // decisions as rendering.
                            // Also extract inner_domain_count or uniform_cell_count for proper grid dimensions.
                            let grid_num_rows = if let Some(coord_ctx) =
                                FacetCoordinationContext::from_params(params)
                            {
                                if let Some(ref axis_sharing) = coord_ctx.axis_scale_sharing {
                                    for (channel, mode) in axis_sharing {
                                        merged_scale_sharing.insert(channel.clone(), *mode);
                                    }
                                }
                                // Use uniform_cell_count (for Free scaling) or inner_domain_count (for Shared) for grid dimensions
                                // This ensures correct edge detection for axis visibility and uniform band sizing
                                if let Some(uniform_count) = coord_ctx.get_uniform_cell_count() {
                                    uniform_count
                                } else if coord_ctx.inner_domain_count > 0 {
                                    coord_ctx.inner_domain_count
                                } else {
                                    num_rows
                                }
                            } else {
                                num_rows
                            };

                            let facet_ctx = FacetContext {
                                position: (row_idx, parent_col),
                                grid_dimensions: (grid_num_rows, parent_num_cols),
                                unified_channels,
                                scale_sharing: merged_scale_sharing,
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

                    // Aggregate computed overflow
                    for overflow_item in &computed_overflow {
                        top = top.max(overflow_item.top);
                        bottom = bottom.max(overflow_item.bottom);
                    }
                    if let Some(first) = computed_overflow.first() {
                        max_left = max_left.max(first.left);
                    }
                    if let Some(last) = computed_overflow.last() {
                        max_right = max_right.max(last.right);
                    }
                } else {
                    // No facet expression available - use fallback (data_override path)
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetRowGuide [source {}]: No facet expression in data_override path, using fallback",
                            source_idx
                        );
                    }
                    return Ok((30.0, 30.0, 40.0, 20.0));
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
                    let inter_row_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_coordinated_spacing("inter_row_gap"))
                        .unwrap_or(0.0);
                    let total_gap = inter_row_gap * (num_rows.saturating_sub(1)) as f32;
                    let band_height = (plot_height - total_gap) / num_rows as f32;

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

                    // Merge axis_scale_sharing from FacetCoordinationContext
                    // This contains the per-channel scale sharing modes (x, y) computed
                    // from channel configs, ensuring measurement uses same visibility
                    // decisions as rendering.
                    if let Some(coord_ctx) = FacetCoordinationContext::from_params(params) {
                        if let Some(ref axis_sharing) = coord_ctx.axis_scale_sharing {
                            for (channel, mode) in axis_sharing {
                                scale_sharing.insert(channel.clone(), *mode);
                            }
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
                            let facet_ctx = FacetContext {
                                position: (row_idx, 0), // Correct row position
                                grid_dimensions: (num_rows, 1),
                                unified_channels: RowDimensionConfig::unified_channels(),
                                scale_sharing: scale_sharing.clone(),
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

                    // Aggregate computed overflow
                    for overflow_item in &computed_overflow {
                        top = top.max(overflow_item.top);
                        bottom = bottom.max(overflow_item.bottom);
                    }
                    if let Some(first) = computed_overflow.first() {
                        max_left = max_left.max(first.left);
                    }
                    if let Some(last) = computed_overflow.last() {
                        max_right = max_right.max(last.right);
                    }
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetRowGuide [source {}]: No facet expression from source data, using fallback",
                            source_idx
                        );
                    }
                    return Ok((30.0, 30.0, 40.0, 20.0));
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
                return Ok((30.0, 30.0, 40.0, 20.0));
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
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
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
        let (mut top, mut bottom, mut max_left, mut max_right) = self
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
        // This ensures plot areas align even when scales are not shared
        {
            use crate::facet::coordination::FacetCoordinationContext;
            let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();
            if let Some(shared_left) = coord_ctx.get_coordinated_spacing("shared_overflow_left") {
                max_left = max_left.max(shared_left);
            }
            if let Some(shared_right) = coord_ctx.get_coordinated_spacing("shared_overflow_right") {
                max_right = max_right.max(shared_right);
            }
            if let Some(shared_top) = coord_ctx.get_coordinated_spacing("shared_overflow_top") {
                top = top.max(shared_top);
            }
            if let Some(shared_bottom) = coord_ctx.get_coordinated_spacing("shared_overflow_bottom") {
                bottom = bottom.max(shared_bottom);
            }
        }

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

        // Resolve visibility decisions using single source of truth
        // (moved before measurement so we can use render_facet_title)
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
            facet_scale_sharing,
        };
        let visibility = FacetRowVisibility::resolve(&visibility_input, params);

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

        // Adjust max_left/max_right based on edge position AND scale sharing
        let (adjusted_max_left, adjusted_max_right) =
            visibility.adjusted_subplot_overflow(max_left, max_right);

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
        let gap_axis = if self.unified_y_title.is_some() && !parent_unified_y {
            10.0
        } else {
            0.0
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
                    gap_axis + unified_y_height
                } else {
                    0.0
                }
        };
        let right_final = if axis_on_right {
            // Axis on right: right side has axis overflow + unified y-title
            adjusted_max_right
                + if unified_y_height > 0.0 && is_right_edge {
                    gap_axis + unified_y_height
                } else {
                    0.0
                }
        } else {
            // Axis on left: right side has facet labels (no unified y-title here)
            adjusted_max_right + adjusted_estimated_right
        };
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetRowGuide measure_overflow: left_final={:.3} right_final={:.3} (max_left={:.3} adj_max_left={:.3} max_right={:.3} adj_max_right={:.3} estimated_right={:.3} adj_estimated_right={:.3} unified_y_height={:.3} gap_axis={:.3} axis_on_right={} is_left_edge={} is_right_edge={} render_facet_labels={})",
                left_final,
                right_final,
                max_left,
                adjusted_max_left,
                max_right,
                adjusted_max_right,
                estimated_right,
                adjusted_estimated_right,
                unified_y_height,
                gap_axis,
                axis_on_right,
                is_left_edge,
                is_right_edge,
                visibility.render_facet_labels
            );
        }
        Ok(OverflowSpaceRequirement {
            top,
            bottom,
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
        use crate::facet::coordination::FacetCoordinationContext;
        use crate::facet::guide_measurement::{aggregate_overflow, calculate_inter_row_gap};
        use tracing::debug;

        /// Safety margin for gap calculation (accounts for potential measurement variance)
        const GAP_SAFETY_MARGIN: f32 = 1.0;

        // Guard: Invalid dimensions
        if plot_height <= 0.0 || plot_width <= 0.0 {
            return Ok(MeasurementResult::default());
        }

        // Guard: No facet sources
        if self.facet_sources.is_empty() {
            return Ok(MeasurementResult::default());
        }

        // Parse coordination context ONCE
        let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();

        // RECURSION GUARD: If already coordinated, use single pass (measure_overflow)
        if coord_ctx
            .get_coordinated_spacing(spacing_keys::INTER_ROW_GAP)
            .is_some()
        {
            debug!(
                "FacetRowGuide: Recursion guard - using pre-coordinated inter_row_gap"
            );
            let overflow = self
                .measure_overflow(
                    scales,
                    _row_overflow,
                    col_overflow,
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
                "FacetRowGuide: Nesting guard - NESTED_MEASUREMENT set, using single-pass"
            );
            let overflow = self
                .measure_overflow(
                    scales,
                    _row_overflow,
                    col_overflow,
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

        // Extract domain values
        let mut domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // For uniform Free scaling, use max cell count instead of actual domain length
        let num_rows = coord_ctx
            .get_uniform_cell_count()
            .unwrap_or(domain_vals.len())
            .max(1);

        // === PASS 1: Measure with zero gap ===
        let pass1_band_height = plot_height / num_rows as f32;

        debug!(
            "FacetRowGuide: Pass 1 - measuring {} rows with zero gap, band_height={:.2}",
            num_rows, pass1_band_height
        );

        let mut row_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_rows);
        let mut aggregated_spacing: std::collections::HashMap<String, f32> =
            std::collections::HashMap::new();

        // Get the first facet source (primary source for measurement)
        let source = &self.facet_sources[0];

        // Get facet expression
        let facet_expr = source
            .data
            .channels()
            .get(RowDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(ctx));

        // Get the dataframe for measurement
        let df = data_override
            .cloned()
            .or_else(|| source.data.dataframe_with_context(ctx));

        if let (Some(expr), Some(df)) = (facet_expr, df) {
            use datafusion::logical_expr::lit;

            // Build scales for subplot measurement
            let builder = source
                .subplot
                .build_scale_builder_from_dataframe(ctx, params, &df)
                .await?;
            let subplot_scales = source
                .subplot
                .build_scales_from_builder(&builder, plot_width, pass1_band_height, ctx, params)
                .await?;

            // Convert to ConfiguredScale for measure_with_coordination trait method
            let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            // Compute scale sharing from marks
            let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

            // Measure each row in Pass 1
            for (row_idx, domain_val) in domain_vals.iter().enumerate() {
                use crate::facet::context::FacetContext;

                let filter_df = df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?;

                // Create FacetContext for this cell, including NESTED_MEASUREMENT marker
                // to prevent child facet guides from doing their own 2-pass measurement
                let measure_params = {
                    let mut updated_params = params.clone();
                    let facet_ctx = FacetContext {
                        position: (row_idx, 0),
                        grid_dimensions: (num_rows, 1),
                        unified_channels: RowDimensionConfig::unified_channels(),
                        scale_sharing: scale_sharing.clone(),
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
                let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    guide
                        .measure_with_coordination(
                            &subplot_scales_configured,
                            None,
                            col_overflow,
                            plot_width,
                            pass1_band_height,
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
                            plot_width,
                            pass1_band_height,
                            ctx,
                            &measure_params,
                            &subplot_scales,
                            Some(&filter_df),
                        )
                        .await?;
                    MeasurementResult::new(total_overflow)
                };

                row_overflows.push(child_result.overflow);

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
            let overflow = self
                .measure_overflow(
                    scales,
                    _row_overflow,
                    col_overflow,
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
        let computed_gap = calculate_inter_row_gap(&row_overflows, GAP_SAFETY_MARGIN);

        debug!(
            "FacetRowGuide: Computed inter_row_gap={:.2} (with {:.1}px safety margin)",
            computed_gap, GAP_SAFETY_MARGIN
        );

        // === PASS 2: Re-measure with computed gap ===
        let total_gap = computed_gap * (num_rows.saturating_sub(1)) as f32;
        let pass2_band_height = (plot_height - total_gap) / num_rows as f32;

        debug!(
            "FacetRowGuide: Pass 2 - re-measuring with gap={:.2}, band_height={:.2}",
            computed_gap, pass2_band_height
        );

        // Create params with computed gap for nested guides
        // IMPORTANT: Also include aggregated spacing from Pass 1 to enable recursion guards
        // for nested facets (e.g., FacetRow inside FacetCol should not re-compute its gap)
        let pass2_params = {
            let mut new_ctx = coord_ctx.clone();
            // Add our own computed gap
            new_ctx
                .coordinated_spacing
                .insert(spacing_keys::INTER_ROW_GAP.to_string(), computed_gap);
            // Add all aggregated spacing from children (enables recursion guards for nested facets)
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

        let source = &self.facet_sources[0];
        let facet_expr = source
            .data
            .channels()
            .get(RowDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(ctx));

        let df = data_override
            .cloned()
            .or_else(|| source.data.dataframe_with_context(ctx));

        let mut final_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_rows);
        aggregated_spacing.clear(); // Reset for Pass 2 aggregation

        if let (Some(expr), Some(df)) = (facet_expr, df) {
            use datafusion::logical_expr::lit;

            // Rebuild scales with Pass 2 band height
            let builder = source
                .subplot
                .build_scale_builder_from_dataframe(ctx, &pass2_params, &df)
                .await?;
            let subplot_scales = source
                .subplot
                .build_scales_from_builder(&builder, plot_width, pass2_band_height, ctx, &pass2_params)
                .await?;

            // Convert to ConfiguredScale for measure_with_coordination trait method
            let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

            for (row_idx, domain_val) in domain_vals.iter().enumerate() {
                use crate::facet::context::FacetContext;

                let filter_df = df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?;

                // Create cell params - pass2_params already includes NESTED_MEASUREMENT from the
                // coord_ctx propagation, so child facets will skip 2-pass
                let measure_params = {
                    let mut updated_params = pass2_params.clone();
                    let facet_ctx = FacetContext {
                        position: (row_idx, 0),
                        grid_dimensions: (num_rows, 1),
                        unified_channels: RowDimensionConfig::unified_channels(),
                        scale_sharing: scale_sharing.clone(),
                    };
                    for (k, v) in facet_ctx.to_params() {
                        updated_params.insert(k, v);
                    }
                    updated_params
                };

                let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    guide
                        .measure_with_coordination(
                            &subplot_scales_configured,
                            None,
                            col_overflow,
                            plot_width,
                            pass2_band_height,
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
                            plot_width,
                            pass2_band_height,
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
        Ok(MeasurementResult::new(final_overflow)
            .with_spacing(spacing_keys::INTER_ROW_GAP, computed_gap)
            .merge_spacing_needs(aggregated_spacing))
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
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
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
                        "FacetRowGuide evaluate: uniform sizing - creating temp scale with {} values (actual={}) for band positioning",
                        uniform_count,
                        labels.len()
                    );
                }

                // Create temporary scale with padded domain
                let padded_array =
                    StdArc::new(StringArray::from(padded_labels)) as datafusion::arrow::array::ArrayRef;
                let temp_scale = row_scale.clone().with_domain(padded_array);

                // Get positions from the padded scale (only use first labels.len() positions)
                let all_positions: Vec<_> =
                    BandPositionIterator::from_configured_scale(&temp_scale)?.collect();

                // Return only the positions for actual labels (not placeholders)
                all_positions.into_iter().take(labels.len()).collect()
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

        // Compute subplot overflow using shared helper (no caching needed between calls)
        // Pass data_override so nested facets use filtered data (matching measure_overflow behavior)
        let (_top, _bottom, max_left_child, max_right_child) = self
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

        // Extract y-axis position from subplot guide for visibility resolution
        let y_axis_position = self
            .facet_sources
            .first()
            .and_then(|source| source.subplot.compiled_guide.as_ref())
            .and_then(|guide| guide.axis_position("y"));

        // Resolve visibility decisions using single source of truth (same as measure_overflow)
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
            facet_scale_sharing,
        };
        let visibility = FacetRowVisibility::resolve(&visibility_input, params);

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
                let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
                let y_color = theme.text_color(&y_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let y_family_owned = theme
                    .font_family(&y_ctx)
                    .unwrap_or_else(|| "sans-serif".to_string());
                // Use axis_on_right from visibility struct
                let axis_on_right = visibility.axis_on_right;
                let gap = 6.0_f32;
                // Position title adjacent to subplot axis labels (max_left/right_child is just label space)
                // Center the title in the gap between labels and plot edge
                let x_bottom = if axis_on_right {
                    plot_bounds.x + plot_width + max_right_child + gap
                } else {
                    plot_bounds.x - max_left_child - gap
                };
                let y_center = plot_bounds.y + 0.5 * plot_height;
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetRowGuide RENDER unified_y_title='{}' at x={:.3} y={:.3} (plot_bounds.x={:.3} max_left_child={:.3} gap={:.3} axis_on_right={})",
                        y_title,
                        x_bottom,
                        y_center,
                        plot_bounds.x,
                        max_left_child,
                        gap,
                        axis_on_right
                    );
                }
                let y_mark = SceneTextMark {
                    text: y_title.clone().into(),
                    x: x_bottom.into(),
                    y: y_center.into(),
                    align: TextAlign::Center.into(),
                    baseline: TextBaseline::Bottom.into(),
                    font: y_family_owned.clone().into(),
                    font_size: y_font_px.into(),
                    angle: if axis_on_right {
                        90.0_f32.into()
                    } else {
                        (-90.0_f32).into()
                    },
                    color: avenger_common::types::ColorOrGradient::Color(y_color).into(),
                    zindex: Some(6),
                    ..Default::default()
                };
                marks.push(SceneMark::Text(StdArc::new(y_mark)));
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
                    channel: "y".to_string(),
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
        channel == "y"
    }
}

// ============================================================================
// FacetColGuide - Column Faceting Guide
// ============================================================================

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
        domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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

                // Aggregate top/bottom across all subplots for proper unified title positioning
                for overflow_item in per_facet_overflow {
                    top_max = top_max.max(overflow_item.top);
                    bottom_max = bottom_max.max(overflow_item.bottom);
                }

                // Use first subplot's left overflow (leftmost column edge)
                if let Some(first) = per_facet_overflow.first() {
                    left_max = left_max.max(first.left);
                }

                // Use last subplot's right overflow (rightmost column edge)
                if let Some(last) = per_facet_overflow.last() {
                    right_max = right_max.max(last.right);
                }
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
                        !matches!(ctx.inner_scale_sharing, ScaleSharing::Free)
                    } else {
                        // No coordination context - check the facet's scale sharing config
                        // This is stored in the FacetSource (from CompiledFacetCol.facet_scale_sharing)
                        source
                            .facet_scale_sharing
                            .map(|mode| !matches!(mode, ScaleSharing::Free))
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
                    let inter_col_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_coordinated_spacing("inter_col_gap"))
                        .unwrap_or(0.0);
                    let total_gap = inter_col_gap * (num_cols.saturating_sub(1)) as f32;
                    let band_width = (plot_width - total_gap) / num_cols as f32;

                    // Build scales for subplot measurement using two-step pattern for data override
                    let builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, df)
                        .await?;
                    let subplot_scales = source
                        .subplot
                        .build_scales_from_builder(&builder, band_width, plot_height, ctx, params)
                        .await?;

                    // Measure each subplot with correct per-subplot FacetContext
                    let has_unified_y = self.unified_y_title.is_some();
                    let mut computed_overflow = Vec::new();

                    // Compute scale sharing from marks so subplots know which axes to measure
                    // Use nested facet version to correctly handle FacetRow inside FacetCol
                    let computed_scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

                    for (col_idx, (domain_val, has_data)) in iteration_domain.iter().enumerate() {
                        // Get the DataFrame for this cell - either filtered data or empty
                        let cell_df = if *has_data {
                            df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?
                        } else {
                            // Empty cell: create an empty DataFrame with the same schema
                            df.clone().limit(0, Some(0))?
                        };

                        // Create FacetContext with correct position for this subplot
                        // so that should_show_title/should_show_labels work correctly
                        let subplot_measure_params = {
                            use crate::facet::context::FacetContext;
                            let mut updated_params = params.clone();
                            let mut unified_channels = ColumnDimensionConfig::unified_channels();

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

                    // Aggregate computed overflow - first column for left, ALL columns for right
                    // (each column may have its own legend extending to the right)
                    if let Some(first) = computed_overflow.first() {
                        left_max = left_max.max(first.left);
                    }
                    for overflow_item in &computed_overflow {
                        right_max = right_max.max(overflow_item.right);
                        top_max = top_max.max(overflow_item.top);
                        bottom_max = bottom_max.max(overflow_item.bottom);
                    }
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide [source {}]: No facet expression, using fallback",
                            source_idx
                        );
                    }
                    return Ok((30.0, 30.0, 40.0, 20.0));
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
                    let inter_col_gap = coord_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.get_coordinated_spacing("inter_col_gap"))
                        .unwrap_or(0.0);
                    let total_gap = inter_col_gap * (num_cols.saturating_sub(1)) as f32;
                    let band_width = (plot_width - total_gap) / num_cols as f32;

                    // Build scales for subplot measurement using two-step pattern for data override
                    let builder = source
                        .subplot
                        .build_scale_builder_from_dataframe(ctx, params, &df)
                        .await?;
                    let subplot_scales = source
                        .subplot
                        .build_scales_from_builder(&builder, band_width, plot_height, ctx, params)
                        .await?;

                    // Measure each subplot with correct per-subplot FacetContext
                    let has_unified_y = self.unified_y_title.is_some();
                    let mut computed_overflow = Vec::new();
                    let num_cols = domain_vals.len();

                    // Compute scale sharing from marks so subplots know which axes to measure
                    // Use nested facet version to correctly handle FacetRow inside FacetCol
                    let scale_sharing =
                        compute_scale_sharing_for_nested_facet(&source.subplot.marks);

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
                            let mut unified_channels = ColumnDimensionConfig::unified_channels();

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

                            // Merge axis_scale_sharing from FacetCoordinationContext
                            // This contains the per-channel scale sharing modes (x, y) computed
                            // from channel configs, ensuring measurement uses same visibility
                            // decisions as rendering.
                            use crate::facet::coordination::FacetCoordinationContext;
                            if let Some(coord_ctx) = FacetCoordinationContext::from_params(params) {
                                if let Some(ref axis_sharing) = coord_ctx.axis_scale_sharing {
                                    for (channel, mode) in axis_sharing {
                                        merged_scale_sharing.insert(channel.clone(), *mode);
                                    }
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

                    // Aggregate computed overflow - first column for left, ALL columns for right
                    // (each column may have its own legend extending to the right)
                    if let Some(first) = computed_overflow.first() {
                        left_max = left_max.max(first.left);
                    }
                    for overflow_item in &computed_overflow {
                        right_max = right_max.max(overflow_item.right);
                        top_max = top_max.max(overflow_item.top);
                        bottom_max = bottom_max.max(overflow_item.bottom);
                    }
                } else {
                    // No facet expression available - use fallback
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "FacetColGuide [source {}]: No facet expression from source data, using fallback",
                            source_idx
                        );
                    }
                    return Ok((30.0, 30.0, 40.0, 20.0));
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
                return Ok((30.0, 30.0, 40.0, 20.0));
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
        let (mut top_max, mut bottom_max, mut left_max, mut right_max) = self
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
        // This ensures plot areas align even when scales are not shared
        {
            use crate::facet::coordination::FacetCoordinationContext;
            let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();
            if let Some(shared_left) = coord_ctx.get_coordinated_spacing("shared_overflow_left") {
                left_max = left_max.max(shared_left);
            }
            if let Some(shared_right) = coord_ctx.get_coordinated_spacing("shared_overflow_right") {
                right_max = right_max.max(shared_right);
            }
            if let Some(shared_top) = coord_ctx.get_coordinated_spacing("shared_overflow_top") {
                top_max = top_max.max(shared_top);
            }
            if let Some(shared_bottom) = coord_ctx.get_coordinated_spacing("shared_overflow_bottom") {
                bottom_max = bottom_max.max(shared_bottom);
            }
        }

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
            max_label_height + gap_title + title_height + 1.0  // Original - gap_title handles spacing
        } else {
            max_label_height + rule_tick_space + 1.0  // Add rule_tick_space when no title
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
        let visibility = FacetColVisibility::resolve(&visibility_input, params);

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
        use crate::facet::coordination::FacetCoordinationContext;
        use crate::facet::guide_measurement::{aggregate_overflow, calculate_inter_col_gap};
        use tracing::debug;

        /// Safety margin for gap calculation (accounts for potential measurement variance)
        const GAP_SAFETY_MARGIN: f32 = 1.0;

        // Guard: Invalid dimensions
        if plot_height <= 0.0 || plot_width <= 0.0 {
            return Ok(MeasurementResult::default());
        }

        // Guard: No facet sources
        if self.facet_sources.is_empty() {
            return Ok(MeasurementResult::default());
        }

        // Parse coordination context ONCE
        let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();

        // RECURSION GUARD: If already coordinated, use single pass (measure_overflow)
        if coord_ctx
            .get_coordinated_spacing(spacing_keys::INTER_COL_GAP)
            .is_some()
        {
            debug!(
                "FacetColGuide: Recursion guard - using pre-coordinated inter_col_gap"
            );
            let overflow = self
                .measure_overflow(
                    scales,
                    row_overflow,
                    _col_overflow,
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
        // This happens when FacetRow contains FacetCol or vice versa.
        if coord_ctx
            .get_coordinated_spacing(spacing_keys::NESTED_MEASUREMENT)
            .is_some()
        {
            debug!(
                "FacetColGuide: Nesting guard - NESTED_MEASUREMENT set, using single-pass"
            );
            let overflow = self
                .measure_overflow(
                    scales,
                    row_overflow,
                    _col_overflow,
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

        // Extract domain values
        let mut domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // For uniform Free scaling, use max cell count instead of actual domain length
        let num_cols = coord_ctx
            .get_uniform_cell_count()
            .unwrap_or(domain_vals.len())
            .max(1);

        // === PASS 1: Measure with zero gap ===
        let pass1_band_width = plot_width / num_cols as f32;

        debug!(
            "FacetColGuide: Pass 1 - measuring {} cols with zero gap, band_width={:.2}",
            num_cols, pass1_band_width
        );

        let mut col_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_cols);
        let mut aggregated_spacing: std::collections::HashMap<String, f32> =
            std::collections::HashMap::new();

        // Get the first facet source (primary source for measurement)
        let source = &self.facet_sources[0];

        // Get facet expression
        let facet_expr = source
            .data
            .channels()
            .get(ColumnDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(ctx));

        // Get the dataframe for measurement
        let df = data_override
            .cloned()
            .or_else(|| source.data.dataframe_with_context(ctx));

        if let (Some(expr), Some(df)) = (facet_expr, df) {
            use datafusion::logical_expr::lit;

            // Build scales for subplot measurement
            let builder = source
                .subplot
                .build_scale_builder_from_dataframe(ctx, params, &df)
                .await?;
            let subplot_scales = source
                .subplot
                .build_scales_from_builder(&builder, pass1_band_width, plot_height, ctx, params)
                .await?;

            // Convert to ConfiguredScale for measure_with_coordination trait method
            let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            // Compute scale sharing from marks
            let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

            // Measure each column in Pass 1
            for (col_idx, domain_val) in domain_vals.iter().enumerate() {
                use crate::facet::context::FacetContext;

                let filter_df = df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?;

                // Create FacetContext for this cell with NESTED_MEASUREMENT marker
                let measure_params = {
                    let mut updated_params = params.clone();
                    let facet_ctx = FacetContext {
                        position: (0, col_idx),
                        grid_dimensions: (1, num_cols),
                        unified_channels: ColumnDimensionConfig::unified_channels(),
                        scale_sharing: scale_sharing.clone(),
                    };
                    for (k, v) in facet_ctx.to_params() {
                        updated_params.insert(k, v);
                    }
                    // Add NESTED_MEASUREMENT marker to coordination context
                    // This tells child guides to skip 2-pass measurement
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
                let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    guide
                        .measure_with_coordination(
                            &subplot_scales_configured,
                            row_overflow,
                            None,
                            pass1_band_width,
                            plot_height,
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
                            pass1_band_width,
                            plot_height,
                            ctx,
                            &measure_params,
                            &subplot_scales,
                            Some(&filter_df),
                        )
                        .await?;
                    MeasurementResult::new(total_overflow)
                };

                col_overflows.push(child_result.overflow);

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
            let overflow = self
                .measure_overflow(
                    scales,
                    row_overflow,
                    _col_overflow,
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
        let computed_gap = calculate_inter_col_gap(&col_overflows, GAP_SAFETY_MARGIN);

        debug!(
            "FacetColGuide: Computed inter_col_gap={:.2} (with {:.1}px safety margin)",
            computed_gap, GAP_SAFETY_MARGIN
        );

        // === PASS 2: Re-measure with computed gap ===
        let total_gap = computed_gap * (num_cols.saturating_sub(1)) as f32;
        let pass2_band_width = (plot_width - total_gap) / num_cols as f32;

        debug!(
            "FacetColGuide: Pass 2 - re-measuring with gap={:.2}, band_width={:.2}",
            computed_gap, pass2_band_width
        );

        // Create params with computed gap for nested guides
        // IMPORTANT: Also include aggregated spacing from Pass 1 to enable recursion guards
        // for nested facets (e.g., FacetCol inside FacetRow should not re-compute its gap)
        let pass2_params = {
            let mut new_ctx = coord_ctx.clone();
            // Add our own computed gap
            new_ctx
                .coordinated_spacing
                .insert(spacing_keys::INTER_COL_GAP.to_string(), computed_gap);
            // Add all aggregated spacing from children (enables recursion guards for nested facets)
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

        let source = &self.facet_sources[0];
        let facet_expr = source
            .data
            .channels()
            .get(ColumnDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(ctx));

        let df = data_override
            .cloned()
            .or_else(|| source.data.dataframe_with_context(ctx));

        let mut final_overflows: Vec<OverflowSpaceRequirement> = Vec::with_capacity(num_cols);
        aggregated_spacing.clear(); // Reset for Pass 2 aggregation

        if let (Some(expr), Some(df)) = (facet_expr, df) {
            use datafusion::logical_expr::lit;

            // Rebuild scales with Pass 2 band width
            let builder = source
                .subplot
                .build_scale_builder_from_dataframe(ctx, &pass2_params, &df)
                .await?;
            let subplot_scales = source
                .subplot
                .build_scales_from_builder(&builder, pass2_band_width, plot_height, ctx, &pass2_params)
                .await?;

            // Convert to ConfiguredScale for measure_with_coordination trait method
            let subplot_scales_configured: HashMap<String, ConfiguredScale> = subplot_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            let scale_sharing = compute_scale_sharing_for_nested_facet(&source.subplot.marks);

            for (col_idx, domain_val) in domain_vals.iter().enumerate() {
                use crate::facet::context::FacetContext;

                let filter_df = df.clone().filter(expr.clone().eq(lit(domain_val.clone())))?;

                let measure_params = {
                    let mut updated_params = pass2_params.clone();
                    let facet_ctx = FacetContext {
                        position: (0, col_idx),
                        grid_dimensions: (1, num_cols),
                        unified_channels: ColumnDimensionConfig::unified_channels(),
                        scale_sharing: scale_sharing.clone(),
                    };
                    for (k, v) in facet_ctx.to_params() {
                        updated_params.insert(k, v);
                    }
                    updated_params
                };

                let child_result = if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    guide
                        .measure_with_coordination(
                            &subplot_scales_configured,
                            row_overflow,
                            None,
                            pass2_band_width,
                            plot_height,
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
                            pass2_band_width,
                            plot_height,
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
        Ok(MeasurementResult::new(final_overflow)
            .with_spacing(spacing_keys::INTER_COL_GAP, computed_gap)
            .merge_spacing_needs(aggregated_spacing))
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
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
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
                let padded_array =
                    StdArc::new(StringArray::from(padded_labels)) as datafusion::arrow::array::ArrayRef;
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
        let visibility = FacetColVisibility::resolve(&visibility_input, params);

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
                let unified_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                    .child("facet")
                    .child("title");
                let unified_font_px = theme.font_size(&unified_ctx).unwrap_or(12.0_f32);
                let unified_font_family_owned = theme
                    .font_family(&unified_ctx)
                    .or_else(|| theme.font_family(&root_ctx))
                    .unwrap_or_else(|| "sans-serif".to_string());
                let unified_font_family = unified_font_family_owned.as_str();

                // Use visibility struct for x-axis position (already computed)
                let x_axis_at_top = visibility.x_axis_at_top;

                // Configurable gap (matching measure_overflow and FacetRowGuide)
                let gap_axis = 6.0_f32;

                let y_unified = if x_axis_at_top {
                    // X-axis at top: unified title goes above plot, positioned above subplot guides
                    // subplot_max_top tells us where the x-axis labels end above the plot
                    plot_bounds.y - subplot_max_top - gap_axis
                } else {
                    // X-axis at bottom (default): unified title goes just below x-axis labels
                    // subplot_max_bottom tells us where the x-axis labels end
                    plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
                };

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "FacetColGuide RENDER unified_x_title='{}' at y={:.3} (plot_y={:.3} plot_height={:.3} subplot_max_bottom={:.3} gap_axis={:.3})",
                        unified_title, y_unified, plot_bounds.y, plot_height, subplot_max_bottom, gap_axis
                    );
                }

                let unified_mark = SceneTextMark {
                    text: unified_title.clone().into(),
                    x: (plot_bounds.x + plot_width / 2.0).into(),
                    y: y_unified.into(),
                    align: TextAlign::Center.into(),
                    baseline: if x_axis_at_top {
                        TextBaseline::Bottom
                    } else {
                        TextBaseline::Top
                    }
                    .into(),
                    angle: 0.0_f32.into(),
                    font: unified_font_family.to_string().into(),
                    font_size: unified_font_px.into(),
                    color: avenger_common::types::ColorOrGradient::Color(
                        theme
                            .text_color(&unified_ctx)
                            .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                    )
                    .into(),
                    zindex: Some(6),
                    ..Default::default()
                };
                marks.push(SceneMark::Text(StdArc::new(unified_mark)));
            }
        }

        // Render unified y-axis title if available (rotated 90 degrees, on the left side)
        if let Some(unified_y_title) = &self.unified_y_title {
            let unified_y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let unified_y_font_px = theme.font_size(&unified_y_ctx).unwrap_or(12.0_f32);
            let unified_y_font_family_owned = theme
                .font_family(&unified_y_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            // Use visibility struct for y-axis position (already computed)
            let y_axis_on_right = visibility.y_axis_on_right;

            // Gap between subplot axis labels and unified title
            let gap_y_axis = 6.0_f32;

            // Measure the title width (height when rotated) for proper centering
            let title_measurer = avenger_text::measurement::default_text_measurer();
            let title_cfg = avenger_text::measurement::TextMeasurementConfig {
                text: unified_y_title,
                font: unified_y_font_family_owned.as_str(),
                font_size: unified_y_font_px,
                font_weight: &avenger_text::types::FontWeight::Name(
                    avenger_text::types::FontWeightNameSpec::Normal,
                ),
                font_style: &avenger_text::types::FontStyle::Normal,
            };
            let title_bounds = title_measurer.measure_text_bounds(&title_cfg);
            let title_height = title_bounds.height; // becomes width when rotated

            let x_unified_y = if y_axis_on_right {
                // Y-axis on right: unified y title goes to the right of the subplot guides
                // For nested facets, _right already includes the inner guide's unified title space.
                // Position the title at the right edge of allocated space minus half title height
                // (centered within the title allocation at the right edge).
                plot_bounds.x + plot_width + _right - gap_y_axis - title_height / 2.0
            } else {
                // Y-axis on left (default): unified y title goes to the left of subplot guides
                // Position centered in the left overflow space (between edge and subplot axis labels)
                // The title space starts at (plot_bounds.x - _left) and goes for y_axis_title_space pixels
                plot_bounds.x - _left + gap_y_axis + title_height / 2.0
            };

            let y_center = plot_bounds.y + plot_height / 2.0;

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FacetColGuide RENDER unified_y_title='{}' at x={:.3} y={:.3} (y_axis_on_right={} _left={:.3} _right={:.3})",
                    unified_y_title, x_unified_y, y_center, y_axis_on_right, _left, _right
                );
            }

            let unified_y_mark = SceneTextMark {
                text: unified_y_title.clone().into(),
                x: x_unified_y.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Bottom.into(),
                angle: if y_axis_on_right {
                    90.0_f32.into()
                } else {
                    (-90.0_f32).into()
                },
                font: unified_y_font_family_owned.to_string().into(),
                font_size: unified_y_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(
                    theme
                        .text_color(&unified_y_ctx)
                        .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                )
                .into(),
                zindex: Some(6),
                ..Default::default()
            };
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
}

// ============================================================================
// GridFacetGuide - 2D Grid Faceting Guide
// ============================================================================

/// Guide for GridFacet coordinate system
///
/// Renders facet labels for both row (vertical) and column (horizontal) dimensions,
/// creating a 2D grid of subplots. Row labels appear on the left, column labels
/// appear above or below based on x-axis position.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GridFacetGuide {
    facet_sources: Vec<FacetSource>,

    // Row dimension
    pub row_title: Option<String>,
    unified_y_title: Option<String>,
    unifiable_row_channel: Option<String>,

    // Column dimension
    pub col_title: Option<String>,
    unified_x_title: Option<String>,
    unifiable_col_channel: Option<String>,
}

impl GridFacetGuide {}

impl CoordinateGuide for GridFacetGuide {
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
                .downcast_ref::<crate::facet::marks::facet::CompiledFacetGrid>()
            {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                    user_title: None, // Grid stores separate row/col titles
                    facet_scale_sharing: None, // Grid facets have implicit shared scales
                });

                // Use user-specified titles from facet if available
                if self.row_title.is_none() && facet.row_title.is_some() {
                    self.row_title = facet.row_title.clone();
                }
                if self.col_title.is_none() && facet.col_title.is_some() {
                    self.col_title = facet.col_title.clone();
                }
            }
        }

        // Derive row title from row channel (fallback if not explicitly set)
        if self.row_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src.data.channels().get("row") {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.row_title = Some(name);
                    }
                }
            }
        }

        // Derive col title from col channel (fallback if not explicitly set)
        if self.col_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                if let Some(cv) = src.data.channels().get("column") {
                    if let Some(name) = cv.as_column_name(_session_context) {
                        self.col_title = Some(name);
                    }
                }
            }
        }

        // Derive unified y-title from subplot guide (for row dimension)
        if self.unified_y_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                use crate::facet::dimension_config::RowDimensionConfig;
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        RowDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_row_channel = Some(info.channel);
                    self.unified_y_title = info.title;
                }
            }
        }

        // Derive unified x-title from subplot guide (for col dimension)
        if self.unified_x_title.is_none() {
            if let Some(src) = self.facet_sources.first() {
                use crate::facet::dimension_config::ColumnDimensionConfig;
                if let Some(info) = src.subplot.compiled_guide.as_ref().and_then(|g| {
                    g.facet_unifiable_channel(
                        ColumnDimensionConfig::facet_direction(),
                        src.subplot.marks(),
                        _session_context,
                    )
                }) {
                    self.unifiable_col_channel = Some(info.channel);
                    self.unified_x_title = info.title;
                }
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for GridFacetGuide {
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
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Get row and col scales (may not exist for degenerate single-value cases)
        let row_scale_opt = scales.get("row");
        let col_scale_opt = scales.get("column");

        // If either scale is missing, this is a degenerate case - return empty marks
        if row_scale_opt.is_none() || col_scale_opt.is_none() {
            return Ok(OverflowSpaceRequirement::default());
        }

        let row_scale = row_scale_opt.unwrap();
        let col_scale = col_scale_opt.unwrap();

        // Extract domain values
        let mut row_domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        let mut col_domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        // Sort to ensure deterministic facet ordering
        row_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        col_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Skip if single row or single col (degenerate grid)
        let skip_row_labels = row_domain_vals.len() <= 1;
        let skip_col_labels = col_domain_vals.len() <= 1;

        let mut max_top: f32 = 0.0;
        let mut max_bottom: f32 = 0.0;
        let mut max_left: f32 = 0.0;
        let mut max_right: f32 = 0.0;

        // Measure subplot overflows using GridSubplotIterator
        for source in &self.facet_sources {
            let row_expr = source
                .data
                .channels()
                .get("row")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'row' channel not found in guide".into(),
                    )
                })?;
            let col_expr = source
                .data
                .channels()
                .get("column")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet 'col' channel not found in guide".into(),
                    )
                })?;

            // Use data_override if provided (for nested facets), otherwise use compiled data
            let df = if let Some(override_df) = data_override {
                override_df.clone()
            } else {
                source.data.dataframe_with_context(ctx).ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet guide could not access data".into(),
                    )
                })?
            };

            // Compute scale sharing for ALL channels used by marks (not just coord channels)
            // This ensures non-coordinate channels like fill, color, size also get per-subplot scales
            let mut scale_sharing_by_channel = std::collections::HashMap::new();

            // Collect all unique channel names from all marks
            let mut all_channels = std::collections::HashSet::new();
            for m in &source.subplot.marks {
                for ch in m.data_context().channels().keys() {
                    all_channels.insert(ch.as_str());
                }
            }

            // Determine sharing mode for each channel
            for ch in all_channels {
                let mut mode = ScaleSharing::Free;
                for m in &source.subplot.marks {
                    if let Some(cv) = m.data_context().channels().get(ch) {
                        if let Some(share_mode) = cv.get_share_mode() {
                            mode = match (mode, share_mode) {
                                (ScaleSharing::Free, new_mode) => new_mode,
                                (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                (existing, _) => existing,
                            };
                        }
                    }
                }
                scale_sharing_by_channel.insert(ch.to_string(), mode);
            }

            use crate::facet::dimension_config::{ColumnDimensionConfig, RowDimensionConfig};
            use crate::facet::subplot_iterator::SubplotIterator;

            let num_rows = row_domain_vals.len();
            let num_cols = col_domain_vals.len();
            let band_w = plot_width / num_cols.max(1) as f32;
            let band_h = plot_height / num_rows.max(1) as f32;

            // Build base scales from full DataFrame (used as fallback for empty cells)
            let base_scales = source
                .subplot
                .build_scales_for_dataframe(&df, band_w, band_h, ctx, params)
                .await?;

            // Use nested SubplotIterators to iterate over grid cells
            let row_iter = SubplotIterator::<RowDimensionConfig>::new(
                row_domain_vals.clone(),
                params.clone(),
                scale_sharing_by_channel.clone(),
            );

            for row_iteration in row_iter {
                let col_iter = SubplotIterator::<ColumnDimensionConfig>::new(
                    col_domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );

                for col_iteration in col_iter {
                    // Merge row and col contexts into unified GridFacet context
                    let merged_params = crate::facet::marks::facet::merge_grid_facet_contexts(
                        &row_iteration,
                        &col_iteration,
                        num_rows,
                        num_cols,
                    );

                    // Filter to rows matching both row AND col values
                    let filter_df = df
                        .clone()
                        .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
                        .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

                    // Check if this cell has any data
                    let cell_count = filter_df.clone().count().await?;
                    let cell_is_empty = cell_count == 0;

                    // Build scales for this subplot
                    let mut inner_scales = base_scales.clone();

                    // For independent (free) channels, rebuild from cell data if cell is non-empty
                    if !cell_is_empty {
                        for (ch, sharing_mode) in &scale_sharing_by_channel {
                            if *sharing_mode != ScaleSharing::Shared {
                                // This channel is independent - rebuild from filtered data
                                let facet_scales = source
                                    .subplot
                                    .build_scales_for_dataframe(
                                        &filter_df,
                                        band_w,
                                        band_h,
                                        ctx,
                                        &merged_params,
                                    )
                                    .await?;

                                if let Some(s) = facet_scales.get(ch) {
                                    inner_scales.insert(ch.clone(), s.clone());
                                }
                            }
                        }
                    }

                    // Use build_plot_components to get accurate overflow including legends
                    use crate::plot::compiled::scale_provider::PrebuiltScaleProvider;

                    let provider = PrebuiltScaleProvider {
                        scales: inner_scales.clone(),
                    };

                    let components = source
                        .subplot
                        .build_plot_components(
                            band_w,
                            band_h,
                            ctx,
                            &merged_params,
                            &provider,
                            crate::plot::compiled::EvaluationMode::Measure,
                            Some(&filter_df),
                            true, // dimensions_are_plot_area
                        )
                        .await?;

                    let overflow = components.overflow.unwrap_or_default();

                    // Track top/bottom overflow
                    // For top: use first row (where x-axis typically is)
                    // For bottom: use MAX across ALL rows when labels will be at bottom,
                    //             or last row when labels will be at top
                    if row_iteration.index == 0 {
                        max_top = max_top.max(overflow.top);
                    }
                    // When column labels will be at bottom (x-axis at top), we need to check
                    // bottom overflow across all rows since all subplots may have bottom legends
                    // We don't know place_col_below yet, so conservatively take max across all rows
                    max_bottom = max_bottom.max(overflow.bottom);

                    // Track left/right overflow across ALL columns
                    // (legends can appear in any subplot and vary in size)
                    max_left = max_left.max(overflow.left);
                    max_right = max_right.max(overflow.right);
                }
            }
        }

        // Measure row labels (if not degenerate)
        let row_label_space = if !skip_row_labels {
            let row_labels = row_scale.domain_labels()?;
            let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_font_family = theme
                .font_family(&guide_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_font_family = theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            use crate::facet::guide_utils::{
                FacetLabelMeasurementConfig, measure_facet_label_slab,
            };
            let measurement_config = FacetLabelMeasurementConfig {
                labels: row_labels,
                is_rotated: true, // Row labels are vertical
                font_family: label_font_family,
                font_size_px: label_font_px,
                title: self.row_title.clone(),
                title_font_family,
                title_font_size_px: title_font_px,
                render_title: true, // GridFacetGuide is top-level, always renders title
            };
            measure_facet_label_slab(&measurement_config)
        } else {
            0.0
        };

        // Measure col labels (if not degenerate)
        let col_label_space = if !skip_col_labels {
            let col_labels = col_scale.domain_labels()?;
            let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("label");
            let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
            let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
            let label_font_family = theme
                .font_family(&guide_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
            let title_font_family = theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            use crate::facet::guide_utils::{
                FacetLabelMeasurementConfig, measure_facet_label_slab,
            };
            let measurement_config = FacetLabelMeasurementConfig {
                labels: col_labels,
                is_rotated: false, // Col labels are horizontal
                font_family: label_font_family,
                font_size_px: label_font_px,
                title: self.col_title.clone(),
                title_font_family,
                title_font_size_px: title_font_px,
                render_title: true, // GridFacetGuide is top-level, always renders title
            };
            measure_facet_label_slab(&measurement_config)
        } else {
            0.0
        };

        // Measure unified y-title (rotated, on right side)
        let unified_y_height = if let Some(y_title) = &self.unified_y_title {
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
        };

        // Measure unified x-title (horizontal, position based on x-axis)
        let unified_x_height = if let Some(x_title) = &self.unified_x_title {
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
        };

        // Determine if col labels go below (when x-axis at top)
        let place_col_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,
                    // Fallback: infer from overflow (larger top overflow suggests x-axis at bottom)
                    None => max_top > max_bottom,
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Determine row label placement (opposite side of y-axis, like FacetRow)
        // Check actual y-axis position first, only fall back to overflow inference if None
        let place_row_on_left = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("y") {
                    Some(crate::cartesian::axis::AxisPosition::Right) => true, // y-axis right -> labels left
                    Some(crate::cartesian::axis::AxisPosition::Left) => false, // y-axis left -> labels right
                    None => max_right > max_left, // Fallback: infer from overflow
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Allocate overflow space
        let gap_axis = 6.0_f32;

        // Left/right overflow depends on row label placement
        let (left_final, right_final) = if place_row_on_left {
            // Row labels on left, y-axis on right
            (
                row_label_space + max_left,
                max_right
                    + if unified_y_height > 0.0 {
                        gap_axis + unified_y_height
                    } else {
                        0.0
                    },
            )
        } else {
            // Row labels on right, y-axis on left
            (
                max_left
                    + if unified_y_height > 0.0 {
                        gap_axis + unified_y_height
                    } else {
                        0.0
                    },
                row_label_space + max_right,
            )
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FACET_COL final overflow: left_final={:.3} right_final={:.3} (max_left_child={:.3} max_right_child={:.3} place_row_on_left={} unified_y_title_present={})",
                left_final,
                right_final,
                max_left,
                max_right,
                place_row_on_left,
                self.unified_y_title.is_some()
            );
        }

        // Top/bottom overflow depends on col label placement
        let (top_final, bottom_final) = if place_col_below {
            (
                // Top: subplot top overflow + optional unified x-title
                max_top
                    + if unified_x_height > 0.0 {
                        gap_axis + unified_x_height
                    } else {
                        0.0
                    },
                // Bottom: col labels + subplot bottom overflow
                col_label_space + max_bottom,
            )
        } else {
            (
                // Top: col labels + subplot top overflow
                col_label_space + max_top,
                // Bottom: subplot bottom overflow + optional unified x-title
                max_bottom
                    + if unified_x_height > 0.0 {
                        gap_axis + unified_x_height
                    } else {
                        0.0
                    },
            )
        };

        Ok(OverflowSpaceRequirement {
            top: top_final,
            bottom: bottom_final,
            left: left_final,
            right: right_final,
        })
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};
        use std::sync::Arc as StdArc;

        let mut marks: Vec<SceneMark> = Vec::new();

        // Get row and col scales (may not exist for degenerate single-value cases)
        let row_scale = match scales.get("row") {
            Some(s) => s,
            None => return Ok(marks), // Degenerate case: no row scale
        };
        let col_scale = match scales.get("column") {
            Some(s) => s,
            None => return Ok(marks), // Degenerate case: no col scale
        };

        // Extract domain values and labels
        let mut row_domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        let mut col_domain_vals = match col_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };
        // Sort to ensure deterministic facet ordering
        row_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        col_domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let row_labels = row_scale.domain_labels()?;
        let col_labels = col_scale.domain_labels()?;

        // Skip if single row or single col (degenerate grid)
        let skip_row_labels = row_domain_vals.len() <= 1;
        let skip_col_labels = col_domain_vals.len() <= 1;

        // Get band positions for both dimensions
        // These scales have the correct spacing from the facet mark (via scale updates).
        // We'll use .center() on each BandPosition for label/tick positioning.
        use crate::facet::band_positions::BandPositionIterator;
        let row_band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(row_scale)?.collect();
        let col_band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(col_scale)?.collect();

        // Debug: log scale options and computed band centers used for labels
        if cfg!(debug_assertions) || std::env::var("RUST_LOG").is_ok() {
            use avenger_scales::scales::band;
            let row_bw = band::bandwidth(&row_scale.config)?;
            let col_bw = band::bandwidth(&col_scale.config)?;
            let row_pad_inner_px = row_scale.config.option_f32("padding_inner_px", 0.0);
            let col_pad_inner_px = col_scale.config.option_f32("padding_inner_px", 0.0);
            tracing::debug!(
                row_bw = row_bw,
                col_bw = col_bw,
                row_pad_inner_px = row_pad_inner_px,
                col_pad_inner_px = col_pad_inner_px,
                "GridFacetGuide: bandwidth and padding_inner_px"
            );
            for (i, bp) in col_band_positions.iter().enumerate() {
                let label = col_labels.get(i).cloned().unwrap_or_default();
                tracing::debug!(
                    i = i,
                    ?label,
                    center = bp.center(),
                    bw = col_bw,
                    "GridFacetGuide: col label center position"
                );
            }
            for (i, bp) in row_band_positions.iter().enumerate() {
                let label = row_labels.get(i).cloned().unwrap_or_default();
                tracing::debug!(
                    i = i,
                    ?label,
                    center = bp.center(),
                    bw = row_bw,
                    "GridFacetGuide: row label center position"
                );
            }
        }

        // Measure subplot overflows to determine row and col label placement
        // Use pre-computed overflow from row_overflow/col_overflow if available (includes legends),
        // otherwise fall back to recomputing (which won't include legend dimensions).
        let mut subplot_max_top: f32 = 0.0;
        let mut subplot_max_bottom: f32 = 0.0;
        let mut subplot_max_left: f32 = 0.0;
        let mut subplot_max_right: f32 = 0.0;

        // Extract overflow from pre-computed row_overflow and col_overflow
        // row_overflow contains one entry per row, col_overflow contains one entry per column
        // For facet labels, we need the max overflow across all rows/cols
        let has_precomputed_overflow = row_overflow.is_some() && col_overflow.is_some();
        if let Some(row_of) = row_overflow {
            for r in row_of {
                subplot_max_top = subplot_max_top.max(r.top);
                subplot_max_bottom = subplot_max_bottom.max(r.bottom);
                // For right side (last column), use rightmost overflow
                // We track max since all subplots may have legends
                subplot_max_right = subplot_max_right.max(r.right);
            }
            // Left overflow: use first row's left
            if let Some(first) = row_of.first() {
                subplot_max_left = subplot_max_left.max(first.left);
            }
        }
        if let Some(col_of) = col_overflow {
            for c in col_of {
                subplot_max_top = subplot_max_top.max(c.top);
                subplot_max_bottom = subplot_max_bottom.max(c.bottom);
                subplot_max_left = subplot_max_left.max(c.left);
                subplot_max_right = subplot_max_right.max(c.right);
            }
        }

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "GridFacetGuide evaluate: has_precomputed_overflow={} subplot_max(T={:.3} B={:.3} L={:.3} R={:.3})",
                has_precomputed_overflow,
                subplot_max_top,
                subplot_max_bottom,
                subplot_max_left,
                subplot_max_right
            );
        }

        // If no pre-computed overflow, fall back to measuring (won't include legend dimensions)
        if !has_precomputed_overflow {
            if let Some(source) = self.facet_sources.first() {
                let row_expr = source
                    .data
                    .channels()
                    .get("row")
                    .and_then(|cv| cv.expr(ctx))
                    .ok_or_else(|| {
                        crate::error::AvengerChartError::InternalError(
                            "Facet 'row' channel not found".into(),
                        )
                    })?;
                let col_expr = source
                    .data
                    .channels()
                    .get("column")
                    .and_then(|cv| cv.expr(ctx))
                    .ok_or_else(|| {
                        crate::error::AvengerChartError::InternalError(
                            "Facet 'col' channel not found".into(),
                        )
                    })?;

                let df = source.data.dataframe_with_context(ctx).ok_or_else(|| {
                    crate::error::AvengerChartError::InternalError(
                        "Facet guide could not access data".into(),
                    )
                })?;

                // Compute scale sharing for ALL channels used by marks (not just coord channels)
                // This ensures non-coordinate channels like fill, color, size also get per-subplot scales
                let mut scale_sharing_by_channel = std::collections::HashMap::new();

                // Collect all unique channel names from all marks
                let mut all_channels = std::collections::HashSet::new();
                for m in &source.subplot.marks {
                    for ch in m.data_context().channels().keys() {
                        all_channels.insert(ch.as_str());
                    }
                }

                // Determine sharing mode for each channel
                for ch in all_channels {
                    let mut mode = ScaleSharing::Free;
                    for m in &source.subplot.marks {
                        if let Some(cv) = m.data_context().channels().get(ch) {
                            if let Some(share_mode) = cv.get_share_mode() {
                                mode = match (mode, share_mode) {
                                    (ScaleSharing::Free, new_mode) => new_mode,
                                    (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                                    (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                                    (existing, _) => existing,
                                };
                            }
                        }
                    }
                    scale_sharing_by_channel.insert(ch.to_string(), mode);
                }

                use crate::facet::dimension_config::{ColumnDimensionConfig, RowDimensionConfig};
                use crate::facet::subplot_iterator::SubplotIterator;

                let num_rows = row_domain_vals.len();
                let num_cols = col_domain_vals.len();
                let band_w = plot_width / num_cols.max(1) as f32;
                let band_h = plot_height / num_rows.max(1) as f32;

                // Build base scales from full DataFrame (used as fallback for empty cells)
                let base_scales = source
                    .subplot
                    .build_scales_for_dataframe(&df, band_w, band_h, ctx, params)
                    .await?;

                // Use nested SubplotIterators to iterate over grid cells
                let row_iter = SubplotIterator::<RowDimensionConfig>::new(
                    row_domain_vals.clone(),
                    params.clone(),
                    scale_sharing_by_channel.clone(),
                );

                for row_iteration in row_iter {
                    let col_iter = SubplotIterator::<ColumnDimensionConfig>::new(
                        col_domain_vals.clone(),
                        params.clone(),
                        scale_sharing_by_channel.clone(),
                    );

                    for col_iteration in col_iter {
                        // Merge row and col contexts into unified GridFacet context
                        let merged_params = crate::facet::marks::facet::merge_grid_facet_contexts(
                            &row_iteration,
                            &col_iteration,
                            num_rows,
                            num_cols,
                        );

                        let filter_df = df
                            .clone()
                            .filter(row_expr.clone().eq(datafusion::logical_expr::lit(
                                row_iteration.facet_value.clone(),
                            )))?
                            .filter(col_expr.clone().eq(datafusion::logical_expr::lit(
                                col_iteration.facet_value.clone(),
                            )))?;

                        // Check if this cell has any data
                        let cell_count = filter_df.clone().count().await?;
                        let cell_is_empty = cell_count == 0;

                        // Build scales for this subplot
                        let mut inner_scales = base_scales.clone();

                        // For independent (free) channels, rebuild from cell data if cell is non-empty
                        if !cell_is_empty {
                            for (ch, sharing_mode) in &scale_sharing_by_channel {
                                if *sharing_mode != ScaleSharing::Shared {
                                    // This channel is independent - rebuild from filtered data
                                    let facet_scales = source
                                        .subplot
                                        .build_scales_for_dataframe(
                                            &filter_df,
                                            band_w,
                                            band_h,
                                            ctx,
                                            &merged_params,
                                        )
                                        .await?;

                                    if let Some(s) = facet_scales.get(ch) {
                                        inner_scales.insert(ch.clone(), s.clone());
                                    }
                                }
                            }
                        }

                        // Use build_plot_components to get accurate overflow including legends
                        use crate::plot::compiled::scale_provider::PrebuiltScaleProvider;

                        let provider = PrebuiltScaleProvider {
                            scales: inner_scales.clone(),
                        };

                        let components = source
                            .subplot
                            .build_plot_components(
                                band_w,
                                band_h,
                                ctx,
                                &merged_params,
                                &provider,
                                crate::plot::compiled::EvaluationMode::Measure,
                                Some(&filter_df),
                                true, // dimensions_are_plot_area
                            )
                            .await?;

                        let overflow = components.overflow.unwrap_or_default();

                        subplot_max_top = subplot_max_top.max(overflow.top);
                        subplot_max_bottom = subplot_max_bottom.max(overflow.bottom);
                        // Track left/right overflow across ALL columns (legends can vary)
                        subplot_max_left = subplot_max_left.max(overflow.left);
                        subplot_max_right = subplot_max_right.max(overflow.right);
                    }
                }
            }
        } // end if !has_precomputed_overflow

        // Determine row label placement (opposite side of y-axis, like FacetRow)
        // Check actual y-axis position first, only fall back to overflow inference if None
        let place_row_on_left = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("y") {
                    Some(crate::cartesian::axis::AxisPosition::Right) => true, // y-axis right -> labels left
                    Some(crate::cartesian::axis::AxisPosition::Left) => false, // y-axis left -> labels right
                    None => subplot_max_right > subplot_max_left, // Fallback: infer from overflow
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Determine col label placement
        let place_col_below = if let Some(source) = self.facet_sources.first() {
            if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                match guide.axis_position("x") {
                    Some(crate::cartesian::axis::AxisPosition::Top) => true,
                    Some(crate::cartesian::axis::AxisPosition::Bottom) => false,
                    None => subplot_max_top > subplot_max_bottom,
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        };

        // Theme contexts
        let guide_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("label");
        let label_font_px = theme.font_size(&guide_ctx).unwrap_or(12.0_f32);
        let root_ctx = crate::theme::ThemeContext::new(":root", params.clone());
        let label_font_family = theme
            .font_family(&guide_ctx)
            .or_else(|| theme.font_family(&root_ctx))
            .unwrap_or_else(|| "sans-serif".to_string());

        let title_ctx = crate::theme::ThemeContext::new("guide", params.clone())
            .child("facet")
            .child("title");
        let title_font_px = theme.font_size(&title_ctx).unwrap_or(12.0_f32);
        let title_font_family = theme
            .font_family(&title_ctx)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Render row labels (on opposite side of y-axis)
        if !skip_row_labels {
            use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

            // Adjust plot bounds to account for subplot overflow when positioning facet labels
            // When labels are on right, extend width by right overflow so labels appear after it
            // When labels are on left, shift x by left overflow so labels appear before it
            let render_plot_bounds = if place_row_on_left {
                LayoutBounds {
                    x: plot_bounds.x - subplot_max_left,
                    y: plot_bounds.y,
                    width: plot_width + subplot_max_left,
                    height: plot_height,
                }
            } else {
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_width + subplot_max_right,
                    height: plot_height,
                }
            };

            let row_config = FacetLabelRenderConfig {
                labels: row_labels.clone(),
                band_positions: row_band_positions.clone(),
                plot_bounds: render_plot_bounds,
                is_rotated: true,                 // Row labels are vertical
                place_at_end: !place_row_on_left, // Opposite side of y-axis
                font_family: label_font_family.clone(),
                font_size_px: label_font_px,
                title: self.row_title.clone(),
                title_font_family: title_font_family.clone(),
                title_font_size_px: title_font_px,
                render_title: true, // GridFacetGuide is top-level, always renders title
            };
            marks.extend(render_facet_label_slab(&row_config, theme, params));
        }

        // Render col labels (top or bottom, unless degenerate)
        if !skip_col_labels {
            use crate::facet::guide_utils::{FacetLabelRenderConfig, render_facet_label_slab};

            // Adjust plot bounds for rendering facet labels based on their position
            // Facet labels should appear AFTER subplot overflow (axes/legends)
            let render_plot_bounds = if place_col_below {
                // Labels at bottom: extend height to include subplot bottom overflow (legends/axes)
                // so facet labels render below them
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_width,
                    height: plot_height + subplot_max_bottom,
                }
            } else {
                // Labels at top: shift up by subplot top overflow
                LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y - subplot_max_top,
                    width: plot_width,
                    height: plot_height + subplot_max_top,
                }
            };

            let col_config = FacetLabelRenderConfig {
                labels: col_labels.clone(),
                band_positions: col_band_positions.clone(),
                plot_bounds: render_plot_bounds,
                is_rotated: false, // Col labels are horizontal
                place_at_end: place_col_below,
                font_family: label_font_family.clone(),
                font_size_px: label_font_px,
                title: self.col_title.clone(),
                title_font_family: title_font_family.clone(),
                title_font_size_px: title_font_px,
                render_title: true, // GridFacetGuide is top-level, always renders title
            };
            marks.extend(render_facet_label_slab(&col_config, theme, params));
        }

        // Render unified y-axis title (rotated, on same side as y-axis, opposite of row labels)
        if let Some(y_title) = &self.unified_y_title {
            let y_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let y_font_px = theme.font_size(&y_ctx).unwrap_or(12.0_f32);
            let y_color = theme.text_color(&y_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let y_family_owned = theme
                .font_family(&y_ctx)
                .unwrap_or_else(|| "sans-serif".to_string());

            // Check actual y-axis position from subplot guide (not inferred from label position)
            let axis_on_right = if let Some(source) = self.facet_sources.first() {
                if let Some(guide) = source.subplot.compiled_guide.as_ref() {
                    match guide.axis_position("y") {
                        Some(crate::cartesian::axis::AxisPosition::Right) => true,
                        Some(crate::cartesian::axis::AxisPosition::Left) => false,
                        // Default to using label-inferred position when not specified
                        None | _ => place_row_on_left,
                    }
                } else {
                    place_row_on_left
                }
            } else {
                place_row_on_left
            };
            let gap = 6.0_f32;

            let x_bottom = if axis_on_right {
                plot_bounds.x + plot_width + subplot_max_right + gap
            } else {
                plot_bounds.x - subplot_max_left - gap
            };
            let y_center = plot_bounds.y + 0.5 * plot_height;

            let y_mark = SceneTextMark {
                text: y_title.clone().into(),
                x: x_bottom.into(),
                y: y_center.into(),
                align: TextAlign::Center.into(),
                baseline: TextBaseline::Bottom.into(),
                font: y_family_owned.clone().into(),
                font_size: y_font_px.into(),
                angle: if axis_on_right {
                    90.0_f32.into()
                } else {
                    (-90.0_f32).into()
                },
                color: avenger_common::types::ColorOrGradient::Color(y_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(y_mark)));
        }

        // Render unified x-axis title (horizontal, position based on x-axis)
        if let Some(x_title) = &self.unified_x_title {
            let x_ctx = crate::theme::ThemeContext::new("guide", params.clone())
                .child("facet")
                .child("title");
            let x_font_px = theme.font_size(&x_ctx).unwrap_or(12.0_f32);
            let x_color = theme.text_color(&x_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let x_family_owned = theme
                .font_family(&x_ctx)
                .or_else(|| theme.font_family(&root_ctx))
                .unwrap_or_else(|| "sans-serif".to_string());

            let gap_axis = 6.0_f32;
            let x_axis_at_top = place_col_below;

            let y_unified = if x_axis_at_top {
                plot_bounds.y - subplot_max_top - gap_axis
            } else {
                plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
            };

            let unified_mark = SceneTextMark {
                text: x_title.clone().into(),
                x: (plot_bounds.x + plot_width / 2.0).into(),
                y: y_unified.into(),
                align: TextAlign::Center.into(),
                baseline: if x_axis_at_top {
                    TextBaseline::Bottom
                } else {
                    TextBaseline::Top
                }
                .into(),
                angle: 0.0_f32.into(),
                font: x_family_owned.to_string().into(),
                font_size: x_font_px.into(),
                color: avenger_common::types::ColorOrGradient::Color(x_color).into(),
                zindex: Some(6),
                ..Default::default()
            };
            marks.push(SceneMark::Text(StdArc::new(unified_mark)));
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
}
