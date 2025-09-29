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
use avenger_scales::scales::ConfiguredScale;
use crate::serialization::{SerializableDataFrame, LogicalExprNodeExt, LogicalPlanNodeExt};
use crate::theme::{Theme, css::CssTheme};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct CompiledPlot {
    /// Coordinate system transform for position mapping
    pub(crate) coord_transform: Box<dyn CoordinateSystemTransform>,

    /// Guide renderer for axes/grids
    pub(crate) guide_renderer: Option<Arc<dyn CompiledGuide>>,

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
    ) -> Result<Scale, AvengerChartError> {
        use crate::channel::value::strip_trailing_numbers;

        // Strip trailing numbers to get the base scale name
        let base_name = strip_trailing_numbers(name);

        // Build the default scale for the base name
        let mut base_scale = self.create_default_scale_for_channel_internal(base_name, ctx)?;

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
        let context = RenderContext::new(theme, 0.0, 0.0, Arc::new(ctx.clone()));

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
    fn gather_scale_domain_expressions_with_radius(
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
            let plot_df = self
                .data
                .as_ref()
                .and_then(|node| {
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
    fn gather_scale_domain_expressions(
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
    fn create_title(
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
    fn create_subtitle(
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
    fn apply_channel_scale(
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
                if mark.preferred_legend_renderer(channel, &scale.configured).is_none() {
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
    fn get_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScaleWithSpec,
    ) -> Option<Arc<dyn crate::legend::renderer::LegendRenderer>> {
        // Find the first mark that has this channel and get its preference
        for mark in &self.marks {
            if mark.data_context().channels().contains_key(channel) {
                return mark.preferred_legend_renderer(channel, &scale.configured);
            }
        }
        None
    }

    /// Get legends with theme applied (matching PlotRenderer behavior)
    fn get_legends_with_theme(
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
    fn validate_positional_channel_types(
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
    fn build_legend_channel(
        &self,
        channel_name: &str,
        channel_value: &crate::channel::value::ChannelValue,
        scale: &ConfiguredScaleWithSpec,
        mark: &dyn crate::marks::CompiledMark,
        mark_index: usize,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
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
                        scale: other_scale.configured.clone(),
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
            scale: scale.configured.clone(),
            channel_type: channel_name.to_string(), // Use channel name as type
            mark_type,
            mark_index,
            related_channels,
        }
    }

    // ===== Phase 3: Scale Building Methods =====

    /// Validate that all required positional scales exist
    fn validate_positional_scales_exist(
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

    /// Build a configured scale with radius context for polar coordinates
    async fn build_configured_scale_with_radius_context(
        &self,
        mut scale: Scale,
        name: &str,
        context: &crate::render::RenderContext,
        configured_non_positional: Option<&HashMap<String, ConfiguredScaleWithSpec>>,
    ) -> Result<ConfiguredScaleWithSpec, AvengerChartError> {
        // Process domain with radius if applicable
        if scale
            .domain
            .as_ref()
            .map(|d| {
                matches!(
                    &d.default_domain,
                    crate::scales::ScaleDefaultDomain::DomainExprs(_)
                )
            })
            .unwrap_or(false)
        {
            // Only use radius-aware gathering for positional scales that support it
            if let Some(configured_non_positional) = configured_non_positional {
                // Check if this is a positional channel (including interval variants)
                let is_positional = self
                    .coord_transform
                    .required_channels()
                    .iter()
                    .any(|&ch| ch == name || name == &format!("{}2", ch));

                if scale
                    .get_scale_impl()
                    .map(|impl_| impl_.supports_radius_expansion())
                    .unwrap_or(false)
                    && is_positional
                {
                    // Use the method that gathers radius information
                    let data_expressions_with_radius = self
                        .gather_scale_domain_expressions_with_radius(
                            name,
                            configured_non_positional,
                            context,
                        )?;

                    // Check if any expressions actually have radius
                    let has_radius = data_expressions_with_radius
                        .iter()
                        .any(|(_, _, radius)| radius.is_some());

                    if !data_expressions_with_radius.is_empty() && has_radius {
                        // Use the method that accepts radius
                        scale = scale.domain_data_fields_with_radius(data_expressions_with_radius);
                    } else if !data_expressions_with_radius.is_empty() {
                        // Convert to standard expressions (without radius)
                        let data_expressions: Vec<(
                            Arc<DataFrame>,
                            datafusion::logical_expr::Expr,
                        )> = data_expressions_with_radius
                            .into_iter()
                            .map(|(df, expr, _)| (df, expr))
                            .collect();
                        scale = scale.domain_data_fields(data_expressions);
                    }
                } else {
                    // Use standard domain gathering for non-linear scales
                    let data_expressions =
                        self.gather_scale_domain_expressions(name, &context.session_context)?;
                    if !data_expressions.is_empty() {
                        scale = scale.domain_data_fields(data_expressions);
                    }
                }
            } else {
                // No radius context - use standard domain gathering
                let data_expressions =
                    self.gather_scale_domain_expressions(name, &context.session_context)?;
                if !data_expressions.is_empty() {
                    scale = scale.domain_data_fields(data_expressions);
                }
            }
        }

        // Apply coordinate system defaults if this is a positional channel
        let base_name = strip_trailing_numbers(name);
        if let Some((min, max)) = self.coord_transform.default_range(
            base_name,
            context.plot_width as f64,
            context.plot_height as f64,
        ) {
            use datafusion::prelude::lit;
            scale = scale.range_interval(lit(min), lit(max));
        }

        // Step 3: Infer domain from data if needed (resolve DomainExprs)
        if scale
            .domain
            .as_ref()
            .map(|d| {
                matches!(
                    &d.default_domain,
                    crate::scales::ScaleDefaultDomain::DomainExprs(_)
                )
            })
            .unwrap_or(false)
        {
            scale = scale
                .infer_domain_from_data(
                    context.plot_width,
                    context.plot_height,
                    &context.session_context,
                )
                .await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(
                context.plot_width,
                context.plot_height,
                &context.session_context,
            )
            .await?;

        // Step 5: Apply mark-specific range if not a position channel AND no range is set
        // Position channels already have their ranges set
        let is_position = self
            .coord_transform
            .required_channels()
            .contains(&name.as_ref());

        // Check if scale already has a user-specified range
        // The default range is [0, 1], so check if it's been customized from that
        let has_user_range = match scale.get_range() {
            Some(crate::scales::ScaleRange::Color(_)) => true, // Custom color range
            Some(crate::scales::ScaleRange::Discrete(_)) => true, // Custom discrete values
            Some(crate::scales::ScaleRange::Numeric(start, end)) => {
                // Check if it's not the default [0, 1] range
                use datafusion::logical_expr::Expr;
                use datafusion_common::ScalarValue;
                let is_default = if let (Ok(start_expr), Ok(end_expr)) = (
                    start.to_expr(&context.session_context),
                    end.to_expr(&context.session_context),
                ) {
                    
                    match (&start_expr, &end_expr) {
                        (Expr::Literal(v1, _), Expr::Literal(v2, _)) => match (v1, v2) {
                            (ScalarValue::Float64(Some(v1)), ScalarValue::Float64(Some(v2))) => {
                                (v1 - 0.0).abs() < 0.001 && (v2 - 1.0).abs() < 0.001
                            }
                            (ScalarValue::Float32(Some(v1)), ScalarValue::Float32(Some(v2))) => {
                                (*v1 as f64 - 0.0).abs() < 0.001 && (*v2 as f64 - 1.0).abs() < 0.001
                            }
                            _ => false,
                        },
                        _ => false,
                    }
                } else {
                    false
                };
                !is_default
            }
            None => false, // No range set, use default
        };

        if !is_position && !has_user_range {
            // Find the first mark that uses this channel
            for mark in &self.marks {
                if mark.data_context().channels().contains_key(name) {
                    // Get data type from the channel expression
                    // First resolve channel references
                    let channels = mark.data_context().channels();
                    let resolved_channels = crate::channel::resolution::resolve_all_channel_refs(
                        channels,
                        &context.session_context,
                    )
                    .ok()
                    .unwrap_or_else(|| channels.clone());

                    let data_type = resolved_channels
                        .get(name)
                        .and_then(|channel_value| channel_value.expr(&context.session_context))
                        .and_then(|expr| {
                            // Try to get data type from mark's dataframe using context from RenderContext
                            let mark_df = mark
                                .data_context()
                                .dataframe_with_context(&context.session_context);
                            let plot_df = self
                                .data
                                .as_ref()
                                .and_then(|node| {
                                    node.to_logical_plan(&context.session_context)
                                        .ok()
                                        .map(|plan| DataFrame::new(context.session_context.state().clone(), plan))
                                });
                            let df = mark_df.or(plot_df)?;
                            use datafusion::logical_expr::ExprSchemable;
                            expr.get_type(df.schema()).ok()
                        });

                    if let Some(dt) = data_type {
                        let theme = self.get_theme();
                        // Convert domain to ResolvedDomain if available
                        if let (Some(scale_impl), Some(domain)) =
                            (scale.get_scale_impl(), scale.get_domain())
                        {
                            let resolved_domain = domain.to_resolved()?;
                            if let Some(mark_range) = mark.default_channel_range(
                                name,
                                scale_impl.as_ref(),
                                &resolved_domain,
                                &dt,
                                theme.as_ref(),
                            ) {
                                scale = scale.range(mark_range);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Create the configured scale and wrap with the original spec
        let configured = scale
            .clone()
            .create_configured_scale(
                context.plot_width,
                context.plot_height,
                &context.session_context,
            )
            .await?;

        Ok(ConfiguredScaleWithSpec::new(scale, configured))
    }

    /// Build initial scales with estimated dimensions
    pub async fn build_initial_scales(
        &self,
        context: &crate::render::RenderContext,
    ) -> Result<
        (
            HashMap<String, Scale>,
            HashMap<String, ConfiguredScaleWithSpec>,
            HashMap<String, ConfiguredScaleWithSpec>,
        ),
        AvengerChartError,
    > {
        // Collect all channels that need scales
        let mut channels_with_scales =
            self.collect_channels_needing_scales(&context.session_context);

        // Also include any channels with explicit scale specs
        for channel in self.scale_specs.keys() {
            channels_with_scales.insert(channel.clone());
        }

        // Build initial scale definitions
        let mut initial_scales = HashMap::new();
        for channel in &channels_with_scales {
            let scale = self.build_scale(
                channel,
                context.plot_width as f64,
                context.plot_height as f64,
                &context.session_context,
            )?;
            initial_scales.insert(channel.clone(), scale);
        }

        // Separate scales into positional and non-positional
        let mut positional_scales = HashMap::new();
        let mut non_positional_scales = HashMap::new();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .coord_transform
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        for (name, scale) in &initial_scales {
            // Check if this is a positional channel or its interval variant
            let is_positional = required_channels
                .iter()
                .any(|ch| name == ch || name == &format!("{}2", ch));

            if is_positional {
                positional_scales.insert(name.clone(), scale.clone());
            } else {
                non_positional_scales.insert(name.clone(), scale.clone());
            }
        }

        // Build configured non-positional scales first (they don't depend on plot dimensions)
        let mut configured_non_positional = HashMap::new();
        for (name, scale) in non_positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale, &name, context, None, // No radius context for non-positional scales
                )
                .await?;
            configured_non_positional.insert(name, configured);
        }

        // Build configured positional scales with estimated dimensions
        let mut configured_positional = HashMap::new();
        for (name, scale) in positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale,
                    &name,
                    context,
                    Some(&configured_non_positional),
                )
                .await?;
            configured_positional.insert(name, configured);
        }

        Ok((
            initial_scales,
            configured_non_positional,
            configured_positional,
        ))
    }

    /// Rebuild positional scales with final dimensions after layout
    pub async fn rebuild_scales_with_final_dimensions(
        &self,
        initial_scales: &HashMap<String, Scale>,
        configured_non_positional: &HashMap<String, ConfiguredScaleWithSpec>,
        context: &crate::render::RenderContext,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let mut final_configured_scales = configured_non_positional.clone();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .coord_transform
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        // Rebuild positional scales with final dimensions
        for (name, scale) in initial_scales {
            // Check if this is a positional channel or its interval variant
            let is_positional = required_channels
                .iter()
                .any(|ch| name == ch || name == &format!("{}2", ch));

            if is_positional {
                let configured = self
                    .build_configured_scale_with_radius_context(
                        scale.clone(),
                        name,
                        context,
                        Some(configured_non_positional),
                    )
                    .await?;
                final_configured_scales.insert(name.clone(), configured);
            }
        }

        Ok(final_configured_scales)
    }

    // ===== Phase 4: Legend Channel Methods =====

    /// Build legend channels for a specific channel
    pub fn build_legend_channels_for_channel(
        &self,
        channel: &str,
        _legend: &Legend,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
    ) -> Result<Vec<crate::legend::LegendChannel>, AvengerChartError> {
        // Find the mark that has this channel
        let (mark_index, mark) = self
            .marks
            .iter()
            .enumerate()
            .find(|(_, m)| m.data_context().channels().contains_key(channel))
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Channel '{}' not found in any mark",
                    channel
                ))
            })?;

        let channel_value = mark.data_context().channels().get(channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!("Channel '{}' not found in mark", channel))
        })?;

        let scale = scales.get(channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!("Scale for channel '{}' not found", channel))
        })?;

        let mut legend_channels = Vec::new();

        // Always add the primary channel
        let primary_channel = self.build_legend_channel(
            channel,
            channel_value,
            scale,
            mark.as_ref(),
            mark_index,
            scales,
            ctx,
        );
        legend_channels.push(primary_channel);

        // Note: We're not handling merged_channels from the legend config here
        // as that would require accessing legend.merged_channels which might not exist yet
        // This can be enhanced later if needed

        Ok(legend_channels)
    }

    /// Merge legend channels based on merge keys
    pub fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
    ) -> (
        Vec<Vec<crate::legend::LegendChannel>>,
        IndexMap<String, Legend>,
    ) {
        use crate::legend::MergeKey;

        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for (mark_index, mark) in self.marks.iter().enumerate() {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Skip if no scale or no legend config
                if !configured_scales.contains_key(channel_name)
                    || !all_legends.contains_key(channel_name)
                {
                    continue;
                }

                let legend_config = &all_legends[channel_name];
                if matches!(legend_config.visible, crate::maybe::Maybe::Set(false)) {
                    continue;
                }

                let scale = &configured_scales[channel_name];

                let legend_channel = self.build_legend_channel(
                    channel_name,
                    channel_value,
                    scale,
                    mark.as_ref(),
                    mark_index,
                    configured_scales,
                    ctx,
                );

                all_channels.push(legend_channel);
            }
        }

        // Group channels by MergeKey
        let mut channel_groups: Vec<Vec<crate::legend::LegendChannel>> = Vec::new();

        for channel in all_channels {
            let merge_key = MergeKey::from_channel(&channel);

            if merge_key.is_none() {
                // Continuous scales or channels without expressions should not be merged
                channel_groups.push(vec![channel]);
            } else {
                // Find if this key already exists in any group
                let mut found = false;
                for group in channel_groups.iter_mut() {
                    if !group.is_empty() {
                        // Check if this group has the same merge key
                        let group_key = MergeKey::from_channel(&group[0]);
                        if group_key == merge_key {
                            group.push(channel.clone());
                            found = true;
                            break;
                        }
                    }
                }

                if !found {
                    // Create a new group for this merge key
                    channel_groups.push(vec![channel]);
                }
            }
        }

        // Sort channel groups by their legend order
        let mut groups_with_order: Vec<(Vec<crate::legend::LegendChannel>, i32)> = Vec::new();

        for channels in channel_groups {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    let order = legend_config.order.clone().unwrap_or(i32::MAX);
                    groups_with_order.push((channels, order));
                }
            }
        }

        // Sort by order value
        groups_with_order.sort_by_key(|(_, order)| *order);

        // Extract sorted channel groups
        let sorted_channel_groups: Vec<Vec<crate::legend::LegendChannel>> = groups_with_order
            .iter()
            .map(|(channels, _)| channels.clone())
            .collect();

        // Create the legends map with merged channel info for layout
        let mut legends_map: IndexMap<String, Legend> = IndexMap::new();
        for (channels, _) in groups_with_order {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    if !matches!(legend_config.visible, crate::maybe::Maybe::Set(false)) {
                        // Clone the legend config and add merged channel information
                        let mut legend_with_merged = legend_config.clone();
                        // Populate merged_channels with all channel types in this group
                        legend_with_merged.merged_channels =
                            channels.iter().map(|ch| ch.channel_type.clone()).collect();
                        legends_map.insert(primary_channel.name.clone(), legend_with_merged);
                    }
                }
            }
        }

        (sorted_channel_groups, legends_map)
    }

    /// Prepare legend measurements for layout computation
    /// Note: This should be called with the legends_map from merge_legend_channels
    /// to ensure measurements match rendering
    pub fn prepare_legend_measurements(
        &self,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_space: taffy::Size<f32>,
        ctx: &SessionContext,
    ) -> Result<crate::render::LegendMeasurements, AvengerChartError> {
        use crate::layout::legend::measure_legend_size_with_channels;

        let mut legend_measurements = crate::render::LegendMeasurements::new();

        // Get all legends including channel-level configs
        let all_legends = self.get_legends_with_theme(scales, ctx);

        // Merge channels to get the same groups that will be used for rendering
        let (sorted_channel_groups, _) = self.merge_legend_channels(&all_legends, scales, ctx);

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            // Get legend config - first try the passed-in legends (from merge),
            // then fall back to all_legends
            let legend = legends
                .get(&primary_channel.name)
                .or_else(|| all_legends.get(&primary_channel.name))
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Legend configuration not found for channel '{}'",
                        primary_channel.name
                    ))
                })?;

            // Get scale for primary channel
            let scale = scales.get(&primary_channel.name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Scale not found for channel '{}'",
                    primary_channel.name
                ))
            })?;

            // Determine the appropriate renderer for this group of channels
            let renderer = if channels.len() > 1 {
                // Multiple channels - try to get a merged renderer
                let mark_opt = self.marks.get(primary_channel.mark_index);
                // Extract ConfiguredScale from ConfiguredScaleWithSpec for mark's renderer
                let configured_scales: HashMap<String, ConfiguredScale> = scales
                    .iter()
                    .map(|(k, v)| (k.clone(), v.configured.clone()))
                    .collect();
                mark_opt
                    .and_then(|mark| mark.preferred_merged_legend_renderer(&channels, &configured_scales))
                    .or_else(|| self.get_legend_renderer(&primary_channel.channel_type, scale))
            } else {
                // Single channel - use the unified renderer selection
                self.get_legend_renderer(&primary_channel.channel_type, scale)
            };

            if let Some(renderer) = renderer {
                // Measure the legend with the same channels that will be used for rendering
                let (size, flexible) = measure_legend_size_with_channels(
                    &channels,
                    legend,
                    renderer,
                    available_space,
                )?;
                legend_measurements.insert(primary_channel.name.clone(), (size, flexible));
            }
        }

        Ok(legend_measurements)
    }

    // ===== Phase 5: Core Rendering Methods =====

    /// Render a single mark with its data and transformations
    pub async fn render_mark(
        &self,
        mark: &dyn CompiledMark,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references (e.g., ":x" -> actual x expression)
        let channels = crate::channel::resolution::resolve_all_channel_refs(channels, ctx)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                // Convert to Expr to check column refs
                expr.to_expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false)
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    let cond_has_refs = condition
                        .to_expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    let value_has_refs = value
                        .expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    cond_has_refs || value_has_refs
                }) || otherwise
                    .expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false)
            }
        });

        // Determine data source - convert from LogicalPlans to DataFrames using the context
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
            // Mark has explicit data
            Some(mark_df)
        } else if !references_columns {
            // No column references - use unit data
            None
        } else if let Some(plot_df) = self.data.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            // Inherit from plot
            Some(plot_df)
        } else {
            // No data available but columns are referenced
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Check if mark has a sorting channel and apply sorting if needed
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    // Apply sorting transformation
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales)?;

                    // Sort the DataFrame by the sorting expression
                    let sorted_df = df_ref.sort(vec![sort_expr.sort(true, false)])?;
                    Arc::new(sorted_df)
                } else {
                    Arc::new(df_ref)
                }
            } else {
                Arc::new(df_ref)
            }
        } else {
            // Unit data source - create minimal DataFrame with single row
            let empty_df = ctx
                .sql("SELECT 1 as _dummy")
                .await
                .map_err(|e| AvengerChartError::DataFusionError(e))?;
            Arc::new(empty_df)
        };

        // Get supported channels from the mark
        let supported_channels = mark.supported_channels();

        // Separate channels into those that need array data vs scalar data
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;

        for channel_desc in &supported_channels {
            if let Some(channel_value) = channels.get(channel_desc.name) {
                // Apply scaling to get the final expression
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales)?;

                // Check if this channel references columns (needs array data)
                if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                    array_channels.push((channel_desc.name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_desc.name, scaled_expr));
                }
            }
        }

        // Build array data batch if needed
        let data_batch = if has_array_data {
            let mut select_exprs = vec![];
            for (name, expr) in &array_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            let batch = (*df).clone().select(select_exprs)?.collect().await?;

            if batch.is_empty() {
                None
            } else {
                Some(batch[0].clone())
            }
        } else {
            None
        };

        // Build scalar data batch
        let mut scalar_select_exprs = vec![];
        for (name, expr) in &scalar_channels {
            scalar_select_exprs.push(expr.clone().alias(*name));
        }

        let scalar_batch = if !scalar_select_exprs.is_empty() {
            let batch = (*df).clone().select(scalar_select_exprs)?.collect().await?;
            if batch.is_empty() {
                // Create empty batch with correct schema
                return Ok(vec![]);
            } else {
                batch[0].clone()
            }
        } else {
            // Create an empty record batch
            use datafusion::arrow::array::Int32Array;
            use datafusion::arrow::datatypes::{DataType, Field, Schema};
            datafusion::arrow::record_batch::RecordBatch::try_new(
                Arc::new(Schema::new(vec![Field::new(
                    "_dummy",
                    DataType::Int32,
                    false,
                )])),
                vec![Arc::new(Int32Array::from(vec![0]))],
            )?
        };

        // Validate positional channel types
        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        // Create render context with theme and dimensions
        let theme = self.get_theme();
        let context = crate::render::RenderContext::new(
            theme,
            plot_width,
            plot_height,
            Arc::new(ctx.clone()),
        );

        // Clone the coordinate transform
        let coord_transform = self.coord_transform.clone_box();

        // Render the mark with data
        mark.render_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            coord_transform,
        )
    }

    /// Create guide marks (axes, grids) for the coordinate system
    pub async fn create_guide_marks(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use the pre-built guide renderer if available
        if let Some(guide_renderer) = &self.guide_renderer {
            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured.clone()))
                .collect();
            guide_renderer
                .render(&configured_scales, plot_width, plot_height, plot_bounds, theme.as_ref())
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    // ===== Phase 6: Layout and Legend Rendering =====

    /// Compute layout with overflow measurement
    pub async fn compute_layout(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
    ) -> Result<crate::render::LayoutSolution, AvengerChartError> {
        use crate::layout::ChartLayout;
        const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the guide needs if we have one
        let overflow = if let Some(guide_renderer) = &self.guide_renderer {
            let width_estimate = width * INITIAL_PLOT_AREA_RATIO;
            let height_estimate = height * INITIAL_PLOT_AREA_RATIO;

            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured.clone()))
                .collect();
            guide_renderer
                .measure_overflow(&configured_scales, width_estimate, height_estimate, theme.as_ref())
                .await?
        } else {
            // No guide renderer - no overflow
            crate::guide::OverflowSpaceRequirement::default()
        };

        // Get legends with theme applied
        let all_legends = self.get_legends_with_theme(scales, ctx);

        // Use the helper to merge legend channels
        let (_channel_groups, legends_map) = self.merge_legend_channels(&all_legends, scales, ctx);

        // Prepare legend measurements
        let available_size = taffy::Size {
            width: width * INITIAL_PLOT_AREA_RATIO,
            height: height * INITIAL_PLOT_AREA_RATIO,
        };
        let legend_measurements =
            self.prepare_legend_measurements(&legends_map, scales, available_size, ctx)?;

        // Create ChartLayout with overflow directly
        let layout_spec = self.get_layout_spec();
        let mut layout = ChartLayout::new_with_overflow(
            &overflow,
            &legends_map,
            layout_spec,
            self.get_title(),
            self.get_subtitle(),
            self.get_theme().as_ref(),
            &legend_measurements,
        )?;

        // Compute layout using the layout spec and return it directly
        layout.compute(layout_spec)
    }

    /// Create legends positioned according to layout
    pub fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::layout::LayoutResult,
        _plot_width: f32,
        _plot_height: f32,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales, ctx);

        // Use the helper to merge legend channels
        let (sorted_channel_groups, _legends_map) =
            self.merge_legend_channels(&all_legend_configs, scales, ctx);

        // Create legend marks positioned according to layout
        let mut legend_marks = Vec::new();

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            // Get legend config for primary channel
            let legend = &all_legend_configs[&primary_channel.name];

            // Get layout bounds for this legend
            if let Some(bounds) = layout.legends.get(&primary_channel.name) {
                // Determine the appropriate renderer for this group of channels
                let renderer_opt = if channels.len() > 1 {
                    // Multiple channels - try to get a merged renderer
                    // Find the mark that these channels belong to
                    let mark_opt = self.marks.get(primary_channel.mark_index);
                    // Extract ConfiguredScale from ConfiguredScaleWithSpec for mark's renderer
                    let configured_scales: HashMap<String, ConfiguredScale> = scales
                        .iter()
                        .map(|(k, v)| (k.clone(), v.configured.clone()))
                        .collect();

                    mark_opt
                        .and_then(|mark| mark.preferred_merged_legend_renderer(&channels, &configured_scales))
                } else {
                    // Single channel - use the unified renderer selection
                    scales.get(&primary_channel.name).and_then(|scale| {
                        self.get_legend_renderer(&primary_channel.channel_type, scale)
                    })
                };

                // Skip this legend group if no renderer is available
                if let Some(renderer) = renderer_opt {
                    // Render the legend with the determined renderer
                    let group_opt = renderer.render(
                        &channels,
                        legend,
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    )?;

                    // Add the legend group mark if it was rendered
                    if let Some(group) = group_opt {
                        legend_marks.push(SceneMark::Group(group));
                    }
                }
            }
        }

        Ok(legend_marks)
    }

    /// Render all components (marks, axes, legends, titles)
    async fn render_all_components(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::render::LayoutSolution,
        _width: f32,
        _height: f32,
        ctx: &SessionContext,
    ) -> Result<
        (
            Vec<SceneMark>, // mark_groups
            Vec<SceneMark>, // axis_marks
            Vec<SceneMark>, // legend_marks
            Vec<SceneMark>, // title_marks
            Vec<SceneMark>, // subtitle_marks
        ),
        AvengerChartError,
    > {
        let plot_bounds = layout.plot_area_bounds();
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // Render marks
        let mut mark_groups = Vec::new();
        for mark in &self.marks {
            let scene_marks = self
                .render_mark(
                    mark.as_ref(),
                    scales,
                    plot_area_width,
                    plot_area_height,
                    ctx,
                )
                .await?;
            mark_groups.extend(scene_marks);
        }

        // Create guide marks (axes, grids, backgrounds)
        let guide_marks = self
            .create_guide_marks(scales, plot_area_width, plot_area_height, plot_bounds)
            .await?;

        // Create legends
        let legend_marks = self.create_legends_with_layout(
            scales,
            &layout.taffy_layout,
            plot_area_width,
            plot_area_height,
            ctx,
        )?;

        // Create title
        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
            self.create_title(Some(*title_bounds))?
        } else {
            Vec::new()
        };

        // Create subtitle
        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
            self.create_subtitle(Some(*subtitle_bounds))?
        } else {
            Vec::new()
        };

        Ok((
            mark_groups,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
        ))
    }

    /// Render the plot to a scene graph
    pub async fn render(
        &self,
        ctx: &SessionContext,
    ) -> Result<crate::render::RenderResult, AvengerChartError> {
        use crate::render::RenderContext;
        use avenger_scenegraph::marks::group::SceneGroup;
        use avenger_scenegraph::scene_graph::SceneGraph;
        const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

        // Get layout spec and estimate initial dimensions
        let layout_spec = &self.layout_spec;

        // For now, we'll use a simple estimation for canvas size
        // This will be refined after we compute the actual layout
        let (estimated_width, estimated_height) = match &layout_spec.canvas {
            crate::layout::SizeMode::Fixed { width, height } => (*width, *height),
            _ => (400.0, 300.0), // Default for Auto or other modes
        };

        // Use estimated dimensions for initial scale construction
        let estimated_plot_width = estimated_width * INITIAL_PLOT_AREA_RATIO;
        let estimated_plot_height = estimated_height * INITIAL_PLOT_AREA_RATIO;

        // Create initial RenderContext with estimated dimensions and SessionContext
        let theme = self.get_theme();
        let initial_context = RenderContext::new(
            theme.clone(),
            estimated_plot_width,
            estimated_plot_height,
            Arc::new(ctx.clone()),
        );

        let (initial_scales, configured_non_positional, configured_positional) =
            self.build_initial_scales(&initial_context).await?;

        // Merge configured scales for layout computation
        let mut initial_configured_scales = configured_non_positional.clone();
        initial_configured_scales.extend(configured_positional.clone());

        // STAGE 2: COMPUTE LAYOUT USING INITIAL SCALES
        let layout = self
            .compute_layout(
                estimated_width,
                estimated_height,
                &initial_configured_scales,
                ctx,
            )
            .await?;
        let plot_bounds = layout.plot_area_bounds();
        let (final_width, final_height) = layout.canvas_size;
        let plot_area_x = plot_bounds.x;
        let plot_area_y = plot_bounds.y;
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // STAGE 3: REBUILD POSITIONAL SCALES WITH FINAL DIMENSIONS
        // Create final RenderContext with actual plot dimensions
        let final_context = RenderContext::new(
            theme.clone(),
            plot_area_width,
            plot_area_height,
            Arc::new(ctx.clone()),
        );

        let final_configured_scales = self
            .rebuild_scales_with_final_dimensions(
                &initial_scales,
                &configured_non_positional,
                &final_context,
            )
            .await?;

        // STAGE 4: RENDER ALL COMPONENTS WITH FINAL SCALES
        let all_component_marks = self
            .render_all_components(
                &final_configured_scales,
                &layout,
                final_width,
                final_height,
                ctx,
            )
            .await?;

        let (mark_groups, guide_marks, legend_marks, title_marks, subtitle_marks) =
            all_component_marks;

        // Compose all elements into a scene graph
        // A single Plot should produce a single top-level group
        let mut all_marks = Vec::new();

        // Get the appropriate clipping region from the coordinate system
        // Get the appropriate clipping region from the guide renderer if available
        let clip = if let Some(ref guide) = self.guide_renderer {
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = final_configured_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured.clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured_scales)
        } else {
            // Default to rectangular clip for plot area
            avenger_scenegraph::marks::group::Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        };

        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip,
            zindex: Some(0), // Data marks have lowest z-index
            ..Default::default()
        };

        // Add background rect if theme specifies one
        if let Some(bg_color) = theme.canvas_background() {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            // Parse the color string to RGBA - fail if color is invalid
            let color = crate::utils::parse_color_to_array_strict(&bg_color)?;

            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(final_width.into()),
                height: Some(final_height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // No stroke
                stroke_width: 0.0.into(),
                zindex: Some(-100), // Ensure it's behind everything
                ..Default::default()
            };
            all_marks.push(SceneMark::Rect(background_rect));
        }

        // Add marks in proper z-order:
        // 1. Clipped data marks (background)
        all_marks.push(SceneMark::Group(data_marks_group));

        // 2. Guide marks (axes, grids, backgrounds - can overflow the plot area)
        all_marks.extend(guide_marks);

        // 3. Legends (positioned outside plot area)
        all_marks.extend(legend_marks);

        // 4. Title (can overflow, rendered on top)
        all_marks.extend(title_marks);

        // 5. Subtitle (can overflow, rendered on top)
        all_marks.extend(subtitle_marks);

        // 6. Debug: Add layout bounds visualization if AVENGER_CHART_DEBUG_LAYOUT is set
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            all_marks.extend(crate::render::debug::create_debug_layout_rects(
                &layout.taffy_layout,
            ));
        }

        // Wrap everything in a single root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        // Use the computed canvas size from layout
        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };

        // Build spatial index for hit testing
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(crate::render::RenderResult {
            scene_graph,
            rtree: Some(rtree),
        })
    }
}

