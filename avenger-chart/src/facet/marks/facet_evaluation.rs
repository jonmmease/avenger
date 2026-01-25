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
    FacetCoordinationContext, LevelChannelKey, SHARED_OVERFLOW_BOTTOM, SHARED_OVERFLOW_LEFT,
    SHARED_OVERFLOW_RIGHT, SHARED_OVERFLOW_TOP,
};
use crate::facet::coordination_strategy::CoordinationStrategy;
use crate::facet::dimension_config::{FacetDimensionConfig, RowDimensionConfig};
use crate::facet::guide::{
    build_partition_for_facet, compute_scale_sharing_for_nested_facet, extend_partition_list,
};
use crate::facet::marks::facet::determine_facet_band_align;
use crate::facet::nesting::detect_nested_facet_and_compute_coordination;
use crate::facet::phantom_cells::PhantomPlacement;
use crate::facet::scalar_cmp::scalar_total_cmp;
use crate::facet::scale_helpers::build_scales_per_channel;
use crate::facet::subplot_iterator::SubplotIteration;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::builder::ScaleBuilder;
use crate::scales::extensions::ConfiguredScaleLegendExt;
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
    shared_data_extents: Option<HashMap<String, crate::scales::DomainExtent>>,
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

/// Measure facet layout and overflow (Pass 1) while keeping the outer future small
#[allow(clippy::too_many_arguments)]
async fn measure_pass<DimConfig: FacetDimensionConfig, SubplotDimsFn>(
    facet_coord: &dyn crate::coords::CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,
    dimension_scale: &ConfiguredScaleWithSpec,
    scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
    scale_sharing_for_visibility: &HashMap<String, ScaleSharing>,
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
    let coordination_context = context.coordination_context();
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && coordination_context.is_some() {
        let ctx = coordination_context.as_ref().unwrap();
        eprintln!(
            "evaluate_facet: coord_ctx present, nesting_depth={}, level_domains_count={}, channel_sharing_levels={:?}",
            ctx.nesting_depth,
            ctx.level_domains.len(),
            ctx.channel_sharing_levels
        );
        for (key, value) in &ctx.level_domains {
            eprintln!(
                "  level_domain: level={} channel={} -> {:?}",
                key.level, key.channel, value
            );
        }
    }
    let current_channel = DimConfig::channel_name();
    let domain_from_coordination = coordination_context.and_then(|ctx| {
        // Only use coordination domain if the channel matches
        // The channel check prevents outer facet from consuming inner facet's domain
        ctx.get_inner_domain_for_channel(current_channel)
    });

    // Extract uniform_cell_count early for adjusted bandwidth computation
    // When uniform Free scaling is enabled, shared scales need to use adjusted bandwidth
    // so the axis extent matches the actual subplot extent, not full column height
    let early_uniform_cell_count = coordination_context.and_then(|ctx| ctx.get_uniform_cell_count());

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
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "evaluate_facet: shared_data_extents present={}, channels={:?}",
            shared_data_extents.is_some(),
            shared_data_extents
                .as_ref()
                .map(|e| e.keys().collect::<Vec<_>>())
        );
    }

    // ========== LEVEL-BASED DOMAIN EXTRACTION (Level(N) sharing) ==========
    // For channels with Level(N) sharing where N >= 1, extract domains from level_domains.
    // This enables hierarchical scale sharing in nested facets using the Level(N) API.
    //
    // SILENT BEHAVIOR: When coordination_context is None (non-nested facet or
    // coordination disabled), Level(N) modes silently fall back to Free behavior
    // since there's no parent to share domains with. This ensures graceful
    // degradation in all contexts without errors.
    let level_based_extents: HashMap<String, crate::scales::DomainExtent> =
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

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!("  scale_sharing_by_channel: {:?}", scale_sharing_by_channel);
        if !level_based_extents.is_empty() {
            eprintln!(
                "  Level-based extents extracted for channels: {:?}, values={:?}, coord_nesting_depth={:?}",
                level_based_extents.keys().collect::<Vec<_>>(),
                level_based_extents
                    .iter()
                    .map(|(k, v)| (k, format!("{:?}", v)))
                    .collect::<Vec<_>>(),
                coordination_context.as_ref().map(|c| c.nesting_depth)
            );
        }
    }

    // Determine shared-scale usage (Shared or Level(255))
    let any_shared = scale_sharing_by_channel
        .values()
        .any(|v| v.is_fully_shared());

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

        // Extend with shared data extents ONLY for channels with is_fully_shared (Shared/Level(255))
        if let Some(ref extents) = shared_data_extents {
            let shared_only_extents: std::collections::HashMap<String, _> = extents
                .iter()
                .filter(|(channel, _)| {
                    scale_sharing_by_channel
                        .get(*channel)
                        .map(|mode| mode.is_fully_shared())
                        .unwrap_or(false)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !shared_only_extents.is_empty() {
                builder.extend_with_domain_extents(&shared_only_extents);
            }
        }

        // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
        // These extents come from the parent facet's level_domains
        if !level_based_extents.is_empty() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FACET_LEVEL: BEFORE extension, builder y len={}",
                    builder
                        .channel_builders()
                        .get("y")
                        .map(|b| match b {
                            crate::scales::builder::ChannelScaleBuilder::RadiusAware {
                                position_data,
                                ..
                            } => position_data.len(),
                            _ => 0,
                        })
                        .unwrap_or(0)
                );
            }
            builder.extend_with_domain_extents(&level_based_extents);
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "FACET_LEVEL: AFTER extension, builder y len={}, extents={:?}",
                    builder
                        .channel_builders()
                        .get("y")
                        .map(|b| match b {
                            crate::scales::builder::ChannelScaleBuilder::RadiusAware {
                                position_data,
                                ..
                            } => position_data.len(),
                            _ => 0,
                        })
                        .unwrap_or(0),
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
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FACET_LEVEL: Building initial_shared_scales, builder y len={}",
                builder
                    .channel_builders()
                    .get("y")
                    .map(|b| match b {
                        crate::scales::builder::ChannelScaleBuilder::RadiusAware {
                            position_data,
                            ..
                        } => position_data.len(),
                        _ => 0,
                    })
                    .unwrap_or(0)
            );
        }
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
    let uniform_cell_count = context
        .coordination_context()
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
    use crate::facet::subplot_iterator::ADDITIONAL_UNIFIED_CHANNELS_KEY;
    use crate::facet::subplot_iterator::SubplotIteration;
    use crate::facet::subplot_iterator::SubplotIterator;

    // Compute phantom cell layout for uniform free scaling
    // This centralizes phantom positioning logic for use in both measurement and rendering
    let phantom_layout =
        PhantomCellLayout::compute(band_align, domain_vals.len(), uniform_cell_count);

    let mut subplot_params = context.params.clone();

    // Check for same-type nesting and inject additional unified channels
    // For Row facets wrapping Row facets, we need to unify "x" (perpendicular axis)
    // For Col facets wrapping Col facets, we need to unify "y" (perpendicular axis)
    if let Some(guide) = compiled_subplot.compiled_guide.as_ref() {
        // Check if the inner subplot has a guide of the same facet type
        let is_same_type_nesting = {
            let channel = DimConfig::channel_name();
            if channel == "row" {
                // Check if inner guide is FacetRowGuide
                guide
                    .as_any()
                    .downcast_ref::<crate::facet::guide::FacetRowGuide>()
                    .is_some()
            } else if channel == "col" {
                // Check if inner guide is FacetColGuide
                guide
                    .as_any()
                    .downcast_ref::<crate::facet::guide::FacetColGuide>()
                    .is_some()
            } else {
                false
            }
        };

        if is_same_type_nesting {
            // Add perpendicular channel to unified channels
            let perpendicular_channel = if DimConfig::channel_name() == "row" {
                "x" // Row facet unifies y by default, add x for same-type nesting
            } else {
                "y" // Col facet unifies x by default, add y for same-type nesting
            };
            subplot_params.insert(
                ADDITIONAL_UNIFIED_CHANNELS_KEY.to_string(),
                ScalarValue::Utf8(Some(perpendicular_channel.to_string())),
            );
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "facet_evaluation: same-type nesting detected for {}, adding {} to unified channels",
                    DimConfig::channel_name(),
                    perpendicular_channel
                );
            }
        }
    }

    let subplot_iter = SubplotIterator::<DimConfig>::new(
        domain_vals.clone(),
        subplot_params.clone(),
        scale_sharing_for_visibility.clone(),
        context.coordination_context().cloned(),
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
        coordination_context: Option<&crate::facet::coordination::FacetCoordinationContext>,
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
            .measure_with_scales(width, height, ctx, params, scales, coordination_context, Some(filter_df))
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
    let outer_uniform_cell_count = context.coordination_context().and_then(|ctx| {
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

    // Pre-compute per-parent-cell domains for Level(N >= 1) channels BEFORE cell iteration.
    // This ensures the parent-level domain (e.g., per-Division) is available and won't be
    // overwritten by child-cell domains (e.g., per-Department).
    // For Level(1) in 3-level structures, this enables per-row sharing in FacetRow > FacetColumn
    // hierarchies by extracting domains from the filtered per-species data.
    //
    // Track coordination context as owned value that can be cloned into iterations.
    // Also includes phantom_prepend_count for uniform free scaling position adjustment.
    let pre_iteration_coord_ctx: Option<FacetCoordinationContext> =
        if let Some(coord_ctx) = coordination_context {
            // Check if any channels have Level(N >= 1) sharing
            let has_level_sharing = coord_ctx
                .channel_sharing_levels
                .values()
                .any(|&level| level >= 1);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "PRE-ITER check: has_level_sharing={}, channel_sharing_levels={:?}, nesting_depth={}",
                    has_level_sharing, coord_ctx.channel_sharing_levels, coord_ctx.nesting_depth
                );
            }

            let mut updated_ctx = if has_level_sharing {
                // Build scale builder from full df (parent-cell filtered data)
                let parent_scale_builder = compiled_subplot
                    .build_scale_builder_from_dataframe(&context.session_context, &subplot_params, df)
                    .await?;

                // Collect channels that need per-parent-cell domains
                let level_channels: Vec<String> = coord_ctx
                    .channel_sharing_levels
                    .iter()
                    .filter(|(_, level)| **level >= 1)
                    .map(|(ch, _)| ch.clone())
                    .collect();

                let channel_refs: Vec<&str> = level_channels.iter().map(|s| s.as_str()).collect();
                let extents = parent_scale_builder.extract_domain_extents(&channel_refs);

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "PRE-ITER extents (measurement): level_channels={:?}, extents.len()={}, extents={:?}",
                        level_channels,
                        extents.len(),
                        extents
                    );
                }

                if !extents.is_empty() {
                    let cell_depth = coord_ctx.nesting_depth + 1;
                    let mut new_level_domains = coord_ctx.level_domains.clone();

                    for (channel, serializable) in extents {
                        let key = LevelChannelKey::new(cell_depth, &channel);
                        // Always add parent-level domains - they take precedence
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "PRE-ITER: Adding per-parent domain for {} at depth={}: {:?}",
                                channel, cell_depth, serializable
                            );
                        }
                        new_level_domains.insert(key, serializable);
                    }

                    coord_ctx.clone().with_level_domains(new_level_domains)
                } else {
                    coord_ctx.clone()
                }
            } else {
                coord_ctx.clone()
            };

            // Add phantom prepend count for uniform free scaling position adjustment
            let prepend_count = phantom_layout.prepend_count();
            if prepend_count > 0 {
                updated_ctx.phantom_prepend_count = prepend_count;
            }

            Some(updated_ctx)
        } else {
            None
        };

    // Debug: check coordination context before cell iteration
    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        if let Some(ref ctx) = pre_iteration_coord_ctx {
            let keys: Vec<_> = ctx.level_domains.keys().collect();
            eprintln!("BEFORE-ITER: pre_iteration_coord_ctx level_domains keys: {:?}", keys);
        }
    }

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let subplot_dims = subplot_dims.clone();
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            // Start with subplot_params and iteration params merged
            let mut params_base = subplot_params.clone();
            params_base.extend(iteration.params.clone());

            // Build iteration-specific coordination context from pre_iteration_coord_ctx.
            // Update outer position for this iteration.
            // When phantoms are prepended, adjust idx to reflect rendered position.
            let adjusted_idx = idx + phantom_offset;
            let iteration_coord_ctx: Option<FacetCoordinationContext> =
                pre_iteration_coord_ctx
                    .clone()
                    .map(|ctx| ctx.with_outer_position(adjusted_idx, outer_count));

            // Coordination context is passed directly to build_plot_components (no params serialization)

            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let initial_shared_scales = initial_shared_scales.clone();
            let mut fallback_builder = fallback_builder.clone();
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
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!("CELL: Building free_scale_builder from filtered data");
                }
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
                                .map(|mode| mode.is_fully_shared())
                                .unwrap_or(false)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if !shared_only_extents.is_empty() {
                        free_scale_builder.extend_with_domain_extents(&shared_only_extents);
                    }
                }

                // For Level(N >= 2) channels, add per-cell domains to level_domains at depth+1.
                // This enables Level(N) lookups to find domains at different hierarchy levels.
                // We extract domains from free_scale_builder which was built from filtered (per-cell) data.
                // NOTE: We do NOT increment nesting_depth here - that would throw off Level(N) calculations.
                // The domains are stored at depth+1 to indicate they're from this cell's filtered data.
                let iteration_coord_ctx = if let Some(coord_ctx) = iteration_coord_ctx {
                    // Check if any channels have Level(N >= 2) sharing
                    let has_level_2_plus = coord_ctx
                        .channel_sharing_levels
                        .values()
                        .any(|&level| level >= 2);

                    if has_level_2_plus {
                        // Add per-cell domains at depth = nesting_depth + 1
                        let cell_depth = coord_ctx.nesting_depth + 1;
                        let mut new_level_domains = coord_ctx.level_domains.clone();

                        // Collect channels that need per-cell domains
                        let level_channels: Vec<String> = coord_ctx
                            .channel_sharing_levels
                            .iter()
                            .filter(|(_, level)| **level >= 2)
                            .map(|(ch, _)| ch.clone())
                            .collect();

                        // Extract domains from free_scale_builder for Level(N >= 2) channels
                        let channel_refs: Vec<&str> = level_channels.iter().map(|s| s.as_str()).collect();
                        let extents = free_scale_builder.extract_domain_extents(&channel_refs);
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "CELL: level_channels={:?}, extracted extents count={}",
                                level_channels, extents.len()
                            );
                        }

                        for (channel, serializable) in extents {
                            let key = LevelChannelKey::new(cell_depth, &channel);
                            // Only add if not already present - don't overwrite parent facet's
                            // per-cell domains. This ensures outer facet (e.g., Region) domains
                            // take precedence over inner facet (e.g., Country) domains.
                            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                let existing_keys: Vec<_> = new_level_domains.keys().collect();
                                eprintln!(
                                    "CELL: checking key {:?}, existing keys: {:?}",
                                    key, existing_keys
                                );
                            }
                            if !new_level_domains.contains_key(&key) {
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "CELL: Adding per-cell domain for {} at depth={}: {:?}",
                                        channel, cell_depth, serializable
                                    );
                                }
                                new_level_domains.insert(key, serializable);
                            } else if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "CELL: Skipping per-cell domain for {} at depth={} (already exists)",
                                    channel, cell_depth
                                );
                            }
                        }

                        // Create updated coord_ctx with new domains (but keep same nesting_depth)
                        Some(coord_ctx.with_level_domains(new_level_domains))
                    } else {
                        Some(coord_ctx)
                    }
                } else {
                    None
                };

                // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
                // Extract from coordination context (which was updated per-iteration)
                // NOTE: We use channel_sharing_levels from coord_ctx, not scale_sharing_by_channel,
                // because scale_sharing_by_channel only has the current facet level's channels,
                // while channel_sharing_levels has sharing info from all parent levels.
                if let Some(ref coord_ctx) = iteration_coord_ctx {
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "CELL: coord_ctx present, nesting_depth={}, level_domains_count={}, channel_sharing_levels={:?}, scale_sharing={:?}",
                            coord_ctx.nesting_depth,
                            coord_ctx.level_domains.len(),
                            coord_ctx.channel_sharing_levels,
                            scale_sharing_by_channel
                        );
                    }
                    let level_extents: std::collections::HashMap<String, _> =
                        coord_ctx.channel_sharing_levels
                            .iter()
                            .filter_map(|(channel, &level)| {
                                // Level >= 1 && < u8::MAX means the channel uses Level(N) sharing
                                // We exclude u8::MAX (Shared) because it's handled separately
                                // via extend_with_domain_extents from shared_data_extents
                                if level >= 1 && level < u8::MAX {
                                    let domain = coord_ctx.get_domain_for_channel(channel);
                                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                        eprintln!(
                                            "CELL: get_domain_for_channel({}) with Level({}) -> {:?}",
                                            channel, level, domain
                                        );
                                    }
                                    domain.map(|extents| (channel.clone(), extents.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect();
                    if !level_extents.is_empty() {
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "CELL: applying level_extents: {:?}",
                                level_extents
                            );
                        }
                        free_scale_builder.extend_with_domain_extents(&level_extents);
                        // ALSO extend in fallback_builder since it may have channels (like y)
                        // that aren't in the free_scale_builder at this facet level.
                        // The y scale is often backfilled from fallback_builder.
                        if let Some(ref mut builder) = fallback_builder {
                            builder.extend_with_domain_extents(&level_extents);
                        }
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            // Debug: check channel_builders after extension
                            for (ch, builder) in free_scale_builder.channel_builders() {
                                if ch == "y" {
                                    eprintln!(
                                        "CELL: after extension, channel {} builder={:?}",
                                        ch,
                                        match builder {
                                            crate::scales::builder::ChannelScaleBuilder::RadiusAware { position_data, .. } => {
                                                format!("RadiusAware(len={})", position_data.len())
                                            }
                                            crate::scales::builder::ChannelScaleBuilder::Standard { .. } => "Standard".to_string(),
                                            crate::scales::builder::ChannelScaleBuilder::ExplicitDomain { .. } => "ExplicitDomain".to_string(),
                                        }
                                    );
                                }
                            }
                            // Also check fallback_builder
                            if let Some(ref builder) = fallback_builder {
                                for (ch, builder) in builder.channel_builders() {
                                    if ch == "y" {
                                        eprintln!(
                                            "CELL: after extension, fallback channel {} builder={:?}",
                                            ch,
                                            match builder {
                                                crate::scales::builder::ChannelScaleBuilder::RadiusAware { position_data, .. } => {
                                                    format!("RadiusAware(len={})", position_data.len())
                                                }
                                                crate::scales::builder::ChannelScaleBuilder::Standard { .. } => "Standard".to_string(),
                                                crate::scales::builder::ChannelScaleBuilder::ExplicitDomain { .. } => "ExplicitDomain".to_string(),
                                            }
                                        );
                                    }
                                }
                            }
                        }
                    }
                } else if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!("CELL: No coord_ctx from params");
                }

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!("CELL: About to build scales from builder");
                }

                // Coordination context is passed directly (no params serialization)

                // Build scales directly per-channel based on sharing mode
                // This eliminates the two-step (initial build + overlay) approach
                let scales = build_scales_per_channel(
                    &compiled_subplot,
                    &scale_sharing_by_channel,
                    &initial_shared_scales,
                    &Some(free_scale_builder.clone()),
                    iteration_coord_ctx.as_ref(),
                    &fallback_builder,
                    width,
                    height,
                    &ctx,
                    &params_base,
                )
                .await?;

                let (guide_only, total_overflow, legend_positions, spacing_needs) =
                    measure_subplot(
                        &compiled_subplot,
                        width,
                        height,
                        &ctx,
                        &params_base,
                        &scales,
                        iteration_coord_ctx.as_ref(),
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
        let coord_ctx = context.coordination_context();

        // If inter_row_gap is already in coordinated_spacing, we're in the second pass - don't recompute
        let already_computed = coord_ctx
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
                .map(|ctx| ctx.inner_channel.as_deref() == Some("row"))
                .unwrap_or(false);

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  measure_pass for FacetColumn: has_nested_row_facet={} overflow_measurements.len()={} coord_ctx={:?}",
                    has_nested_row_facet,
                    overflow_measurements.len(),
                    coord_ctx.map(|c| c.inner_channel.as_deref())
                );
            }

            if has_nested_row_facet && !overflow_measurements.is_empty() {
                // Get inner domain count (number of rows) from coordination context
                let inner_domain_count = coord_ctx.map(|ctx| ctx.inner_domain_count).unwrap_or(0);

                // Check if uniform Free scaling is enabled
                // When uniform Free scaling is active, we need to coordinate inter_row_gap
                // even for columns with only 1 row, so they use the same gap as other columns
                let uniform_cell_count = coord_ctx.and_then(|ctx| ctx.get_uniform_cell_count());

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
        let coord_ctx = context.coordination_context();

        // If inter_col_gap is already in coordinated_spacing, we're in the second pass - don't recompute
        let already_computed = coord_ctx
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
                .map(|ctx| ctx.inner_channel.as_deref() == Some("column"))
                .unwrap_or(false);

            if has_nested_col_facet && !overflow_measurements.is_empty() {
                let inner_domain_count = coord_ctx.map(|ctx| ctx.inner_domain_count).unwrap_or(0);

                // Check if uniform Free scaling is enabled
                // When uniform Free scaling is active, we need to coordinate inter_col_gap
                // even for rows with only 1 column, so they use the same gap as other rows
                let uniform_cell_count = coord_ctx.and_then(|ctx| ctx.get_uniform_cell_count());

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
    scale_sharing_for_visibility: HashMap<String, ScaleSharing>,
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

    let mut subplot_params_pass2 = context.params.clone();

    // Check for same-type nesting and inject additional unified channels for pass 2 as well
    use crate::facet::subplot_iterator::ADDITIONAL_UNIFIED_CHANNELS_KEY;
    if let Some(guide) = compiled_subplot.compiled_guide.as_ref() {
        let is_same_type_nesting = {
            let channel = DimConfig::channel_name();
            if channel == "row" {
                guide
                    .as_any()
                    .downcast_ref::<crate::facet::guide::FacetRowGuide>()
                    .is_some()
            } else if channel == "col" {
                guide
                    .as_any()
                    .downcast_ref::<crate::facet::guide::FacetColGuide>()
                    .is_some()
            } else {
                false
            }
        };

        if is_same_type_nesting {
            let perpendicular_channel = if DimConfig::channel_name() == "row" {
                "x"
            } else {
                "y"
            };
            subplot_params_pass2.insert(
                ADDITIONAL_UNIFIED_CHANNELS_KEY.to_string(),
                ScalarValue::Utf8(Some(perpendicular_channel.to_string())),
            );
        }
    }

    // Use scale_sharing_for_visibility for SubplotIterator to control axis visibility
    // (not scale_sharing_by_channel which includes paired channels for scale domain building)
    let subplot_iter_pass2 = SubplotIterator::<DimConfig>::new(
        domain_vals_final,
        subplot_params_pass2.clone(),
        scale_sharing_for_visibility.clone(),
        context.coordination_context().cloned(),
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
        facet_spec: Arc<crate::facet::computed_facet_spec::EvaluatedFacetTree>,
        coordination_context: Option<FacetCoordinationContext>,
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
                facet_spec,
                coordination_context,
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
    let coordination_context = context.coordination_context();
    let outer_uniform_cell_count = coordination_context.and_then(|ctx| {
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

    // Pre-compute per-parent-cell domains for Level(N >= 1) channels BEFORE cell iteration.
    // This ensures the parent-level domain (e.g., per-Division) is available and won't be
    // overwritten by child-cell domains (e.g., per-Department).
    // For Level(1) in 3-level structures, this enables per-row sharing in FacetRow > FacetColumn
    // hierarchies by extracting domains from the filtered per-species data.
    //
    // Track coordination context as owned value that can be cloned into iterations.
    // Also includes phantom_prepend_count for uniform free scaling position adjustment.
    let pre_iteration_coord_ctx: Option<FacetCoordinationContext> =
        if let Some(coord_ctx) = coordination_context {
            // Check if any channels have Level(N >= 1) sharing
            let has_level_sharing = coord_ctx
                .channel_sharing_levels
                .values()
                .any(|&level| level >= 1);

            let mut updated_ctx = if has_level_sharing {
                // Build scale builder from full df (parent-cell filtered data)
                let parent_scale_builder = compiled_subplot
                    .build_scale_builder_from_dataframe(
                        &context.session_context,
                        &subplot_params_pass2,
                        df,
                    )
                    .await?;

                // Collect channels that need per-parent-cell domains
                let level_channels: Vec<String> = coord_ctx
                    .channel_sharing_levels
                    .iter()
                    .filter(|(_, level)| **level >= 1)
                    .map(|(ch, _)| ch.clone())
                    .collect();

                let channel_refs: Vec<&str> = level_channels.iter().map(|s| s.as_str()).collect();
                let extents = parent_scale_builder.extract_domain_extents(&channel_refs);

                if !extents.is_empty() {
                    let cell_depth = coord_ctx.nesting_depth + 1;
                    let mut new_level_domains = coord_ctx.level_domains.clone();

                    for (channel, domain_extent) in extents {
                        let key = LevelChannelKey::new(cell_depth, &channel);
                        // Always add parent-level domains - they take precedence
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "PRE-ITER: Adding per-parent domain for {} at depth={}: {:?}",
                                channel, cell_depth, domain_extent
                            );
                        }
                        new_level_domains.insert(key, domain_extent);
                    }

                    coord_ctx.clone().with_level_domains(new_level_domains)
                } else {
                    coord_ctx.clone()
                }
            } else {
                coord_ctx.clone()
            };

            // Add phantom prepend count for uniform free scaling position adjustment
            // (reuse phantom_offset which was computed from pass1.phantom_layout.prepend_count())
            if phantom_offset > 0 {
                updated_ctx.phantom_prepend_count = phantom_offset;
            }

            Some(updated_ctx)
        } else {
            None
        };

    let results: Vec<_> = stream::iter(work_items)
        .map(|(idx, iteration, rect)| {
            let compiled_subplot = Arc::clone(compiled_subplot);
            let facet_expr = facet_expr.clone();
            let ctx = context.session_context.clone();
            // Start with subplot_params_pass2 and iteration params merged
            let mut params_base = subplot_params_pass2.clone();
            params_base.extend(iteration.params.clone());

            // Build iteration-specific coordination context from pre_iteration_coord_ctx.
            // Update outer position for this iteration.
            // When phantoms are prepended, adjust idx to reflect rendered position.
            let adjusted_idx = idx + phantom_offset;
            let iteration_coord_ctx: Option<FacetCoordinationContext> =
                pre_iteration_coord_ctx
                    .clone()
                    .map(|ctx| ctx.with_outer_position(adjusted_idx, outer_count));

            // Coordination context is passed directly to build_plot_components (no params serialization)

            let scale_sharing_by_channel = scale_sharing_by_channel.clone();
            let final_shared_scales = pass1.final_shared_scales.clone();
            let mut fallback_builder = fallback_builder.clone();
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
                                .map(|mode| mode.is_fully_shared())
                                .unwrap_or(false)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if !shared_only_extents.is_empty() {
                        free_scale_builder.extend_with_domain_extents(&shared_only_extents);
                    }
                }

                // For Level(N >= 2) channels, add per-cell domains to level_domains at depth+1.
                // This enables Level(N) lookups to find domains at different hierarchy levels.
                // NOTE: We do NOT increment nesting_depth here - that would throw off Level(N) calculations.
                let iteration_coord_ctx = if let Some(coord_ctx) = iteration_coord_ctx {
                    // Check if any channels have Level(N >= 2) sharing
                    let has_level_2_plus = coord_ctx
                        .channel_sharing_levels
                        .values()
                        .any(|&level| level >= 2);

                    if has_level_2_plus {
                        // Add per-cell domains at depth = nesting_depth + 1
                        let cell_depth = coord_ctx.nesting_depth + 1;
                        let mut new_level_domains = coord_ctx.level_domains.clone();

                        // Collect channels that need per-cell domains
                        let level_channels: Vec<String> = coord_ctx
                            .channel_sharing_levels
                            .iter()
                            .filter(|(_, level)| **level >= 2)
                            .map(|(ch, _)| ch.clone())
                            .collect();

                        // Extract domains from free_scale_builder for Level(N >= 2) channels
                        let channel_refs: Vec<&str> = level_channels.iter().map(|s| s.as_str()).collect();
                        let extents = free_scale_builder.extract_domain_extents(&channel_refs);
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "CELL: level_channels={:?}, extracted extents count={}",
                                level_channels, extents.len()
                            );
                        }

                        for (channel, domain_extent) in extents {
                            let key = LevelChannelKey::new(cell_depth, &channel);
                            // Only add if not already present - don't overwrite parent facet's
                            // per-cell domains. This ensures outer facet (e.g., Region) domains
                            // take precedence over inner facet (e.g., Country) domains.
                            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                let existing_keys: Vec<_> = new_level_domains.keys().collect();
                                eprintln!(
                                    "CELL: checking key {:?}, existing keys: {:?}",
                                    key, existing_keys
                                );
                            }
                            if !new_level_domains.contains_key(&key) {
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "CELL: Adding per-cell domain for {} at depth={}: {:?}",
                                        channel, cell_depth, domain_extent
                                    );
                                }
                                new_level_domains.insert(key, domain_extent);
                            } else if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "CELL: Skipping per-cell domain for {} at depth={} (already exists)",
                                    channel, cell_depth
                                );
                            }
                        }

                        // Create updated coord_ctx with new domains (but keep same nesting_depth)
                        Some(coord_ctx.with_level_domains(new_level_domains))
                    } else {
                        Some(coord_ctx)
                    }
                } else {
                    None
                };

                // Extend with level-based extents for channels with Level(N) sharing (N >= 1)
                // Extract from coordination context (which was updated per-iteration)
                // NOTE: We use channel_sharing_levels from coord_ctx, not scale_sharing_by_channel,
                // because scale_sharing_by_channel only has the current facet level's channels,
                // while channel_sharing_levels has sharing info from all parent levels.
                if let Some(ref coord_ctx) = iteration_coord_ctx {
                    let level_extents: std::collections::HashMap<String, _> = coord_ctx.channel_sharing_levels
                        .iter()
                        .filter_map(|(channel, &level)| {
                            // Level >= 1 && < u8::MAX means the channel uses Level(N) sharing
                            // We exclude u8::MAX (Shared) because it's handled separately
                            // via extend_with_domain_extents from shared_data_extents
                            if level >= 1 && level < u8::MAX {
                                coord_ctx.get_domain_for_channel(channel)
                                    .map(|extents| (channel.clone(), extents.clone()))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !level_extents.is_empty() {
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            eprintln!(
                                "Level(N): applying extents for channels={:?}, nesting_depth={}, values={:?}",
                                level_extents.keys().collect::<Vec<_>>(),
                                coord_ctx.nesting_depth,
                                level_extents.iter().map(|(k, v)| (k, format!("{:?}", v))).collect::<Vec<_>>()
                            );
                        }
                        free_scale_builder.extend_with_domain_extents(&level_extents);
                        // ALSO extend in fallback_builder since it may have channels (like y)
                        // that aren't in the free_scale_builder at this facet level.
                        // The y scale is often backfilled from fallback_builder.
                        if let Some(ref mut builder) = fallback_builder {
                            builder.extend_with_domain_extents(&level_extents);
                        }
                    }
                }

                // Coordination context is passed directly (no params serialization)

                // Build scales directly per-channel based on sharing mode
                // This eliminates the two-step (initial build + overlay) approach
                // and ensures Pass 2 uses identical logic to Pass 1
                let scales = build_scales_per_channel(
                    &compiled_subplot,
                    &scale_sharing_by_channel,
                    &final_shared_scales,
                    &Some(free_scale_builder.clone()),
                    iteration_coord_ctx.as_ref(),
                    &fallback_builder,
                    width,
                    height,
                    &ctx,
                    &params_base,
                )
                .await?;

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
                    context.facet_spec.clone(),
                    iteration_coord_ctx.clone(),
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
    // When multiple marks specify different sharing modes for the same channel,
    // the maximum level wins (higher level = more global sharing):
    // - Level(0)/Free = most local (each cell independent)
    // - Level(N) = share N levels up the hierarchy
    // - Level(255)/Shared = most global (share across all facets)
    //
    // For consistent behavior, use the same scale sharing mode across all marks
    // in a subplot. Mixed modes may produce unexpected results.
    //
    // We compute TWO maps:
    // 1. scale_sharing_for_scales: Includes paired channels (x2/y2) for scale domain building.
    //    This ensures that if y2 has Level(1), the y-scale uses the unified domain.
    //    Uses shallow iteration (one level deep) since paired channel handling is local.
    // 2. scale_sharing_for_visibility: Uses recursive extraction to find sharing in nested facets.
    //    This ensures that for Col>Col>Col>Col with Level(4) y sharing, all levels see the
    //    correct sharing mode for global edge computation.
    let mut scale_sharing_for_scales: HashMap<String, ScaleSharing> = HashMap::new();

    // Compute scale_sharing_for_scales with paired channel handling (shallow iteration)
    for &ch in &required_channels {
        let mut max_level_for_scales: u8 = 0; // Includes paired channels

        // Determine the paired channel (x2 for x, y2 for y)
        let paired_ch = match ch {
            "x" => Some("x2"),
            "y" => Some("y2"),
            _ => None,
        };

        for m in &compiled_subplot.marks {
            // Check the primary channel
            if let Some(cv) = m.data_context().channels().get(ch) {
                if let Some(share_mode) = cv.get_share_mode() {
                    let level = share_mode.to_level();
                    max_level_for_scales = max_level_for_scales.max(level);
                }
            }
            // Check the paired channel (x2/y2) - for scale domain building only
            if let Some(paired) = paired_ch {
                if let Some(cv) = m.data_context().channels().get(paired) {
                    if let Some(share_mode) = cv.get_share_mode() {
                        max_level_for_scales = max_level_for_scales.max(share_mode.to_level());
                    }
                }
            }
        }
        scale_sharing_for_scales.insert(
            ch.to_string(),
            ScaleSharing::from_level(max_level_for_scales),
        );
    }

    // Compute scale_sharing_for_visibility using recursive extraction through nested facets.
    // This ensures Level(N) sharing from deeply nested marks is visible at all nesting levels
    // for correct global edge computation in SubplotIterator.
    let scale_sharing_for_visibility =
        compute_scale_sharing_for_nested_facet(&compiled_subplot.marks);

    // scale_sharing_by_channel is used for scale building (includes paired channels)
    let mut scale_sharing_by_channel: HashMap<String, ScaleSharing> = scale_sharing_for_scales;

    // Extract incoming coordination context from outer facet (if nested)
    // This is used for:
    // 1. Merging channel sharing levels for Cartesian subplot measurement
    // 2. Passing to nested facet detection for Level(N>1) domain propagation
    let incoming_coord_ctx = context.coordination_context();

    // Merge channel_sharing_levels from incoming coordination context (from outer facet)
    // This enables inner facets to know the x/y scale sharing for Cartesian subplot measurement
    if let Some(coord_ctx) = incoming_coord_ctx {
        for (channel, level) in &coord_ctx.channel_sharing_levels {
            // Only add if not already present (don't override explicit channel settings)
            scale_sharing_by_channel
                .entry(channel.clone())
                .or_insert_with(|| ScaleSharing::from_level(*level));
        }
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
            && !coord_ctx.channel_sharing_levels.is_empty()
        {
            eprintln!(
                "  Merged channel_sharing_levels from coordination context: {:?}",
                coord_ctx.channel_sharing_levels
            );
        }
    }

    // ========== BUILD OUTER FACET'S PARTITION ==========
    // Before detecting nested facets, add the current (outer) facet's partition to the
    // coordination context. This enables the grammar-based visibility model to track
    // the complete partition hierarchy from outermost to innermost facet.
    //
    // The partition list flows:
    // 1. Incoming partition list (from grandparent facets, if any)
    // 2. Add this facet's partition
    // 3. Pass to detect_nested_facet which adds the inner facet's partition
    // 4. Complete list flows to deeply nested facets

    // Extract domain values from the dimension scale
    let outer_domain_vals: Vec<ScalarValue> = dimension_scale
        .domain_values()
        .ok()
        .map(|dv| match dv {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        })
        .unwrap_or_default();

    // Determine the outer facet's direction
    let outer_direction = if DimConfig::is_row_facet() {
        crate::guide::FacetDirection::Row
    } else {
        crate::guide::FacetDirection::Column
    };

    // Try to extract facet scale sharing from the channel definition
    // Default to Free if not found (conservative - each cell computes own domain)
    let outer_facet_scale_sharing = state
        .data
        .channels()
        .get(DimConfig::channel_name())
        .and_then(|cv| cv.get_share_mode());

    // Build the outer facet's partition
    let outer_partition = build_partition_for_facet(
        DimConfig::channel_name(),
        outer_direction,
        &outer_domain_vals,
        outer_facet_scale_sharing,
    );

    // Extend the incoming partition list with the outer facet's partition,
    // but ONLY if this facet wasn't already added by a parent's detect_nested_facet_and_compute_coordination().
    // When a parent facet detects this facet as its inner facet, it adds the partition to the list.
    // We detect this by checking if the last partition in the incoming list matches our channel.
    let incoming_partition_list = incoming_coord_ctx.and_then(|ctx| ctx.partition_list.as_ref());

    let should_add_partition = match incoming_partition_list {
        Some(list) => {
            // Check if the last partition already matches this facet's channel
            // If so, the parent already added us - don't double-add
            let last_partition = list.partitions.last();
            !matches!(last_partition, Some(p) if p.field == DimConfig::channel_name())
        }
        None => true, // No incoming list, definitely add
    };

    let updated_partition_list = if should_add_partition {
        extend_partition_list(incoming_partition_list, outer_partition)
    } else {
        // Use the incoming list as-is (our partition was already added)
        incoming_partition_list.cloned().unwrap_or_default()
    };

    // Create an updated coordination context with the partition list
    // If there's no incoming context, create a minimal one just for the partition list
    let updated_coord_ctx: Option<FacetCoordinationContext> = if let Some(ctx) = incoming_coord_ctx
    {
        let mut updated = ctx.clone();
        updated.partition_list = Some(updated_partition_list);
        Some(updated)
    } else {
        Some(FacetCoordinationContext::default().with_partition_list(updated_partition_list))
    };

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        if let Some(ref ctx) = updated_coord_ctx {
            if let Some(ref pl) = ctx.partition_list {
                if should_add_partition {
                    eprintln!(
                        "  Added outer facet partition: channel={} direction={:?} domain_len={} partition_list_depth={}",
                        DimConfig::channel_name(),
                        outer_direction,
                        outer_domain_vals.len(),
                        pl.depth()
                    );
                } else {
                    eprintln!(
                        "  Skipped adding partition (already present): channel={} partition_list_depth={}",
                        DimConfig::channel_name(),
                        pl.depth()
                    );
                }
            }
        }
    }

    // Detect nested facets and compute coordination context
    // This enables guide ownership coordination (axis label visibility) for nested facets.
    // Note: We do NOT propagate domain by default - let each inner facet use its own filtered domain.
    // Domain propagation (for grid-like behavior) can be enabled via explicit configuration.
    // Pass updated_coord_ctx (with outer partition) to enable complete partition list building.
    // Use pre-computed facet spec from context for query optimization
    let coordination_context = detect_nested_facet_and_compute_coordination(
        compiled_subplot,
        &df,
        ctx,
        DimConfig::is_row_facet(), // Whether this (outer) facet is a row facet
        &facet_expr,               // Outer facet's expression for inner cell counting
        &context.params,           // Parameters for evaluating explicit domains
        updated_coord_ctx.as_ref(), // Updated context with outer facet's partition
        context.facet_spec(),      // Pre-computed facet spec for efficient domain lookups
    )
    .await?;

    // Create modified context with coordination context if a nested facet was detected
    let modified_context: Option<RenderContext>;
    let effective_context: &RenderContext = if let Some(coord_ctx) = coordination_context {
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Injecting coordination context for nested facet: inner_domain_len={:?}, scale_sharing={:?}",
                coord_ctx.inner_domain.as_ref().map(|d| d.len()),
                coord_ctx.inner_scale_sharing
            );
        }
        let new_ctx = RenderContext::new(
            context.theme.clone(),
            context.plot_width,
            context.plot_height,
            context.session_context.clone(),
            context.params.clone(),
            context.scales.clone(),
            context.facet_spec.clone(),
            Some(coord_ctx),
        );
        modified_context = Some(new_ctx);
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
        &scale_sharing_for_visibility,
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
            let mut coord_ctx = effective_context
                .coordination_context()
                .cloned()
                .unwrap_or_default();
            coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Phase 1.5 (Rerun): Re-running measure_pass with coordinated_spacing={:?}",
                    pass1.spacing_needs
                );
            }

            let updated_ctx = RenderContext::new(
                effective_context.theme.clone(),
                effective_context.plot_width,
                effective_context.plot_height,
                effective_context.session_context.clone(),
                effective_context.params.clone(),
                effective_context.scales.clone(),
                effective_context.facet_spec.clone(),
                Some(coord_ctx),
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
                &scale_sharing_for_visibility,
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
            let mut coord_ctx = effective_context
                .coordination_context()
                .cloned()
                .unwrap_or_default();
            coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "  Phase 1.5 (UpdateContextOnly): coordinated_spacing={:?}",
                    pass1.spacing_needs
                );
            }

            let updated_ctx = RenderContext::new(
                effective_context.theme.clone(),
                effective_context.plot_width,
                effective_context.plot_height,
                effective_context.session_context.clone(),
                effective_context.params.clone(),
                effective_context.scales.clone(),
                effective_context.facet_spec.clone(),
                Some(coord_ctx),
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
        scale_sharing_for_visibility,
        final_pass,
        &group_origin,
    )
    .await
}
