//! Core Plot struct and its basic implementations

use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};

use crate::channel::value::{ChannelValue, ConditionalValue, strip_trailing_numbers};
use crate::coords::{CoordinateSystem, CoordinateSystemTransform};
use crate::error::AvengerChartError;
use crate::guide::{CompiledGuide, CoordinateGuideBuilder};
use crate::layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
use crate::legend::Legend;
use crate::marks::{CompiledMark, Mark};
use crate::render::RenderContext;
use crate::scales::{ConfiguredScaleWithSpec, Scale};
use crate::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt, SerializableDataFrame};
use crate::theme::{Theme, css::CssTheme};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
    pub(crate) theme: Option<Arc<dyn Theme>>,

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
    pub fn get_theme(&self) -> Arc<dyn Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(CssTheme::light()))
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

    /// Collect all channels that need scales from marks
    pub fn collect_channels_needing_scales(&self, ctx: &SessionContext) -> HashSet<String> {
        use crate::channel::resolution::resolve_all_channel_refs;

        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            // Try to resolve, but use original channels if resolution fails
            let resolved_encodings =
                resolve_all_channel_refs(encodings, ctx).unwrap_or_else(|_| encodings.clone());
            for (channel_name, channel_value) in resolved_encodings {
                if channel_value.get_scale_name(&channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }

    /// Build a scale by name, applying any configured transformations
    pub fn build_scale(
        &self,
        name: &str,
        plot_width: f64,
        plot_height: f64,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Scale, AvengerChartError> {
        use crate::channel::value::strip_trailing_numbers;

        // Strip trailing numbers to get the base scale name
        let base_name = strip_trailing_numbers(name);

        // Build the default scale for the base name
        let mut base_scale =
            self.create_default_scale_for_channel_internal(base_name, ctx, params)?;

        // Apply coordinate-specific default range if applicable
        if let Some(range) = self.get_coordinate_default_range(name, plot_width, plot_height) {
            use datafusion::prelude::lit;
            base_scale = base_scale.range_interval(lit(range.0), lit(range.1));
        }

        // Gather domain expressions from marks
        if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name, ctx) {
            if !domain_exprs.is_empty() {
                base_scale = base_scale.domain_data_fields(domain_exprs);
            }
        }

        // Apply user-configured scale options
        match self.scale_specs.get(base_name) {
            Some(ScaleSpec::Local(scale_changes)) => Ok(base_scale.update(scale_changes.clone())),
            None => Ok(base_scale),
        }
    }

    /// Get the default range for a coordinate channel based on plot area dimensions
    fn get_coordinate_default_range(
        &self,
        name: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        // Check if this scale is mapped to a coordinate channel
        let coord_channel = self
            .scale_to_coord_channel
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name);

        self.coord_transform
            .default_range(coord_channel, plot_area_width, plot_area_height)
    }

    /// Create a default scale for a channel based on mark data types and preferences
    fn create_default_scale_for_channel_internal(
        &self,
        channel: &str,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Scale, AvengerChartError> {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::render::RenderContext;
        use crate::scales::create_default_scale_for_channel;
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        use std::sync::Arc;

        // Try to infer the data type and use mark-based scale preferences
        let mut scale_spec = None;
        let mut data_type = None;
        let mut found_mark = None;

        // Look through marks to find the expression for this channel
        for mark in &self.marks {
            let channels = mark.data_context().channels();

            // Try to resolve channel references first
            let resolved_channels = match resolve_all_channel_refs(channels, ctx) {
                Ok(resolved) => resolved,
                Err(e) => {
                    // If resolution failed (e.g., due to cycles), return an error
                    if channels.contains_key(channel) {
                        return Err(AvengerChartError::InternalError(format!(
                            "Cannot create scale for channel '{}': {}",
                            channel, e
                        )));
                    }
                    // Channel doesn't exist in this mark, continue to next
                    continue;
                }
            };

            if let Some(channel_value) = resolved_channels.get(channel) {
                // Get the dataframe for this mark using the provided context
                let mark_df = mark.data_context().dataframe_with_context(ctx);
                let plot_df = self.data.as_ref().and_then(|node| {
                    node.to_logical_plan(ctx)
                        .ok()
                        .map(|plan| DataFrame::new(ctx.state().clone(), plan))
                });
                let df = mark_df.or(plot_df);

                // Try to get the data type of the channel
                if let Some(df) = df {
                    let schema = df.schema();
                    match channel_value.get_data_type(schema, ctx) {
                        Ok(dt) => {
                            data_type = Some(dt.clone());
                            scale_spec = mark.preferred_scale_type(channel, &dt);
                            found_mark = Some(mark);
                            break;
                        }
                        Err(_) => {
                            // Continue to next mark to try to find a valid data type
                        }
                    }
                }
            }
        }

        let scale_spec = scale_spec.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Failed to infer scale specification for channel '{}'",
                channel
            ))
        })?;

        // Create render context with theme for scale defaults
        // Use placeholder dimensions since we're not rendering yet
        let theme = self.get_theme();
        let context = RenderContext::new(theme, 0.0, 0.0, Arc::new(ctx.clone()), params.clone());

        // Create scale with theme-based defaults
        let mut scale = create_default_scale_for_channel(channel, scale_spec, &context)?;

        // Apply coordinate system and mark-specific scale options
        // These override theme defaults
        if let (Some(dt), Some(mark)) = (&data_type, found_mark) {
            let mut default_options = HashMap::new();

            // Create ScaleImpl from ScaleSpec for methods that need it
            let scale_impl = scale.get_scale_impl().ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Failed to create scale implementation for channel '{}'",
                    channel
                ))
            })?;

            // First get coordinate system defaults (for all channels, not just position)
            // The coord_transform has a default_scale_options method that returns ScalarValue
            let coord_options = self
                .coord_transform
                .default_scale_options(channel, scale_impl.as_ref());

            // Convert ScalarValue to Expr for compatibility
            let coord_options_expr: HashMap<String, datafusion::logical_expr::Expr> = coord_options
                .into_iter()
                .map(|(k, v)| (k, lit(v)))
                .collect();
            default_options.extend(coord_options_expr);

            // Then get mark-specific scale option preferences
            // We already know this mark has the channel since we found it above
            let mark_options = mark.default_scale_options(channel, scale_impl.as_ref(), dt);
            // Mark preferences override coordinate system defaults
            default_options.extend(mark_options);

            // Get the supported options for this scale type
            let option_definitions = scale_impl.option_definitions();
            let supported_options: std::collections::HashSet<&str> = option_definitions
                .iter()
                .map(|def| def.name.as_str())
                .collect();

            // Only apply options that are supported by this scale
            for (key, value) in default_options {
                if supported_options.contains(key.as_str()) {
                    scale = scale.option(&key, value);
                }
            }
        }

        Ok(scale)
    }

    /// Gather mark data and encoding expressions that use this scale with radius information
    pub(super) fn gather_scale_domain_expressions_with_radius(
        &self,
        scale_name: &str,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        context: &crate::render::RenderContext,
    ) -> Result<
        Vec<(
            Arc<DataFrame>,
            datafusion::logical_expr::Expr,
            Option<crate::marks::RadiusExpression>,
        )>,
        AvengerChartError,
    > {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::channel::value::ChannelValue;
        use crate::scales::extensions::ConfiguredScaleDataFusionExt;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = resolve_all_channel_refs(channels, &context.session_context)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
                            .column_refs(&context.session_context)
                            .map(|refs| !refs.is_empty())
                            .unwrap_or(false),
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                condition
                                    .column_refs(&context.session_context)
                                    .map(|refs| !refs.is_empty())
                                    .unwrap_or(false)
                                    || value
                                        .expr(&context.session_context)
                                        .ok()
                                        .map(|e| !e.column_refs().is_empty())
                                        .unwrap_or(false)
                            }) || otherwise
                                .expr(&context.session_context)
                                .ok()
                                .map(|e| !e.column_refs().is_empty())
                                .unwrap_or(false)
                        }
                    });

            // Determine DataFrame for this mark using context from RenderContext
            let mark_df = mark
                .data_context()
                .dataframe_with_context(&context.session_context);
            let plot_df = self.data.as_ref().and_then(|node| {
                node.to_logical_plan(&context.session_context)
                    .ok()
                    .map(|plan| DataFrame::new(context.session_context.state().clone(), plan))
            });

            let df = if let Some(mark_df) = mark_df {
                // Mark has explicit data
                Arc::new(mark_df)
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_df) = plot_df {
                // Inherit from plot
                Arc::new(plot_df)
            } else {
                // No data available - skip this mark
                continue;
            };

            // Create channel resolver for this mark (uses configured non-positional scales and theme)
            // This resolver looks up the channel value and applies scaling if needed
            let resolve_channel = |channel_name: &str| -> datafusion::logical_expr::Expr {
                use datafusion::prelude::lit;

                // First check explicit mapping
                if let Some(channel_value) = resolved_channels.get(channel_name) {
                    // Apply scaling if needed
                    match channel_value {
                        ChannelValue::Value { expr } => {
                            // No scaling requested - convert to Expr
                            if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                return expr_df;
                            }
                            return lit(datafusion::common::ScalarValue::Null);
                        }
                        ChannelValue::Scaled {
                            expr,
                            scale_name: custom_scale_name,
                            band,
                            ..
                        } => {
                            // Determine scale name
                            let scale_key =
                                custom_scale_name.as_ref().cloned().unwrap_or_else(|| {
                                    use crate::channel::value::strip_trailing_numbers;
                                    strip_trailing_numbers(channel_name).to_string()
                                });

                            // Use configured scales if available
                            if let Some(configured) = configured_scales.get(&scale_key) {
                                // Convert SerializableExpr to Expr first
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    // Apply the scale transform
                                    if let Some(band_value) = band {
                                        return configured
                                            .to_expr_with_band(expr_df.clone(), *band_value)
                                            .unwrap_or(expr_df);
                                    } else {
                                        return configured
                                            .to_expr(expr_df.clone())
                                            .unwrap_or(expr_df);
                                    }
                                }
                                // If conversion fails, return null
                                return lit(datafusion::common::ScalarValue::Null);
                            } else {
                                // No scale configured - use raw expression, convert to Expr
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    return expr_df;
                                }
                                return lit(datafusion::common::ScalarValue::Null);
                            }
                        }
                        ChannelValue::Conditional { .. } => {
                            // Conditional values are not supported for radius calculations
                            return lit(datafusion::scalar::ScalarValue::Null);
                        }
                    }
                }

                // No explicit mapping - check for mark-provided defaults (including theme)
                if let Some(default_scalar) = mark.default_channel_value(channel_name, context) {
                    // Use mark-provided default (which includes theme defaults)
                    if std::env::var("AVENGER_DEBUG_RADIUS").is_ok() {
                        eprintln!(
                            "DEBUG: Using default for channel '{}': {:?}",
                            channel_name, default_scalar
                        );
                    }
                    return lit(default_scalar);
                }

                // No mapping and no default
                lit(datafusion::scalar::ScalarValue::Null)
            };

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get radius expression from the mark
                        let radius_expr = mark.radius_expression(scale_name, &resolve_channel);

                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                // Convert SerializableExpr to Expr
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    data_expressions.push((df.clone(), expr_df, radius_expr));
                                }
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we use the mark's radius for all branches
                                // since radius doesn't vary by condition
                                let radius_expr_cond =
                                    mark.radius_expression(scale_name, &resolve_channel);

                                // Add expressions from all conditions and the otherwise branch
                                for (_, value) in conditions {
                                    if let Ok(expr) = value.expr(&context.session_context) {
                                        data_expressions.push((
                                            df.clone(),
                                            expr,
                                            radius_expr_cond.clone(),
                                        ));
                                    }
                                }
                                if let Ok(expr) = otherwise.expr(&context.session_context) {
                                    data_expressions.push((df.clone(), expr, radius_expr_cond));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Gather mark data and encoding expressions that use this scale
    pub(super) fn gather_scale_domain_expressions(
        &self,
        scale_name: &str,
        ctx: &SessionContext,
    ) -> Result<Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)>, AvengerChartError> {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::channel::value::ChannelValue;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = resolve_all_channel_refs(channels, ctx)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
                            .column_refs(ctx)
                            .map(|refs| !refs.is_empty())
                            .unwrap_or(false),
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                condition
                                    .column_refs(ctx)
                                    .map(|refs| !refs.is_empty())
                                    .unwrap_or(false)
                                    || value
                                        .expr(ctx)
                                        .ok()
                                        .map(|e| !e.column_refs().is_empty())
                                        .unwrap_or(false)
                            }) || otherwise
                                .expr(ctx)
                                .ok()
                                .map(|e| !e.column_refs().is_empty())
                                .unwrap_or(false)
                        }
                    });

            // Determine DataFrame for this mark using context from RenderContext
            let mark_df = mark.data_context().dataframe_with_context(ctx);
            let plot_df = self.data.as_ref().and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            });

            let df = if let Some(mark_df) = mark_df {
                // Mark has explicit data
                Arc::new(mark_df)
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_df) = plot_df {
                // Inherit from plot
                Arc::new(plot_df)
            } else {
                // No data available - skip this mark
                continue;
            };

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                // Convert SerializableExpr to Expr
                                if let Ok(expr_df) = expr.to_expr(ctx) {
                                    data_expressions.push((df.clone(), expr_df));
                                }
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // Add expressions from all conditions and the otherwise branch
                                for (_, value) in conditions {
                                    if let Ok(expr) = value.expr(ctx) {
                                        data_expressions.push((df.clone(), expr));
                                    }
                                }
                                if let Ok(expr) = otherwise.expr(ctx) {
                                    data_expressions.push((df.clone(), expr));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    // ===== Phase 1: Utility Methods (No dependencies) =====

    /// Check if a data type is numeric
    fn is_numeric_type(dtype: &datafusion::arrow::datatypes::DataType) -> bool {
        use datafusion::arrow::datatypes::DataType;
        match dtype {
            // Standard numeric types
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64 => true,
            // Dictionary types are allowed if their value type is numeric
            // (This happens when categorical data goes through an ordinal scale)
            DataType::Dictionary(_, value_type) => Self::is_numeric_type(value_type),
            _ => false,
        }
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(
        &self,
        channel: &str,
        session_context: &datafusion::prelude::SessionContext,
    ) -> String {
        // First try to extract from marks (like we do for axes)
        use crate::coords::extract_channel_title_from_marks;
        if let Some(title) = extract_channel_title_from_marks(&self.marks, channel, session_context)
        {
            return title;
        }

        // Fallback: convert underscores to spaces and apply title case
        channel
            .split('_')
            .map(|word| {
                // Capitalize first letter of each word
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get default legend position for a channel
    fn default_legend_position(&self, _channel: &str) -> crate::legend::LegendPosition {
        // All legends default to the right
        crate::legend::LegendPosition::Right
    }

    /// Create title mark if configured
    pub(super) fn create_title(
        &self,
        layout_bounds: Option<crate::layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(title) = &self.title else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();

        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

        // Position title within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            (10.0, 20.0)
        };

        let text_mark = SceneTextMark {
            text: title.text.clone().into(),
            x: x.into(),
            y: y.into(),
            color: crate::utils::parse_color_string(&theme.title_color())
                .unwrap_or(avenger_common::types::ColorOrGradient::Color([
                    0.102, 0.102, 0.102, 1.0,
                ]))
                .into(),
            font_size: title.font_size.unwrap_or(theme.title_font_size()).into(),
            font: title
                .font_family
                .clone()
                .unwrap_or_else(|| theme.title_font_family())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.title_font_weight()).into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }

    /// Create subtitle mark if configured
    pub(super) fn create_subtitle(
        &self,
        layout_bounds: Option<crate::layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(subtitle) = &self.subtitle else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();

        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

        // Position subtitle within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            (10.0, 40.0)
        };

        let text_mark = SceneTextMark {
            text: subtitle.text.clone().into(),
            x: x.into(),
            y: y.into(),
            color: crate::utils::parse_color_string(&theme.subtitle_color())
                .unwrap_or(avenger_common::types::ColorOrGradient::Color([
                    0.290, 0.290, 0.290, 1.0,
                ]))
                .into(),
            font_size: subtitle
                .font_size
                .unwrap_or(theme.subtitle_font_size())
                .into(),
            font: subtitle
                .font_family
                .clone()
                .unwrap_or_else(|| theme.subtitle_font_family())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.subtitle_font_weight())
                .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }

    /// Create error for non-numeric positional channel
    fn create_positional_type_error(
        &self,
        channel_name: &str,
        dtype: &datafusion::arrow::datatypes::DataType,
        coord_system_name: &str,
    ) -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Extract coordinate system name (remove module path)
        let coord_system = coord_system_name
            .split("::")
            .last()
            .unwrap_or("Unknown")
            .to_string();

        // Provide helpful suggestion based on the data type
        let (literal_value, suggestion) = match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => (
                "string literal".to_string(),
                "Use col(\"column_name\") to reference a data column instead of a string literal.\n  \
                         If you need a fixed position, use a numeric value like lit(100.0)".to_string(),
            ),
            _ => (
                format!("{:?} value", dtype),
                "Positional channels require numeric values. \
                         Use col(\"column_name\") to reference a numeric column.".to_string(),
            ),
        };

        Err(AvengerChartError::PositionalScaleLiteralError {
            scale_name: channel_name.to_string(),
            coord_system,
            literal_value,
            suggestion,
        })
    }

    // ===== Phase 2: Simple Plot-dependent Methods =====

    /// Apply scaling transformation to a channel expression
    pub(super) fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Result<datafusion::logical_expr::Expr, AvengerChartError> {
        use crate::channel::value::strip_trailing_numbers;

        match channel_value {
            ChannelValue::Value { expr } => {
                // No scaling requested, return expression as-is - convert to Expr
                expr.to_expr(&SessionContext::new())
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build a CASE WHEN expression from the conditions
                use datafusion::arrow::datatypes::DataType;
                use datafusion::logical_expr::{lit, when};
                use datafusion::scalar::ScalarValue;

                // Helper to convert color string literals to the proper ScalarValue format
                let convert_color_literal =
                    |expr: &datafusion::logical_expr::Expr| -> datafusion::logical_expr::Expr {
                        // Check if this is a string literal that might be a color
                        if let datafusion::logical_expr::Expr::Literal(scalar_value, _) = expr {
                            if let ScalarValue::Utf8(Some(s)) = scalar_value {
                                // Use our existing color parsing utility
                                if let Some(color_or_gradient) = crate::utils::parse_color_string(s)
                                {
                                    use avenger_common::types::ColorOrGradient;
                                    if let ColorOrGradient::Color(rgba) = color_or_gradient {
                                        // Convert to List ScalarValue with Float32 values
                                        let values: Vec<ScalarValue> = rgba
                                            .into_iter()
                                            .map(|v| ScalarValue::Float32(Some(v)))
                                            .collect();

                                        // Create the list array and wrap in ScalarValue
                                        let list_array = ScalarValue::new_list_nullable(
                                            &values,
                                            &DataType::Float32,
                                        );
                                        let scalar_list = ScalarValue::List(list_array);
                                        return lit(scalar_list);
                                    }
                                }
                            }
                        }
                        // Not a color literal or failed to parse, return as-is
                        expr.clone()
                    };

                // For conditional values, the scale name is derived from the channel name
                let scale_key = strip_trailing_numbers(channel_name).to_string();

                // Check if we need color conversion (for color channels)
                let needs_color_conversion = matches!(channel_name, "fill" | "stroke" | "color");

                // Helper to apply scale to a conditional value
                let apply_to_conditional = |cond_val: &ConditionalValue| -> Result<
                    datafusion::logical_expr::Expr,
                    AvengerChartError,
                > {
                    match cond_val {
                        ConditionalValue::Scaled { expr } => {
                            // Apply scale transformation
                            if let Some(scale) = scales.get(&scale_key) {
                                use crate::scales::ConfiguredScaleDataFusionExt;
                                // Convert SerializableExpr to Expr first
                                expr.to_expr(&SessionContext::new())
                                    .and_then(|e| scale.to_expr(e))
                            } else {
                                // No scale found, return expression as-is - convert to Expr
                                expr.to_expr(&SessionContext::new())
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            // Pass through literal values unchanged - convert to Expr
                            let expr_df = expr.to_expr(&SessionContext::new())?;
                            if needs_color_conversion {
                                Ok(convert_color_literal(&expr_df))
                            } else {
                                Ok(expr_df)
                            }
                        }
                    }
                };

                // Start with the first condition
                let first_cond = &conditions[0];
                let first_value = apply_to_conditional(&first_cond.1)?;
                // Convert SerializableExpr to Expr
                let first_cond_expr = first_cond.0.to_expr(&SessionContext::new())?;
                let mut case_expr = when(first_cond_expr, first_value);

                // Add remaining conditions
                for (condition, value) in &conditions[1..] {
                    let scaled_value = apply_to_conditional(value)?;
                    // Convert SerializableExpr to Expr
                    let condition_expr = condition.to_expr(&SessionContext::new())?;
                    case_expr = case_expr.when(condition_expr, scaled_value);
                }

                // Add the otherwise clause
                let otherwise_value = apply_to_conditional(otherwise)?;

                Ok(case_expr.otherwise(otherwise_value)?)
            }
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
            } => {
                // Determine the scale to use
                let default_scale_name = strip_trailing_numbers(channel_name).to_string();
                let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);

                // Look up the configured scale
                let scale = scales.get(scale_key).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Scale '{}' not found for channel '{}'",
                        scale_key, channel_name
                    ))
                })?;

                // Apply the scale transformation
                use crate::scales::ConfiguredScaleDataFusionExt;
                // Convert SerializableExpr to Expr first
                let expr_df = expr.to_expr(&SessionContext::new())?;
                if let Some(band) = band {
                    scale.to_expr_with_band(expr_df.clone(), *band)
                } else {
                    scale.to_expr(expr_df)
                }
            }
        }
    }

    /// Create default legends for channels with scales
    fn create_default_legends(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &datafusion::prelude::SessionContext,
    ) -> IndexMap<String, Legend> {
        let mut default_legends = IndexMap::new();

        // Build set of channels to skip for legends
        let mut skip_channels = std::collections::HashSet::new();

        // Add positional channels from the coordinate system
        for &channel in self.coord_transform.required_channels() {
            skip_channels.insert(channel.to_string());
            // Also skip interval variants (e.g., "x2" for "x")
            skip_channels.insert(format!("{}2", channel));
        }

        // Add channels that marks indicate shouldn't have legends
        for (channel, scale) in scales {
            // Find the mark that has this channel
            if let Some(mark) = self
                .marks
                .iter()
                .find(|m| m.data_context().channels().contains_key(channel))
            {
                // If the mark that has the channel says no legend, skip it
                if mark
                    .preferred_legend_renderer(channel, scale.configured())
                    .is_none()
                {
                    skip_channels.insert(channel.clone());
                }
            }
        }

        // Sort channels for deterministic ordering
        let mut sorted_channels: Vec<_> = scales.keys().collect();
        sorted_channels.sort();

        for channel in sorted_channels {
            // Skip channels that don't need legends
            if skip_channels.contains(channel) {
                continue;
            }

            // Skip if legend already configured
            if self.legends.contains_key(channel) {
                continue;
            }

            // Create legend with theme defaults
            let theme = self.get_theme();
            let mut legend = Legend::new()
                .title(self.infer_legend_title(channel, session_context))
                .position(self.default_legend_position(channel))
                .background_padding(theme.legend_background_padding())
                .background_corner_radius(theme.legend_background_corner_radius());

            // Apply optional theme defaults
            if let Some(fill) = theme.legend_background_fill() {
                legend = legend.background_fill(fill);
            }
            if let Some(stroke) = theme.legend_background_stroke() {
                legend = legend.background_stroke(stroke);
            }

            // Set text colors and typography from theme
            legend.title_color = crate::maybe::Maybe::Set(theme.legend_title_color());
            legend.label_color = crate::maybe::Maybe::Set(theme.legend_label_color());
            legend.title_font_family = crate::maybe::Maybe::Set(theme.legend_title_font_family());
            legend.title_font_size = crate::maybe::Maybe::Set(theme.legend_title_font_size());
            legend.title_font_weight = crate::maybe::Maybe::Set(theme.legend_title_font_weight());
            legend.label_font_family = crate::maybe::Maybe::Set(theme.legend_label_font_family());
            legend.label_font_size = crate::maybe::Maybe::Set(theme.legend_label_font_size());
            legend.label_font_weight = crate::maybe::Maybe::Set(theme.legend_label_font_weight());
            legend.tick_font_family = crate::maybe::Maybe::Set(theme.legend_tick_font_family());
            legend.tick_font_size = crate::maybe::Maybe::Set(theme.legend_tick_font_size());
            legend.tick_font_weight = crate::maybe::Maybe::Set(theme.legend_tick_font_weight());
            legend.tick_color = crate::maybe::Maybe::Set(theme.legend_tick_color());

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Get the appropriate legend renderer for a channel
    pub(super) fn get_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScaleWithSpec,
    ) -> Option<Arc<dyn crate::legend::renderer::LegendRenderer>> {
        // Find the first mark that has this channel and get its preference
        for mark in &self.marks {
            if mark.data_context().channels().contains_key(channel) {
                return mark.preferred_legend_renderer(channel, scale.configured());
            }
        }
        None
    }

    /// Get legends with theme applied (matching PlotRenderer behavior)
    pub(super) fn get_legends_with_theme(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &datafusion::prelude::SessionContext,
    ) -> IndexMap<String, Legend> {
        // 1. Start with plot-level legends
        let mut all_legends = self.legends.clone();

        // 2. Apply channel-level legend configs (from mark encodings)
        for mark in &self.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                if let Some(channel_legend) = channel_value.get_legend_config() {
                    all_legends
                        .entry(channel_name.clone())
                        .and_modify(|legend| {
                            *legend = legend.clone().update(channel_legend.clone())
                        })
                        .or_insert_with(|| channel_legend.clone());
                }
            }
        }

        // 3. Apply defaults for channels with scales but no legend config
        let default_legends = self.create_default_legends(scales, session_context);
        for (channel, default_legend) in default_legends {
            all_legends
                .entry(channel)
                .and_modify(|legend| *legend = default_legend.clone().update(legend.clone()))
                .or_insert(default_legend);
        }

        // 4. Apply theme (only for Unset properties)
        let theme = self.get_theme();
        for legend in all_legends.values_mut() {
            // Theme only fills in Unset values
            // Apply theme fonts if not explicitly set
            if matches!(legend.title_color, crate::maybe::Maybe::Unset) {
                legend.title_color = crate::maybe::Maybe::Set(theme.legend_title_color());
            }
            if matches!(legend.label_color, crate::maybe::Maybe::Unset) {
                legend.label_color = crate::maybe::Maybe::Set(theme.legend_label_color());
            }
            if matches!(legend.title_font_family, crate::maybe::Maybe::Unset) {
                legend.title_font_family =
                    crate::maybe::Maybe::Set(theme.legend_title_font_family());
            }
            if matches!(legend.title_font_size, crate::maybe::Maybe::Unset) {
                legend.title_font_size = crate::maybe::Maybe::Set(theme.legend_title_font_size());
            }
            if matches!(legend.title_font_weight, crate::maybe::Maybe::Unset) {
                legend.title_font_weight =
                    crate::maybe::Maybe::Set(theme.legend_title_font_weight());
            }
            if matches!(legend.label_font_family, crate::maybe::Maybe::Unset) {
                legend.label_font_family =
                    crate::maybe::Maybe::Set(theme.legend_label_font_family());
            }
            if matches!(legend.label_font_size, crate::maybe::Maybe::Unset) {
                legend.label_font_size = crate::maybe::Maybe::Set(theme.legend_label_font_size());
            }
            if matches!(legend.label_font_weight, crate::maybe::Maybe::Unset) {
                legend.label_font_weight =
                    crate::maybe::Maybe::Set(theme.legend_label_font_weight());
            }
            if matches!(legend.tick_font_family, crate::maybe::Maybe::Unset) {
                legend.tick_font_family = crate::maybe::Maybe::Set(theme.legend_tick_font_family());
            }
            if matches!(legend.tick_font_size, crate::maybe::Maybe::Unset) {
                legend.tick_font_size = crate::maybe::Maybe::Set(theme.legend_tick_font_size());
            }
            if matches!(legend.tick_font_weight, crate::maybe::Maybe::Unset) {
                legend.tick_font_weight = crate::maybe::Maybe::Set(theme.legend_tick_font_weight());
            }
            if matches!(legend.tick_color, crate::maybe::Maybe::Unset) {
                legend.tick_color = crate::maybe::Maybe::Set(theme.legend_tick_color());
            }
            // Note: Don't apply theme background settings - they're only for default legends
            // This matches PlotRenderer behavior
        }

        all_legends
    }

    /// Validate that positional channels have numeric data types
    pub(super) fn validate_positional_channel_types(
        &self,
        data_batch: &Option<datafusion::arrow::record_batch::RecordBatch>,
        scalar_batch: &datafusion::arrow::record_batch::RecordBatch,
    ) -> Result<(), AvengerChartError> {
        // Get coordinate system name for error messages
        let coord_system_name = std::any::type_name_of_val(&*self.coord_transform);

        // Check each positional channel
        for channel_name in self.coord_transform.required_channels() {
            // Check in data batch first
            if let Some(data) = data_batch {
                if let Some(column) = data.column_by_name(channel_name) {
                    let dtype = column.data_type();
                    if !Self::is_numeric_type(dtype) {
                        return self.create_positional_type_error(
                            channel_name,
                            dtype,
                            coord_system_name,
                        );
                    }
                }
            }

            // Check in scalar batch
            if let Some(column) = scalar_batch.column_by_name(channel_name) {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(
                        channel_name,
                        dtype,
                        coord_system_name,
                    );
                }
            }
        }

        Ok(())
    }

    /// Build a legend channel for a specific channel in a mark
    pub(super) fn build_legend_channel(
        &self,
        channel_name: &str,
        channel_value: &crate::channel::value::ChannelValue,
        scale: &ConfiguredScaleWithSpec,
        mark: &dyn crate::marks::CompiledMark,
        mark_index: usize,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> crate::legend::LegendChannel {
        use crate::legend::{ChannelInfo, LegendChannel};
        use datafusion::logical_expr::lit;

        // Collect related channels from the mark
        let mut related_channels = HashMap::new();

        // First add explicitly set channels
        for (other_name, other_value) in mark.data_context().channels() {
            if other_name != channel_name {
                // Check if this channel has a scale or is constant
                let channel_info = if let Some(other_scale) = configured_scales.get(other_name) {
                    // Channel has a scale
                    ChannelInfo::Scaled {
                        expr: other_value.expr(ctx),
                        scale: other_scale.configured().clone(),
                    }
                } else if let Some(expr) = other_value.expr(ctx) {
                    // Channel has a constant expression
                    ChannelInfo::Constant { expr: expr.clone() }
                } else {
                    // Skip channels without expressions
                    continue;
                };
                related_channels.insert(other_name.clone(), channel_info);
            }
        }

        // For channels not explicitly set, check if they have theme defaults
        // This ensures legend symbols match the chart's actual appearance
        let context = RenderContext {
            plot_width: 100.0, // Dummy values for getting defaults
            plot_height: 100.0,
            theme: self.get_theme(),
            session_context: Arc::new(ctx.clone()),
            params: params.clone(),
        };

        // Iterate through all supported channels of this mark
        for channel_desc in mark.supported_channels() {
            let other_name = channel_desc.name;
            if other_name != channel_name && !related_channels.contains_key(other_name) {
                // Channel not explicitly set - check for theme default
                if let Some(default_value) = mark.default_channel_value(other_name, &context) {
                    // Add as a constant channel
                    let expr = lit(default_value);
                    related_channels.insert(other_name.to_string(), ChannelInfo::Constant { expr });
                }
            }
        }

        // Get the mark type name
        let mark_type = mark.mark_type().to_string();

        LegendChannel {
            name: channel_name.to_string(),
            expression: channel_value.expr(ctx),
            scale: scale.configured().clone(),
            channel_type: channel_name.to_string(), // Use channel name as type
            mark_type,
            mark_index,
            related_channels,
        }
    }

    // ===== Phase 3: Scale Building Methods =====

    /// Validate that all required positional scales exist
    pub(super) fn validate_positional_scales_exist(
        &self,
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Result<(), AvengerChartError> {
        // Get the required positional channels from the coordinate system
        let required_channels = self.coord_transform.required_channels();

        // Build a set of positional channel names to check
        // Include both base channels and their interval variants (e.g., "x" and "x2")
        let mut positional_channels = HashSet::new();
        for &channel in required_channels {
            positional_channels.insert(channel.to_string());
            // Also check for interval variant (e.g., "x2" for "x")
            positional_channels.insert(format!("{}2", channel));
        }

        // Check which marks use positional channels
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            for channel_name in mark.data_context().channels().keys() {
                if positional_channels.contains(channel_name) {
                    used_channels.insert(channel_name.clone());
                }
            }
        }

        // For now, we'll just return Ok since scale building happens elsewhere
        // In the future, we might want to validate that scales exist for used channels
        Ok(())
    }
}