#[derive(Clone)]
pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axis_specs: HashMap<String, AxisSpec>,
    pub(crate) legends: IndexMap<String, Legend>,
    pub(crate) mark_renderers: Vec<Arc<dyn CompiledMark>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y2" -> "y", "x2" -> "x"
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Layout specification for sizing and margins
    pub(crate) layout_spec: LayoutSpec,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme for visual styling
    pub(crate) theme: Option<Arc<dyn Theme>>,

    /// Guide configuration
    pub(crate) guide_config: Option<C::Guide>,

    /// Built guide renderer (for serialization)
    pub(crate) guide_renderer: Option<Arc<dyn CompiledGuide>>,
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            axis_specs: HashMap::new(),
            legends: IndexMap::new(),
            mark_renderers: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            scale_to_coord_channel: HashMap::new(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            theme: None,
            guide_config: None,
            guide_renderer: None,
        }
    }
}

impl<C: CoordinateSystem + Default> Default for Plot<C> {
    fn default() -> Self {
        Self::with_coord(C::default())
    }
}

impl<C: CoordinateSystem + Default> Plot<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    /// Compile this plot into a renderable form (consuming self)
    pub async fn compile(
        mut self,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        // Always build guide renderer - either from config or default
        if self.guide_renderer.is_none() {
            let mut guide = if let Some(config) = &self.guide_config {
                config.clone()
            } else {
                // Create default guide for the coordinate system
                C::Guide::default()
            };

            // We need to populate axes from axis_specs before building
            // This mirrors what PlotRenderer does in render/guide.rs
            use crate::guide::CoordinateGuideBuilder;

            // Create axes map for the guide
            let mut guide_axes = HashMap::new();

            // Add user-specified axes from axis_specs
            for (channel, axis_spec) in &self.axis_specs {
                let crate::plot::AxisSpec::Local(axis_config) = axis_spec;
                // The axis_config is already the correct type for this coordinate system
                // We need to downcast it to the specific axis type for the guide
                // This is safe because the axis type matches the coordinate system
                if let Some(typed_axis) = axis_config
                    .as_any()
                    .downcast_ref::<<C::Guide as CoordinateGuideBuilder>::Axis>(
                ) {
                    guide_axes.insert(channel.clone(), typed_axis.clone());
                }
            }

            // Set the axes on the guide
            guide.set_axes(guide_axes);

            // Pass mark renderers to the guide so it can extract titles at render time
            guide.set_mark_renderers(self.mark_renderers.clone(), session_context);

            self.guide_renderer = Some(Arc::from(guide.build()));
        }

        Ok(CompiledPlot {
            coord_transform: self.coord_system.create_transform(),
            guide_renderer: self.guide_renderer,
            marks: self.mark_renderers,
            axis_specs: self.axis_specs,
            legends: self.legends,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            scale_to_coord_channel: self.scale_to_coord_channel,
            scale_specs: self.scale_specs,
            data: match self.data {
                Some(df) => {
                    let plan = df.logical_plan().clone();
                    Some(LogicalPlanNode::from_logical_plan(&plan).map_err(|e| {
                        AvengerChartError::InternalError(format!(
                            "Failed to serialize logical plan: {}. \
                            This typically happens when using a DataFrame created with a different SessionContext. \
                            Make sure to use the same SessionContext for creating data and compiling the plot.",
                            e
                        ))
                    })?)
                }
                None => None,
            },
        })
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    /// Get a reference to the scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get a reference to the axis specifications
    pub fn axis_specs(&self) -> &HashMap<String, AxisSpec> {
        &self.axis_specs
    }

    /// Get a reference to the scale to coordinate channel mapping
    pub fn scale_to_coord_channel(&self) -> &HashMap<String, String> {
        &self.scale_to_coord_channel
    }

    /// Get a reference to the mark renderers
    pub fn mark_renderers(&self) -> &[Arc<dyn CompiledMark>] {
        &self.mark_renderers
    }

    /// Get a reference to the legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    /// Build a scale by name, applying any configured transformations
    /// Note: Default range will be applied during rendering when actual dimensions are known
    pub fn get_scale(&self, name: &str, ctx: &SessionContext) -> Result<Scale, AvengerChartError> {
        // Strip trailing numbers to get the base scale name
        // e.g., "x2" -> "x", "y2" -> "y"
        let base_name = strip_trailing_numbers(name);

        // Build the default scale for the base name
        let mut base_scale = self.create_default_scale_for_channel_internal(base_name, ctx)?;

        // Gather domain expressions from marks
        if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name, ctx) {
            if !domain_exprs.is_empty() {
                base_scale = base_scale.domain_data_fields(domain_exprs);
            }
        }

        match self.scale_specs.get(base_name) {
            Some(ScaleSpec::Local(scale_changes)) => {
                // Apply the plot-level scale configuration using update()
                let user_scale = base_scale.update(scale_changes.clone());
                Ok(user_scale)
            }
            None => Ok(base_scale),
        }
    }

    /// Get the default range for a coordinate channel based on plot area dimensions
    /// Returns None if the channel is not a coordinate channel
    pub fn get_coordinate_default_range(
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

        self.coord_system
            .default_range(coord_channel, plot_area_width, plot_area_height)
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Extract scale and legend configurations from the mark's channels
        self.extract_channel_configs(&mark);

        // Build the CompiledMark from the Mark
        let renderer = mark.build();

        // Add the renderer
        self.mark_renderers.push(renderer);

        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Get the layout specification
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    // ====== Layout API ======

    /// Set fixed canvas dimensions (traditional mode)
    /// The plot area will fill the available space within the canvas
    pub fn canvas_size(mut self, width: f32, height: f32) -> Self {
        self.layout_spec =
            LayoutSpec::fixed_canvas(width, height, self.layout_spec.margins.clone());
        self
    }

    /// Set canvas sizing constraint for responsive layouts
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        // If setting aspect ratio, clear plot aspect ratio to avoid conflicts
        if matches!(constraint, CanvasConstraint::PreferredAspectRatio(_))
            && matches!(
                self.layout_spec.plot_area,
                crate::layout::SizeMode::AspectRatio(_)
            )
        {
            self.layout_spec.plot_area = crate::layout::SizeMode::Auto;
        }
        self.layout_spec.canvas = constraint.into();
        self
    }

    /// Set fixed plot area dimensions (data-first mode)
    /// The canvas will expand to accommodate the plot area plus margins, axes, and legends
    pub fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.layout_spec =
            LayoutSpec::fixed_plot_area(width, height, self.layout_spec.margins.clone());
        self
    }

    /// Set plot area sizing constraint for responsive layouts
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.layout_spec.plot_area = match constraint {
            PlotConstraint::Auto => crate::layout::SizeMode::Auto,
            PlotConstraint::AspectRatio(r) => crate::layout::SizeMode::AspectRatio(r),
            PlotConstraint::Width(w) => crate::layout::SizeMode::Width(w),
            PlotConstraint::Height(h) => crate::layout::SizeMode::Height(h),
        };
        // If setting aspect ratio, clear canvas aspect ratio to avoid conflicts
        if matches!(constraint, PlotConstraint::AspectRatio(_))
            && matches!(
                self.layout_spec.canvas,
                crate::layout::SizeMode::AspectRatio(_)
            )
        {
            self.layout_spec.canvas = crate::layout::SizeMode::Auto;
        }
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.layout_spec.margins = margins;
        self
    }

    /// Set the theme for the plot
    pub fn theme(mut self, theme: impl Theme + 'static) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.guide_config = match self.guide_config {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.update(guide);
                Some(updated)
            }
            None => Some(guide),
        };
        self
    }

    /// Access the configured theme (or default if not set)
    pub fn get_theme(&self) -> Arc<dyn Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(CssTheme::light()))
    }
}
