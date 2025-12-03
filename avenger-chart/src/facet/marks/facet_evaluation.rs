/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module provides a parameterized two-pass rendering algorithm that works for
/// both row and column faceting by accepting orientation-specific closures.
use crate::channel::config_traits::ScaleSharing;
use crate::coords::SubplotGeometry;
use crate::coords::SubplotRect;
use crate::error::AvengerChartError;
use crate::facet::coordination::FacetCoordinationContext;
use crate::facet::dimension_config::{FacetDimensionConfig, RowDimensionConfig};
use crate::facet::marks::facet::determine_facet_band_align;
use crate::facet::scale_helpers::build_scales_helper_with_fallback;
use crate::facet::subplot_iterator::SubplotIteration;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::builder::ScaleBuilder;
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
use std::collections::{HashMap, HashSet};
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
    /// Total overflow (including legends) for spacing calculations
    overflow_measurements: Vec<crate::guide::OverflowSpaceRequirement>,
    final_dimension_scale: ConfiguredScaleWithSpec,
    final_shared_scales: Option<HashMap<String, ConfiguredScaleWithSpec>>,
    final_rects: Vec<SubplotRect>,
    /// Fallback scale builder for empty cells (built from full dataset)
    fallback_builder: Option<ScaleBuilder>,
    /// Shared data extents from coordination context for nested facets (ScaleSharing::Shared)
    shared_data_extents:
        Option<HashMap<String, crate::facet::coordination::SerializableDataExtents>>,
    /// Per-row shared data extents from coordination context for nested facets (ScaleSharing::SharedInRow)
    shared_data_extents_by_row: Option<
        HashMap<String, HashMap<String, crate::facet::coordination::SerializableDataExtents>>,
    >,
    /// Per-column shared data extents from coordination context for nested facets (ScaleSharing::SharedInColumn)
    shared_data_extents_for_column:
        Option<HashMap<String, crate::facet::coordination::SerializableDataExtents>>,
    /// Named spacing needs reported by this facet for coordination with parent facets
    ///
    /// Inner facets compute their own gap needs based on cell overflow and report them here.
    /// Parent facets aggregate these by taking the max of each key across all children,
    /// then pass the aggregated values back via FacetCoordinationContext during render pass.
    ///
    /// Standard keys:
    /// - "inter_row_gap": Gap between rows within a row facet
    /// - "inter_col_gap": Gap between columns within a column facet
    /// - "legend_right": Right margin for legend alignment
    /// - "legend_bottom": Bottom margin for legend alignment
    spacing_needs: HashMap<String, f32>,
    /// Total cell count including phantoms for uniform sizing (used for guide ownership)
    uniform_cell_count: Option<usize>,
    /// Number of phantom cells prepended (0 if phantoms appended or no uniform sizing)
    phantom_offset: usize,
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

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass initial: channel={} initial_bandwidth={:.3}",
            DimConfig::channel_name(),
            initial_bandwidth
        );
    }

    // ========== COORDINATION CONTEXT EXTRACTION ==========
    // Extract coordination context early since we need shared_data_extents for scale building
    let coordination_context = FacetCoordinationContext::from_params(&context.params);
    let current_channel = DimConfig::channel_name();
    let domain_from_coordination = coordination_context.as_ref().and_then(|ctx| {
        // Only use coordination domain if the channel matches
        // The channel check prevents outer facet from consuming inner facet's domain
        ctx.get_inner_domain_for_channel(current_channel)
    });

    // Extract uniform_cell_count early for adjusted bandwidth computation
    // When uniform Free scaling is enabled, shared scales need to use adjusted bandwidth
    // so the axis extent matches the actual subplot extent, not full column height
    let early_uniform_cell_count = coordination_context
        .as_ref()
        .and_then(|ctx| ctx.get_uniform_cell_count());

    // Compute band alignment early - needed for phantom cell placement in uniform free scaling
    // When align=1.0, phantoms should be prepended (at start) so actual data is at the end (bottom)
    // When align=0.0, phantoms should be appended (at end) so actual data is at the start (top)
    let is_row_facet = DimConfig::channel_name() == RowDimensionConfig::channel_name();
    let band_align = determine_facet_band_align(is_row_facet, compiled_subplot);

    // Compute adjusted bandwidth for shared scale building when uniform_cell_count is set
    // This ensures shared Y axis extent matches actual subplot extent, not phantom cell space
    let adjusted_initial_bandwidth = if let Some(uniform_count) = early_uniform_cell_count {
        let domain_count = configured.config.domain.len();
        if uniform_count > domain_count && domain_count > 0 {
            // Scale down bandwidth proportionally: if domain has 1 element but uniform is 2,
            // the actual subplot is half the size, so bandwidth should be halved
            let adjusted = initial_bandwidth * (domain_count as f32 / uniform_count as f32);
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Adjusted initial_bandwidth for shared scales: {:.3} -> {:.3} (domain_count={}, uniform_count={})",
                    initial_bandwidth, adjusted, domain_count, uniform_count
                );
            }
            adjusted
        } else {
            initial_bandwidth
        }
    } else {
        initial_bandwidth
    };

    // Extract shared data extents from coordination context for nested facets
    // This allows inner facet scales to use the full dataset extent (not just filtered data)
    let shared_data_extents = coordination_context
        .as_ref()
        .and_then(|ctx| ctx.shared_data_extents.clone());

    // Extract per-row shared data extents for SharedInRow mode
    let shared_data_extents_by_row = coordination_context
        .as_ref()
        .and_then(|ctx| ctx.shared_data_extents_by_row.clone());

    // Extract per-column shared data extents for SharedInColumn mode
    let shared_data_extents_for_column = coordination_context
        .as_ref()
        .and_then(|ctx| ctx.shared_data_extents_for_column.clone());

    // ========== LEVEL-BASED DOMAIN EXTRACTION (Level(N) sharing) ==========
    // For channels with Level(N) sharing where N >= 1, extract domains from level_domains
    // This enables hierarchical scale sharing in nested facets using the new Level(N) API.
    let level_based_extents: HashMap<String, crate::facet::coordination::SerializableDataExtents> =
        if let Some(ref ctx) = coordination_context {
            scale_sharing_by_channel
                .iter()
                .filter_map(|(channel, mode)| {
                    // Check if this is a Level(N) mode with N >= 1
                    if let ScaleSharing::Level(n) = mode {
                        if *n >= 1 {
                            // Get the domain for this channel from level_domains
                            ctx.get_domain_for_channel(channel)
                                .map(|extents| (channel.clone(), extents.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            HashMap::new()
        };

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && !level_based_extents.is_empty() {
        eprintln!(
            "  Level-based extents extracted for channels: {:?}",
            level_based_extents.keys().collect::<Vec<_>>()
        );
    }

    // Determine shared-scale usage
    let any_shared = scale_sharing_by_channel
        .values()
        .any(|v| *v == ScaleSharing::Shared);

    // Check if any channels use Level(N) sharing with N >= 1
    let any_level_shared = scale_sharing_by_channel.values().any(|v| {
        if let ScaleSharing::Level(n) = v {
            *n >= 1
        } else {
            false
        }
    });

    // Build shared scale builder once if needed, extending with shared_data_extents
    // to ensure inner facet scales use the full dataset range for Shared channels
    // Also build if there are Level(N) channels to extend with level_based_extents
    let shared_scale_builder = if any_shared || any_level_shared {
        let mut builder = compiled_subplot
            .build_scale_builder_from_dataframe(&context.session_context, &context.params, df)
            .await?;

        // Extend with shared data extents ONLY for channels with ScaleSharing::Shared
        if let Some(ref extents) = shared_data_extents {
            let shared_only_extents: std::collections::HashMap<String, _> = extents
                .iter()
                .filter(|(channel, _)| {
                    scale_sharing_by_channel
                        .get(*channel)
                        .map(|mode| *mode == ScaleSharing::Shared)
                        .unwrap_or(false)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !shared_only_extents.is_empty() {
                builder.extend_with_shared_extents(&shared_only_extents);
            }
        }

        // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
        // These extents come from the parent facet's level_domains
        if !level_based_extents.is_empty() {
            builder.extend_with_shared_extents(&level_based_extents);
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Extended builder with level-based extents: {:?}",
                    level_based_extents.keys().collect::<Vec<_>>()
                );
            }
        }

        Some(builder)
    } else {
        None
    };

    // Build initial shared scales for Pass 1 using approximate band size
    // Use adjusted_initial_bandwidth to account for uniform_cell_count (phantom cells)
    let initial_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let (width, height) = subplot_dims(adjusted_initial_bandwidth, context);
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

    // Build fallback scale builder when empty cell handling is enabled
    // This is used when domain propagation creates cells for values not in the filtered data.
    // The fallback builder is built from the FULL dataset (before filtering) to provide
    // scales for empty cells, ensuring axes render correctly.
    let enable_empty_cell_fallback = coordination_context
        .as_ref()
        .map(|ctx| ctx.enable_empty_cell_fallback)
        .unwrap_or(false);
    let fallback_builder = if enable_empty_cell_fallback {
        Some(
            compiled_subplot
                .build_scale_builder_from_dataframe(&context.session_context, &context.params, df)
                .await?,
        )
    } else {
        None
    };

    use crate::facet::keys::FacetKeyExtractor;
    let has_coordination_domain = domain_from_coordination.is_some();
    let mut domain_vals = if let Some(coord_domain) = domain_from_coordination {
        // Use domain from coordination context (computed from full dataset by outer facet)
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Using coordination domain with {} values for channel={}",
                coord_domain.len(),
                DimConfig::channel_name()
            );
        }
        coord_domain
    } else {
        // Extract domain from current data (default behavior)
        let extracted = FacetKeyExtractor::extract_keys(df, facet_expr).await?;
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Extracted {} domain values from data for channel={}",
                extracted.len(),
                DimConfig::channel_name()
            );
        }
        extracted
    };
    domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Check if uniform free scaling is enabled via coordination context
    // If so, we need to use a virtual domain size for band sizing
    let uniform_cell_count = FacetCoordinationContext::from_params(&context.params)
        .and_then(|ctx| ctx.get_uniform_cell_count());

    // If we're using a coordination domain, rebuild the dimension scale with that domain.
    // NOTE: For uniform free scaling, we do NOT pad the domain with placeholder values because
    // that would cause placeholder labels to appear in the guides. Instead, we create a separate
    // temporary scale just for computing positions with correct band sizing.
    let effective_configured = if has_coordination_domain {
        // Build a new domain array from the coordination domain values (actual values only)
        use datafusion::arrow::array::StringArray;
        use std::sync::Arc as StdArc;
        let domain_strings: Vec<String> = domain_vals.iter().map(|v| v.to_string()).collect();
        let domain_array =
            StdArc::new(StringArray::from(domain_strings)) as datafusion::arrow::array::ArrayRef;

        // Create a new configured scale with the updated domain
        let updated = configured.clone().with_domain(domain_array);
        std::borrow::Cow::Owned(updated)
    } else {
        std::borrow::Cow::Borrowed(configured)
    };

    // For uniform free scaling, create a separate scale with padded domain just for position computation
    // This scale is NOT stored - it's only used to compute correct band positions
    let (all_positions, scale_domain_vals) = if let Some(uniform_count) = uniform_cell_count {
        if domain_vals.len() < uniform_count {
            // Create padded domain values for position computation only
            // When band_align >= 0.5, prepend phantoms so actual data ends up at end positions (bottom)
            // When band_align < 0.5, append phantoms so actual data stays at start positions (top)
            let num_phantoms = uniform_count - domain_vals.len();
            let placeholder_template = domain_vals.first().cloned().unwrap_or(ScalarValue::Utf8(None));
            let mut phantoms: Vec<ScalarValue> = (0..num_phantoms)
                .map(|i| match &placeholder_template {
                    ScalarValue::Utf8View(_) => {
                        ScalarValue::Utf8View(Some(format!("__placeholder_{}", i)))
                    }
                    ScalarValue::Utf8(_) => ScalarValue::Utf8(Some(format!("__placeholder_{}", i))),
                    _ => ScalarValue::Utf8(Some(format!("__placeholder_{}", i))),
                })
                .collect();

            let padded = if band_align >= 0.5 {
                // Prepend phantoms: [phantom0, phantom1, ..., actual0, actual1, ...]
                phantoms.extend(domain_vals.clone());
                phantoms
            } else {
                // Append phantoms: [actual0, actual1, ..., phantom0, phantom1, ...]
                let mut result = domain_vals.clone();
                result.extend(phantoms);
                result
            };

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Uniform Free scaling Pass1: creating temp scale with {} values (actual={}) band_align={:.2} prepend_phantoms={}",
                    uniform_count,
                    domain_vals.len(),
                    band_align,
                    band_align >= 0.5
                );
            }

            // Create temporary scale with padded domain for position computation
            use datafusion::arrow::array::StringArray;
            use std::sync::Arc as StdArc;
            let padded_strings: Vec<String> = padded.iter().map(|v| v.to_string()).collect();
            let padded_array =
                StdArc::new(StringArray::from(padded_strings)) as datafusion::arrow::array::ArrayRef;
            let temp_scale = configured.clone().with_domain(padded_array);

            // Compute positions for ALL values (including phantoms) so geometry gets correct band count
            let positions = temp_scale.scale_scalars_to_numeric(&padded)?;
            (positions, padded)
        } else {
            // No padding needed, use effective_configured directly
            let positions = effective_configured.scale_scalars_to_numeric(&domain_vals)?;
            (positions, domain_vals.clone())
        }
    } else {
        // No uniform sizing, use effective_configured directly
        let positions = effective_configured.scale_scalars_to_numeric(&domain_vals)?;
        (positions, domain_vals.clone())
    };

    use crate::facet::subplot_iterator::SubplotIteration;
    use crate::facet::subplot_iterator::SubplotIterator;

    // Compute phantom_prepend_count for coordination context BEFORE SubplotIterator
    // This enables correct FacetContext.position computation for axis label visibility
    let phantom_prepend_count = if band_align >= 0.5 {
        uniform_cell_count
            .map(|u| u.saturating_sub(domain_vals.len()))
            .unwrap_or(0)
    } else {
        0
    };

    // Update coordination context params with phantom_prepend_count
    let subplot_params = if phantom_prepend_count > 0 {
        // Get existing coordination context and update phantom_prepend_count
        if let Some(mut coord_ctx) =
            FacetCoordinationContext::from_params(&context.params)
        {
            coord_ctx.phantom_prepend_count = phantom_prepend_count;
            let mut updated_params = context.params.clone();
            updated_params.extend(coord_ctx.to_params());
            updated_params
        } else {
            context.params.clone()
        }
    } else {
        context.params.clone()
    };

    let subplot_iter = SubplotIterator::<DimConfig>::new(
        domain_vals.clone(),
        subplot_params.clone(),
        scale_sharing_by_channel.clone(),
    );

    let channel_name = DimConfig::channel_name();
    let mut position_channels_pass1 = HashMap::new();
    // Use ALL positions (including placeholders) so geometry gets correct band count
    position_channels_pass1.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(all_positions.clone()),
    );

    let mut position_values_pass1 = HashMap::new();
    // Use padded domain values so geometry knows the full domain size
    position_values_pass1.insert(channel_name, scale_domain_vals.clone());

    let initial_geometry = facet_coord.transform(
        &position_channels_pass1,
        Some(&position_values_pass1),
        context.plot_width,
        context.plot_height,
    )?;

    let all_initial_rects = initial_geometry
        .as_any()
        .downcast_ref::<SubplotGeometry>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected SubplotGeometry from facet coord transform".into(),
            )
        })?
        .rects
        .clone();

    // For uniform free scaling, we passed padded positions to get correct band sizes,
    // but we only iterate over actual data values. Filter rects to match iteration count.
    // When phantoms were prepended (band_align >= 0.5), take the LAST N rects.
    // When phantoms were appended (band_align < 0.5), take the FIRST N rects.
    let initial_rects = if uniform_cell_count.is_some() && all_initial_rects.len() > domain_vals.len() {
        let num_actual = domain_vals.len();
        let total = all_initial_rects.len();
        if band_align >= 0.5 {
            // Phantoms at start, actual data at end - take last N rects
            all_initial_rects.into_iter().skip(total - num_actual).collect()
        } else {
            // Phantoms at end, actual data at start - take first N rects
            all_initial_rects.into_iter().take(num_actual).collect()
        }
    } else {
        all_initial_rects
    };

    assert_eq!(
        subplot_iter.len(),
        initial_rects.len(),
        "SubplotIterator and initial_rects length mismatch in Pass 1"
    );

    let mut overflow_measurements = Vec::new();
    let mut guide_only_measurements = Vec::new();
    let mut all_legend_positions: HashSet<crate::legend::LegendPosition> = HashSet::new();

    // Small helper to measure one subplot to keep the parent future small
    // Returns (guide_only_overflow, total_overflow, legend_positions, spacing_needs)
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
            crate::guide::OverflowSpaceRequirement,
            HashSet<crate::legend::LegendPosition>,
            std::collections::HashMap<String, f32>, // spacing_needs from inner guide
        ),
        AvengerChartError,
    > {
        let (guide_only, total_overflow, _legend_info, legend_positions) = compiled_subplot
            .measure_with_scales(width, height, ctx, params, scales, Some(filter_df))
            .await?;
        // Also get spacing_needs from inner guide's measure_with_coordination
        let spacing_needs = compiled_subplot
            .get_guide_spacing_needs(width, height, ctx, params, scales, Some(filter_df))
            .await?;
        // Return both overflows: guide_only for alignment, total for spacing, plus spacing_needs
        Ok((guide_only, total_overflow, legend_positions, spacing_needs))
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

    // Total count for coordination context outer_count
    // When uniform sizing adds phantom cells, outer_count should include them
    // so that guide ownership (axis label visibility) is computed correctly.
    // IMPORTANT: Only use uniform_cell_count if it's for THIS facet's dimension.
    // If inner_channel doesn't match our channel, the uniform sizing is for a nested facet.
    let outer_uniform_cell_count = FacetCoordinationContext::from_params(&context.params)
        .and_then(|ctx| {
            // Check if uniform sizing is for this facet's dimension
            if ctx.inner_channel.as_deref() == Some(DimConfig::channel_name()) {
                ctx.get_uniform_cell_count()
            } else {
                // Uniform sizing is for a different dimension (nested facet), not us
                None
            }
        });
    let outer_count = outer_uniform_cell_count.unwrap_or(work_items.len());
    // Use phantom_prepend_count computed earlier for position adjustment
    let phantom_offset = phantom_prepend_count;

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let subplot_dims = subplot_dims.clone();
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            // Start with subplot_params which has updated phantom_prepend_count
            let mut params_base = subplot_params.clone();
            // Merge iteration params (these take precedence for iteration-specific values)
            params_base.extend(iteration.params.clone());
            // Update coordination context with outer position for this iteration
            // When phantoms are prepended, adjust idx to reflect rendered position
            let adjusted_idx = idx + phantom_offset;
            let params_base = FacetCoordinationContext::update_outer_position_in_params(
                &params_base,
                adjusted_idx,
                outer_count,
            );
            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let initial_shared_scales = initial_shared_scales.clone();
            let fallback_builder = fallback_builder.clone();
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
                    // All scales are Free - build from filtered data without shared extent extension
                    let builder = compiled_subplot
                        .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
                        .await?;
                    Some(builder)
                };

                let scales = build_scales_helper_with_fallback(
                    &compiled_subplot,
                    &initial_shared_scales,
                    &free_scale_builder_pass1,
                    &fallback_builder,
                    &filter_df,
                    width,
                    height,
                    &ctx,
                    &params_base,
                )
                .await?;

                let (guide_only, total_overflow, legend_positions, spacing_needs) = measure_subplot(
                    &compiled_subplot,
                    width,
                    height,
                    &ctx,
                    &params_base,
                    &scales,
                    &filter_df,
                )
                .await?;

                Ok::<_, AvengerChartError>((idx, guide_only, total_overflow, legend_positions, spacing_needs))
            }
        })
        .buffer_unordered(MAX_CONCURRENT_MEASURE)
        .collect::<Vec<_>>()
        .await;

    // Collect results in index order for determinism.
    let mut sorted = results
        .into_iter()
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    sorted.sort_by_key(|(idx, _, _, _, _)| *idx);

    // Aggregate spacing_needs from all subplots using max per key
    let mut aggregated_spacing_needs: std::collections::HashMap<String, f32> =
        std::collections::HashMap::new();

    for (_, guide_only, total_overflow, legend_positions, spacing_needs) in sorted {
        guide_only_measurements.push(guide_only);
        overflow_measurements.push(total_overflow);
        all_legend_positions.extend(legend_positions);

        // Aggregate spacing_needs: use max for each key
        for (key, value) in spacing_needs {
            aggregated_spacing_needs
                .entry(key)
                .and_modify(|existing| *existing = existing.max(value))
                .or_insert(value);
        }
    }

    // Compute global maximum guide-only overflow for unified alignment
    let global_max_overflow = {
        let mut max_overflow = crate::guide::OverflowSpaceRequirement::default();
        for overflow in &guide_only_measurements {
            max_overflow.top = max_overflow.top.max(overflow.top);
            max_overflow.bottom = max_overflow.bottom.max(overflow.bottom);
            max_overflow.left = max_overflow.left.max(overflow.left);
            max_overflow.right = max_overflow.right.max(overflow.right);
        }
        max_overflow
    };

    let mut max_required_gap = 0.0f32;
    for i in 0..overflow_measurements.len().saturating_sub(1) {
        let gap = DimConfig::calculate_adjacent_overflow(
            &overflow_measurements[i],
            &overflow_measurements[i + 1],
        );
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "measure_pass gap[{}->{}]: gap={:.3} overflow[{}]={{top={:.3},bottom={:.3},left={:.3},right={:.3}}} overflow[{}]={{top={:.3},bottom={:.3},left={:.3},right={:.3}}}",
                i,
                i + 1,
                gap,
                i,
                overflow_measurements[i].top,
                overflow_measurements[i].bottom,
                overflow_measurements[i].left,
                overflow_measurements[i].right,
                i + 1,
                overflow_measurements[i + 1].top,
                overflow_measurements[i + 1].bottom,
                overflow_measurements[i + 1].left,
                overflow_measurements[i + 1].right
            );
        }
        max_required_gap = max_required_gap.max(gap);
    }
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass final: channel={} max_required_gap={:.3} num_subplots={}",
            DimConfig::channel_name(),
            max_required_gap,
            overflow_measurements.len()
        );
    }

    // Check if outer facet provided a pre-computed gap via coordination context
    // For FacetRow: check coordinated_spacing["inter_row_gap"] (computed by outer FacetColumn)
    // For FacetColumn: check coordinated_spacing["inter_col_gap"] (computed by outer FacetRow)
    let coordinated_gap_key = if DimConfig::is_row_facet() {
        "inter_row_gap"
    } else {
        "inter_col_gap"
    };
    let measured_gap_from_outer = coordination_context
        .as_ref()
        .and_then(|ctx| ctx.get_coordinated_spacing(coordinated_gap_key));

    let rounded_gap = if let Some(measured_gap) = measured_gap_from_outer {
        // Use the pre-computed gap from outer facet's coordination context
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Using coordinated_spacing['{}'] from coordination context: {:.1} (own computed: {:.1})",
                coordinated_gap_key, measured_gap, max_required_gap
            );
        }
        measured_gap
    } else {
        // Compute gap from this facet's own overflow measurements
        let spacing = if let Some(explicit_spacing) = facet_spacing {
            explicit_spacing
        } else if let Some(inner_spacing) = coordination_context
            .as_ref()
            .and_then(|ctx| ctx.inner_facet_spacing)
        {
            // For nested facets, use the inner facet's spacing for the outer dimension too.
            // This matches grid facet behavior where a single spacing value applies to both dimensions.
            inner_spacing
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
        (max_required_gap + spacing).ceil()
    };

    let padding_spec = crate::coords::PaddingSpec::Single {
        padding_px: rounded_gap,
        overflow: overflow_measurements.clone(),
    };
    let updated_facet_coord = facet_coord.with_measured_padding(&padding_spec);

    // Rebuild the facet dimension scale with measured padding
    // IMPORTANT: Use effective_configured (which includes coordination domain) rather than
    // dimension_scale.configured() to ensure the final scale has the correct domain
    let mut new_config = effective_configured.config.clone();
    new_config.options.insert(
        "padding_inner_px".to_string(),
        Scalar::from_f32(rounded_gap),
    );

    // Use the band_align computed earlier (needed for phantom cell placement)
    // Log it here for debugging visibility
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass band alignment: channel={} is_row_facet={} band_align={:.3}",
            DimConfig::channel_name(),
            is_row_facet,
            band_align
        );
    }
    new_config
        .options
        .insert("align".to_string(), Scalar::from_f32(band_align));

    let updated_spec = dimension_scale
        .spec()
        .clone()
        .option(
            "padding_inner_px",
            lit(ScalarValue::Float32(Some(rounded_gap))),
        )
        .option("align", lit(ScalarValue::Float32(Some(band_align))));

    let updated_configured = avenger_scales::scales::ConfiguredScale {
        scale_impl: effective_configured.scale_impl.clone(),
        config: new_config,
    };

    let final_dimension_scale = ConfiguredScaleWithSpec::new(updated_spec, updated_configured);

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass building final_dimension_scale: channel={} rounded_gap={:.3}",
            DimConfig::channel_name(),
            rounded_gap
        );
    }

    let mut updated_scales = HashMap::with_capacity(1);
    updated_scales.insert(
        DimConfig::channel_name().to_string(),
        final_dimension_scale.clone(),
    );

    let final_configured = final_dimension_scale.configured();

    // For uniform free scaling, create a temporary scale with padded domain for position computation
    // This mirrors the approach in Pass 1 - the final_dimension_scale has only actual values,
    // but we need positions computed as if there were uniform_count bands
    let (final_all_positions, final_scale_domain_vals) = if let Some(uniform_count) = uniform_cell_count {
        if domain_vals.len() < uniform_count {
            // Create temporary scale with padded domain for position computation
            use datafusion::arrow::array::StringArray;
            use std::sync::Arc as StdArc;
            let padded_strings: Vec<String> = scale_domain_vals.iter().map(|v| v.to_string()).collect();
            let padded_array =
                StdArc::new(StringArray::from(padded_strings)) as datafusion::arrow::array::ArrayRef;

            // Clone the final_configured config and create temp scale with padded domain
            let mut temp_config = final_configured.config.clone();
            temp_config.domain = padded_array;
            let temp_scale = avenger_scales::scales::ConfiguredScale {
                scale_impl: final_configured.scale_impl.clone(),
                config: temp_config,
            };
            let positions = temp_scale.scale_scalars_to_numeric(&scale_domain_vals)?;
            (positions, scale_domain_vals.clone())
        } else {
            let positions = final_configured.scale_scalars_to_numeric(&domain_vals)?;
            (positions, domain_vals.clone())
        }
    } else {
        let positions = final_configured.scale_scalars_to_numeric(&domain_vals)?;
        (positions, domain_vals.clone())
    };

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass final_positions: channel={} positions={:?}",
            DimConfig::channel_name(),
            final_all_positions,
        );
    }

    let mut temp_position_channels = HashMap::new();
    // Use ALL positions (including placeholders) so geometry gets correct band count
    temp_position_channels.insert(
        channel_name,
        avenger_common::value::ScalarOrArray::new_array(final_all_positions),
    );

    let mut temp_position_values = HashMap::new();
    // Use padded domain values so geometry knows the full domain size
    temp_position_values.insert(channel_name, final_scale_domain_vals.clone());

    let final_geometry_with_padding = updated_facet_coord.transform(
        &temp_position_channels,
        Some(&temp_position_values),
        context.plot_width,
        context.plot_height,
    )?;

    let all_final_rects_raw = final_geometry_with_padding
        .as_any()
        .downcast_ref::<SubplotGeometry>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected SubplotGeometry from facet coord transform in Pass 2".into(),
            )
        })?
        .rects
        .clone();

    // For uniform free scaling, filter to only actual data rects (exclude placeholder rects)
    // When phantoms were prepended (band_align >= 0.5), take the LAST N rects.
    // When phantoms were appended (band_align < 0.5), take the FIRST N rects.
    let mut final_rects: Vec<SubplotRect> = if uniform_cell_count.is_some() && all_final_rects_raw.len() > domain_vals.len() {
        let num_actual = domain_vals.len();
        let total = all_final_rects_raw.len();
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  rect selection Pass2: channel={} total={} num_actual={} band_align={:.2} all_rects_y={:?}",
                DimConfig::channel_name(),
                total,
                num_actual,
                band_align,
                all_final_rects_raw.iter().map(|r| r.y).collect::<Vec<_>>()
            );
        }
        let result = if band_align >= 0.5 {
            // Phantoms at start, actual data at end - take last N rects
            all_final_rects_raw.into_iter().skip(total - num_actual).collect::<Vec<_>>()
        } else {
            // Phantoms at end, actual data at start - take first N rects
            all_final_rects_raw.into_iter().take(num_actual).collect::<Vec<_>>()
        };
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  rect selection Pass2 result: channel={} selected_rects_y={:?}",
                DimConfig::channel_name(),
                result.iter().map(|r| r.y).collect::<Vec<_>>()
            );
        }
        result
    } else {
        all_final_rects_raw
    };

    // Adjust the last rect to fill remaining space, avoiding rounding gaps
    // This ensures the last band reaches exactly to the plot boundary
    // NOTE: For uniform sizing, we DON'T adjust the last rect since empty space should appear below
    if uniform_cell_count.is_none() {
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
    }

    let final_shared_scales = if let Some(ref builder) = shared_scale_builder {
        let final_bandwidth = band::bandwidth(&final_dimension_scale.configured().config)?;

        // Apply same uniform_cell_count adjustment as for initial_shared_scales
        let adjusted_final_bandwidth = if let Some(uniform_count) = early_uniform_cell_count {
            let final_domain_count = final_dimension_scale.configured().config.domain.len();
            if uniform_count > final_domain_count && final_domain_count > 0 {
                let adjusted = final_bandwidth * (final_domain_count as f32 / uniform_count as f32);
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Adjusted final_bandwidth for shared scales: {:.3} -> {:.3} (domain_count={}, uniform_count={})",
                        final_bandwidth, adjusted, final_domain_count, uniform_count
                    );
                }
                adjusted
            } else {
                final_bandwidth
            }
        } else {
            final_bandwidth
        };

        let (width, height) = subplot_dims(adjusted_final_bandwidth, context);

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

    // Compute inter_row_gap for FacetColumn with nested FacetRow
    // This requires 2D overflow analysis across all columns
    // Skip if inter_row_gap is already in coordinated_spacing (to prevent infinite recursion in two-phase measurement)
    let measured_row_gap = if DimConfig::is_col_facet() {
        // Check if coordination context indicates a nested FacetRow
        let coord_ctx = FacetCoordinationContext::from_params(&context.params);

        // If inter_row_gap is already in coordinated_spacing, we're in the second pass - don't recompute
        let already_computed = coord_ctx
            .as_ref()
            .map(|ctx| ctx.get_coordinated_spacing("inter_row_gap").is_some())
            .unwrap_or(false);

        if already_computed {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  measure_pass for FacetColumn: skipping inter_row_gap computation (already in coordinated_spacing)"
                );
            }
            None
        } else {
            let has_nested_row_facet = coord_ctx
                .as_ref()
                .map(|ctx| ctx.inner_channel.as_deref() == Some("row"))
                .unwrap_or(false);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  measure_pass for FacetColumn: has_nested_row_facet={} overflow_measurements.len()={} coord_ctx={:?}",
                    has_nested_row_facet,
                    overflow_measurements.len(),
                    coord_ctx.as_ref().map(|c| c.inner_channel.as_deref())
                );
            }

            if has_nested_row_facet && !overflow_measurements.is_empty() {
                // Get inner domain count (number of rows) from coordination context
                let inner_domain_count = coord_ctx
                    .as_ref()
                    .map(|ctx| ctx.inner_domain_count)
                    .unwrap_or(0);

                // Check if uniform Free scaling is enabled
                // When uniform Free scaling is active, we need to coordinate inter_row_gap
                // even for columns with only 1 row, so they use the same gap as other columns
                let uniform_cell_count = coord_ctx
                    .as_ref()
                    .and_then(|ctx| ctx.get_uniform_cell_count());

                if inner_domain_count > 1 || uniform_cell_count.is_some() {
                    // Extract inter_row_gap from aggregated spacing_needs (computed by inner FacetRowGuide)
                    // This is the correct gap computed using calculate_inter_row_gap which does:
                    // max(bottom[row_i] + top[row_i+1]) + spacing
                    if let Some(&gap) = aggregated_spacing_needs
                        .get(crate::guide::spacing_keys::INTER_ROW_GAP)
                    {
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "  Extracted measured_row_gap from inner guide: {:.1} (inner_domain_count={}, uniform_cell_count={:?})",
                                gap, inner_domain_count, uniform_cell_count
                            );
                        }
                        Some(gap)
                    } else {
                        // Fallback: no spacing_needs from inner guide (shouldn't happen for properly nested facets)
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "  No inter_row_gap in aggregated_spacing_needs, using None"
                            );
                        }
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }
    } else {
        None
    };

    // Compute inter_col_gap for FacetRow with nested FacetColumn (symmetric with inter_row_gap)
    let measured_col_gap = if DimConfig::is_row_facet() {
        let coord_ctx = FacetCoordinationContext::from_params(&context.params);

        // If inter_col_gap is already in coordinated_spacing, we're in the second pass - don't recompute
        let already_computed = coord_ctx
            .as_ref()
            .map(|ctx| ctx.get_coordinated_spacing("inter_col_gap").is_some())
            .unwrap_or(false);

        if already_computed {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  measure_pass for FacetRow: skipping inter_col_gap computation (already in coordinated_spacing)"
                );
            }
            None
        } else {
            let has_nested_col_facet = coord_ctx
                .as_ref()
                .map(|ctx| ctx.inner_channel.as_deref() == Some("column"))
                .unwrap_or(false);

            if has_nested_col_facet && !overflow_measurements.is_empty() {
                let inner_domain_count = coord_ctx
                    .as_ref()
                    .map(|ctx| ctx.inner_domain_count)
                    .unwrap_or(0);

                // Check if uniform Free scaling is enabled
                // When uniform Free scaling is active, we need to coordinate inter_col_gap
                // even for rows with only 1 column, so they use the same gap as other rows
                let uniform_cell_count = coord_ctx
                    .as_ref()
                    .and_then(|ctx| ctx.get_uniform_cell_count());

                if inner_domain_count > 1 || uniform_cell_count.is_some() {
                    // Extract inter_col_gap from aggregated spacing_needs (computed by inner FacetColGuide)
                    // This is the correct gap computed using calculate_inter_col_gap which does:
                    // max(right[col_i] + left[col_i+1]) + spacing
                    if let Some(&gap) = aggregated_spacing_needs
                        .get(crate::guide::spacing_keys::INTER_COL_GAP)
                    {
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "  Extracted measured_col_gap from inner guide: {:.1} (inner_domain_count={}, uniform_cell_count={:?})",
                                gap, inner_domain_count, uniform_cell_count
                            );
                        }
                        Some(gap)
                    } else {
                        // Fallback: no spacing_needs from inner guide (shouldn't happen for properly nested facets)
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "  No inter_col_gap in aggregated_spacing_needs, using None"
                            );
                        }
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }
    } else {
        None
    };

    // Compute spacing_needs to report this facet's gap requirement for coordination
    // Inner facets report their computed gap; outer facets aggregate and pass back via coordination context
    let mut spacing_needs = HashMap::new();
    let spacing_key = if DimConfig::is_row_facet() {
        "inter_row_gap"
    } else {
        "inter_col_gap"
    };
    spacing_needs.insert(spacing_key.to_string(), rounded_gap);

    // If this outer facet computed a gap for its nested inner facet, also add that to spacing_needs
    // FacetColumn computes measured_row_gap for nested FacetRow
    if let Some(row_gap) = measured_row_gap {
        spacing_needs.insert("inter_row_gap".to_string(), row_gap);
    }
    // FacetRow computes measured_col_gap for nested FacetColumn
    if let Some(col_gap) = measured_col_gap {
        spacing_needs.insert("inter_col_gap".to_string(), col_gap);
    }

    // Add legend overflow for cross-subplot legend alignment
    // These values represent the maximum guide-only overflow across all subplots
    // for the legend positions that are used
    use crate::legend::LegendPosition;
    if all_legend_positions.contains(&LegendPosition::Right) {
        spacing_needs.insert("legend_right".to_string(), global_max_overflow.right);
    }
    if all_legend_positions.contains(&LegendPosition::Left) {
        spacing_needs.insert("legend_left".to_string(), global_max_overflow.left);
    }
    if all_legend_positions.contains(&LegendPosition::Top) {
        spacing_needs.insert("legend_top".to_string(), global_max_overflow.top);
    }
    if all_legend_positions.contains(&LegendPosition::Bottom) {
        spacing_needs.insert("legend_bottom".to_string(), global_max_overflow.bottom);
    }

    // Add shared overflow for uniform padding across columns (for nested facets)
    // This ensures plot areas align across all columns, even when scales are not shared.
    // We only add these when there's a nested facet (indicated by measured cross-dimension gap)
    // to avoid unnecessary re-measurement for standalone facets.
    let has_nested_facet = measured_row_gap.is_some() || measured_col_gap.is_some();
    if has_nested_facet {
        // Use guide-only overflow (not total) to avoid legend interference
        spacing_needs.insert("shared_overflow_left".to_string(), global_max_overflow.left);
        spacing_needs.insert("shared_overflow_right".to_string(), global_max_overflow.right);
        spacing_needs.insert("shared_overflow_top".to_string(), global_max_overflow.top);
        spacing_needs.insert("shared_overflow_bottom".to_string(), global_max_overflow.bottom);
    }

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass returning final_rects: channel={} rects={:?}",
            DimConfig::channel_name(),
            final_rects.iter().map(|r| (r.x, r.width)).collect::<Vec<_>>()
        );
    }

    Ok(FacetPass1Result {
        overflow_measurements,
        final_dimension_scale,
        final_shared_scales,
        final_rects,
        fallback_builder,
        shared_data_extents,
        shared_data_extents_by_row,
        shared_data_extents_for_column,
        spacing_needs,
        uniform_cell_count,
        phantom_offset,
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

    // Update coordination context with phantom_prepend_count from pass1 for render pass
    let subplot_params_pass2 = if pass1.phantom_offset > 0 {
        if let Some(mut coord_ctx) = FacetCoordinationContext::from_params(&context.params) {
            coord_ctx.phantom_prepend_count = pass1.phantom_offset;
            let mut updated_params = context.params.clone();
            updated_params.extend(coord_ctx.to_params());
            updated_params
        } else {
            context.params.clone()
        }
    } else {
        context.params.clone()
    };

    let subplot_iter_pass2 = SubplotIterator::<DimConfig>::new(
        domain_vals_final,
        subplot_params_pass2.clone(),
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
        // For faceted subplots, use build_plot_components directly.
        // The dimensions are already final from facet layout, and scales are pre-coordinated.
        // The measure_for_render pattern is designed for top-level plots, not subplots.
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

    // Total count for coordination context outer_count
    // Use uniform_cell_count if available to include phantom cells in guide ownership calculation
    // IMPORTANT: Only use uniform_cell_count if it's for THIS facet's dimension.
    // If inner_channel doesn't match our channel, the uniform sizing is for a nested facet.
    let outer_uniform_cell_count = FacetCoordinationContext::from_params(&context.params)
        .and_then(|ctx| {
            // Check if uniform sizing is for this facet's dimension
            if ctx.inner_channel.as_deref() == Some(DimConfig::channel_name()) {
                ctx.get_uniform_cell_count()
            } else {
                // Uniform sizing is for a different dimension (nested facet), not us
                None
            }
        });
    let outer_count = outer_uniform_cell_count.unwrap_or(work_items.len());
    // Phantom offset from pass1 for adjusting subplot positions
    let phantom_offset = pass1.phantom_offset;

    // Clone fallback builder for render pass
    let fallback_builder = pass1.fallback_builder.clone();

    // Wrap shared_data_extents_by_row in Arc to avoid cloning the large HashMap for each work item
    // This reduces async future size and prevents stack overflow
    let shared_data_extents_by_row: Arc<
        Option<
            std::collections::HashMap<
                String,
                std::collections::HashMap<
                    String,
                    crate::facet::coordination::SerializableDataExtents,
                >,
            >,
        >,
    > = Arc::new(pass1.shared_data_extents_by_row.clone());

    // Wrap shared_data_extents_for_column in Arc for same reasons
    let shared_data_extents_for_column: Arc<
        Option<
            std::collections::HashMap<String, crate::facet::coordination::SerializableDataExtents>,
        >,
    > = Arc::new(pass1.shared_data_extents_for_column.clone());

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            // Start with subplot_params_pass2 which has updated phantom_prepend_count
            let mut params_base = subplot_params_pass2.clone();
            // Merge iteration params (these take precedence for iteration-specific values)
            params_base.extend(iteration.params.clone());
            // Update coordination context with outer position for this iteration
            // When phantoms are prepended, adjust idx to reflect rendered position
            let adjusted_idx = idx + phantom_offset;
            let params_base =
                FacetCoordinationContext::update_outer_position_in_params(&params_base, adjusted_idx, outer_count);
            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let final_shared_scales = pass1.final_shared_scales.clone();
            let fallback_builder = fallback_builder.clone();
            let shared_data_extents = pass1.shared_data_extents.clone();
            // Clone Arc references (cheap) instead of the large HashMaps
            let shared_data_extents_by_row = Arc::clone(&shared_data_extents_by_row);
            let shared_data_extents_for_column = Arc::clone(&shared_data_extents_for_column);
            let df = df.clone();
            let overflow_dbg = pass1
                .overflow_measurements
                .get(idx)
                .cloned()
                .unwrap_or_default();
            let semaphore = Arc::clone(&semaphore);

            // Box::pin the async block to heap-allocate the future, reducing stack usage
            // This prevents stack overflow in deeply nested facet evaluations
            Box::pin(async move {
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

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "render_pass subplot: idx={} rect.x={:.1} rect.width={:.1} -> width={:.1}",
                        idx, rect.x, rect.width, width
                    );
                }

                let mut free_scale_builder = compiled_subplot
                    .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
                    .await?;
                // Extend with shared data extents ONLY for channels with ScaleSharing::Shared
                // For Free scales, we want the local (filtered) data range, not the full dataset range
                if let Some(ref extents) = shared_data_extents {
                    // Filter shared_data_extents to only include channels with Shared mode
                    let shared_only_extents: std::collections::HashMap<String, _> = extents
                        .iter()
                        .filter(|(channel, _)| {
                            scale_sharing_by_channel
                                .get(*channel)
                                .map(|mode| *mode == ScaleSharing::Shared)
                                .unwrap_or(false)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if !shared_only_extents.is_empty() {
                        free_scale_builder.extend_with_shared_extents(&shared_only_extents);
                    }
                }

                // Extend with per-row shared data extents for channels with ScaleSharing::SharedInRow
                // These extents were computed at the outer facet level for each row value
                if let Some(ref extents_by_row) = *shared_data_extents_by_row {
                    let row_key = scalar_to_string_key(&iteration.facet_value);
                    if let Some(row_extents) = extents_by_row.get(&row_key) {
                        // Filter to only SharedInRow channels
                        let shared_in_row_extents: std::collections::HashMap<String, _> = row_extents
                            .iter()
                            .filter(|(channel, _)| {
                                scale_sharing_by_channel
                                    .get(*channel)
                                    .map(|mode| *mode == ScaleSharing::SharedInRow)
                                    .unwrap_or(false)
                            })
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        if !shared_in_row_extents.is_empty() {
                            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "SharedInRow: applying extents for row_key={} channels={:?}",
                                    row_key,
                                    shared_in_row_extents.keys().collect::<Vec<_>>()
                                );
                            }
                            free_scale_builder.extend_with_shared_extents(&shared_in_row_extents);
                        }
                    }
                }

                // Extend with per-column shared data extents for channels with ScaleSharing::SharedInColumn
                // These extents were computed at the outer facet level for this column
                if let Some(ref col_extents) = *shared_data_extents_for_column {
                    // Filter to only SharedInColumn channels
                    let shared_in_col_extents: std::collections::HashMap<String, _> = col_extents
                        .iter()
                        .filter(|(channel, _)| {
                            scale_sharing_by_channel
                                .get(*channel)
                                .map(|mode| *mode == ScaleSharing::SharedInColumn)
                                .unwrap_or(false)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if !shared_in_col_extents.is_empty() {
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "SharedInColumn: applying extents for channels={:?}",
                                shared_in_col_extents.keys().collect::<Vec<_>>()
                            );
                        }
                        free_scale_builder.extend_with_shared_extents(&shared_in_col_extents);
                    }
                }

                // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
                // Extract from coordination context (which was updated per-iteration)
                if let Some(coord_ctx) = FacetCoordinationContext::from_params(&params_base) {
                    let level_extents: std::collections::HashMap<String, _> = scale_sharing_by_channel
                        .iter()
                        .filter_map(|(channel, mode)| {
                            if let ScaleSharing::Level(n) = mode {
                                if *n >= 1 {
                                    coord_ctx.get_domain_for_channel(channel)
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
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "Level(N): applying extents for channels={:?}",
                                level_extents.keys().collect::<Vec<_>>()
                            );
                        }
                        free_scale_builder.extend_with_shared_extents(&level_extents);
                    }
                }

                // Compute SharedInColumn extents to pass to inner facets BEFORE moving free_scale_builder
                // These are computed from the column's filtered data and apply to all rows in this column
                let shared_in_column_channels: Vec<&str> = scale_sharing_by_channel
                    .iter()
                    .filter(|(_, mode)| **mode == ScaleSharing::SharedInColumn)
                    .map(|(ch, _)| ch.as_str())
                    .collect();

                let column_extents_for_inner = if !shared_in_column_channels.is_empty() {
                    let extents = free_scale_builder.extract_serializable_extents(&shared_in_column_channels);
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && !extents.is_empty() {
                        eprintln!(
                            "SharedInColumn: computed extents for column={:?} channels={:?}",
                            iteration.facet_value,
                            extents.keys().collect::<Vec<_>>()
                        );
                    }
                    Some(extents)
                } else {
                    None
                };

                let mut scales = build_scales_helper_with_fallback(
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
                    &fallback_builder,
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
                    let facet_scales = build_scales_helper_with_fallback(
                        &compiled_subplot,
                        &None,
                        &Some(free_scale_builder),
                        &fallback_builder,
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

                // Add unified overflow params for cross-subplot legend alignment.
                // These override the measured overflow to ensure all subplots use
                // identical overflow, causing legends to naturally align during layout.
                let mut merged_params = params_base.clone();

                // Pass SharedInColumn extents to inner facets (precomputed before free_scale_builder was moved)
                if let Some(column_extents) = column_extents_for_inner {
                    if !column_extents.is_empty() {
                        merged_params = FacetCoordinationContext::update_shared_data_extents_for_column_in_params(
                            &merged_params,
                            column_extents,
                        );
                    }
                }

                // Note: Legend alignment is now handled via coordinated_spacing in the coordination context
                // (legend_right, legend_left, legend_top, legend_bottom keys).
                // The rendering code reads from coordinated_spacing first, then falls back to
                // __unified_overflow_* params for backward compatibility with non-faceted subplots.
                // The legacy param injection has been removed since the named spacing system handles it.

                let components = render_subplot(
                    &compiled_subplot,
                    width,
                    height,
                    &ctx,
                    &merged_params,
                    &scale_provider,
                    &filter_df,
                )
                .await?;

                let position = if DimConfig::is_row_facet() {
                    rect.y
                } else {
                    rect.x
                };
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "render_pass subplot: idx={} rect.y={:.1} rect.x={:.1} position={:.1} is_row={}",
                        idx, rect.y, rect.x, position, DimConfig::is_row_facet()
                    );
                }
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
            })
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

    let mut scale_sharing_by_channel: HashMap<String, ScaleSharing> = scale_sharing_by_channel
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

    // Merge axis_scale_sharing from incoming coordination context (from outer facet)
    // This enables inner facets to know the x/y scale sharing for Cartesian subplot measurement
    if let Some(incoming_coord_ctx) = FacetCoordinationContext::from_params(&context.params) {
        if let Some(ref axis_sharing) = incoming_coord_ctx.axis_scale_sharing {
            for (channel, mode) in axis_sharing {
                // Only add if not already present (don't override explicit channel settings)
                scale_sharing_by_channel
                    .entry(channel.clone())
                    .or_insert(*mode);
            }
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Merged axis_scale_sharing from coordination context: {:?}",
                    axis_sharing
                );
            }
        }
    }

    // Detect nested facets and compute coordination context
    // This enables guide ownership coordination (axis label visibility) for nested facets.
    // Note: We do NOT propagate domain by default - let each inner facet use its own filtered domain.
    // Domain propagation (for grid-like behavior) can be enabled via explicit configuration.
    let coordination_context = detect_nested_facet_and_compute_coordination(
        compiled_subplot,
        &df,
        ctx,
        DimConfig::is_row_facet(), // Whether this (outer) facet is a row facet
        &facet_expr,               // Outer facet's expression for inner cell counting
    )
    .await?;

    // Create modified context with coordination params if a nested facet was detected
    let modified_context: Option<RenderContext>;
    let effective_context: &RenderContext = if let Some(coord_ctx) = coordination_context {
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Injecting coordination context for nested facet: inner_domain_len={:?}, scale_sharing={:?}",
                coord_ctx.inner_domain.as_ref().map(|d| d.len()),
                coord_ctx.inner_scale_sharing
            );
        }
        let mut new_params = context.params.clone();
        new_params.extend(coord_ctx.to_params());
        modified_context = Some(RenderContext::new(
            context.theme.clone(),
            context.plot_width,
            context.plot_height,
            context.session_context.clone(),
            new_params,
            context.scales.clone(),
        ));
        modified_context.as_ref().unwrap()
    } else {
        // No nested facet detected, use original context directly
        context
    };

    // Phase 1: Initial measurement to compute measured_row_gap for nested FacetRow
    let pass1 = measure_pass::<DimConfig, _>(
        facet_coord,
        compiled_subplot,
        dimension_scale,
        &scale_sharing_by_channel,
        &df,
        &facet_expr,
        facet_spacing,
        effective_context,
        &subplot_dims,
    )
    .await?;

    // Phase 1.5: Build coordination context with spacing values for rendering
    //
    // Two cases:
    // 1. Cross-dimension gaps (nested facets): Re-run measure_pass so inner facets can use coordinated gaps
    //    - FacetColumn with nested FacetRow: spacing_needs contains "inter_row_gap"
    //    - FacetRow with nested FacetColumn: spacing_needs contains "inter_col_gap"
    // 2. Only own-dimension spacing_needs (standalone facets): Just pass coordination context to render_pass
    //
    // Cross-dimension gap detection: check for gap key that would only exist from 2D overflow analysis
    let has_cross_dimension_gap = if DimConfig::is_col_facet() {
        // FacetColumn: check for inter_row_gap (from nested FacetRow)
        pass1.spacing_needs.contains_key("inter_row_gap")
    } else {
        // FacetRow: check for inter_col_gap (from nested FacetColumn)
        pass1.spacing_needs.contains_key("inter_col_gap")
    };
    let has_spacing_needs = !pass1.spacing_needs.is_empty();

    let (final_pass, final_context) = if has_cross_dimension_gap {
        // Nested facets: need to re-run measure_pass with coordination context
        let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
            .unwrap_or_default();

        // Apply coordinated_spacing from spacing_needs (contains both dimensions' gaps)
        coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Re-running measure_pass with coordinated_spacing={:?}",
                pass1.spacing_needs
            );
        }

        let mut new_params = context.params.clone();
        new_params.extend(coord_ctx.to_params());

        let updated_ctx = RenderContext::new(
            context.theme.clone(),
            context.plot_width,
            context.plot_height,
            context.session_context.clone(),
            new_params,
            context.scales.clone(),
        );

        // Re-run measure_pass with updated context so inner facets use correct gap
        // IMPORTANT: Use pass1.final_dimension_scale which has padding_inner_px applied,
        // so subplot widths will be correct (e.g., 163px instead of 165px with 3px gap)
        //
        // Also create an updated facet_coord with padding_px so that compute_band_layout
        // correctly subtracts the gap from the step size to get the actual bandwidth.
        let padding_inner_px = pass1
            .final_dimension_scale
            .configured()
            .config
            .options
            .get("padding_inner_px")
            .and_then(|v| v.as_f32().ok())
            .unwrap_or(0.0);
        let updated_padding_spec = crate::coords::PaddingSpec::Single {
            padding_px: padding_inner_px,
            overflow: pass1.overflow_measurements.clone(),
        };
        let updated_facet_coord = facet_coord.with_measured_padding(&updated_padding_spec);

        let pass2 = measure_pass::<DimConfig, _>(
            updated_facet_coord.as_ref(),
            compiled_subplot,
            &pass1.final_dimension_scale,
            &scale_sharing_by_channel,
            &df,
            &facet_expr,
            facet_spacing,
            &updated_ctx,
            &subplot_dims,
        )
        .await?;

        (pass2, updated_ctx)
    } else if has_spacing_needs {
        // Standalone facets: just pass coordination context for legend alignment in render_pass
        // No need to re-run measure_pass since there are no inner facets
        let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
            .unwrap_or_default();

        coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Applying coordinated_spacing={:?} to coordination context (no re-measure)",
                pass1.spacing_needs
            );
        }

        let mut new_params = context.params.clone();
        new_params.extend(coord_ctx.to_params());

        let updated_ctx = RenderContext::new(
            context.theme.clone(),
            context.plot_width,
            context.plot_height,
            context.session_context.clone(),
            new_params,
            context.scales.clone(),
        );

        // Use pass1 results (no re-measure needed for standalone facets)
        (pass1, updated_ctx)
    } else {
        (pass1, effective_context.clone())
    };

    render_pass::<DimConfig, _>(
        compiled_subplot,
        &df,
        &facet_expr,
        &final_context,
        scale_sharing_by_channel,
        final_pass,
        &group_origin,
    )
    .await
}

