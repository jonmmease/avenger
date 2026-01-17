/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module provides a parameterized three-phase rendering algorithm that works for
/// both row and column faceting by accepting orientation-specific closures.
///
/// # Three-Phase Algorithm
///
/// The facet evaluation follows a three-phase pattern:
///
/// **Phase 1 (Measurement)**: Measure guide overflow to determine spacing needs
/// - Build scales with approximate band size
/// - Measure each subplot's axis labels/titles
/// - Calculate max overflow based on dimension-specific adjacency rules
/// - Collect spacing requirements for coordination
///
/// **Phase 1.5 (Coordination)**: Coordinate spacing across nested facets
/// - Aggregate overflow measurements across all cells
/// - Propagate shared spacing requirements to nested facets
/// - Compute uniform cell counts for Free scaling mode
/// - Re-measure with coordinated spacing if needed
///
/// **Phase 2 (Rendering)**: Render with final dimensions
/// - Rebuild shared scales with final band size (CRITICAL for data alignment)
/// - Position and render each subplot with correct spacing
/// - Facet labels are rendered by the guide system (not by this function)
use crate::channel::config_traits::ScaleSharing;
use crate::coords::SubplotGeometry;
use crate::coords::SubplotRect;
use crate::error::AvengerChartError;
use crate::facet::coordination::{
    FacetCoordinationContext, SHARED_OVERFLOW_BOTTOM, SHARED_OVERFLOW_LEFT, SHARED_OVERFLOW_RIGHT,
    SHARED_OVERFLOW_TOP,
};
use crate::facet::dimension_config::{FacetDimensionConfig, RowDimensionConfig};
use crate::facet::marks::facet::determine_facet_band_align;
use crate::facet::phantom_cells::PhantomPlacement;
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::facet::scale_helpers::build_scales_helper_with_fallback;
use crate::facet::subplot_iterator::SubplotIteration;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::builder::ScaleBuilder;
use avenger_scales::scalar::Scalar;
use avenger_scales::scales::DomainKind;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{ExprSchemable, lit};
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
    /// Phantom cell layout for uniform free scaling
    ///
    /// Computed in Pass 1 and reused in Pass 2 to avoid duplicating phantom cell
    /// positioning logic. Contains band alignment, phantom counts, and cell counts.
    phantom_layout: crate::facet::phantom_cells::PhantomCellLayout,
}

/// Decision strategy for Phase 1.5 coordination
///
/// Phase 1.5 coordinates spacing between nested facets after the initial measurement
/// pass. The strategy determines whether re-measurement is needed based on the
/// spacing requirements detected during Pass 1.
///
/// # Variants
///
/// - `Rerun`: Re-run measure_pass with coordinated spacing (nested facets with cross-dimensional gaps)
/// - `UpdateContextOnly`: Update coordination context without re-measurement (standalone facets)
/// - `NoCoordination`: No coordination needed (simple facets)
#[derive(Debug, Clone, PartialEq, Eq)]
enum CoordinationStrategy {
    /// Re-run measure_pass with coordinated spacing
    ///
    /// Used for nested facets where cross-dimensional gaps need coordination.
    /// For example, a FacetColumn containing FacetRow needs to coordinate row gaps.
    Rerun,

    /// Update coordination context only, no re-measurement
    ///
    /// Used for standalone facets that have spacing requirements (like legend alignment)
    /// but don't need to re-measure because there are no inner facets.
    UpdateContextOnly,

    /// No coordination needed
    ///
    /// Used for simple facets with no spacing requirements.
    NoCoordination,
}

impl CoordinationStrategy {
    /// Determine the coordination strategy based on spacing needs
    ///
    /// # Arguments
    /// * `spacing_needs` - Named spacing requirements from Pass 1
    ///
    /// # Type Parameters
    /// * `DimConfig` - Facet dimension configuration (Row or Column)
    ///
    /// # Returns
    /// The appropriate coordination strategy
    fn determine<DimConfig: FacetDimensionConfig>(spacing_needs: &HashMap<String, f32>) -> Self {
        // Cross-dimension key depends on facet orientation:
        // - FacetColumn looks for "inter_row_gap" (from nested FacetRow)
        // - FacetRow looks for "inter_col_gap" (from nested FacetColumn)
        let cross_dim_key = if DimConfig::is_col_facet() {
            "inter_row_gap"
        } else {
            "inter_col_gap"
        };

        if spacing_needs.contains_key(cross_dim_key) {
            Self::Rerun
        } else if !spacing_needs.is_empty() {
            Self::UpdateContextOnly
        } else {
            Self::NoCoordination
        }
    }

