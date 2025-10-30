//! CompiledPlot - Immutable, serializable plot ready for rendering

pub(crate) mod expr_eval;
mod legends;
pub(crate) mod rendering;
pub(crate) mod scales; // Made public so plot.rs can call build_scale_builder_from_marks
pub mod scale_provider;
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
use crate::plot::compiled::scales::build_scale_builder_from_marks;
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
    pub(crate) async fn build_scale_builder_from_dataframe(
        &self,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        df: &datafusion::dataframe::DataFrame,
    ) -> Result<crate::scales::builder::ScaleBuilder, crate::error::AvengerChartError> {
        build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            Some(df.clone()),
            ctx,
            params,
            self.get_theme().as_ref(),
        )
        .await
    }

    /// Build scales with specific dimensions for the current evaluation.
    ///
    /// We rebuild a temporary ScaleBuilder on each call using the current params
    /// so domain inference reflects paramized data queries. This keeps direct
    /// and serialized paths identical and avoids stale caches.
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
                    if let Some(expr) = channel_value.expr(ctx) {
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
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
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
                    plot_area_width,
                    plot_area_height,
                    theme.as_ref(),
                    params,
                    ctx,
                )
                .await
        } else {
            Ok(crate::guide::OverflowSpaceRequirement::default())
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
}
