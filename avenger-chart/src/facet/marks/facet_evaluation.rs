/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module provides a parameterized two-pass rendering algorithm that works for
/// both row and column faceting by accepting orientation-specific closures.
use crate::channel::config_traits::ScaleSharing;
use crate::coords::SubplotGeometry;
use crate::error::AvengerChartError;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::facet::scale_helpers::build_scales_helper;
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
#[allow(dead_code)]
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
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32),
    // Returns [x, y] translation given band position
    group_origin: impl Fn(f32) -> [f32; 2],
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    eprintln!(
        "evaluate_facet entering: channel={}",
        DimConfig::channel_name()
    );

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

    // Extract domain values and bandwidth directly from scale
    use crate::scales::extensions::ConfiguredScaleLegendExt;
    use avenger_scales::scales::band;

    let configured = dimension_scale.configured();
    // REMOVED: extraction of initial_domain_vals from scale.
    // We now drive layout using keys extracted from the data to ensure sync with SubplotIterator.

    let initial_bandwidth = band::bandwidth(&configured.config)?;

    // Determine data source: parent override OR compiled data
    let ctx = &context.session_context;
    let df = if let Some(override_df) = data_override {
        // Nested facet: use filtered data from parent
        eprintln!("✅ Using data_override (nested facet with filtered data)");
        override_df.clone()
    } else {
        // Top-level facet: use compiled data
        eprintln!("⚠️  Using compiled state data (top-level facet)");
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

    let mut all_marks: Vec<SceneMark> = Vec::new();

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
                    // Upgrade to more restrictive sharing
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

    // Normalize partial sharing modes for row/col faceting
    // For row-only faceting: SharedInColumn -> Shared (only one column)
    // For col-only faceting: SharedInRow -> Shared (only one row)
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

    // If any channel is shared, build a ScaleBuilder once for reuse across passes
    // This enables radius-aware domain inference for faceted plots
    // Use the full dataset (df, not filtered) since shared scales span all facets
    let any_shared = scale_sharing_by_channel
        .values()
        .any(|v| *v == ScaleSharing::Shared);
    let shared_scale_builder = if any_shared {
        Some(
            compiled_subplot
                .build_scale_builder_from_dataframe(ctx, &context.params, &df)
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
                .build_scales_from_builder(builder, width, height, ctx, &context.params)
                .await?,
        )
    } else {
        None
    };

    // ========== PASS 1: MEASUREMENT PHASE ==========
    eprintln!("Pass 1 starting for channel={}", DimConfig::channel_name());
    // Extract facet keys at render time from actual data
    use crate::facet::keys::FacetKeyExtractor;
    let mut domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;
    eprintln!(
        "🔑 Extracted {} keys for channel '{}'",
        domain_vals.len(),
        DimConfig::channel_name()
    );

    // Sort domain values to ensure deterministic facet ordering
    domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Calculate positions for the extracted keys
    // This uses the scale's domain to map keys to positions, but only for keys present in the data
    let initial_positions = configured.scale_scalars_to_numeric(&domain_vals)?;

    // Create SubplotIterator for logical iteration (FacetContext management)
    use crate::facet::subplot_iterator::SubplotIterator;
    let subplot_iter = SubplotIterator::<DimConfig>::new(
        domain_vals.clone(),
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    // Build position_channels and position_values for facet coord transform (Pass 1)
    let channel_name = DimConfig::channel_name();

    let mut position_channels_pass1 = HashMap::new();
    position_channels_pass1.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(initial_positions.clone()),
    );

    let mut position_values_pass1 = HashMap::new();
    position_values_pass1.insert(channel_name, domain_vals.clone());

    // Call facet coord transform to get initial geometry
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

    // Safety check: subplot iterator and rects must have same length
    assert_eq!(
        subplot_iter.len(),
        initial_rects.len(),
        "SubplotIterator and initial_rects length mismatch in Pass 1"
    );

    let mut overflow_measurements = Vec::new();

    // Small helper to measure one subplot to keep the parent future small
    async fn measure_subplot(
        compiled_subplot: &Arc<CompiledPlot>,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        filter_df: &DataFrame,
    ) -> Result<crate::guide::OverflowSpaceRequirement, AvengerChartError> {
        compiled_subplot
            .measure_with_scales(width, height, ctx, params, scales, Some(filter_df))
            .await
    }

    for (iteration, rect) in subplot_iter.zip(initial_rects.iter()) {
        eprintln!("Pass 1 iteration");
        let band_size = if DimConfig::is_row_facet() {
            rect.height
        } else {
            rect.width
        };
        let (width, height) = subplot_dims(band_size, context);
        // Filter df by facet_value
        let filter_df: DataFrame = df
            .clone()
            .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;

        // Build scales for this partition (use shared scales if available)

        // For free-scale facets, build a per-facet ScaleBuilder
        let free_scale_builder_pass1 = if any_shared {
            None
        } else {
            Some(
                compiled_subplot
                    .build_scale_builder_from_dataframe(ctx, &iteration.params, &filter_df)
                    .await?,
            )
        };

        let scales = build_scales_helper(
            compiled_subplot,
            &initial_shared_scales,
            &free_scale_builder_pass1,
            &filter_df,
            width,
            height,
            ctx,
            &iteration.params, // Use SubplotIterator's params (has FacetContext)
        )
        .await?;

        // Measure guide AND legend overflow for this partition with FacetContext applied
        // Use evaluate_in_canvas with Measure mode to get full layout including legends
        let _scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
            scales: scales.clone(),
        };

        // Lightweight measurement: avoid full render recursion; measure overflow using provided scales and filtered data.
        let overflow = measure_subplot(
            compiled_subplot,
            width,
            height,
            ctx,
            &iteration.params,
            &scales,
            &filter_df,
        )
        .await?;
        overflow_measurements.push(overflow);
    }

    // Calculate required padding based on adjacent overflow measurements
    let mut max_required_gap = 0.0f32;
    for i in 0..overflow_measurements.len().saturating_sub(1) {
        let gap = DimConfig::calculate_adjacent_overflow(
            &overflow_measurements[i],
            &overflow_measurements[i + 1],
        );
        max_required_gap = max_required_gap.max(gap);
    }

    // Add spacing from facet configuration or theme
    // Priority: 1) facet_spacing field, 2) theme 'facet { spacing }', 3) DEFAULT_FACET_SPACING
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

    // Round gap to nearest integer for pixel alignment
    // Use standard ceil() rounding - band scale and manual rounding handle filling the range
    let rounded_gap = max_required_gap.ceil();

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "PASS1: max_required_gap={:.3} -> rounded_gap={:.3} (will set as padding_inner_px)",
            max_required_gap, rounded_gap
        );
    }

    // Update facet coord with measured padding for Pass 2
    let padding_spec = crate::coords::PaddingSpec::Single {
        padding_px: rounded_gap,
        overflow: overflow_measurements.clone(),
    };
    let updated_facet_coord = facet_coord.with_measured_padding(&padding_spec);

    // ========== PASS 2: RENDERING PHASE ==========
    // STEP 1: Rebuild the facet dimension scale with measured padding FIRST
    // This ensures coord.transform() receives positions that match its internal padding state
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

    // STEP 2: Extract positions from the REBUILT scale (now with correct padding)
    let final_configured = final_dimension_scale.configured();
    // Use the same domain_vals as Pass 1 to ensure consistency
    let final_positions = final_configured.scale_scalars_to_numeric(&domain_vals)?;

    let mut temp_position_channels = HashMap::new();
    temp_position_channels.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(final_positions),
    );

    let mut temp_position_values = HashMap::new();
    temp_position_values.insert(channel_name, domain_vals);

    // STEP 3: Call coord.transform() with positions from rebuilt scale
    // Now the scale and coord are in sync (both have padding = rounded_gap)
    let final_geometry_with_padding = updated_facet_coord.transform(
        &temp_position_channels,
        Some(&temp_position_values),
        context.plot_width,
        context.plot_height,
    )?;

    let final_rects_with_padding = final_geometry_with_padding
        .as_any()
        .downcast_ref::<SubplotGeometry>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected SubplotGeometry from facet coord transform in Pass 2".into(),
            )
        })?
        .rects
        .clone();

    // Use the final_rects_with_padding that already has correct positions
    let final_rects = final_rects_with_padding;

    eprintln!("=== FINAL RECTS (Pass 2) ===");
    for (i, rect) in final_rects.iter().enumerate() {
        eprintln!(
            "  rect[{}]: x={}, y={}, width={}, height={}",
            i, rect.x, rect.y, rect.width, rect.height
        );
    }

    // Debug logging for final geometry
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        if let Some(last_rect) = final_rects.last() {
            let (start, bandwidth) = if DimConfig::is_row_facet() {
                (last_rect.y, last_rect.height)
            } else {
                (last_rect.x, last_rect.width)
            };
            eprintln!(
                "PASS2 Band scale: last subplot start={} bandwidth={} end={}",
                start,
                bandwidth,
                start + bandwidth
            );
        }
        eprintln!(
            "PASS2 Band scale range: context.plot_width={} context.plot_height={}",
            context.plot_width, context.plot_height
        );

        eprintln!(
            "PASS2: Band positions from rebuilt scale (count={}):",
            final_rects.len()
        );
        for (i, rect) in final_rects.iter().enumerate() {
            let (start, bw) = if DimConfig::is_row_facet() {
                (rect.y, rect.height)
            } else {
                (rect.x, rect.width)
            };
            eprintln!(
                "  Band {}: value={:?} start={:.3} bandwidth={:.3} end={:.3}",
                i,
                rect.value,
                start,
                bw,
                start + bw
            );
        }
        // Also check the scale config (from updated scale)
        let cfg = final_dimension_scale.configured();
        if let Some(padding_val) = cfg.config.options.get("padding_inner_px") {
            eprintln!("  Updated scale padding_inner_px={:?}", padding_val);
        }
    }

    // CRITICAL: Rebuild shared scales with the NEW band size after padding adjustment
    // The initial_shared_scales were built with the approximate size BEFORE padding,
    // which causes incorrect data scaling (axis doesn't align properly)
    let final_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let final_bandwidth = band::bandwidth(&final_dimension_scale.configured().config)?;
        let (width, height) = subplot_dims(final_bandwidth, context);

        Some(
            compiled_subplot
                .build_scales_from_builder(builder, width, height, ctx, &context.params)
                .await?,
        )
    } else {
        initial_shared_scales
    };

    // Extract domain values from final rects for SubplotIterator
    let domain_vals_final: Vec<ScalarValue> =
        final_rects.iter().map(|rect| rect.value.clone()).collect();

    // Create SubplotIterator for Pass 2 (FacetContext management)
    let subplot_iter_pass2 = SubplotIterator::<DimConfig>::new(
        domain_vals_final,
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    // Safety check: iterators must have same length
    assert_eq!(
        final_rects.len(),
        subplot_iter_pass2.len(),
        "Final geometry rect count mismatch with SubplotIterator"
    );

    // Small helper to render one subplot to keep the outer future small.
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
                true, // Plot area mode: dimensions are already plot area size
            )
            .await
    }

    for (subplot_index, (iteration, rect)) in subplot_iter_pass2.zip(final_rects.iter()).enumerate()
    {
        // Filter df by facet_value
        let filter_df: DataFrame = df
            .clone()
            .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;

        // Build scales and evaluate inner components for this partition
        let band_size = if DimConfig::is_row_facet() {
            rect.height
        } else {
            rect.width
        };
        let (width, height) = subplot_dims(band_size, context);

        // Build a ScaleBuilder from filtered data for radius-aware free scales
        let free_scale_builder = compiled_subplot
            .build_scale_builder_from_dataframe(ctx, &iteration.params, &filter_df)
            .await?;

        // Build initial scales
        let mut scales = build_scales_helper(
            compiled_subplot,
            &final_shared_scales,
            &if any_shared {
                None
            } else {
                Some(free_scale_builder.clone())
            },
            &filter_df,
            width,
            height,
            ctx,
            &iteration.params,
        )
        .await?;

        // If some channels are free (mixed shared/free), build free scales and override free channels
        if any_shared {
            let facet_scales = build_scales_helper(
                compiled_subplot,
                &None,
                &Some(free_scale_builder),
                &filter_df,
                width,
                height,
                ctx,
                &iteration.params,
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

        // Render subplot using evaluate_in_canvas with Render mode
        // This creates all marks including legends, titles, and subtitles
        let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
            scales: scales.clone(),
        };

        eprintln!(
            "Calling build_plot_components (Render) for iteration {}",
            subplot_index
        );
        let components = render_subplot(
            compiled_subplot,
            width,
            height,
            ctx,
            &iteration.params,
            &scale_provider,
            &filter_df,
        )
        .await?;
        eprintln!(
            "Finished build_plot_components (Render) for iteration {}",
            subplot_index
        );

        // Use raw group origin for smooth subpixel positioning
        // Position is taken from coord.transform() output (rect.x or rect.y)
        let position = if DimConfig::is_row_facet() {
            rect.y
        } else {
            rect.x
        };
        let origin = group_origin(position);

        // Wrap data marks in a clipped group translated to subplot position
        let data_group = SceneGroup {
            origin,
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        all_marks.push(SceneMark::Group(data_group));

        // Wrap guide marks (axes) in a non-clipped translated group
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

        // Wrap legend marks in a non-clipped translated group
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

        // Wrap title marks in a non-clipped translated group
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

        // Wrap subtitle marks in a non-clipped translated group
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

        // Wrap debug marks in a non-clipped translated group
        // Debug marks are in absolute canvas coordinates relative to the subplot,
        // so we need to translate them to the correct band position
        if !components.debug_marks.is_empty() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                // Use Pass 1 overflow measurement for debug visualization
                // This matches the overflow used to compute the final band scale padding
                let overflow_dbg = overflow_measurements
                    .get(subplot_index)
                    .cloned()
                    .unwrap_or_default();

                // Position within plot-area coordinates (before outer plot translation)
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
                let band_end = band_start + band_size; // Use rect dimensions for end position
                let left_rect_x_rel_plot = band_start - overflow_dbg.left;
                let right_rect_x_rel_plot = band_end; // right overflow placed at band end

                // Identify subplot (row,col) if available
                if let Some(facet_ctx) =
                    crate::facet::context::FacetContext::from_params(&iteration.params)
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
            // Use same origin as other groups for consistency
            let debug_group = SceneGroup {
                origin,
                marks: components.debug_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(100), // High z-index to ensure debug marks render on top
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(debug_group));
        }
    }

    // Store maximum overflow across all subplots in cache for guide to use
    if !overflow_measurements.is_empty() {
        // Compute maximum overflow across all subplots
        let mut max_overflow = crate::guide::OverflowSpaceRequirement::default();
        for overflow in &overflow_measurements {
            max_overflow.top = max_overflow.top.max(overflow.top);
            max_overflow.bottom = max_overflow.bottom.max(overflow.bottom);
            max_overflow.left = max_overflow.left.max(overflow.left);
            max_overflow.right = max_overflow.right.max(overflow.right);
        }
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "Caching max overflow: top={} bottom={} left={} right={}",
                max_overflow.top, max_overflow.bottom, max_overflow.left, max_overflow.right
            );
        }
        // Arc<Mutex> writes removed - overflow now returned in LayoutUpdates
    }

    // Return overflow measurements in LayoutUpdates based on dimension
    eprintln!("=== LAYOUT UPDATES (facet evaluation) ===");
    eprintln!("  overflow_measurements: {:?}", overflow_measurements);

    let layout_updates = if DimConfig::channel_name() == "row" {
        crate::layout::LayoutUpdates::new(updated_scales, Some(overflow_measurements), None)
    } else {
        // column facet
        crate::layout::LayoutUpdates::new(updated_scales, None, Some(overflow_measurements))
    };

    Ok((all_marks, layout_updates))
}
