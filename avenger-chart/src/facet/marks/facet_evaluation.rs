/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module provides a parameterized two-pass rendering algorithm that works for
/// both row and column faceting by accepting orientation-specific closures.
use crate::channel::config_traits::ScaleSharing;
use crate::coords::SubplotGeometry;
use crate::coords::SubplotRect;
use crate::error::AvengerChartError;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::facet::scale_helpers::build_scales_helper;
use crate::facet::subplot_iterator::SubplotIteration;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use avenger_scales::scalar::Scalar;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::lit;
use datafusion::prelude::SessionContext;
use futures::{StreamExt, stream};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Convert RecordBatch to DataFrame using DataFusion's read_batch method
///
/// This creates a DataFrame from a RecordBatch using DataFusion's built-in `read_batch()`
/// method, which internally creates an unnamed MemTable (with table name "?table?").
/// This avoids manual table registration and naming collisions.
///
/// # Arguments
/// * `batch` - The RecordBatch to convert
/// * `ctx` - The SessionContext for DataFrame operations
///
/// # Returns
/// A DataFrame backed by an in-memory table containing the batch data
pub fn batch_to_dataframe(
    batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    // Use DataFusion's built-in read_batch which creates an unnamed table
    ctx.read_batch(batch.clone()).map_err(|e| {
        AvengerChartError::InternalError(format!(
            "Failed to create DataFrame from RecordBatch: {}",
            e
        ))
    })
}

/// Default spacing between facets in pixels when not specified by theme or configuration
const DEFAULT_FACET_SPACING: f32 = 3.0;

/// Output of the facet measurement pass (Pass 1)
struct FacetPass1Result {
    overflow_measurements: Vec<crate::guide::OverflowSpaceRequirement>,
    final_dimension_scale: ConfiguredScaleWithSpec,
    final_shared_scales: Option<HashMap<String, ConfiguredScaleWithSpec>>,
    final_rects: Vec<SubplotRect>,
    /// Aggregated legend positions across all subplots for cross-subplot alignment
    legend_alignment: crate::layout::LegendAlignmentInfo,
}

