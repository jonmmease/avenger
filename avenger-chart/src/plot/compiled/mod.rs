//! CompiledPlot - Immutable, serializable plot ready for rendering

pub(crate) mod expr_eval;
mod legends;
pub(crate) mod rendering;
mod scales;
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

    /// Scale specifications (temporarily kept for building scales)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

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
                .normalize_domain(plot_area_width, plot_area_height, ctx)
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

    /// Evaluate inner plot components (data + guide) using provided DF, scales, and dimensions.
    pub async fn evaluate_components_with_scales(
        &self,
        df: &datafusion::dataframe::DataFrame,
        scales: &std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<SubplotComponents, crate::error::AvengerChartError> {
        use avenger_scenegraph::marks::group::Clip;

        // Project required columns from provided DataFrame to ensure schema contains all referenced fields
        let mut required_cols: std::collections::HashSet<String> = std::collections::HashSet::new();
        for mark in &self.marks {
            let channels = mark.data_context().channels();
            for (_name, cv) in channels {
                if let Some(expr) = cv.expr(ctx) {
                    for col in expr.column_refs() {
                        required_cols.insert(col.name.clone());
                    }
                }
            }
        }

        let projected_df = if !required_cols.is_empty() {
            let select_exprs: Vec<datafusion::logical_expr::Expr> = required_cols
                .iter()
                .map(|c| datafusion::prelude::col(c))
                .collect();
            df.clone().select(select_exprs)?
        } else {
            df.clone()
        };

        // Evaluate marks using projected DataFrame as the plot-level fallback
        let mut data_marks = Vec::new();
        for mark in &self.marks {
            let marks = self
                .evaluate_mark_with_plot_df(
                    mark.as_ref(),
                    scales,
                    plot_area_width,
                    plot_area_height,
                    ctx,
                    params,
                    Some(&projected_df),
                )
                .await?;
            data_marks.extend(marks);
        }

        // Plot bounds are the full plot area for subplots
        let plot_bounds = crate::layout::LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_area_width,
            height: plot_area_height,
        };

        // Evaluate guide marks
        // Note: facet context (e.g., facet_unified_y) is set by the facet mark and passed via params
        let guide_marks = self
            .create_guide_marks(
                scales,
                plot_area_width,
                plot_area_height,
                &plot_bounds,
                params,
                ctx,
            )
            .await?;

        // Clip region for data marks (rectangular plot area)
        let clip = if let Some(ref guide) = self.compiled_guide {
            let configured: std::collections::HashMap<
                String,
                avenger_scales::scales::ConfiguredScale,
            > = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured)
        } else {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        };

        Ok(SubplotComponents {
            data_marks,
            guide_marks,
            plot_bounds,
            clip,
        })
    }
}

/// Components for a single subplot evaluation
pub struct SubplotComponents {
    pub data_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,
    pub guide_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,
    pub plot_bounds: crate::layout::LayoutBounds,
    pub clip: avenger_scenegraph::marks::group::Clip,
}
