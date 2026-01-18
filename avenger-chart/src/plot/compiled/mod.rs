//! CompiledPlot - Immutable, serializable plot ready for rendering

pub(crate) mod expr_eval;
mod legends;
pub(crate) mod rendering;
pub mod scale_provider;
pub(crate) mod scales; // Made public so plot.rs can call build_scale_builder_from_marks
mod titles;
mod validation;

use std::collections::HashMap;
use std::sync::Arc;

use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};
use crate::coords::CoordinateSystemTransform;
use crate::guide::{CompiledGuide, OverflowSpaceRequirement};
use crate::layout::{LayoutBounds, LayoutResult, LayoutSpec};
use crate::legend::Legend;
use crate::marks::CompiledMark;
use crate::plot::compiled::scales::build_scale_builder_from_marks;
use crate::scales::ConfiguredScaleWithSpec;
use crate::serialization::SerializableDataFrame;
use crate::theme::Theme;

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct CompiledPlot {
    /// Coordinate system transform for position mapping
    pub(crate) coord_transform: Box<dyn CoordinateSystemTransform>,

    /// Guide renderer for axes/grids
    pub(crate) compiled_guide: Option<Arc<dyn CompiledGuide>>,

    /// Mark renderers
    pub(crate) marks: Vec<Arc<dyn CompiledMark>>,

    /// Axis specifications
    pub(crate) axis_specs: HashMap<String, AxisSpec>,

    /// Legends
    pub(crate) legends: IndexMap<String, Legend>,

    /// Layout specification
    pub(crate) layout_spec: LayoutSpec,

    /// Plot title
    pub(crate) title: Option<PlotTitle>,

    /// Plot subtitle
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme
    pub(crate) theme: Option<Arc<Theme>>,

    /// Mapping from scale names to coordinate channel
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Scale specifications (temporarily kept for building scales)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    // Note: We intentionally do not persist a ScaleBuilder here. Scales are
    // rebuilt per evaluation using current params to ensure correctness for
    // paramized data queries and to keep direct vs serialized paths identical.
    /// Plot-level data (temporarily kept for mark inheritance)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    pub(crate) data: Option<LogicalPlanNode>,

    /// Default parameter values for prepared statements
    #[serde_as(as = "FromInto<crate::serialization::SerializableScalarMap>")]
    pub(crate) default_params: IndexMap<String, datafusion::common::ScalarValue>,
}