/// Measure facet layout and overflow (Pass 1) while keeping the outer future small
#[allow(clippy::too_many_arguments)]
async fn measure_pass<DimConfig: FacetDimensionConfig, SubplotDimsFn>(
    facet_coord: &dyn crate::coords::CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,
    dimension_scale: &ConfiguredScaleWithSpec,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
    df: &DataFrame,
    facet_expr: &datafusion::logical_expr::Expr,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    subplot_dims: &SubplotDimsFn,
) -> Result<FacetPass1Result, AvengerChartError>
where
    SubplotDimsFn: Fn(f32, &RenderContext) -> (f32, f32) + Clone,
{
    // Extract the band size from the configured dimension scale
    use crate::scales::extensions::ConfiguredScaleLegendExt;
    use avenger_scales::scales::band;
    let configured = dimension_scale.configured();
    let initial_bandwidth = band::bandwidth(&configured.config)?;

    // Determine shared-scale usage
    let any_shared = scale_sharing_by_channel
        .values()
        .any(|v| *v == ScaleSharing::Shared);
    // Build shared scale builder once if needed
    let shared_scale_builder = if any_shared {
        Some(
            compiled_subplot
                .build_scale_builder_from_dataframe(&context.session_context, &context.params, df)
                .await?,
        )
    } else {
        None
    };

    // Build initial shared scales for Pass 1 using approximate band size
    let initial_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let (width, height) = subplot_dims(initial_bandwidth, context);
        Some(
            compiled_subplot
                .build_scales_from_builder(
                    builder,
                    width,
                    height,
                    &context.session_context,
                    &context.params,
                )
                .await?,
        )
    } else {
        None
    };

    // ========== PASS 1: MEASUREMENT PHASE ==========
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!("Pass 1 starting for channel={}", DimConfig::channel_name());
    }
    use crate::facet::keys::FacetKeyExtractor;
    let mut domain_vals = FacetKeyExtractor::extract_keys(df, facet_expr).await?;
    domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let initial_positions = configured.scale_scalars_to_numeric(&domain_vals)?;

    use crate::facet::subplot_iterator::SubplotIteration;
    use crate::facet::subplot_iterator::SubplotIterator;
    let subplot_iter = SubplotIterator::<DimConfig>::new(
        domain_vals.clone(),
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    let channel_name = DimConfig::channel_name();
    let mut position_channels_pass1 = HashMap::new();
    position_channels_pass1.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(initial_positions.clone()),
    );

    let mut position_values_pass1 = HashMap::new();
    position_values_pass1.insert(channel_name, domain_vals.clone());

    let initial_geometry = facet_coord.transform(
        &position_channels_pass1,
        Some(&position_values_pass1),
        context.plot_width,
        context.plot_height,
    )?;

    let initial_rects = initial_geometry
        .as_any()
        .downcast_ref::<SubplotGeometry>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected SubplotGeometry from facet coord transform".into(),
            )
        })?
        .rects
        .clone();

    assert_eq!(
        subplot_iter.len(),
        initial_rects.len(),
        "SubplotIterator and initial_rects length mismatch in Pass 1"
    );

    let mut overflow_measurements = Vec::new();
    let mut legend_infos = Vec::new();

    // Small helper to measure one subplot to keep the parent future small
    async fn measure_subplot(
        compiled_subplot: &Arc<CompiledPlot>,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        filter_df: &DataFrame,
    ) -> Result<
        (
            crate::guide::OverflowSpaceRequirement,
            crate::layout::LegendLayoutInfo,
        ),
        AvengerChartError,
    > {
        compiled_subplot
            .measure_with_scales(width, height, ctx, params, scales, Some(filter_df))
            .await
    }

    // Bound concurrency to avoid overwhelming DataFusion while still flattening stack growth.
    const MAX_CONCURRENT_MEASURE: usize = 4;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MEASURE));

    // Pre-materialize subplot work items to decouple from iterator lifetimes.
    let work_items: Vec<(usize, SubplotIteration, SubplotRect)> = subplot_iter
        .zip(initial_rects.into_iter())
        .enumerate()
        .map(|(idx, (iter, rect))| (idx, iter, rect))
        .collect();

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let subplot_dims = subplot_dims.clone();
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            let params_base = iteration.params.clone();
            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let initial_shared_scales = initial_shared_scales.clone();
            let df = df.clone();
            let semaphore = Arc::clone(&semaphore);

            async move {
                let _permit = semaphore.acquire().await.unwrap();

                let band_size = if DimConfig::is_row_facet() {
                    rect.height
                } else {
                    rect.width
                };
                let (width, height) = subplot_dims(band_size, context);

                let filter_df: DataFrame = df
                    .clone()
                    .filter(facet_expr.eq(lit(iteration.facet_value.clone())))?;

                let free_scale_builder_pass1 = if scale_sharing_by_channel
                    .values()
                    .any(|v| *v == ScaleSharing::Shared)
                {
                    None
                } else {
                    Some(
                        compiled_subplot
                            .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
                            .await?,
                    )
                };

                let scales = build_scales_helper(
                    &compiled_subplot,
                    &initial_shared_scales,
                    &free_scale_builder_pass1,
                    &filter_df,
                    width,
                    height,
                    &ctx,
                    &params_base,
                )
                .await?;

                let (overflow, legend_info) = measure_subplot(
                    &compiled_subplot,
                    width,
                    height,
                    &ctx,
                    &params_base,
                    &scales,
                    &filter_df,
                )
                .await?;

                Ok::<_, AvengerChartError>((idx, overflow, legend_info))
            }
        })
        .buffer_unordered(MAX_CONCURRENT_MEASURE)
        .collect::<Vec<_>>()
        .await;

    // Collect results in index order for determinism.
    let mut sorted = results
        .into_iter()
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    sorted.sort_by_key(|(idx, _, _)| *idx);
    for (_, overflow, legend_info) in sorted {
        overflow_measurements.push(overflow);
        legend_infos.push(legend_info);
    }

    let mut max_required_gap = 0.0f32;
    for i in 0..overflow_measurements.len().saturating_sub(1) {
        let gap = DimConfig::calculate_adjacent_overflow(
            &overflow_measurements[i],
            &overflow_measurements[i + 1],
        );
        max_required_gap = max_required_gap.max(gap);
    }

    let spacing = if let Some(explicit_spacing) = facet_spacing {
        explicit_spacing
    } else {
        let facet_ctx = context
            .theme
            .facet_context_with_params(context.params.clone());
        context
            .theme
            .query(&facet_ctx, "spacing")
            .and_then(|v| v.as_number())
            .map(|n| n as f32)
            .unwrap_or(DEFAULT_FACET_SPACING)
    };
    max_required_gap += spacing;
    let rounded_gap = max_required_gap.ceil();

    let padding_spec = crate::coords::PaddingSpec::Single {
        padding_px: rounded_gap,
        overflow: overflow_measurements.clone(),
    };
    let updated_facet_coord = facet_coord.with_measured_padding(&padding_spec);

    // Rebuild the facet dimension scale with measured padding
    let mut new_config = dimension_scale.configured().config.clone();
    new_config.options.insert(
        "padding_inner_px".to_string(),
        Scalar::from_f32(rounded_gap),
    );

    let updated_spec = dimension_scale.spec().clone().option(
        "padding_inner_px",
        lit(ScalarValue::Float32(Some(rounded_gap))),
    );

    let updated_configured = avenger_scales::scales::ConfiguredScale {
        scale_impl: dimension_scale.configured().scale_impl.clone(),
        config: new_config,
    };

    let final_dimension_scale = ConfiguredScaleWithSpec::new(updated_spec, updated_configured);

    let mut updated_scales = HashMap::with_capacity(1);
    updated_scales.insert(
        DimConfig::channel_name().to_string(),
        final_dimension_scale.clone(),
    );

    let final_configured = final_dimension_scale.configured();
    let final_positions = final_configured.scale_scalars_to_numeric(&domain_vals)?;

    let mut temp_position_channels = HashMap::new();
    temp_position_channels.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(final_positions),
    );

    let mut temp_position_values = HashMap::new();
    temp_position_values.insert(channel_name, domain_vals);

    let final_geometry_with_padding = updated_facet_coord.transform(
        &temp_position_channels,
        Some(&temp_position_values),
        context.plot_width,
        context.plot_height,
    )?;

    let mut final_rects = final_geometry_with_padding
        .as_any()
        .downcast_ref::<SubplotGeometry>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected SubplotGeometry from facet coord transform in Pass 2".into(),
            )
        })?
        .rects
        .clone();

    // Adjust the last rect to fill remaining space, avoiding rounding gaps
    // This ensures the last band reaches exactly to the plot boundary
    if let Some(last_rect) = final_rects.last_mut() {
        if DimConfig::is_row_facet() {
            // For row faceting, adjust height so last band reaches plot_height
            let expected_end = context.plot_height;
            let current_end = last_rect.y + last_rect.height;
            if (expected_end - current_end).abs() > 0.001 {
                last_rect.height = expected_end - last_rect.y;
            }
        } else {
            // For column faceting, adjust width so last band reaches plot_width
            let expected_end = context.plot_width;
            let current_end = last_rect.x + last_rect.width;
            if (expected_end - current_end).abs() > 0.001 {
                last_rect.width = expected_end - last_rect.x;
            }
        }
    }

    let final_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let final_bandwidth = band::bandwidth(&final_dimension_scale.configured().config)?;
        let (width, height) = subplot_dims(final_bandwidth, context);

        Some(
            compiled_subplot
                .build_scales_from_builder(
                    builder,
                    width,
                    height,
                    &context.session_context,
                    &context.params,
                )
                .await?,
        )
    } else {
        initial_shared_scales
    };

    // Aggregate legend layout info for cross-subplot alignment
    let legend_alignment = crate::layout::LegendAlignmentInfo::aggregate(&legend_infos);

    Ok(FacetPass1Result {
        overflow_measurements,
        final_dimension_scale,
        final_shared_scales,
        final_rects,
        legend_alignment,
    })
}

