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
use crate::guide::CompiledGuide;
use crate::layout::LayoutSpec;
use crate::legend::Legend;
use crate::marks::CompiledMark;
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

    /// Scale specifications for building scales
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    // Note: We intentionally do not persist a ScaleBuilder here. Scales are
    // rebuilt per evaluation using current params to ensure correctness for
    // paramized data queries and to keep direct vs serialized paths identical.
    /// Plot-level data for mark data inheritance
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

    /// Build scales from an existing ScaleBuilder with specific dimensions.
    ///
    /// Reuses cached data queries from the provided builder, making it efficient
    /// for building scales multiple times (e.g., initial layout pass and final
    /// render pass).
    ///
    /// Typical usage:
    /// ```ignore
    /// // Build once (queries data)
    /// let builder = build_scale_builder_from_marks(...).await?;
    ///
    /// // Reuse multiple times (no queries)
    /// let scales1 = plot.build_scales_from_builder(&builder, 400.0, 300.0, ctx, params).await?;
    /// let scales2 = plot.build_scales_from_builder(&builder, 800.0, 600.0, ctx, params).await?;
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

        // Convert to sorted Vec for deterministic iteration order
        let mut sorted_channels: Vec<String> = channels_with_scales.into_iter().collect();
        sorted_channels.sort();

        let mut configured: std::collections::HashMap<String, ConfiguredScaleWithSpec> =
            std::collections::HashMap::new();

        // Build each scale using provided DataFrame for type inference and domain collection
        for channel in sorted_channels.iter() {
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
            // Use base scale name (strip trailing numbers like y2 -> y) as key
            // to match lookup semantics during rendering
            use crate::channel::value::strip_trailing_numbers;
            let scale_key = strip_trailing_numbers(channel).to_string();
            configured.insert(
                scale_key,
                ConfiguredScaleWithSpec::new(scale, configured_scale),
            );
        }

        Ok(configured)
    }
}

/// Measurement results from `measure_plot_components`
///
/// This captures all the computation needed for layout coordination without
/// actually rendering any marks. The render pass uses this to avoid re-measuring.
///
/// Overflow info is available via `layout.overflow` (guide only) and
/// `layout.total_overflow` (guide + legends).
pub struct ComponentsMeasurement {
    /// Coordinate-system-specific measurement data (e.g., facet cell layout)
    ///
    /// This is computed by `coord_transform.measure()` and contains layout data
    /// that's available to both guides and marks during rendering. For facet
    /// coordinate systems, this includes cell positions, subplot measurements,
    /// and computed padding. For non-facet coordinate systems, this is an
    /// `EmptyCoordMeasurement`.
    pub coord_measurement: Box<dyn crate::coords::CoordMeasurement>,

    /// Scales for rendering
    pub scales: std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,

    /// Plot area dimensions
    pub plot_area_width: f32,
    pub plot_area_height: f32,

    /// Canvas size
    pub canvas_size: (f32, f32),

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Layout solution (contains overflow, total_overflow, legends/titles positioning)
    pub layout: crate::render::LayoutSolution,

    /// Merged params (defaults + provided + canvas dimensions)
    pub params: indexmap::IndexMap<String, datafusion::common::ScalarValue>,
}

impl std::fmt::Debug for ComponentsMeasurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentsMeasurement")
            .field("coord_measurement", &"<coord_measurement>")
            .field("scales", &format!("{} scales", self.scales.len()))
            .field("plot_area_width", &self.plot_area_width)
            .field("plot_area_height", &self.plot_area_height)
            .field("canvas_size", &self.canvas_size)
            .field("layout", &"LayoutSolution")
            .finish()
    }
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

    /// Debug marks (layout visualization) - these are in absolute canvas coordinates
    pub debug_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,
}