impl CompiledPlot {
    /// Get the theme or create default if not set
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }

    /// Get title if configured
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Get subtitle if configured
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }

    /// Get layout spec
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    /// Get default parameter values
    pub fn get_default_params(&self) -> &IndexMap<String, datafusion::common::ScalarValue> {
        &self.default_params
    }

    /// Get compiled mark renderers
    pub fn marks(&self) -> &[Arc<dyn CompiledMark>] {
        &self.marks
    }

    /// Build a ScaleBuilder using a provided DataFrame override.
    ///
    /// This is primarily used by faceting when building free scales per facet, so that
    /// domain inference (including radius-aware padding) runs against the facet-filtered data.
    ///
    /// If the params contain a FacetCoordinationContext with shared_data_extents,
    /// the builder will be extended to include those extents. This ensures nested facets
    /// use the full dataset range for their scales.
    pub(crate) async fn build_scale_builder_from_dataframe(
        &self,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        df: &datafusion::dataframe::DataFrame,
    ) -> Result<crate::scales::builder::ScaleBuilder, crate::error::AvengerChartError> {
        let builder = build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            Some(df.clone()),
            ctx,
            params,
            self.get_theme().as_ref(),
        )
        .await?;

        Ok(builder)
    }

    /// Build scales with specific dimensions for the current evaluation.
    ///
    /// We rebuild a temporary ScaleBuilder on each call using the current params
    /// so domain inference reflects paramized data queries. This keeps direct
    /// and serialized paths identical and avoids stale caches.
    ///
    /// If params contain a FacetCoordinationContext with shared_data_extents,
    /// the builder will be extended to include those extents.
    pub async fn build_scales_with_dimensions(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        crate::error::AvengerChartError,
    > {
        // Always rebuild a temporary ScaleBuilder using current params.
        let builder = build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            None,
            ctx,
            params,
            self.get_theme().as_ref(),
        )
        .await?;

        // Build coordinate system ranges map
        let mut coord_system_ranges = std::collections::HashMap::new();
        for channel in builder.channel_builders().keys() {
            use crate::channel::value::strip_trailing_numbers;
            let base = strip_trailing_numbers(channel);
            if let Some((min, max)) = self.coord_transform.default_range(
                base,
                plot_area_width as f64,
                plot_area_height as f64,
            ) {
                coord_system_ranges.insert(channel.clone(), (min, max));
            }
        }

        let theme = self.get_theme();
        let built = builder
            .build_scales(
                plot_area_width,
                plot_area_height,
                &coord_system_ranges,
                &self.scale_specs,
                &self.marks,
                theme.as_ref(),
                ctx,
                params,
            )
            .await?;

        Ok(built)
    }

    /// Build scales from an existing ScaleBuilder with specific dimensions.
    ///
    /// This is more efficient than `build_scales_with_dimensions` when you need to
    /// build scales multiple times (e.g., initial layout pass and final render pass)
    /// because it reuses the cached data queries from the provided builder.
    ///
    /// Typical usage:
    /// ```ignore
    /// // Build once (queries data)
    /// let builder = build_scale_builder_from_marks(...).await?;
    ///
    /// // Reuse multiple times (no queries)
    /// let initial_scales = plot.build_scales_from_builder(&builder, 400.0, 300.0, ctx, params).await?;
    /// let final_scales = plot.build_scales_from_builder(&builder, 800.0, 600.0, ctx, params).await?;
    /// ```
    pub async fn build_scales_from_builder(
        &self,
        builder: &crate::scales::ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        crate::error::AvengerChartError,
    > {
        // Build coordinate system ranges map
        let mut coord_system_ranges = std::collections::HashMap::new();
        for channel in builder.channel_builders().keys() {
            use crate::channel::value::strip_trailing_numbers;
            let base = strip_trailing_numbers(channel);
            if let Some((min, max)) = self.coord_transform.default_range(
                base,
                plot_area_width as f64,
                plot_area_height as f64,
            ) {
                coord_system_ranges.insert(channel.clone(), (min, max));
            }
        }

        let theme = self.get_theme();
        let built = builder
            .build_scales(
                plot_area_width,
                plot_area_height,
                &coord_system_ranges,
                &self.scale_specs,
                &self.marks,
                theme.as_ref(),
                ctx,
                params,
            )
            .await?;

        Ok(built)
    }

    /// Get scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    /// Build configured scales for a provided DataFrame and plot-area dimensions.
    /// Note: This initial implementation does not yet override the plot-level data source
    /// stored in this CompiledPlot; full support will be added in the next phase.
    pub async fn build_scales_for_dataframe(
        &self,
        df: &datafusion::dataframe::DataFrame,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        crate::error::AvengerChartError,
    > {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::scales::ConfiguredScaleWithSpec;
        use crate::scales::{Scale, spec::Auto};
        use avenger_scales::scales::ScaleImpl;
        use datafusion::logical_expr::{Expr, lit};

        // Collect channels that need scales
        let mut channels_with_scales = self.collect_channels_needing_scales(ctx);
        // Also include any explicit plot-level scales
        for ch in self.scale_specs.keys() {
            channels_with_scales.insert(ch.clone());
        }

        let mut configured: std::collections::HashMap<String, ConfiguredScaleWithSpec> =
            std::collections::HashMap::new();

        // Build each scale using provided DataFrame for type inference and domain collection
        for channel in channels_with_scales.iter() {
            // Find first mark that uses this channel and get its expr and preferred scale type
            let mut chosen_spec: Option<Box<dyn crate::scales::ScaleSpec>> = None;
            let mut data_type: Option<datafusion::arrow::datatypes::DataType> = None;
            let mut expr_opt: Option<Expr> = None;

            for mark in &self.marks {
                let channels = mark.data_context().channels();
                let resolved =
                    resolve_all_channel_refs(channels, ctx).unwrap_or_else(|_| channels.clone());
                if let Some(channel_value) = resolved.get(channel) {
                    // Use scale_input_expr for type inference - only values that
                    // pass through the scale matter. Literal branches get NULL.
                    if let Some(expr) = channel_value.scale_input_expr(ctx) {
                        // Try to infer type directly from schema for simple column refs
                        let inferred_dt = match &expr {
                            datafusion::logical_expr::Expr::Column(col) => {
                                let name = col.name.clone();
                                df.schema()
                                    .field_with_unqualified_name(&name)
                                    .ok()
                                    .map(|f| f.data_type().clone())
                            }
                            _ => None,
                        };

                        if let Some(dt) = inferred_dt {
                            data_type = Some(dt.clone());
                            chosen_spec = mark.preferred_scale_type(channel, &dt);
                            expr_opt = Some(expr);
                            break;
                        } else if let Ok(projected) =
                            df.clone().select(vec![expr.clone().alias("__t")])
                        {
                            // Fallback: project the expression
                            let dt = projected.schema().field(0).data_type().clone();
                            data_type = Some(dt.clone());
                            chosen_spec = mark.preferred_scale_type(channel, &dt);
                            expr_opt = Some(expr);
                            break;
                        }
                    }
                }
            }

            let scale_spec = chosen_spec.ok_or_else(|| {
                crate::error::AvengerChartError::InternalError(format!(
                    "Failed to infer scale specification for channel '{}' (no matching mark expr)",
                    channel
                ))
            })?;

            let mut scale = Scale::<Auto>::from_spec(scale_spec);

            // Apply coord and mark default options
            if let Some(dt) = &data_type {
                // Get an impl for option discovery
                let scale_impl: std::sync::Arc<dyn ScaleImpl> =
                    scale.get_scale_impl().ok_or_else(|| {
                        crate::error::AvengerChartError::InternalError(format!(
                            "Failed to create scale impl for '{}'",
                            channel
                        ))
                    })?;

                // Coordinate defaults
                let coord_opts = self
                    .coord_transform
                    .default_scale_options(channel, scale_impl.as_ref());
                for (k, v) in coord_opts {
                    scale = scale.option(&k, lit(v));
                }
                // Mark defaults (first mark that had expr)
                if let Some(mark) = self
                    .marks
                    .iter()
                    .find(|m| m.data_context().channels().contains_key(channel.as_str()))
                {
                    let mark_opts = mark.default_scale_options(channel, scale_impl.as_ref(), dt);
                    for (k, v) in mark_opts {
                        scale = scale.option(&k, v);
                    }
                }
            }

            // Apply default range for positional channels
            if let Some((min, max)) = self.coord_transform.default_range(
                channel,
                plot_area_width as f64,
                plot_area_height as f64,
            ) {
                scale = scale.range_interval(lit(min), lit(max));
            }

            // Domain: use provided df + expr
            if let Some(expr) = expr_opt.clone() {
                scale = scale.domain_data_fields(vec![(std::sync::Arc::new(df.clone()), expr)]);
            }

            // Infer domain and normalize
            scale = scale
                .infer_domain_from_data(plot_area_width, plot_area_height, ctx, params)
                .await?;
            scale = scale
                .normalize_domain(plot_area_width, plot_area_height, ctx, params)
                .await?;

            // Apply default range for non-positional channels (color, size, etc.) if not already set
            if scale.get_range().is_none() {
                let range_kind = scale
                    .get_scale_impl()
                    .map(|impl_arc| impl_arc.range_kind())
                    .unwrap_or(avenger_scales::scales::RangeKind::Continuous);

                let range = crate::scales::default_range_for_channel(channel, range_kind);
                scale = scale.range(range);
            }

            // Create configured
            let configured_scale = scale
                .clone()
                .create_configured_scale(plot_area_width, plot_area_height, ctx, params)
                .await?;
            configured.insert(
                channel.clone(),
                ConfiguredScaleWithSpec::new(scale, configured_scale),
            );
        }

        Ok(configured)
    }

    /// Measure guide overflow for the inner plot using provided scales and dimensions.
    pub async fn measure_guide_overflow_with_scales(
        &self,
        scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        row_overflow: Option<&Vec<crate::guide::OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<crate::guide::OverflowSpaceRequirement>>,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<crate::guide::OverflowSpaceRequirement, crate::error::AvengerChartError> {
        if let Some(guide) = &self.compiled_guide {
            // Downcast to ConfiguredScale for the guide API
            let configured: HashMap<String, avenger_scales::scales::ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            let theme = self.get_theme();
            guide
                .measure_overflow(
                    &configured,
                    row_overflow,
                    col_overflow,
                    plot_area_width,
                    plot_area_height,
                    theme.as_ref(),
                    params,
                    data_override,
                    ctx,
                )
                .await
        } else {
            Ok(crate::guide::OverflowSpaceRequirement::default())
        }
    }

    /// Lightweight measurement entry point used by facet Pass 1 to avoid full render recursion.
    ///
    /// - Assumes `width`/`height` are plot-area dimensions.
    /// - Uses provided scales (no scale building).
    /// - Forwards `data_override` so nested facets measure with filtered data.
    ///
    /// Returns:
    /// - `guide_only_overflow`: Overflow for axis tick labels/titles only (for cross-subplot alignment)
    /// - `total_overflow`: Guide overflow + legend dimensions (for outer facet positioning)
    /// - `legend_info`: Legend layout info for cross-subplot alignment
    /// - `legend_positions`: Set of legend positions used in this subplot
    pub async fn measure_with_scales(
        &self,
        width: f32,
        height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<
        (
            crate::guide::OverflowSpaceRequirement, // guide_only_overflow
            crate::guide::OverflowSpaceRequirement, // total_overflow (guide + legends)
            crate::layout::LegendLayoutInfo,
            std::collections::HashSet<crate::legend::LegendPosition>, // legend_positions
        ),
        crate::error::AvengerChartError,
    > {
        // Measure guide overflow using provided scales and data.
        // Legends and titles also contribute; reuse the layout computation but skip mark rendering.
        let layout = self
            .compute_layout_with_fixed_plot_area(width, height, scales, ctx, params, data_override)
            .await?;

        // Collect legend positions from the layout
        let legend_positions: std::collections::HashSet<crate::legend::LegendPosition> = layout
            .taffy_layout
            .legends_by_position
            .keys()
            .cloned()
            .collect();

        // Compute total overflow by adding legend dimensions to guide overflow.
        // The guide_only_overflow is axis tick labels/titles.
        // For facet label positioning, we need guide overflow + legend dimensions.
        let mut total_overflow = layout.guide_only_overflow.clone();

        // Add legend dimensions for each position
        use crate::legend::LegendPosition;
        for (position, legend_names) in &layout.taffy_layout.legends_by_position {
            // Sum up dimensions for all legends at this position
            let mut total_width = 0.0f32;
            let mut total_height = 0.0f32;
            for name in legend_names {
                if let Some(bounds) = layout.taffy_layout.legends.get(name) {
                    total_width = total_width.max(bounds.width);
                    total_height = total_height.max(bounds.height);
                }
            }

            match position {
                LegendPosition::Right => total_overflow.right += total_width,
                LegendPosition::Left => total_overflow.left += total_width,
                LegendPosition::Top => total_overflow.top += total_height,
                LegendPosition::Bottom => total_overflow.bottom += total_height,
            }
        }

        // Return both guide-only overflow (for alignment) and total overflow (for positioning)
        Ok((
            layout.guide_only_overflow,
            total_overflow,
            layout.legend_info,
            legend_positions,
        ))
    }

    /// Measure only intrinsic subplot overflow (excluding facet-level content)
    ///
    /// This is similar to `measure_with_scales()` but calls the guide's
    /// `measure_intrinsic_overflow()` method instead of `measure_overflow()`.
    /// This returns only the overflow needed by the Cartesian axes, without
    /// facet labels, titles, or unified axis titles.
    ///
    /// Used by outer facets when measuring nested facet subplots to avoid
    /// double-counting facet-level spacing.
    pub async fn measure_intrinsic_with_scales(
        &self,
        width: f32,
        height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<crate::guide::OverflowSpaceRequirement, crate::error::AvengerChartError> {
        // Get theme for measurement
        let theme = self.get_theme();

        // Call guide's measure_intrinsic_overflow if guide exists
        if let Some(ref compiled_guide) = self.compiled_guide {
            let configured_scales: HashMap<String, avenger_scales::scales::ConfiguredScale> =
                scales
                    .iter()
                    .map(|(k, v)| (k.clone(), v.configured().clone()))
                    .collect();

            compiled_guide
                .measure_intrinsic_overflow(
                    &configured_scales,
                    None,
                    None,
                    width,
                    height,
                    &theme,
                    params,
                    data_override,
                    ctx,
                )
                .await
        } else {
            // No guide - return zero overflow
            Ok(crate::guide::OverflowSpaceRequirement {
                top: 0.0,
                bottom: 0.0,
                left: 0.0,
                right: 0.0,
            })
        }
    }

    /// Get spacing needs from the guide's self-coordinating measurement
    ///
    /// This calls the guide's `measure_with_coordination()` method and extracts
    /// the `spacing_needs` from the `MeasurementResult`. This is used by outer
    /// facets to extract spacing needs computed by inner guides (e.g., inter_row_gap,
    /// inter_col_gap) so they can be aggregated and used for layout coordination.
    ///
    /// # Returns
    /// A HashMap of spacing keys to values (e.g., "inter_row_gap" -> 45.0).
    /// Returns empty HashMap if no guide or no spacing needs.
    pub async fn get_guide_spacing_needs(
        &self,
        width: f32,
        height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<HashMap<String, f32>, crate::error::AvengerChartError> {
        if let Some(ref compiled_guide) = self.compiled_guide {
            // Convert ConfiguredScaleWithSpec -> ConfiguredScale for guide API
            let configured_scales: HashMap<String, avenger_scales::scales::ConfiguredScale> =
                scales
                    .iter()
                    .map(|(k, v)| (k.clone(), v.configured().clone()))
                    .collect();

            let theme = self.get_theme();
            let result = compiled_guide
                .measure_with_coordination(
                    &configured_scales,
                    None, // row_overflow
                    None, // col_overflow
                    width,
                    height,
                    &theme,
                    params,
                    data_override,
                    ctx,
                )
                .await?;

            Ok(result.spacing_needs)
        } else {
            // No guide - return empty spacing needs
            Ok(HashMap::new())
        }
    }
}

/// Evaluation mode for two-pass rendering
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EvaluationMode {
    /// Measure overflow only (Pass 1 of two-pass rendering)
    Measure,
    /// Full rendering with all components (Pass 2 of two-pass rendering)
    Render,
}

/// Extended components returned from plot evaluation
///
/// This structure supports both measurement and rendering modes,
/// and includes all plot components (data, guides, legends, titles).
pub struct PlotComponents {
    /// Data mark scene graph elements
    pub data_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Guide mark scene graph elements (axes, grids)
    pub guide_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Legend scene graph elements
    pub legend_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Title scene graph elements
    pub title_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Subtitle scene graph elements
    pub subtitle_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Plot area bounds (data rectangle)
    pub plot_bounds: crate::layout::LayoutBounds,

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Size dimensions used for this evaluation
    pub size: (f32, f32),

    /// Whether `size` represents canvas dimensions (true) or plot area dimensions (false)
    pub size_is_canvas: bool,

    /// Guide overflow measurement (populated in Measure mode)
    pub overflow: Option<crate::guide::OverflowSpaceRequirement>,

    /// Debug marks (layout visualization) - these are in absolute canvas coordinates
    pub debug_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,
}

/// Complete plot measurement result that render pass uses directly.
///
/// This struct captures all layout decisions made during measurement so that
/// the render pass can use them without recomputation. The critical invariant is:
/// **scales MUST have ranges matching plot_area_size**.
///
/// Note: This is distinct from `guide::overflow::MeasurementResult` which is used
/// for guide overflow measurement. This struct captures the entire plot's measurement
/// including scales, layout, and overflow.
///
/// # Invariants
///
/// - `scales` ranges must match `plot_area_size` (validated in debug builds)
/// - `plot_bounds.width` == `plot_area_size.0`
/// - `plot_bounds.height` == `plot_area_size.1`
///
/// # Usage
///
/// ```ignore
/// // Measurement pass produces the result
/// let measurement = plot.measure_for_render(ctx, params).await?;
///
/// // Render pass uses it directly - NO recomputation
/// let components = plot.render_with_measurement(ctx, params, &measurement).await?;
/// ```
#[derive(Debug)]
pub struct PlotMeasurementResult {
    // ═══════════════════════════════════════════════════════════════
    // FINAL DIMENSIONS - render uses directly, never recomputes
    // ═══════════════════════════════════════════════════════════════
    /// Final plot area dimensions (width, height)
    pub plot_area_size: (f32, f32),

    /// Final canvas dimensions (width, height)
    pub canvas_size: (f32, f32),

    /// Plot bounds with position (x, y, width, height)
    pub plot_bounds: LayoutBounds,

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    // ═══════════════════════════════════════════════════════════════
    // VALIDATED SCALES - ranges MUST match plot_area_size
    // ═══════════════════════════════════════════════════════════════
    /// Scales built with final dimensions (NOT initial estimate).
    ///
    /// CRITICAL: These scales have ranges that match `plot_area_size`.
    /// If scales were built with width=165 but final layout is width=163,
    /// we rebuild them with width=163 before storing here.
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,

    // ═══════════════════════════════════════════════════════════════
    // OVERFLOW AND LAYOUT - for guide/legend positioning
    // ═══════════════════════════════════════════════════════════════
    /// Guide overflow measurements
    pub overflow: OverflowSpaceRequirement,

    /// Complete layout result for legends/titles positioning
    pub layout_result: LayoutResult,

    /// Per-row overflow (for nested facet coordination)
    pub row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,

    /// Per-column overflow (for nested facet coordination)
    pub col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl PlotMeasurementResult {
    /// Validate that scales have ranges consistent with plot_area_size.
    ///
    /// This is called in debug builds to catch bugs where scales are built
    /// with one dimension but stored alongside a different dimension.
    #[cfg(debug_assertions)]
    pub fn validate_scale_ranges(&self) {
        let (plot_width, plot_height) = self.plot_area_size;

        for (name, scale_with_spec) in &self.scales {
            let configured = scale_with_spec.configured();

            // Check if this is a position scale (x or y channel)
            let is_x_scale = name == "x" || name.starts_with("x");
            let is_y_scale = name == "y" || name.starts_with("y");

            if is_x_scale || is_y_scale {
                let expected_range = if is_x_scale { plot_width } else { plot_height };

                // Get the numeric range from the scale
                if let Ok((range_min, range_max)) = configured.numeric_interval_range() {
                    let range_span = (range_max - range_min).abs();

                    // Allow small tolerance for floating point
                    let tolerance = 0.1;
                    if (range_span - expected_range).abs() > tolerance {
                        eprintln!(
                            "WARNING: Scale '{}' has range span {:.2} but expected {:.2} (plot_area_size)",
                            name, range_span, expected_range
                        );
                    }
                }
            }
        }
    }

    /// Create a new MeasurementResult with validation in debug builds.
    pub fn new(
        plot_area_size: (f32, f32),
        canvas_size: (f32, f32),
        plot_bounds: LayoutBounds,
        clip: avenger_scenegraph::marks::group::Clip,
        scales: HashMap<String, ConfiguredScaleWithSpec>,
        overflow: OverflowSpaceRequirement,
        layout_result: LayoutResult,
        row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
        col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    ) -> Self {
        let result = Self {
            plot_area_size,
            canvas_size,
            plot_bounds,
            clip,
            scales,
            overflow,
            layout_result,
            row_overflow_by_facet,
            col_overflow_by_facet,
        };

        #[cfg(debug_assertions)]
        result.validate_scale_ranges();

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that PlotMeasurementResult correctly validates scale ranges.
    #[test]
    fn test_plot_measurement_result_validation() {
        use avenger_scenegraph::marks::group::Clip;

        // Create a minimal MeasurementResult for testing
        let plot_area_size = (200.0, 150.0);
        let canvas_size = (300.0, 250.0);
        let plot_bounds = LayoutBounds {
            x: 50.0,
            y: 50.0,
            width: 200.0,
            height: 150.0,
        };
        let clip = Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 150.0,
        };
        let scales = HashMap::new();
        let overflow = OverflowSpaceRequirement::default();
        let layout_result = LayoutResult {
            plot_area: plot_bounds,
            guide_overflows: HashMap::new(),
            legends: indexmap::IndexMap::new(),
            legends_by_position: indexmap::IndexMap::new(),
            title: None,
            subtitle: None,
        };

        // This should not panic
        let result = PlotMeasurementResult::new(
            plot_area_size,
            canvas_size,
            plot_bounds,
            clip,
            scales,
            overflow,
            layout_result,
            None,
            None,
        );

        // Verify fields were set correctly
        assert_eq!(result.plot_area_size, plot_area_size);
        assert_eq!(result.canvas_size, canvas_size);
        assert_eq!(result.plot_bounds.width, plot_area_size.0);
        assert_eq!(result.plot_bounds.height, plot_area_size.1);
    }
}