/// Detect if compiled_subplot contains a nested facet and compute coordination context
///
/// This helper scans the compiled subplot's marks for facet marks (facet_row or facet_col).
/// If found, it:
/// 1. Extracts the inner facet's channel expression
/// 2. Determines the scale sharing mode for that channel
/// 3. Computes the domain from the full dataset
/// 4. Computes shared data extents for x/y channels from the full dataset
/// 5. Returns a FacetCoordinationContext with the domain and shared extents
///
/// # Arguments
/// * `compiled_subplot` - The subplot to scan for nested facets
/// * `df` - The full dataset (before any filtering)
/// * `ctx` - Session context for expression evaluation
/// * `outer_is_row_facet` - Whether the outer (calling) facet is a row facet
/// * `outer_facet_expr` - The outer facet's expression (used to iterate over outer values for inner cell counting)
///
/// # Returns
/// Some(FacetCoordinationContext) if a nested facet is found, None otherwise
async fn detect_nested_facet_and_compute_coordination(
    compiled_subplot: &Arc<CompiledPlot>,
    df: &DataFrame,
    ctx: &SessionContext,
    _outer_is_row_facet: bool,
    outer_facet_expr: &datafusion::logical_expr::Expr,
) -> Result<Option<FacetCoordinationContext>, AvengerChartError> {
    use crate::facet::coordination::{GuideOwnership, LevelChannelKey, SerializableDataExtents};
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
    // SharedInRow/SharedInColumn are DEPRECATED for nested facets.
    // These modes are grid-centric and don't map cleanly to nested FacetRow/FacetColumn.
    // They are treated as Level(1) for backward compatibility.
    let should_share_domain = match configured_scale_sharing {
        ScaleSharing::Shared => true,
        ScaleSharing::Free => false,
        #[allow(deprecated)]
        ScaleSharing::SharedInRow => {
            // DEPRECATED: SharedInRow doesn't apply cleanly to nested facets.
            // Use Level(N) with FacetRow > FacetColumn nesting instead.
            // Fall back to Level(1) behavior (share with immediate parent).
            eprintln!(
                "[DEPRECATION WARNING] SharedInRow scale sharing is deprecated for nested facets. \
                 Use Level(1) with FacetRow > FacetColumn nesting for row-based sharing."
            );
            true // Treat as Level(1)
        }
        #[allow(deprecated)]
        ScaleSharing::SharedInColumn => {
            // DEPRECATED: SharedInColumn doesn't apply cleanly to nested facets.
            // Use Level(N) with FacetColumn > FacetRow nesting instead.
            // Fall back to Level(1) behavior (share with immediate parent).
            eprintln!(
                "[DEPRECATION WARNING] SharedInColumn scale sharing is deprecated for nested facets. \
                 Use Level(1) with FacetColumn > FacetRow nesting for column-based sharing."
            );
            true // Treat as Level(1)
        }
        ScaleSharing::Level(n) => {
            // Hierarchical level-based sharing
            // Level(0) = Free: don't share domain
            // Level(1+) = Share with parent: share domain
            n > 0
        }
    };

    // For guide ownership coordination, we still use Shared mode so axis labels
    // only appear on edges, regardless of domain sharing configuration
    let inner_scale_sharing = ScaleSharing::Shared;

    // Compute domain from full dataset
    let mut domain_vals = FacetKeyExtractor::extract_keys(df, &expr).await?;
    domain_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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

    let mut shared_data_extents: HashMap<String, SerializableDataExtents> = HashMap::new();
    let mut shared_data_extents_by_row: HashMap<String, HashMap<String, SerializableDataExtents>> =
        HashMap::new();

    // First, collect channel expressions and their share modes
    let mut channel_info: Vec<(String, datafusion::logical_expr::Expr, ScaleSharing)> = Vec::new();

    if let Some(inner_subplot) = inner_subplot {
        for channel_name in ["x", "y"] {
            for mark in &inner_subplot.marks {
                let channels = mark.data_context().channels();
                if let Some(channel_value) = channels.get(channel_name) {
                    if let Some(channel_expr) = channel_value.expr(ctx) {
                        let share_mode =
                            channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                        channel_info.push((channel_name.to_string(), channel_expr, share_mode));
                        break;
                    }
                }
            }
        }

        // Now compute extents based on share mode
        for (channel_name, channel_expr, share_mode) in &channel_info {
            match share_mode {
                ScaleSharing::Shared => {
                    // Compute from full dataset
                    if let Ok(extents) = compute_numeric_extents(df, channel_expr, ctx).await {
                        shared_data_extents.insert(channel_name.clone(), extents);
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "  Computed shared extents for {}: {:?}",
                                channel_name,
                                shared_data_extents.get(channel_name)
                            );
                        }
                    }
                }
                ScaleSharing::SharedInRow => {
                    // Compute per-row extents
                    // For FacetColumn(FacetRow(Cartesian)), we need extents per row value
                    // Filter by row value and compute extents
                    if inner_is_row_facet {
                        for row_val in &domain_vals {
                            // Filter dataframe to this row value
                            let row_key = scalar_to_string_key(row_val);
                            let filter_df = df.clone().filter(
                                expr.clone()
                                    .eq(datafusion::logical_expr::lit(row_val.clone())),
                            );
                            if let Ok(filter_df) = filter_df {
                                if let Ok(extents) =
                                    compute_numeric_extents(&filter_df, channel_expr, ctx).await
                                {
                                    shared_data_extents_by_row
                                        .entry(row_key.clone())
                                        .or_default()
                                        .insert(channel_name.clone(), extents);
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Free or SharedInColumn - no pre-computation needed at outer level
                    // SharedInColumn would need different handling (per-column extents)
                }
            }
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
        coord_ctx = coord_ctx.with_shared_data_extents(shared_data_extents);
    }

    // Add per-row extents for SharedInRow channels
    if !shared_data_extents_by_row.is_empty() {
        coord_ctx = coord_ctx.with_shared_data_extents_by_row(shared_data_extents_by_row);
    }

    // Collect and add per-channel scale sharing modes for x/y axes
    // This enables measurement to use the same axis visibility decisions as rendering
    //
    // IMPORTANT: The axis_scale_sharing comes from the Cartesian subplot's channel configs,
    // not from the facet dimension's scale sharing. For example:
    // - `.x_with(col("sepal_length"), |c| c.with_scale_sharing(ScaleSharing::Shared))`
    // - `.y_with(col("sepal_width"), |c| c.with_scale_sharing(ScaleSharing::Free))`
    //
    // The channel_info collected earlier (lines 1593-1613) contains this information.
    let axis_scale_sharing: HashMap<String, ScaleSharing> = {
        let mut sharing = HashMap::new();
        for (channel_name, _, share_mode) in &channel_info {
            sharing.insert(channel_name.clone(), *share_mode);
        }
        sharing
    };
    if !axis_scale_sharing.is_empty() {
        coord_ctx = coord_ctx.with_axis_scale_sharing(axis_scale_sharing.clone());
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Added axis_scale_sharing to coord context: {:?}",
                coord_ctx.axis_scale_sharing
            );
        }
    }

    // ========== EXTRACT CHANNEL SHARING LEVELS (Level-based scale sharing) ==========
    // Convert ScaleSharing modes to level values for the new hierarchical system.
    // This enables get_channel_level() and get_domain_for_channel() lookups.
    let channel_sharing_levels: HashMap<String, u8> = {
        let mut levels = HashMap::new();
        for (channel_name, _, share_mode) in &channel_info {
            levels.insert(channel_name.clone(), share_mode.to_level());
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

    // ========== BUILD LEVEL_DOMAINS HashMap (Level-based domain propagation) ==========
    // For channels with Level(N) where N >= 1, compute domain extents and store
    // at the appropriate level. Level 1 domains come from the outer's filtered df.
    // For 2-level nesting (outer > inner), we're at nesting_depth = 1.
    //
    // Note: This function is called from the OUTER facet while processing nested facets.
    // The df parameter is the OUTER facet's full dataset (before per-cell filtering).
    // Level 1 domains use this full dataset to unify scales across all cells.
    let mut level_domains: HashMap<LevelChannelKey, SerializableDataExtents> = HashMap::new();

    for (channel_name, channel_expr, share_mode) in &channel_info {
        let level = share_mode.to_level();
        if level >= 1 {
            // For Level(1+) channels, compute domain from the outer's full dataset
            // This ensures all inner facets share the same scale domain
            if let Ok(extents) = compute_numeric_extents(df, channel_expr, ctx).await {
                let key = LevelChannelKey::new(1, channel_name);
                level_domains.insert(key, extents);
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Added level_domain for level=1, channel={}: {:?}",
                        channel_name,
                        level_domains.get(&LevelChannelKey::new(1, channel_name))
                    );
                }
            }
        }
    }

    if !level_domains.is_empty() {
        // Set nesting depth to 1 (we're one level deep from the outer facet)
        coord_ctx = coord_ctx
            .with_nesting_depth(1)
            .with_level_domains(level_domains);
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Added level_domains to coord context: {} entries, nesting_depth=1",
                coord_ctx.level_domains.len()
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

/// Convert a ScalarValue to a string key for HashMap lookups
///
/// This is used for the `shared_data_extents_by_row` HashMap keys.
fn scalar_to_string_key(value: &ScalarValue) -> String {
    use crate::facet::coordination::SerializableDomainValue;
    let serializable = SerializableDomainValue::from_scalar(value);
    serde_json::to_string(&serializable).unwrap_or_else(|_| "null".to_string())
}

/// Compute numeric extents (min, max) for an expression from a DataFrame
///
/// This is a lightweight operation that runs a simple SQL aggregate query
/// without building full scales (which would cause recursion in nested facets).
async fn compute_numeric_extents(
    df: &DataFrame,
    expr: &datafusion::logical_expr::Expr,
    _ctx: &SessionContext,
) -> Result<crate::facet::coordination::SerializableDataExtents, AvengerChartError> {
    use crate::facet::coordination::SerializableDataExtents;
    use datafusion::functions_aggregate::min_max::{max, min};

    let agg_df = df.clone().aggregate(
        vec![],
        vec![
            min(expr.clone()).alias("min_val"),
            max(expr.clone()).alias("max_val"),
        ],
    )?;

    let batches = agg_df.collect().await?;

    if batches.is_empty() || batches[0].num_rows() == 0 {
        return Err(AvengerChartError::InternalError(
            "Empty result from extent computation".to_string(),
        ));
    }

    let batch = &batches[0];
    let min_col = batch.column(0);
    let max_col = batch.column(1);

    // Try to extract numeric values
    let min_val = extract_f64_from_array(min_col, 0)?;
    let max_val = extract_f64_from_array(max_col, 0)?;

    Ok(SerializableDataExtents::interval(min_val, max_val))
}

/// Extract f64 value from an Arrow array at the given index
fn extract_f64_from_array(
    array: &std::sync::Arc<dyn datafusion::arrow::array::Array>,
    index: usize,
) -> Result<f64, AvengerChartError> {
    use datafusion::arrow::array::*;
    use datafusion::arrow::datatypes::DataType;

    if array.is_null(index) {
        return Err(AvengerChartError::InternalError(
            "Null value in extent computation".to_string(),
        ));
    }

    match array.data_type() {
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            Ok(arr.value(index))
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int16 => {
            let arr = array.as_any().downcast_ref::<Int16Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int8 => {
            let arr = array.as_any().downcast_ref::<Int8Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt16 => {
            let arr = array.as_any().downcast_ref::<UInt16Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt8 => {
            let arr = array.as_any().downcast_ref::<UInt8Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        dt => Err(AvengerChartError::InternalError(format!(
            "Unsupported data type for extent: {:?}",
            dt
        ))),
    }
}