    /// Whether this strategy requires re-running the measurement pass
    #[allow(dead_code)]
    fn requires_remeasurement(&self) -> bool {
        matches!(self, Self::Rerun)
    }
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

    // ========== LEVEL-BASED DOMAIN EXTRACTION (Level(N) sharing) ==========
    // For channels with Level(N) sharing where N >= 1, extract domains from level_domains.
    // This enables hierarchical scale sharing in nested facets using the Level(N) API.
    //
    // SILENT BEHAVIOR: When coordination_context is None (non-nested facet or
    // coordination disabled), Level(N) modes silently fall back to Free behavior
    // since there's no parent to share domains with. This ensures graceful
    // degradation in all contexts without errors.
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
            // No coordination context - Level(N) degrades to Free behavior
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
    domain_vals.sort_by(scalar_total_cmp);

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
    let phantom_placement = uniform_cell_count.map(|uniform_count| {
        PhantomPlacement::compute(band_align, domain_vals.len(), uniform_count)
    });

    let (all_positions, scale_domain_vals) = if let Some(ref placement) = phantom_placement {
        if placement.phantom_count > 0 {
            let placeholder_template = domain_vals
                .first()
                .cloned()
                .unwrap_or(ScalarValue::Utf8(None));
            let padded = placement.pad_domain(&domain_vals, &placeholder_template);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Uniform Free scaling Pass1: creating temp scale with {} values (actual={}) band_align={:.2} prepend_phantoms={}",
                    padded.len(),
                    domain_vals.len(),
                    band_align,
                    placement.prepend
                );
            }

            // Create temporary scale with padded domain for position computation
            use datafusion::arrow::array::StringArray;
            use std::sync::Arc as StdArc;
            let padded_strings: Vec<String> = padded.iter().map(|v| v.to_string()).collect();
            let padded_array = StdArc::new(StringArray::from(padded_strings))
                as datafusion::arrow::array::ArrayRef;
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

    use crate::facet::phantom_cells::PhantomCellLayout;
    use crate::facet::subplot_iterator::SubplotIteration;
    use crate::facet::subplot_iterator::SubplotIterator;

    // Compute phantom cell layout for uniform free scaling
    // This centralizes phantom positioning logic for use in both measurement and rendering
    let phantom_layout =
        PhantomCellLayout::compute(band_align, domain_vals.len(), uniform_cell_count);

    // Update coordination context params with phantom_prepend_count
    let subplot_params = phantom_layout.update_params_with_phantom_context(&context.params);

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
    let initial_rects =
        if uniform_cell_count.is_some() && all_initial_rects.len() > domain_vals.len() {
            let num_actual = domain_vals.len();
            let total = all_initial_rects.len();
            if band_align >= 0.5 {
                // Phantoms at start, actual data at end - take last N rects
                all_initial_rects
                    .into_iter()
                    .skip(total - num_actual)
                    .collect()
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
    let outer_uniform_cell_count =
        FacetCoordinationContext::from_params(&context.params).and_then(|ctx| {
            // Check if uniform sizing is for this facet's dimension
            if ctx.inner_channel.as_deref() == Some(DimConfig::channel_name()) {
                ctx.get_uniform_cell_count()
            } else {
                // Uniform sizing is for a different dimension (nested facet), not us
                None
            }
        });
    let outer_count = outer_uniform_cell_count.unwrap_or(work_items.len());
    // Use phantom layout's prepend count for position adjustment
    let phantom_offset = phantom_layout.prepend_count();

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
            let shared_data_extents = shared_data_extents.clone();
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

                // Per-channel scale building: ALWAYS build from filtered data, then selectively
                // extend only Shared/Level(N) channels with their respective extents.
                // This ensures Free channels use the local (filtered) data range.
                let mut free_scale_builder = compiled_subplot
                    .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
                    .await?;

                // Extend with shared data extents ONLY for channels with ScaleSharing::Shared
                // For Free scales, we want the local (filtered) data range, not the full dataset range
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
                        free_scale_builder.extend_with_shared_extents(&shared_only_extents);
                    }
                }

                // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
                // Extract from coordination context (which was updated per-iteration)
                if let Some(coord_ctx) = FacetCoordinationContext::from_params(&params_base) {
                    let level_extents: std::collections::HashMap<String, _> =
                        scale_sharing_by_channel
                            .iter()
                            .filter_map(|(channel, mode)| {
                                if let ScaleSharing::Level(n) = mode {
                                    if *n >= 1 {
                                        coord_ctx
                                            .get_domain_for_channel(channel)
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
                        free_scale_builder.extend_with_shared_extents(&level_extents);
                    }
                }

                // Determine if we should pass free_scale_builder to the helper
                // When any channel is Shared, we rely on initial_shared_scales and overlay later
                let free_scale_builder_pass1 = if scale_sharing_by_channel
                    .values()
                    .any(|v| *v == ScaleSharing::Shared)
                {
                    None
                } else {
                    Some(free_scale_builder.clone())
                };

                let mut scales = build_scales_helper_with_fallback(
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

                // Overlay step: For non-Shared channels, replace shared scales with free scales
                // This ensures Free/Level(N) channels use their correct (per-channel) domains
                // while Shared channels continue to use the full dataset domain.
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

                let (guide_only, total_overflow, legend_positions, spacing_needs) =
                    measure_subplot(
                        &compiled_subplot,
                        width,
                        height,
                        &ctx,
                        &params_base,
                        &scales,
                        &filter_df,
                    )
                    .await?;

                Ok::<_, AvengerChartError>((
                    idx,
                    guide_only,
                    total_overflow,
                    legend_positions,
                    spacing_needs,
                ))
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
    let (final_all_positions, final_scale_domain_vals) =
        if let Some(uniform_count) = uniform_cell_count {
            if domain_vals.len() < uniform_count {
                // Create temporary scale with padded domain for position computation
                use datafusion::arrow::array::StringArray;
                use std::sync::Arc as StdArc;
                let padded_strings: Vec<String> =
                    scale_domain_vals.iter().map(|v| v.to_string()).collect();
                let padded_array = StdArc::new(StringArray::from(padded_strings))
                    as datafusion::arrow::array::ArrayRef;

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
    // Uses PhantomPlacement.extract_actual() to handle prepend/append logic consistently
    let mut final_rects: Vec<SubplotRect> = if let Some(ref placement) = phantom_placement {
        if placement.phantom_count > 0 && all_final_rects_raw.len() > domain_vals.len() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  rect selection Pass2: channel={} total={} num_actual={} band_align={:.2} prepend={} all_rects_y={:?}",
                    DimConfig::channel_name(),
                    all_final_rects_raw.len(),
                    domain_vals.len(),
                    band_align,
                    placement.prepend,
                    all_final_rects_raw.iter().map(|r| r.y).collect::<Vec<_>>()
                );
            }
            let result = placement.extract_actual(all_final_rects_raw);
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
        }
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
                    if let Some(&gap) =
                        aggregated_spacing_needs.get(crate::guide::spacing_keys::INTER_ROW_GAP)
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
                            eprintln!("  No inter_row_gap in aggregated_spacing_needs, using None");
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
                    if let Some(&gap) =
                        aggregated_spacing_needs.get(crate::guide::spacing_keys::INTER_COL_GAP)
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
                            eprintln!("  No inter_col_gap in aggregated_spacing_needs, using None");
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
        spacing_needs.insert(SHARED_OVERFLOW_LEFT.to_string(), global_max_overflow.left);
        spacing_needs.insert(SHARED_OVERFLOW_RIGHT.to_string(), global_max_overflow.right);
        spacing_needs.insert(SHARED_OVERFLOW_TOP.to_string(), global_max_overflow.top);
        spacing_needs.insert(
            SHARED_OVERFLOW_BOTTOM.to_string(),
            global_max_overflow.bottom,
        );
    }

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "measure_pass returning final_rects: channel={} rects={:?}",
            DimConfig::channel_name(),
            final_rects
                .iter()
                .map(|r| (r.x, r.width))
                .collect::<Vec<_>>()
        );
    }

    Ok(FacetPass1Result {
        overflow_measurements,
        final_dimension_scale,
        final_shared_scales,
        final_rects,
        fallback_builder,
        shared_data_extents,
        spacing_needs,
        phantom_layout,
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
    // Reuse the phantom layout computed in Pass 1 to avoid duplicating logic
    let subplot_params_pass2 = pass1
        .phantom_layout
        .update_params_with_phantom_context(&context.params);

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
    let outer_uniform_cell_count =
        FacetCoordinationContext::from_params(&context.params).and_then(|ctx| {
            // Check if uniform sizing is for this facet's dimension
            if ctx.inner_channel.as_deref() == Some(DimConfig::channel_name()) {
                ctx.get_uniform_cell_count()
            } else {
                // Uniform sizing is for a different dimension (nested facet), not us
                None
            }
        });
    let outer_count = outer_uniform_cell_count.unwrap_or(work_items.len());
    // Phantom offset from pass1 for adjusting subplot positions (reuse layout from Pass 1)
    let phantom_offset = pass1.phantom_layout.prepend_count();

    // Clone fallback builder for render pass
    let fallback_builder = pass1.fallback_builder.clone();

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
                let merged_params = params_base.clone();

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

/// Generic three-phase facet evaluation parameterized by dimension
///
/// # Algorithm
///
/// The evaluation follows a three-phase pattern to handle nested facets and
/// coordinated spacing:
///
/// **Phase 1 (Measurement)**: Measure guide overflow to determine spacing needs
/// - Build scales with approximate band size
/// - Measure each subplot's axis labels/titles
/// - Calculate max overflow based on dimension-specific adjacency rules
/// - Collect spacing requirements for coordination across nested facets
///
/// **Phase 1.5 (Coordination)**: Coordinate spacing between facet levels
/// - This phase is handled by the caller (CompiledFacetRow/CompiledFacetCol)
/// - Aggregates overflow measurements across all cells
/// - Propagates shared spacing requirements via FacetCoordinationContext
/// - Re-measures with coordinated spacing if nested facets require it
///
/// **Phase 2 (Rendering)**: Render with final dimensions
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

    // Extract scale sharing modes from subplot marks.
    //
    // SILENT BEHAVIOR: When multiple marks specify different sharing modes for the
    // same channel, the following precedence applies:
    // 1. ScaleSharing::Shared always wins (once set, cannot be overridden)
    // 2. First non-Free mode wins over Free
    // 3. Subsequent non-Shared modes are ignored
    //
    // For consistent behavior, use the same scale sharing mode across all marks
    // in a subplot. Mixed modes may produce unexpected results.
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

    let mut scale_sharing_by_channel: HashMap<String, ScaleSharing> = scale_sharing_by_channel;

    // Merge channel_sharing_levels from incoming coordination context (from outer facet)
    // This enables inner facets to know the x/y scale sharing for Cartesian subplot measurement
    if let Some(incoming_coord_ctx) = FacetCoordinationContext::from_params(&context.params) {
        for (channel, level) in &incoming_coord_ctx.channel_sharing_levels {
            // Only add if not already present (don't override explicit channel settings)
            scale_sharing_by_channel
                .entry(channel.clone())
                .or_insert_with(|| ScaleSharing::from_level(*level));
        }
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
            && !incoming_coord_ctx.channel_sharing_levels.is_empty()
        {
            eprintln!(
                "  Merged channel_sharing_levels from coordination context: {:?}",
                incoming_coord_ctx.channel_sharing_levels
            );
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
        &context.params,           // Parameters for evaluating explicit domains
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

    // Phase 1.5: Coordinate spacing across nested facets
    //
    // The coordination strategy determines whether re-measurement is needed:
    // - Rerun: Nested facets with cross-dimensional gaps need re-measurement
    // - UpdateContextOnly: Standalone facets just need context update for legend alignment
    // - NoCoordination: Simple facets with no spacing requirements
    let strategy = CoordinationStrategy::determine::<DimConfig>(&pass1.spacing_needs);

    let (final_pass, final_context) = match strategy {
        CoordinationStrategy::Rerun => {
            // Nested facets: re-run measure_pass with coordinated spacing
            let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
                .unwrap_or_default();
            coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Phase 1.5 (Rerun): Re-running measure_pass with coordinated_spacing={:?}",
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
        }

        CoordinationStrategy::UpdateContextOnly => {
            // Standalone facets: update coordination context for legend alignment
            let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
                .unwrap_or_default();
            coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Phase 1.5 (UpdateContextOnly): coordinated_spacing={:?}",
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

            (pass1, updated_ctx)
        }

        CoordinationStrategy::NoCoordination => {
            // Simple facets: no coordination needed
            (pass1, effective_context.clone())
        }
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
    params: &indexmap::IndexMap<String, ScalarValue>,
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
    // Use is_free() helper: Level(0) is semantically equivalent to Free
    let should_share_domain = !configured_scale_sharing.is_free();

    // For guide ownership coordination, we still use Shared mode so axis labels
    // only appear on edges, regardless of domain sharing configuration
    let inner_scale_sharing = ScaleSharing::Shared;

    // Compute domain from full dataset
    let mut domain_vals = FacetKeyExtractor::extract_keys(df, &expr).await?;
    domain_vals.sort_by(scalar_total_cmp);

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

    // First, collect channel expressions, share modes, and scale-derived domain kinds
    let mut channel_info: Vec<(
        String,
        datafusion::logical_expr::Expr,
        ScaleSharing,
        Option<DomainKind>,
        Option<crate::facet::coordination::SerializableDataExtents>,
        Option<DomainSort>,
    )> = Vec::new();

    if let Some(inner_subplot) = inner_subplot {
        for channel_name in ["x", "y"] {
            let mut channel_expr: Option<datafusion::logical_expr::Expr> = None;
            let mut share_mode = ScaleSharing::Free;
            let mut domain_kind: Option<DomainKind> = None;
            let mut explicit_domain: Option<crate::facet::coordination::SerializableDataExtents> =
                None;
            let mut domain_sort: Option<DomainSort> = None;

            for mark in &inner_subplot.marks {
                let channels = mark.data_context().channels();
                if let Some(channel_value) = channels.get(channel_name) {
                    if channel_expr.is_none() {
                        channel_expr = channel_value.expr(ctx);
                        share_mode = channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                    }

                    // Prefer categorical if ANY mark explicitly configures a categorical scale.
                    let scale_config = channel_value.get_scale_config();
                    let this_kind = scale_config.and_then(|s| s.domain_kind());
                    if this_kind == Some(DomainKind::Categorical) {
                        domain_kind = Some(DomainKind::Categorical);
                    } else if domain_kind.is_none() {
                        domain_kind = this_kind;
                    }

                    if explicit_domain.is_none() {
                        if let Some(scale) = scale_config {
                            explicit_domain =
                                explicit_domain_extents_from_scale(scale, ctx, params).await?;
                        }
                    }

                    if domain_sort.is_none() {
                        if let Some(scale) = scale_config {
                            domain_sort = scale_sort_order(scale, ctx, params).await?;
                        }
                    }
                }
            }

            if let Some(channel_expr) = channel_expr {
                // Fall back to compiled scale spec if no explicit config was found.
                let domain_kind = domain_kind.or_else(|| {
                    inner_subplot
                        .scale_specs
                        .get(channel_name)
                        .and_then(|spec| match spec {
                            crate::plot::ScaleSpec::Local(scale) => scale.domain_kind(),
                        })
                });

                if explicit_domain.is_none() {
                    if let Some(spec) = inner_subplot.scale_specs.get(channel_name) {
                        match spec {
                            crate::plot::ScaleSpec::Local(scale) => {
                                explicit_domain =
                                    explicit_domain_extents_from_scale(scale, ctx, params).await?;
                            }
                        }
                    }
                }

                if domain_sort.is_none() {
                    if let Some(spec) = inner_subplot.scale_specs.get(channel_name) {
                        match spec {
                            crate::plot::ScaleSpec::Local(scale) => {
                                domain_sort = scale_sort_order(scale, ctx, params).await?;
                            }
                        }
                    }
                }

                channel_info.push((
                    channel_name.to_string(),
                    channel_expr,
                    share_mode,
                    domain_kind,
                    explicit_domain,
                    domain_sort,
                ));
            }
        }

        // Now compute extents based on share mode
        for (channel_name, channel_expr, share_mode, domain_kind, explicit_domain, domain_sort) in
            &channel_info
        {
            if matches!(share_mode, ScaleSharing::Shared) {
                if let Some(explicit_extents) = explicit_domain {
                    shared_data_extents.insert(channel_name.clone(), explicit_extents.clone());
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "  Using explicit domain for shared {}: {:?}",
                            channel_name,
                            shared_data_extents.get(channel_name)
                        );
                    }
                    continue;
                }

                // Determine if channel is categorical:
                // 1. First check scale configuration (handles numeric-coded categories like Int32 + Band)
                // 2. Fall back to Arrow data type detection
                let is_categorical = if *domain_kind == Some(DomainKind::Categorical) {
                    true
                } else {
                    channel_expr
                        .get_type(df.schema())
                        .ok()
                        .map(|dt| is_categorical_data_type(&dt))
                        .unwrap_or(false)
                };

                let is_temporal = if *domain_kind == Some(DomainKind::Temporal) {
                    true
                } else {
                    channel_expr
                        .get_type(df.schema())
                        .ok()
                        .map(|dt| is_temporal_data_type(&dt))
                        .unwrap_or(false)
                };

                // Compute extents using appropriate method based on categorical detection
                let extents_result = if is_categorical {
                    let sort_order = domain_sort.unwrap_or(DomainSort::Ascending);
                    compute_categorical_extents(df, channel_expr, ctx, sort_order).await
                } else if is_temporal {
                    compute_temporal_extents(df, channel_expr, ctx).await
                } else {
                    compute_numeric_extents(df, channel_expr, ctx).await
                };

                if let Ok(extents) = extents_result {
                    shared_data_extents.insert(channel_name.clone(), extents);
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "  Computed shared {} extents for {}: {:?}",
                            if is_categorical {
                                "categorical"
                            } else if is_temporal {
                                "temporal"
                            } else {
                                "numeric"
                            },
                            channel_name,
                            shared_data_extents.get(channel_name)
                        );
                    }
                }
            }
            // For Free and Level(N) modes, no pre-computation needed
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

    // ========== EXTRACT CHANNEL SHARING LEVELS (Level-based scale sharing) ==========
    // Convert ScaleSharing modes to level values for the new hierarchical system.
    // This enables get_channel_level() and get_domain_for_channel() lookups.
    let channel_sharing_levels: HashMap<String, u8> = {
        let mut levels = HashMap::new();
        for (channel_name, _, share_mode, _, _, _) in &channel_info {
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

    // ========== BUILD LEVEL_DOMAINS IndexMap (Level-based domain propagation) ==========
    // For channels with Level(N) where N >= 1, compute domain extents and store
    // at the appropriate level. Level 1 domains come from the outer's filtered df.
    // For 2-level nesting (outer > inner), we're at nesting_depth = 1.
    //
    // Note: This function is called from the OUTER facet while processing nested facets.
    // The df parameter is the OUTER facet's full dataset (before per-cell filtering).
    // Level 1 domains use this full dataset to unify scales across all cells.
    // Uses IndexMap for deterministic iteration order during serialization.
    let mut level_domains: IndexMap<LevelChannelKey, SerializableDataExtents> = IndexMap::new();

    for (channel_name, channel_expr, share_mode, domain_kind, explicit_domain, domain_sort) in
        &channel_info
    {
        let level = share_mode.to_level();
        if level >= 1 {
            if let Some(explicit_extents) = explicit_domain {
                let key = LevelChannelKey::new(1, channel_name);
                level_domains.insert(key, explicit_extents.clone());
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Added explicit level_domain for level=1, channel={}: {:?}",
                        channel_name,
                        level_domains.get(&LevelChannelKey::new(1, channel_name))
                    );
                }
                continue;
            }

            // Determine if channel is categorical:
            // 1. First check scale configuration (handles numeric-coded categories like Int32 + Band)
            // 2. Fall back to Arrow data type detection
            let is_categorical = if *domain_kind == Some(DomainKind::Categorical) {
                true
            } else {
                channel_expr
                    .get_type(df.schema())
                    .ok()
                    .map(|dt| is_categorical_data_type(&dt))
                    .unwrap_or(false)
            };

            let is_temporal = if *domain_kind == Some(DomainKind::Temporal) {
                true
            } else {
                channel_expr
                    .get_type(df.schema())
                    .ok()
                    .map(|dt| is_temporal_data_type(&dt))
                    .unwrap_or(false)
            };

            // For Level(1+) channels, compute domain from the outer's full dataset
            // This ensures all inner facets share the same scale domain
            let extents_result = if is_categorical {
                let sort_order = domain_sort.unwrap_or(DomainSort::Ascending);
                compute_categorical_extents(df, channel_expr, ctx, sort_order).await
            } else if is_temporal {
                compute_temporal_extents(df, channel_expr, ctx).await
            } else {
                compute_numeric_extents(df, channel_expr, ctx).await
            };

            if let Ok(extents) = extents_result {
                let key = LevelChannelKey::new(1, channel_name);
                level_domains.insert(key, extents);
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Added level_domain for level=1, channel={} ({}): {:?}",
                        channel_name,
                        if is_categorical {
                            "categorical"
                        } else if is_temporal {
                            "temporal"
                        } else {
                            "numeric"
                        },
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

/// Compute categorical extents (unique values) for an expression from a DataFrame
///
/// This is a lightweight operation that runs a DISTINCT query to get unique values.
/// Used for categorical scale sharing in nested facets.
async fn compute_categorical_extents(
    df: &DataFrame,
    expr: &datafusion::logical_expr::Expr,
    _ctx: &SessionContext,
    sort_order: DomainSort,
) -> Result<crate::facet::coordination::SerializableDataExtents, AvengerChartError> {
    use crate::facet::coordination::SerializableDataExtents;

    // Use DISTINCT to get unique values
    let distinct_df = df.clone().select(vec![expr.clone()])?.distinct()?;
    let batches = distinct_df.collect().await?;

    // Extract values into Vec<ScalarValue>
    let mut values = Vec::new();
    for batch in &batches {
        let col = batch.column(0);
        for i in 0..col.len() {
            let scalar = ScalarValue::try_from_array(col, i)?;
            values.push(normalize_domain_scalar(scalar));
        }
    }

    if sort_order != DomainSort::None {
        // Sort for consistent ordering across facet cells
        values.sort_by(scalar_total_cmp);
        if sort_order == DomainSort::Descending {
            values.reverse();
        }
    }

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "  compute_categorical_extents: extracted {} unique values",
            values.len()
        );
    }

    Ok(SerializableDataExtents::discrete(values))
}

async fn compute_temporal_extents(
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
    let min_val = ScalarValue::try_from_array(batch.column(0), 0)?;
    let max_val = ScalarValue::try_from_array(batch.column(1), 0)?;

    let min_ms = scalar_to_timestamp_ms(&min_val).ok_or_else(|| {
        AvengerChartError::InternalError("Failed to interpret temporal min value".to_string())
    })?;
    let max_ms = scalar_to_timestamp_ms(&max_val).ok_or_else(|| {
        AvengerChartError::InternalError("Failed to interpret temporal max value".to_string())
    })?;

    Ok(SerializableDataExtents::temporal(min_ms, max_ms))
}

async fn explicit_domain_extents_from_scale(
    scale: &crate::scales::Scale<crate::scales::spec::Auto>,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
) -> Result<Option<crate::facet::coordination::SerializableDataExtents>, AvengerChartError> {
    use crate::scales::ScaleDefaultDomain;

    let Some(domain) = scale.get_domain() else {
        return Ok(None);
    };

    match &domain.default_domain {
        ScaleDefaultDomain::Discrete(values) => {
            use crate::serialization::LogicalExprNodeExt;
            let exprs = values
                .iter()
                .map(|value| value.to_expr(ctx))
                .collect::<Result<Vec<_>, _>>()?;
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let scalars =
                crate::utils::eval_to_scalars(exprs, Some(ctx), datafusion_params.as_ref()).await?;
            let normalized: Vec<ScalarValue> =
                scalars.into_iter().map(normalize_domain_scalar).collect();
            Ok(Some(
                crate::facet::coordination::SerializableDataExtents::discrete(normalized),
            ))
        }
        ScaleDefaultDomain::Interval(start, end) => {
            use crate::serialization::LogicalExprNodeExt;
            use crate::utils::ScalarValueHelpers;
            let start_expr = start.to_expr(ctx)?;
            let end_expr = end.to_expr(ctx)?;
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let scalars = crate::utils::eval_to_scalars(
                vec![start_expr, end_expr],
                Some(ctx),
                datafusion_params.as_ref(),
            )
            .await?;
            let [start_val, end_val] = scalars.as_slice() else {
                return Err(AvengerChartError::InternalError(
                    "Expected two scalar values for interval domain".to_string(),
                ));
            };

            let extents = if scale.domain_kind() == Some(DomainKind::Temporal) {
                let start_ts =
                    scalar_to_timestamp_ms(start_val).unwrap_or(start_val.as_f64()? as i64);
                let end_ts = scalar_to_timestamp_ms(end_val).unwrap_or(end_val.as_f64()? as i64);
                crate::facet::coordination::SerializableDataExtents::temporal(start_ts, end_ts)
            } else {
                crate::facet::coordination::SerializableDataExtents::interval(
                    start_val.as_f64()?,
                    end_val.as_f64()?,
                )
            };

            Ok(Some(extents))
        }
        ScaleDefaultDomain::DomainExprs(_) | ScaleDefaultDomain::NoDefault => Ok(None),
    }
}

fn normalize_domain_scalar(value: ScalarValue) -> ScalarValue {
    match value {
        ScalarValue::Dictionary(_, inner) => normalize_domain_scalar(*inner),
        other => other,
    }
}

fn scalar_to_timestamp_ms(value: &ScalarValue) -> Option<i64> {
    match value {
        ScalarValue::Date32(Some(days)) => Some(*days as i64 * 86_400_000),
        ScalarValue::Date64(Some(ms)) => Some(*ms),
        ScalarValue::TimestampSecond(Some(ts), _) => Some(*ts * 1000),
        ScalarValue::TimestampMillisecond(Some(ts), _) => Some(*ts),
        ScalarValue::TimestampMicrosecond(Some(ts), _) => Some(*ts / 1000),
        ScalarValue::TimestampNanosecond(Some(ts), _) => Some(*ts / 1_000_000),
        _ => None,
    }
}

async fn scale_sort_order(
    scale: &crate::scales::Scale<crate::scales::spec::Auto>,
    ctx: &SessionContext,
    params: &indexmap::IndexMap<String, ScalarValue>,
) -> Result<Option<DomainSort>, AvengerChartError> {
    use crate::serialization::LogicalExprNodeExt;

    let Some(value_node) = scale.get_options().get("sort") else {
        return Ok(None);
    };

    let expr = value_node.to_expr(ctx)?;
    let datafusion_params = crate::utils::params_to_datafusion(params);
    let mut scalars =
        crate::utils::eval_to_scalars(vec![expr], Some(ctx), datafusion_params.as_ref()).await?;
    let scalar = scalars
        .pop()
        .ok_or_else(|| AvengerChartError::InternalError("Missing sort option value".to_string()))?;

    Ok(parse_sort_scalar(&scalar))
}

fn parse_sort_scalar(value: &ScalarValue) -> Option<DomainSort> {
    match value {
        ScalarValue::Boolean(Some(true)) => Some(DomainSort::Ascending),
        ScalarValue::Boolean(Some(false)) => Some(DomainSort::None),
        ScalarValue::Int8(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int16(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int32(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Int64(Some(v)) => Some(parse_sort_int(*v)),
        ScalarValue::UInt8(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt16(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt32(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::UInt64(Some(v)) => Some(parse_sort_int(*v as i64)),
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => parse_sort_string(s),
        _ => None,
    }
}

fn parse_sort_int(value: i64) -> DomainSort {
    if value == 0 {
        DomainSort::None
    } else if value < 0 {
        DomainSort::Descending
    } else {
        DomainSort::Ascending
    }
}

fn parse_sort_string(value: &str) -> Option<DomainSort> {
    match value.trim().to_lowercase().as_str() {
        "asc" | "ascending" => Some(DomainSort::Ascending),
        "desc" | "descending" => Some(DomainSort::Descending),
        "none" | "false" | "natural" | "data" => Some(DomainSort::None),
        "true" => Some(DomainSort::Ascending),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomainSort {
    Ascending,
    Descending,
    None,
}

/// Check if a data type represents categorical data based on Arrow type
///
/// This detection is based on the column's data type, not the scale type.
/// It correctly identifies string, boolean, and dictionary-encoded columns as categorical.
///
/// # Limitation
///
/// Numeric columns (Int*, UInt*, Float*) used with Band/Point/Ordinal scales
/// will NOT be detected as categorical by this function. They will be treated
/// as numeric and compute interval extents instead of discrete extents.
/// This means numeric-coded categories (e.g., 1,2,3 for Low/Medium/High) won't
/// share domains correctly in nested facets.
///
/// A more robust solution would check the scale type (Band/Point/Ordinal) rather
/// than the data type, but that requires architectural changes to pass scale
/// configuration information to the extent computation phase.
fn is_categorical_data_type(data_type: &datafusion::arrow::datatypes::DataType) -> bool {
    use datafusion::arrow::datatypes::DataType;
    matches!(
        data_type,
        DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Boolean
            | DataType::Dictionary(_, _)
    )
}

fn is_temporal_data_type(data_type: &datafusion::arrow::datatypes::DataType) -> bool {
    use datafusion::arrow::datatypes::DataType;
    matches!(
        data_type,
        DataType::Date32
            | DataType::Date64
            | DataType::Timestamp(_, _)
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Duration(_)
    )
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