/// Render faceted subplots (Pass 2) using the measurement results
#[allow(clippy::too_many_arguments)]
async fn render_pass<DimConfig: FacetDimensionConfig, GroupOriginFn>(
    compiled_subplot: &Arc<CompiledPlot>,
    df: &DataFrame,
    facet_expr: &datafusion::logical_expr::Expr,
    context: &RenderContext,
    scale_sharing_by_channel: HashMap<String, ScaleSharing>,
    pass1: FacetPass1Result,
    group_origin: &GroupOriginFn,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError>
where
    GroupOriginFn: Fn(f32) -> [f32; 2] + Clone,
{
    // Extract domain values from final rects for SubplotIterator
    let domain_vals_final: Vec<ScalarValue> = pass1
        .final_rects
        .iter()
        .map(|rect| rect.value.clone())
        .collect();

    use crate::facet::subplot_iterator::SubplotIterator;
    let subplot_iter_pass2 = SubplotIterator::<DimConfig>::new(
        domain_vals_final,
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    assert_eq!(
        pass1.final_rects.len(),
        subplot_iter_pass2.len(),
        "Final geometry rect count mismatch with SubplotIterator"
    );

    async fn render_subplot(
        compiled_subplot: &Arc<CompiledPlot>,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        scale_provider: &dyn crate::plot::compiled::scale_provider::ScaleProvider,
        filter_df: &DataFrame,
    ) -> Result<crate::plot::compiled::PlotComponents, AvengerChartError> {
        compiled_subplot
            .build_plot_components(
                width,
                height,
                ctx,
                params,
                scale_provider,
                crate::plot::compiled::EvaluationMode::Render,
                Some(filter_df),
                true,
            )
            .await
    }

    let mut all_marks: Vec<SceneMark> = Vec::new();

    const MAX_CONCURRENT_RENDER: usize = 4;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_RENDER));
    let work_items: Vec<(usize, SubplotIteration, SubplotRect)> = subplot_iter_pass2
        .zip(pass1.final_rects.iter().cloned())
        .enumerate()
        .map(|(idx, (iter, rect))| (idx, iter, rect))
        .collect();

    // Clone aggregated legend alignment info for threading to subplots
    let legend_alignment = pass1.legend_alignment.clone();

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            let params_base = iteration.params.clone();
            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let final_shared_scales = pass1.final_shared_scales.clone();
            let df = df.clone();
            let overflow_dbg = pass1
                .overflow_measurements
                .get(idx)
                .cloned()
                .unwrap_or_default();
            let semaphore = Arc::clone(&semaphore);
            let legend_alignment = legend_alignment.clone();

            async move {
                let _permit = semaphore.acquire().await.unwrap();

                let filter_df: DataFrame = df
                    .clone()
                    .filter(facet_expr.eq(lit(iteration.facet_value.clone())))?;

                // Use rect dimensions directly - we've already adjusted the last rect
                // to fill remaining space, so avoid double-rounding by using rect values
                let (width, height) = if DimConfig::is_row_facet() {
                    // Row faceting: width is fixed, height varies with band
                    (context.plot_width, rect.height.round())
                } else {
                    // Column faceting: height is fixed, width varies with band
                    (rect.width.round(), context.plot_height)
                };

                let free_scale_builder = compiled_subplot
                    .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
                    .await?;

                let mut scales = build_scales_helper(
                    &compiled_subplot,
                    &final_shared_scales,
                    &if scale_sharing_by_channel
                        .values()
                        .any(|v| *v == ScaleSharing::Shared)
                    {
                        None
                    } else {
                        Some(free_scale_builder.clone())
                    },
                    &filter_df,
                    width,
                    height,
                    &ctx,
                    &params_base,
                )
                .await?;

                if scale_sharing_by_channel
                    .values()
                    .any(|v| *v == ScaleSharing::Shared)
                {
                    let facet_scales = build_scales_helper(
                        &compiled_subplot,
                        &None,
                        &Some(free_scale_builder),
                        &filter_df,
                        width,
                        height,
                        &ctx,
                        &params_base,
                    )
                    .await?;
                    for (ch, sharing_mode) in &scale_sharing_by_channel {
                        if *sharing_mode != ScaleSharing::Shared {
                            if let Some(s) = facet_scales.get(ch) {
                                scales.insert(ch.clone(), s.clone());
                            }
                        }
                    }
                }

                let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
                    scales: scales.clone(),
                };

                // Add legend alignment params for cross-subplot alignment.
                // These target positions enable legends to align at the same absolute
                // position across subplots regardless of individual layout differences.
                let mut params_with_legend_align = params_base.clone();

                // Alignment mode determines which direction alignment applies:
                // - "row" facets: X alignment for Right/Left legends (subplots stacked vertically)
                // - "col" facets: Y alignment for Top/Bottom legends (subplots side by side)
                params_with_legend_align.insert(
                    "__legend_align_mode".to_string(),
                    ScalarValue::Utf8(Some(
                        if DimConfig::is_row_facet() { "row" } else { "col" }.to_string()
                    )),
                );
                params_with_legend_align.insert(
                    "__legend_align_max_right_x".to_string(),
                    ScalarValue::Float32(Some(legend_alignment.max_right_x)),
                );
                params_with_legend_align.insert(
                    "__legend_align_min_left_x".to_string(),
                    ScalarValue::Float32(Some(legend_alignment.min_left_x)),
                );
                params_with_legend_align.insert(
                    "__legend_align_min_top_y".to_string(),
                    ScalarValue::Float32(Some(legend_alignment.min_top_y)),
                );
                params_with_legend_align.insert(
                    "__legend_align_max_bottom_y".to_string(),
                    ScalarValue::Float32(Some(legend_alignment.max_bottom_y)),
                );

                let components = render_subplot(
                    &compiled_subplot,
                    width,
                    height,
                    &ctx,
                    &params_with_legend_align,
                    &scale_provider,
                    &filter_df,
                )
                .await?;

                let position = if DimConfig::is_row_facet() {
                    rect.y
                } else {
                    rect.x
                };
                let origin = group_origin(position);

                Ok::<_, AvengerChartError>((
                    idx,
                    components,
                    origin,
                    overflow_dbg,
                    rect,
                    params_base,
                    width,
                ))
            }
        })
        .buffer_unordered(MAX_CONCURRENT_RENDER)
        .collect::<Vec<_>>()
        .await;

    let mut sorted = results
        .into_iter()
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    sorted.sort_by_key(|(idx, _, _, _, _, _, _)| *idx);

    for (_subplot_index, components, origin, overflow_dbg, rect, params_base, width) in sorted {
        let data_group = SceneGroup {
            origin,
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        all_marks.push(SceneMark::Group(data_group));

        if !components.guide_marks.is_empty() {
            let guide_group = SceneGroup {
                origin,
                marks: components.guide_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(1),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(guide_group));
        }

        if !components.legend_marks.is_empty() {
            let legend_group = SceneGroup {
                origin,
                marks: components.legend_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(2),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(legend_group));
        }

        if !components.title_marks.is_empty() {
            let title_group = SceneGroup {
                origin,
                marks: components.title_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(3),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(title_group));
        }

        if !components.subtitle_marks.is_empty() {
            let subtitle_group = SceneGroup {
                origin,
                marks: components.subtitle_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(4),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(subtitle_group));
        }

        if !components.debug_marks.is_empty() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                let band_start = if DimConfig::is_row_facet() {
                    rect.y
                } else {
                    rect.x
                };
                let band_size = if DimConfig::is_row_facet() {
                    rect.height
                } else {
                    rect.width
                };
                let band_end = band_start + band_size;
                let left_rect_x_rel_plot = band_start - overflow_dbg.left;
                let right_rect_x_rel_plot = band_end;

                if let Some(facet_ctx) =
                    crate::facet::context::FacetContext::from_params(&params_base)
                {
                    let (row, col) = facet_ctx.position;
                    eprintln!(
                        "SUBPLOT r={} c={}: band_start={:.3} w={:.3} overflowL={:.3} overflowR={:.3} -> of-left x_rel_plot={:.3} of-right x_rel_plot={:.3}",
                        row,
                        col,
                        band_start,
                        width,
                        overflow_dbg.left,
                        overflow_dbg.right,
                        left_rect_x_rel_plot,
                        right_rect_x_rel_plot
                    );
                } else {
                    eprintln!(
                        "SUBPLOT: band_start={:.3} w={:.3} overflowL={:.3} overflowR={:.3} -> of-left x_rel_plot={:.3} of-right x_rel_plot={:.3}",
                        band_start,
                        width,
                        overflow_dbg.left,
                        overflow_dbg.right,
                        left_rect_x_rel_plot,
                        right_rect_x_rel_plot
                    );
                }
            }

            let debug_group = SceneGroup {
                origin,
                marks: components.debug_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(100),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(debug_group));
        }
    }

    let layout_updates = if DimConfig::channel_name() == "row" {
        crate::layout::LayoutUpdates::new(
            {
                let mut updated = HashMap::new();
                updated.insert(
                    DimConfig::channel_name().to_string(),
                    pass1.final_dimension_scale.clone(),
                );
                updated
            },
            Some(pass1.overflow_measurements.clone()),
            None,
        )
    } else {
        crate::layout::LayoutUpdates::new(
            {
                let mut updated = HashMap::new();
                updated.insert(
                    DimConfig::channel_name().to_string(),
                    pass1.final_dimension_scale.clone(),
                );
                updated
            },
            None,
            Some(pass1.overflow_measurements.clone()),
        )
    };

    Ok((all_marks, layout_updates))
}

