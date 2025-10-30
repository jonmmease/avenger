/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module provides a parameterized two-pass rendering algorithm that works for
/// both row and column faceting by accepting orientation-specific closures.

use crate::channel::config_traits::ScaleSharing;
use crate::error::AvengerChartError;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::facet::scale_helpers::build_scales_helper;
use crate::layout::LayoutInfo;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::lit;
use std::collections::HashMap;
use std::sync::Arc;

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
    compiled_subplot: &Arc<CompiledPlot>,
    state: &CompiledMarkState,
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    // Orientation-specific closures:
    // Returns (width, height) given band size and context
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32),
    // Returns [x, y] translation given band position
    group_origin: impl Fn(f32) -> [f32; 2],
) -> Result<(Vec<SceneMark>, Box<dyn LayoutInfo>), AvengerChartError> {
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

    // Build initial (facet_value -> (position, bandwidth)) map with current scale
    use crate::facet::band_positions::BandPositionIterator;
    let initial_band_positions: Vec<_> = BandPositionIterator::from_scale(dimension_scale)?
        .map(|bp| (bp.value.clone(), (bp.start(), bp.bandwidth)))
        .collect();

    // Get the inner plot-level DataFrame
    let ctx = &context.session_context;
    let df = state.data.dataframe_with_context(ctx).ok_or_else(|| {
        AvengerChartError::InternalError("Facet mark requires plot or mark data".into())
    })?;

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
    let initial_band_size = extract_band_size(
        &initial_band_positions,
        context.plot_width.max(context.plot_height),
    );
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
        let (width, height) = subplot_dims(initial_band_size, context);
        Some(
            compiled_subplot
                .build_scales_from_builder(builder, width, height, ctx, &context.params)
                .await?,
        )
    } else {
        None
    };

    // ========== PASS 1: MEASUREMENT PHASE ==========
    // Extract domain values from band positions for SubplotIterator
    let domain_vals: Vec<datafusion::common::ScalarValue> = initial_band_positions
        .iter()
        .map(|(val, _)| val.clone())
        .collect();

    // Create SubplotIterator for logical iteration (FacetContext management)
    use crate::facet::subplot_iterator::SubplotIterator;
    let subplot_iter = SubplotIterator::<DimConfig>::new(
        domain_vals,
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    // Create BandPositionIterator for geometric iteration (layout positions)
    let band_iter = BandPositionIterator::from_scale(dimension_scale)?;

    // Safety check: both iterators must have same length
    assert_eq!(
        subplot_iter.len(),
        band_iter.len(),
        "SubplotIterator and BandPositionIterator length mismatch in Pass 1"
    );

    let mut overflow_measurements = Vec::new();

    for (iteration, band_pos) in subplot_iter.zip(band_iter) {
        // Filter df by facet_value
        let filter_df: DataFrame = df
            .clone()
            .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;

        // Build scales for this partition (use shared scales if available)
        let (width, height) = subplot_dims(band_pos.bandwidth, context);

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
        let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
            scales: scales.clone(),
        };

        let components = compiled_subplot
            .build_plot_components(
                width,
                height,
                ctx,
                &iteration.params,
                &scale_provider,
                crate::plot::compiled::EvaluationMode::Measure,
                Some(&filter_df),
                true, // Plot area mode: dimensions are already plot area size
            )
            .await?;

        // Extract overflow from the returned components
        let overflow = components.overflow.unwrap_or_default();
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

    // Rebuild the facet dimension scale with measured padding_inner_px
    let mut updated_scales = HashMap::new();
    if max_required_gap > 0.0 {
        use avenger_scales::scalar::Scalar;

        // Clone the existing scale config and modify padding_inner_px option
        let mut new_config = dimension_scale.configured().config.clone();
        new_config.options.insert(
            "padding_inner_px".to_string(),
            Scalar::from_f32(max_required_gap),
        );

        // Create a new ConfiguredScale with the modified config
        let new_configured = avenger_scales::scales::ConfiguredScale {
            scale_impl: dimension_scale.configured().scale_impl.clone(),
            config: new_config,
        };

        // Wrap in ConfiguredScaleWithSpec
        updated_scales.insert(
            DimConfig::channel_name().to_string(),
            ConfiguredScaleWithSpec::new(dimension_scale.spec().clone(), new_configured),
        );
    }

    // Create a merged scales map for pass 2 rendering
    let mut merged_scales_for_rendering = context.scales.clone();
    merged_scales_for_rendering.extend(updated_scales.clone());

    // ========== PASS 2: RENDERING PHASE ==========
    // Get updated band positions from rebuilt scale
    let final_dimension_scale = merged_scales_for_rendering
        .get(DimConfig::channel_name())
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                format!("Missing rebuilt '{}' scale", DimConfig::channel_name()).into(),
            )
        })?;
    let band_positions: Vec<_> = BandPositionIterator::from_scale(final_dimension_scale)?
        .map(|bp| (bp.value.clone(), (bp.start(), bp.bandwidth)))
        .collect();

    // CRITICAL: Rebuild shared scales with the NEW band size after padding adjustment
    // The initial_shared_scales were built with the approximate size BEFORE padding,
    // which causes incorrect data scaling (axis doesn't align properly)
    let final_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let final_band_size =
            extract_band_size(&band_positions, context.plot_width.max(context.plot_height));
        let (width, height) = subplot_dims(final_band_size, context);

        Some(
            compiled_subplot
                .build_scales_from_builder(builder, width, height, ctx, &context.params)
                .await?,
        )
    } else {
        initial_shared_scales
    };

    // Extract domain values from final band positions for SubplotIterator
    let domain_vals_final: Vec<datafusion::common::ScalarValue> =
        band_positions.iter().map(|(val, _)| val.clone()).collect();

    // Create SubplotIterator for Pass 2 (FacetContext management)
    let subplot_iter_pass2 = SubplotIterator::<DimConfig>::new(
        domain_vals_final,
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    // Create BandPositionIterator for Pass 2 (layout positions)
    let band_iter_pass2 = BandPositionIterator::from_scale(final_dimension_scale)?;

    // Safety check: both iterators must have same length
    assert_eq!(
        subplot_iter_pass2.len(),
        band_iter_pass2.len(),
        "SubplotIterator and BandPositionIterator length mismatch in Pass 2"
    );

    for (iteration, band_pos) in subplot_iter_pass2.zip(band_iter_pass2) {
        // Filter df by facet_value
        let filter_df: DataFrame = df
            .clone()
            .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;

        // Build scales and evaluate inner components for this partition
        let (width, height) = subplot_dims(band_pos.bandwidth, context);

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

        let components = compiled_subplot
            .build_plot_components(
                width,
                height,
                ctx,
                &iteration.params,
                &scale_provider,
                crate::plot::compiled::EvaluationMode::Render,
                Some(&filter_df),
                true, // Plot area mode: dimensions are already plot area size
            )
            .await?;

        // Wrap data marks in a clipped group translated to band position
        let data_group = SceneGroup {
            origin: group_origin(band_pos.start()),
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        all_marks.push(SceneMark::Group(data_group));

        // Wrap guide marks (axes) in a non-clipped translated group
        if !components.guide_marks.is_empty() {
            let guide_group = SceneGroup {
                origin: group_origin(band_pos.start()),
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
                origin: group_origin(band_pos.start()),
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
                origin: group_origin(band_pos.start()),
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
                origin: group_origin(band_pos.start()),
                marks: components.subtitle_marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                zindex: Some(4),
                ..Default::default()
            };
            all_marks.push(SceneMark::Group(subtitle_group));
        }
    }

    Ok((
        all_marks,
        Box::new(crate::layout::ScaleUpdates::new(updated_scales)),
    ))
}

/// Helper to extract band size from band positions vector
///
/// Returns the bandwidth from the first band position, or fallback_size if empty.
fn extract_band_size(
    band_positions: &[(datafusion::common::ScalarValue, (f32, f32))],
    fallback_size: f32,
) -> f32 {
    band_positions
        .first()
        .map(|(_, (_, bandwidth))| *bandwidth)
        .unwrap_or(fallback_size)
}