/// Generic two-pass facet evaluation parameterized by dimension
///
/// # Algorithm
///
/// **Pass 1 (Measurement)**: Measure guide overflow to determine spacing
/// - Build scales with approximate band size
/// - Measure each subplot's axis labels/titles
/// - Calculate max overflow based on dimension-specific adjacency rules
/// - Adjust band spacing to accommodate overflow
///
/// **Pass 2 (Rendering)**: Render with corrected dimensions
/// - Rebuild shared scales with final band size (CRITICAL for data alignment)
/// - Position and render each subplot with correct spacing
/// - Facet labels are rendered by the guide system (not by this function)
///
/// # Orientation Closures
///
/// The generic algorithm accepts two closures that define dimension-specific behavior:
/// - `subplot_dims`: Compute (width, height) from band size and context
/// - `group_origin`: Compute [x, y] translation from band position
///
/// # Type Parameters
///
/// - `DimConfig`: The facet dimension configuration (RowDimensionConfig or ColDimensionConfig)
#[allow(clippy::too_many_arguments)]
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    facet_coord: &dyn crate::coords::CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,
    state: &CompiledMarkState,
    data_override: Option<&datafusion::dataframe::DataFrame>,
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    // Orientation-specific closures:
    // Returns (width, height) given band size and context
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32) + Clone,
    // Returns [x, y] translation given band position
    group_origin: impl Fn(f32) -> [f32; 2] + Clone,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "evaluate_facet entering: channel={}",
            DimConfig::channel_name()
        );
    }

    // Get dimension scale (row or col)
    let dimension_scale = context
        .scales
        .get(DimConfig::channel_name())
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                format!("Missing '{}' scale for faceting", DimConfig::channel_name()).into(),
            )
        })?;

    // Validate that scale is a band scale
    if dimension_scale.configured().scale_impl.scale_type() != "band" {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Faceting requires band scale on '{}' channel, got scale type: {}",
            DimConfig::channel_name(),
            dimension_scale.configured().scale_impl.scale_type()
        )));
    }

    // Determine data source: parent override OR compiled data
    let ctx = &context.session_context;
    let df = if let Some(override_df) = data_override {
        override_df.clone()
    } else {
        state.data.dataframe_with_context(ctx).ok_or_else(|| {
            AvengerChartError::InternalError("Facet mark requires plot or mark data".into())
        })?
    };

    // Extract the raw expression for the facet channel to filter by facet value
    let facet_expr = state
        .data
        .channels()
        .get(DimConfig::channel_name())
        .and_then(|cv| cv.expr(ctx))
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                format!("Facet '{}' channel not found", DimConfig::channel_name()).into(),
            )
        })?;

    // Compute per-channel sharing preferences by scanning inner marks
    let required_channels: Vec<&str> = compiled_subplot
        .coord_transform
        .required_channels()
        .to_vec();
    let mut scale_sharing_by_channel: HashMap<String, ScaleSharing> = HashMap::new();
    for &ch in &required_channels {
        let mut mode = ScaleSharing::Free;
        for m in &compiled_subplot.marks {
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

    let scale_sharing_by_channel: HashMap<String, ScaleSharing> = scale_sharing_by_channel
        .into_iter()
        .map(|(ch, mode)| {
            let normalized = match mode {
                ScaleSharing::SharedInColumn if DimConfig::is_row_facet() => ScaleSharing::Shared,
                ScaleSharing::SharedInRow if DimConfig::is_col_facet() => ScaleSharing::Shared,
                other => other,
            };
            (ch, normalized)
        })
        .collect();

    let pass1 = measure_pass::<DimConfig, _>(
        facet_coord,
        compiled_subplot,
        dimension_scale,
        &scale_sharing_by_channel,
        &df,
        &facet_expr,
        facet_spacing,
        context,
        &subplot_dims,
    )
    .await?;

    render_pass::<DimConfig, _>(
        compiled_subplot,
        &df,
        &facet_expr,
        context,
        scale_sharing_by_channel,
        pass1,
        &group_origin,
    )
    .await
}
