//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use crate::scales::Scale;
use crate::utils::ScalarValueHelpers;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::array::ArrayRef;
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::prelude::DataFrame;
use datafusion_common::ScalarValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Padding around a plot area
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Type of legend to create based on mark type and channel
#[derive(Debug, Clone, Copy, PartialEq)]
enum LegendType {
    Symbol,
    Line,
    Colorbar,
}

/// Parameters for creating a legend
#[derive(Debug, Clone)]
struct LegendParams<'a> {
    channel: &'a str,
    legend: &'a crate::legend::Legend,
    scales: &'a HashMap<String, Scale>, // All available scales
    plot_width: f32,
    plot_height: f32,
    padding: &'a Padding,
    legend_margin: f32,
    y_offset: f32,
}

/// Result of rendering a plot to scene graph components
pub struct RenderResult {
    /// The complete scene graph ready for rendering
    pub scene_graph: SceneGraph,
    /// Spatial index for efficient hit testing
    pub rtree: Option<avenger_geometry::rtree::SceneGraphRTree>,
}

/// Renderer for converting Plot specifications to SceneGraph
pub struct PlotRenderer<'a, C: CoordinateSystem> {
    plot: &'a Plot<C>,
}

/// Helper to parse shape strings
fn parse_shape(s: &str) -> Result<avenger_common::types::SymbolShape, AvengerChartError> {
    avenger_common::types::SymbolShape::from_vega_str(s)
        .map_err(|_| AvengerChartError::InternalError(format!("Invalid shape name: '{}'", s)))
}

/// Helper to parse color from string using the color coercer
fn parse_color_string(color_str: &str) -> Option<avenger_common::types::ColorOrGradient> {
    use avenger_scales::scales::coerce::Coercer;
    use datafusion_common::ScalarValue;

    let coercer = Coercer::default();
    let array = ScalarValue::iter_to_array(
        [ScalarValue::Utf8(Some(color_str.to_string()))]
            .iter()
            .cloned(),
    )
    .ok()?;
    coercer
        .to_color(&array, None)
        .ok()
        .and_then(|colors| colors.as_vec(1, None).first().cloned())
}

impl<'a, C: CoordinateSystem> PlotRenderer<'a, C> {
    pub fn new(plot: &'a Plot<C>) -> Self {
        Self { plot }
    }

    /// Render the plot to a scene graph
    pub async fn render(&self) -> Result<RenderResult, AvengerChartError> {
        // Get plot dimensions from preferred size or default
        let (width, height) = self.plot.get_preferred_size().unwrap_or((400.0, 300.0));

        // Calculate padding for axes, title, etc.
        let padding = self.plot.measure_padding(width as f32, height as f32);

        // Calculate plot area (inside padding)
        let plot_area_x = padding.left;
        let plot_area_y = padding.top;
        let plot_area_width = width as f32 - padding.left - padding.right;
        let plot_area_height = height as f32 - padding.top - padding.bottom;

        // First, collect all channels that need scales
        let channels_with_scales = self.plot.collect_channels_needing_scales();

        // Build all scales on demand, applying configured transformations
        let mut all_scales = HashMap::new();
        for channel in &channels_with_scales {
            let scale = self.plot.get_scale(channel);
            all_scales.insert(channel.clone(), scale);
        }

        // Separate scales into positional and non-positional
        let mut positional_scales = HashMap::new();
        let mut non_positional_scales = HashMap::new();

        for (name, scale) in all_scales {
            if matches!(name.as_str(), "x" | "y") {
                positional_scales.insert(name, scale);
            } else {
                non_positional_scales.insert(name, scale);
            }
        }

        // Stage 1: Process non-positional scales first
        // Collect names first to avoid borrowing issues
        let non_pos_scale_names: Vec<String> = non_positional_scales.keys().cloned().collect();
        for name in non_pos_scale_names {
            let scale = non_positional_scales.get_mut(&name).unwrap();
            self.process_scale(
                scale,
                &name,
                plot_area_width,
                plot_area_height,
                &HashMap::new(), // Non-positional scales don't need other scales for domain inference
                |scale_copy| {
                    // Apply default color range for color channels
                    if matches!(name.as_str(), "fill" | "stroke" | "color") {
                        self.plot.apply_default_color_range(scale_copy, &name);
                    }

                    // Apply default shape range for shape channel
                    if name == "shape" {
                        self.plot.apply_default_shape_range(scale_copy);
                    }
                },
            )
            .await?;
        }

        // Stage 2: Process positional scales
        // Collect names first to avoid borrowing issues
        let pos_scale_names: Vec<String> = positional_scales.keys().cloned().collect();

        // Prepare all scales for radius-aware domain calculation
        let mut all_scales_for_process = non_positional_scales.clone();
        all_scales_for_process.extend(positional_scales.clone());

        for name in pos_scale_names {
            let scale = positional_scales.get_mut(&name).unwrap();
            self.process_scale(
                scale,
                &name,
                plot_area_width,
                plot_area_height,
                &all_scales_for_process,
                |_scale_copy| {
                    // Padding is now handled by radius-aware domain calculation
                },
            )
            .await?;
        }

        // Merge all scales back together for mark rendering
        let mut all_scales = non_positional_scales;
        all_scales.extend(positional_scales);

        // Use the full plot area for clipping
        // Marks should be clipped to the plot area bounds, not the data bounds
        let clip_height = plot_area_height;

        // Process each mark
        let mut mark_groups = Vec::new();

        // Debug rects removed for cleaner output

        for mark in &self.plot.marks {
            let scene_marks = self
                .render_mark(
                    mark.as_ref(),
                    &all_scales,
                    plot_area_width,
                    plot_area_height,
                )
                .await?;
            mark_groups.extend(scene_marks);
        }

        // Create axes
        let axis_marks = self
            .create_axes(&all_scales, plot_area_width, plot_area_height, &padding)
            .await?;

        // Create legends
        let legend_marks = self
            .create_legends(&all_scales, plot_area_width, plot_area_height, &padding)
            .await?;

        // Create title if present
        let title_marks = self.create_title(width as f32, &padding)?;

        // Compose all elements into a scene graph
        // A single Plot should produce a single top-level group
        let mut all_marks = Vec::new();

        // Create a group for data marks with adjusted clipping
        // Note: clip coordinates are relative to the group's origin
        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip: Clip::Rect {
                x: 0.0, // Relative to group origin
                y: 0.0, // Relative to group origin
                width: plot_area_width,
                height: clip_height,
            },
            zindex: Some(0), // Data marks have lowest z-index
            ..Default::default()
        };

        // Add marks in proper z-order:
        // 1. Clipped data marks (background)
        all_marks.push(SceneMark::Group(data_marks_group));

        // 2. Axes (can overflow the plot area)
        all_marks.extend(axis_marks);

        // 3. Legends (positioned outside plot area)
        all_marks.extend(legend_marks);

        // 4. Title (can overflow, rendered on top)
        all_marks.extend(title_marks);

        // Wrap everything in a single root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width: width as f32,
            height: height as f32,
            origin: [0.0, 0.0],
        };

        // Build spatial index for hit testing
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(RenderResult {
            scene_graph,
            rtree: Some(rtree),
        })
    }

    /// Check if an expression references any columns
    fn references_columns(expr: &Expr) -> bool {
        match expr {
            Expr::Column(_) => true,
            Expr::Literal(..) => false,
            Expr::ScalarFunction(func) => func.args.iter().any(|arg| Self::references_columns(arg)),
            Expr::BinaryExpr(binary) => {
                Self::references_columns(&binary.left) || Self::references_columns(&binary.right)
            }
            Expr::Alias(alias) => Self::references_columns(&alias.expr),
            Expr::Cast(cast) => Self::references_columns(&cast.expr),
            Expr::TryCast(cast) => Self::references_columns(&cast.expr),
            Expr::Not(expr) => Self::references_columns(expr),
            Expr::IsNull(expr) => Self::references_columns(expr),
            Expr::IsNotNull(expr) => Self::references_columns(expr),
            Expr::IsTrue(expr) => Self::references_columns(expr),
            Expr::IsFalse(expr) => Self::references_columns(expr),
            Expr::IsUnknown(expr) => Self::references_columns(expr),
            Expr::IsNotTrue(expr) => Self::references_columns(expr),
            Expr::IsNotFalse(expr) => Self::references_columns(expr),
            Expr::IsNotUnknown(expr) => Self::references_columns(expr),
            Expr::Negative(expr) => Self::references_columns(expr),
            Expr::Case(case) => {
                let expr_refs = case
                    .expr
                    .as_ref()
                    .map(|e| Self::references_columns(e))
                    .unwrap_or(false);
                let when_refs = case.when_then_expr.iter().any(|(when, then)| {
                    Self::references_columns(when) || Self::references_columns(then)
                });
                let else_refs = case
                    .else_expr
                    .as_ref()
                    .map(|e| Self::references_columns(e))
                    .unwrap_or(false);
                expr_refs || when_refs || else_refs
            }
            _ => false, // For other expression types, conservatively assume no column references
        }
    }

    /// Render a single mark to scene marks using the new Mark trait
    async fn render_mark(
        &self,
        mark: &dyn Mark<C>,
        scales: &HashMap<String, Scale>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get the data - either from mark or inherit from plot
        let df_ref = match mark.data_source() {
            crate::marks::DataSource::Explicit => mark.data_context().dataframe(),
            crate::marks::DataSource::Inherited => {
                // Get plot-level data
                self.plot.data.as_ref().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Mark expects inherited data but plot has no data".to_string(),
                    )
                })?
            }
        };

        // Get channel mappings from DataContext
        let encodings = mark.data_context().encodings();

        // Check if mark supports order and has order encoding
        let df = if mark.supports_order() {
            if let Some(order_channel) = encodings.get("order") {
                // Apply order transformation
                let order_expr = self.apply_channel_scale("order", order_channel, scales)?;

                // Sort the DataFrame by the order expression
                let sorted_df = df_ref.clone().sort(vec![order_expr.sort(true, false)])?;
                Arc::new(sorted_df)
            } else {
                Arc::new(df_ref.clone())
            }
        } else {
            Arc::new(df_ref.clone())
        };

        // Get supported channels from the mark
        let supported_channels = mark.supported_channels();

        // Separate channels into those that need array data vs scalar data
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;

        for channel_desc in &supported_channels {
            if let Some(channel_value) = encodings.get(channel_desc.name) {
                // Apply scaling to get the final expression
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales)?;

                // Check if this channel references columns (needs array data)
                if channel_desc.allow_column_ref && Self::references_columns(&scaled_expr) {
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

        // Build scalar batch - always needed, even if empty
        let scalar_batch = {
            let mut select_exprs = vec![];
            for (name, expr) in &scalar_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            if select_exprs.is_empty() {
                // Create empty batch with single row - need at least one column
                use datafusion::arrow::array::Int32Array;
                let schema = Arc::new(Schema::new(vec![Field::new(
                    "_dummy",
                    datafusion::arrow::datatypes::DataType::Int32,
                    false,
                )]));
                let array = Arc::new(Int32Array::from(vec![0]));
                RecordBatch::try_new(schema, vec![array]).unwrap()
            } else {
                // Execute query to get scalar values
                let batches = (*df)
                    .clone()
                    .select(select_exprs)?
                    .limit(0, Some(1))? // Only need one row for scalars
                    .collect()
                    .await?;

                if batches.is_empty() {
                    RecordBatch::try_new(Arc::new(Schema::new(vec![] as Vec<Field>)), vec![])
                        .unwrap()
                } else {
                    batches[0].clone()
                }
            }
        };

        // Call the mark's render_from_data method
        mark.render_from_data(data_batch.as_ref(), &scalar_batch)
    }

    /// Apply scaling transformation to a channel expression
    fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, Scale>,
    ) -> Result<datafusion::logical_expr::Expr, AvengerChartError> {
        use crate::marks::channel::strip_trailing_numbers;
        use datafusion::logical_expr::lit;

        let expr = channel_value.expr();

        match channel_value {
            ChannelValue::Identity { .. } => {
                // No scaling requested, return expression as-is
                Ok(expr.clone())
            }
            ChannelValue::Scaled {
                scale_name, band, ..
            } => {
                // Determine scale name (custom or derived from channel)
                let scale_key = scale_name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| strip_trailing_numbers(channel_name).to_string());

                // Scale MUST exist if scaling was requested
                let scale = scales.get(&scale_key).ok_or_else(|| {
                    AvengerChartError::ScaleNotFound(format!(
                        "Scale '{}' requested for channel '{}' but not found",
                        scale_key, channel_name
                    ))
                })?;

                // Handle band parameter for any scale that supports it
                let scale = if let Some(band_value) = band {
                    let scale_type = scale.get_scale_impl().scale_type();
                    if scale_type == "band" || scale_type == "point" {
                        scale.clone().option("band", lit(*band_value))
                    } else {
                        // Ignore band parameter for non-band/point scales
                        scale.clone()
                    }
                } else {
                    scale.clone()
                };

                // Apply the scale transformation
                scale.to_expr(expr.clone())
            }
        }
    }

    /// Create axis marks based on configured axes
    async fn create_axes(
        &self,
        scales: &HashMap<String, Scale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get default axes from the coordinate system
        let default_axes =
            self.plot
                .coord_system()
                .create_default_axes(scales, &self.plot.axes, &self.plot.marks);

        // Combine existing axes with defaults
        let mut all_axes = self.plot.axes.clone();
        for (channel, default_axis) in default_axes {
            all_axes.entry(channel).or_insert(default_axis);
        }

        // Delegate all axis rendering to the coordinate system
        self.plot
            .coord_system()
            .render_axes(&all_axes, scales, plot_width, plot_height, padding)
            .await
    }

    /// Create legend marks based on configured legends
    async fn create_legends(
        &self,
        scales: &HashMap<String, Scale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::legend::LegendPosition;

        // Get default legends for channels with data-driven scales
        let default_legends = self.create_default_legends(scales);

        // Combine existing legends with defaults
        let mut all_legends = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legends.entry(channel).or_insert(default_legend);
        }

        // Filter out invisible legends and pair with their scales
        let visible_legends: Vec<_> = all_legends
            .iter()
            .filter(|(channel, legend)| legend.visible && scales.contains_key(*channel))
            .collect();

        if visible_legends.is_empty() {
            return Ok(Vec::new());
        }

        let mut legend_marks = Vec::new();

        // Group legends by position for better layout
        let mut right_legends = Vec::new();
        let mut left_legends = Vec::new();
        let mut top_legends = Vec::new();
        let mut bottom_legends = Vec::new();

        for (channel, legend) in visible_legends {
            let position = legend.position.unwrap_or(LegendPosition::Right);
            match position {
                LegendPosition::Right => right_legends.push((channel, legend)),
                LegendPosition::Left => left_legends.push((channel, legend)),
                LegendPosition::Top => top_legends.push((channel, legend)),
                LegendPosition::Bottom => bottom_legends.push((channel, legend)),
            }
        }

        // Render legends by position
        // For now, only implement right position
        if !right_legends.is_empty() {
            let mut y_offset = 0.0;
            let legend_margin = 20.0; // Space between plot and legend
            let legend_spacing = 20.0; // Space between multiple legends

            for (channel, legend) in right_legends {
                if let Some(scale) = scales.get(channel) {
                    let legend_type = self.determine_legend_type(channel, scale);

                    // Create legend based on type
                    let params = LegendParams {
                        channel,
                        legend,
                        scales,
                        plot_width,
                        plot_height,
                        padding,
                        legend_margin,
                        y_offset,
                    };

                    let legend_group = match legend_type {
                        LegendType::Symbol => self.create_symbol_legend(params).await?,
                        LegendType::Line => self.create_line_legend(params).await?,
                        LegendType::Colorbar => self.create_colorbar_legend(params).await?,
                    };

                    if let Some(group) = legend_group {
                        // Calculate legend height for spacing
                        // For now, use a fixed height estimate
                        y_offset += 100.0 + legend_spacing;

                        legend_marks.push(SceneMark::Group(group));
                    }
                }
            }
        }

        Ok(legend_marks)
    }

    /// Create default legends for channels with data-driven scales
    fn create_default_legends(
        &self,
        scales: &HashMap<String, Scale>,
    ) -> HashMap<String, crate::legend::Legend> {
        use crate::legend::Legend;

        let mut default_legends = HashMap::new();

        for (channel, scale) in scales {
            // Skip positional channels
            if matches!(channel.as_str(), "x" | "y" | "x2" | "y2" | "r" | "theta") {
                continue;
            }

            // Skip if legend already configured
            if self.plot.legends.contains_key(channel) {
                continue;
            }

            // Check if scale has data-driven domain (uses column expressions)
            let has_data_domain = self.scale_has_data_domain(scale);

            if has_data_domain {
                let legend = Legend::new()
                    .title(self.infer_legend_title(channel, scale))
                    .position(self.default_legend_position(channel));

                default_legends.insert(channel.clone(), legend);
            }
        }

        default_legends
    }

    /// Check if a scale has a data-driven domain
    fn scale_has_data_domain(&self, scale: &Scale) -> bool {
        use crate::scales::ScaleDefaultDomain;

        // Check if the scale's domain references data fields
        match &scale.get_domain().default_domain {
            ScaleDefaultDomain::DomainExprs(_) => true,
            ScaleDefaultDomain::Discrete(exprs) => {
                // Check if any expression references columns
                exprs.iter().any(|expr| {
                    // Simple heuristic: if the expression contains a Column reference
                    format!("{:?}", expr).contains("Column")
                })
            }
            ScaleDefaultDomain::Interval(_, _) => false,
        }
    }

    /// Infer a title for the legend based on channel and scale
    fn infer_legend_title(&self, channel: &str, _scale: &Scale) -> String {
        // TODO: Could potentially extract field name from scale's domain expression
        // For now, just use the channel name with proper casing
        match channel {
            "fill" => "Fill",
            "stroke" => "Stroke",
            "color" => "Color",
            "size" => "Size",
            "shape" => "Shape",
            "opacity" => "Opacity",
            "stroke_width" => "Stroke Width",
            "stroke_dash" => "Stroke Dash",
            _ => channel,
        }
        .to_string()
    }

    /// Get default legend position for a channel
    fn default_legend_position(&self, channel: &str) -> crate::legend::LegendPosition {
        // Color and shape legends typically go on the right
        // Size legends often go on the bottom
        match channel {
            "size" | "stroke_width" => crate::legend::LegendPosition::Bottom,
            _ => crate::legend::LegendPosition::Right,
        }
    }

    /// Determine the type of legend to create based on mark type, channel, and scale
    fn determine_legend_type(&self, channel: &str, scale: &Scale) -> LegendType {
        // Check if scale is continuous (for colorbar)
        let scale_type = scale.get_scale_impl().scale_type();
        let is_continuous = matches!(scale_type, "linear" | "log" | "pow" | "sqrt");

        // Color channels with continuous scales use colorbar
        if matches!(channel, "fill" | "stroke" | "color") && is_continuous {
            return LegendType::Colorbar;
        }

        // Check if any mark is a line mark
        let has_line_mark = self
            .plot
            .marks
            .iter()
            .any(|mark| mark.mark_type() == "line");

        // Use line legend for stroke properties on line marks
        if has_line_mark && matches!(channel, "stroke" | "stroke_width" | "stroke_dash") {
            return LegendType::Line;
        }

        // Default to symbol legend
        LegendType::Symbol
    }

    /// Create a symbol legend
    async fn create_symbol_legend(
        &self,
        params: LegendParams<'_>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_common::value::ScalarOrArray;
        use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};

        // Extract domain values from scale
        let legend_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;
        let domain_values = self.extract_scale_domain_values(legend_scale).await?;
        if domain_values.is_empty() {
            return Ok(None);
        }

        // Create text labels from domain values
        let text_values: Vec<String> = domain_values
            .iter()
            .map(|v| match v {
                datafusion_common::ScalarValue::Utf8(Some(s)) => s.clone(),
                _ => format!("{:?}", v),
            })
            .collect();

        // Check if any mark is a rect mark
        let has_rect_mark = self
            .plot
            .marks
            .iter()
            .any(|mark| mark.mark_type() == "rect");

        // Get mark defaults - use rect defaults if we have rect marks, otherwise symbol defaults
        use crate::coords::Cartesian;
        use crate::marks::{Mark, rect::Rect, symbol::Symbol};

        let (
            default_size,
            default_shape,
            default_angle,
            default_fill,
            default_stroke,
            default_stroke_width,
        ) = if has_rect_mark {
            // For rect marks, use fixed square shape and appropriate size
            let temp_rect = Rect::<Cartesian>::default();
            let temp_rect_ref: &dyn Mark<Cartesian> = &temp_rect;

            let fill = temp_rect_ref
                .default_channel_value("fill")
                .and_then(|scalar| scalar.as_scalar_string().ok())
                .unwrap_or_else(|| "#4682b4".to_string());

            let stroke = temp_rect_ref
                .default_channel_value("stroke")
                .and_then(|scalar| scalar.as_scalar_string().ok())
                .unwrap_or_else(|| "#000000".to_string());

            let stroke_width = temp_rect_ref
                .default_channel_value("stroke_width")
                .and_then(|scalar| scalar.as_f32().ok())
                .unwrap_or(1.0);

            // Use fixed square shape and appropriate size for rect legends
            (100.0, "square".to_string(), 0.0, fill, stroke, stroke_width)
        } else {
            // Use symbol defaults
            let temp_symbol = Symbol::<Cartesian>::default();
            let temp_symbol_ref: &dyn Mark<Cartesian> = &temp_symbol;

            let size = temp_symbol_ref
                .default_channel_value("size")
                .and_then(|scalar| scalar.as_f32().ok())
                .unwrap_or(64.0);

            let shape = temp_symbol_ref
                .default_channel_value("shape")
                .and_then(|scalar| scalar.as_scalar_string().ok())
                .unwrap_or_else(|| "circle".to_string());

            let angle = temp_symbol_ref
                .default_channel_value("angle")
                .and_then(|scalar| scalar.as_f32().ok())
                .unwrap_or(0.0);

            let fill = temp_symbol_ref
                .default_channel_value("fill")
                .and_then(|scalar| scalar.as_scalar_string().ok())
                .unwrap_or_else(|| "#4682b4".to_string());

            let stroke = temp_symbol_ref
                .default_channel_value("stroke")
                .and_then(|scalar| scalar.as_scalar_string().ok())
                .unwrap_or_else(|| "#000000".to_string());

            let stroke_width = temp_symbol_ref
                .default_channel_value("stroke_width")
                .and_then(|scalar| scalar.as_f32().ok())
                .unwrap_or(1.0);

            (size, shape, angle, fill, stroke, stroke_width)
        };

        // Initialize config with defaults
        let mut config = SymbolLegendConfig {
            title: params.legend.title.clone(),
            text: ScalarOrArray::new_array(text_values),
            inner_width: 0.0, // Don't offset internally, we'll position the whole group
            inner_height: 100.0, // Will be calculated by legend
            ..Default::default()
        };

        // Analyze mark encodings to determine how to set each channel
        // We'll look at all marks to find symbol or rect marks and check their encodings
        let mut mark_encodings = HashMap::new();
        for mark in &self.plot.marks {
            // Check if this is a symbol or rect mark by checking the mark type
            let mark_type = mark.mark_type();
            let is_relevant = mark_type == "symbol" || mark_type == "rect";
            if is_relevant {
                let encodings = mark.data_context().encodings();
                for (channel, value) in encodings {
                    mark_encodings.insert(channel.clone(), value.clone());
                }
            }
        }

        // Get the legend channel's expression for comparison
        let legend_channel_expr = mark_encodings.get(params.channel).map(|v| v.expr());

        // Shape channel
        let default_shape_parsed = parse_shape(&default_shape)?;
        config.shape = ScalarOrArray::new_scalar(default_shape_parsed);

        if params.channel == "shape" {
            // Shape is the legend channel - map domain values to shapes
            // Try to get shapes from the scale's range if available
            let shape_names = if params.channel == "shape" {
                self.extract_shape_range_from_scale(legend_scale).await?
            } else {
                // Not the legend channel, use defaults
                crate::scales::shape_defaults::DEFAULT_SHAPES
                    .iter()
                    .map(|&s| s.to_string())
                    .collect()
            };

            let shapes: Result<Vec<_>, _> = domain_values
                .iter()
                .enumerate()
                .map(|(i, _)| parse_shape(&shape_names[i % shape_names.len()]))
                .collect();
            config.shape = ScalarOrArray::new_array(shapes?);
        } else if let Some(channel_value) = mark_encodings.get("shape") {
            // Check if this uses the same expression as the legend channel
            if let Some(legend_expr) = legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - vary together
                    // Try to get shape scale and extract its range
                    // Get shape scale from legend_scale
                    let shape_scale = params.scales.get("shape").ok_or_else(|| {
                        AvengerChartError::InternalError("Shape scale not found".to_string())
                    })?;
                    let shape_names = self.extract_shape_range_from_scale(shape_scale).await?;

                    let shapes: Result<Vec<_>, _> = domain_values
                        .iter()
                        .enumerate()
                        .map(|(i, _)| parse_shape(&shape_names[i % shape_names.len()]))
                        .collect();
                    config.shape = ScalarOrArray::new_array(shapes?);
                } else if !Self::references_columns(channel_value.expr()) {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(ScalarValue::Utf8(Some(s))) = scalars.into_iter().next() {
                            config.shape = ScalarOrArray::new_scalar(parse_shape(&s)?);
                        }
                    }
                }
            }
        }

        // Size channel
        config.size = ScalarOrArray::new_scalar(default_size);

        if params.channel == "size" {
            // Size is the legend channel - map through scale
            let sizes = self
                .map_values_through_scale_numeric(legend_scale, &domain_values)
                .await?;
            config.size = ScalarOrArray::new_array(sizes);
        } else if let Some(channel_value) = mark_encodings.get("size") {
            // Check if this uses the same expression as the legend channel
            if let Some(legend_expr) = legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through size scale if available
                    // Try to get the size scale from legend_scale
                    let size_scale = params.scales.get("size").unwrap_or(legend_scale); // Fall back to legend scale
                    if size_scale.has_explicit_domain() {
                        let sizes = self
                            .map_values_through_scale_numeric(size_scale, &domain_values)
                            .await?;
                        config.size = ScalarOrArray::new_array(sizes);
                    } else {
                        // No size scale, use constant default size for all symbols
                        let sizes: Vec<_> = domain_values.iter().map(|_| default_size).collect();
                        config.size = ScalarOrArray::new_array(sizes);
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(value) =
                            scalars.into_iter().next().and_then(|s| s.as_f32().ok())
                        {
                            config.size = ScalarOrArray::new_scalar(value);
                        }
                    }
                }
            }
        }

        // Fill channel
        let default_fill_color = parse_color_string(&default_fill).unwrap_or(
            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
        );
        config.fill = ScalarOrArray::new_scalar(default_fill_color.clone());

        if params.channel == "fill" || params.channel == "color" {
            // Fill/color is the legend channel - map through scale
            let colors = self
                .map_values_through_scale(legend_scale, &domain_values)
                .await?;
            config.fill = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );
        } else if let Some(channel_value) = mark_encodings.get("fill") {
            // Check if this uses the same expression as the legend channel
            if let Some(legend_expr) = legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through fill scale if available
                    // Try to get the fill scale from legend_scale
                    let fill_scale = params.scales.get("fill").unwrap_or(legend_scale); // Fall back to legend scale
                    if fill_scale.has_explicit_domain() {
                        let colors = self
                            .map_values_through_scale(fill_scale, &domain_values)
                            .await?;
                        config.fill = ScalarOrArray::new_array(
                            colors
                                .into_iter()
                                .map(avenger_common::types::ColorOrGradient::Color)
                                .collect(),
                        );
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Ok(color_array) = ScalarValue::iter_to_array(scalars.iter().cloned())
                        {
                            use avenger_scales::scales::coerce::Coercer;
                            let coercer = Coercer::default();
                            if let Ok(colors) = coercer.to_color(&color_array, None) {
                                if let Some(color) = colors.as_vec(1, None).first() {
                                    config.fill = ScalarOrArray::new_scalar(color.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Stroke channel
        let default_stroke_color = parse_color_string(&default_stroke).unwrap_or(
            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
        );
        config.stroke = ScalarOrArray::new_scalar(default_stroke_color.clone());

        if params.channel == "stroke" {
            // Stroke is the legend channel - map through scale
            let colors = self
                .map_values_through_scale(legend_scale, &domain_values)
                .await?;
            config.stroke = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );
        } else if let Some(channel_value) = mark_encodings.get("stroke") {
            // Check if this uses the same expression as the legend channel
            if let Some(legend_expr) = legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through stroke scale if available
                    if let Some(stroke_scale) = params.scales.get("stroke") {
                        if stroke_scale.has_explicit_domain() {
                            let colors = self
                                .map_values_through_scale(stroke_scale, &domain_values)
                                .await?;
                            config.stroke = ScalarOrArray::new_array(
                                colors
                                    .into_iter()
                                    .map(avenger_common::types::ColorOrGradient::Color)
                                    .collect(),
                            );
                        }
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Ok(color_array) = ScalarValue::iter_to_array(scalars.iter().cloned())
                        {
                            use avenger_scales::scales::coerce::Coercer;
                            let coercer = Coercer::default();
                            if let Ok(colors) = coercer.to_color(&color_array, None) {
                                if let Some(color) = colors.as_vec(1, None).first() {
                                    config.stroke = ScalarOrArray::new_scalar(color.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Stroke width channel - start with default
        config.stroke_width = Some(default_stroke_width);

        if let Some(channel_value) = mark_encodings.get("stroke_width") {
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(value) = scalars.into_iter().next().and_then(|s| s.as_f32().ok()) {
                        config.stroke_width = Some(value);
                    }
                }
            }
        }

        // Angle channel
        config.angle = ScalarOrArray::new_scalar(default_angle);

        if params.channel == "angle" {
            // Angle is the legend channel - map through scale
            let angles = self
                .map_values_through_scale_numeric(legend_scale, &domain_values)
                .await?;
            config.angle = ScalarOrArray::new_array(angles);
        } else if let Some(channel_value) = mark_encodings.get("angle") {
            // Check if this uses the same expression as the legend channel
            if let Some(legend_expr) = legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through angle scale if available
                    // Try to get the angle scale from legend_scale
                    let angle_scale = params.scales.get("angle").unwrap_or(legend_scale); // Fall back to legend scale
                    if angle_scale.has_explicit_domain() {
                        let angles = self
                            .map_values_through_scale_numeric(angle_scale, &domain_values)
                            .await?;
                        config.angle = ScalarOrArray::new_array(angles);
                    } else {
                        // No angle scale, use constant default angle for all symbols
                        let angles: Vec<_> = domain_values.iter().map(|_| default_angle).collect();
                        config.angle = ScalarOrArray::new_array(angles);
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(value) =
                            scalars.into_iter().next().and_then(|s| s.as_f32().ok())
                        {
                            config.angle = ScalarOrArray::new_scalar(value);
                        }
                    }
                }
            }
        }

        // Create the legend marks
        let legend_group = make_symbol_legend(&config)?;

        // Position the legend
        let x = params.padding.left + params.plot_width + params.legend_margin;
        let y = params.padding.top + params.y_offset;

        Ok(Some(SceneGroup {
            origin: [x, y],
            marks: legend_group.marks,
            zindex: Some(10), // Legends above data but below title
            ..Default::default()
        }))
    }

    /// Convert dash pattern names to numeric arrays using the coercer
    fn convert_dash_pattern(pattern: &str) -> Option<Vec<f32>> {
        use avenger_scales::scales::coerce::Coercer;
        use datafusion::arrow::array::StringArray;

        // Create a single-element string array with the pattern
        let array = StringArray::from(vec![Some(pattern)]);
        let array_ref = Arc::new(array) as ArrayRef;

        // Use coercer to convert
        let coercer = Coercer::default();
        if let Ok(dash_result) = coercer.to_stroke_dash(&array_ref) {
            // Get the first element from the ScalarOrArray result
            if let Some(dash_vec) = dash_result.first() {
                if dash_vec.is_empty() {
                    None // solid pattern
                } else {
                    Some(dash_vec.clone())
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Map domain values to dash patterns through a scale
    async fn map_dash_patterns(
        &self,
        domain_values: &[datafusion_common::ScalarValue],
        scale: &Scale,
    ) -> Result<Vec<Option<Vec<f32>>>, AvengerChartError> {
        use datafusion::arrow::array::{Array, StringArray};

        if scale.has_explicit_domain() {
            // Create a ConfiguredScale
            let configured_scale = scale.create_configured_scale(f32::NAN, f32::NAN).await?;

            // Convert scalar values to an arrow array
            let domain_array = ScalarValue::iter_to_array(domain_values.iter().cloned())?;

            // Apply the scale transformation
            let scaled_array = configured_scale.scale(&domain_array)?;

            // The result could be string patterns or dictionary array
            // Try to handle both cases
            use datafusion::arrow::datatypes::DataType;

            // Check if it's a dictionary array
            if let DataType::Dictionary(_, _) = scaled_array.data_type() {
                // It's a dictionary array - extract the values
                use avenger_scales::scales::coerce::Coercer;
                let coercer = Coercer::default();

                // Convert the dictionary array to stroke dash patterns
                if let Ok(dash_result) = coercer.to_stroke_dash(&scaled_array) {
                    let patterns: Vec<Option<Vec<f32>>> = dash_result
                        .as_vec(domain_values.len(), None)
                        .into_iter()
                        .map(|dash_vec| {
                            if dash_vec.is_empty() {
                                None // solid pattern
                            } else {
                                Some(dash_vec)
                            }
                        })
                        .collect();
                    Ok(patterns)
                } else {
                    // Default to solid for all
                    Ok(vec![None; domain_values.len()])
                }
            } else if let Some(string_array) = scaled_array.as_any().downcast_ref::<StringArray>() {
                // It's a string array - convert each pattern
                let patterns: Vec<Option<Vec<f32>>> = (0..string_array.len())
                    .map(|i| {
                        if string_array.is_null(i) {
                            None
                        } else {
                            Self::convert_dash_pattern(string_array.value(i))
                        }
                    })
                    .collect();
                Ok(patterns)
            } else {
                // Default to solid for all
                Ok(vec![None; domain_values.len()])
            }
        } else {
            // No explicit domain, use defaults
            Ok(vec![None; domain_values.len()])
        }
    }

    /// Create a line legend
    async fn create_line_legend(
        &self,
        params: LegendParams<'_>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_common::value::ScalarOrArray;
        use avenger_guides::legend::line::{LineLegendConfig, make_line_legend};
        use std::collections::HashMap;

        // Extract domain values from scale
        let legend_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;
        let domain_values = self.extract_scale_domain_values(legend_scale).await?;
        if domain_values.is_empty() {
            return Ok(None);
        }

        // Create text labels from domain values
        let text_values: Vec<String> = domain_values
            .iter()
            .map(|v| match v {
                datafusion_common::ScalarValue::Utf8(Some(s)) => s.clone(),
                _ => format!("{:?}", v),
            })
            .collect();

        // Get mark defaults from a default line mark instance
        use crate::coords::Cartesian;
        use crate::marks::{Mark, line::Line};
        let temp_line = Line::<Cartesian>::default();
        let temp_line_ref: &dyn Mark<Cartesian> = &temp_line;

        // Extract defaults using the mark's default_channel_value method
        let default_stroke = temp_line_ref
            .default_channel_value("stroke")
            .and_then(|scalar| scalar.as_scalar_string().ok())
            .unwrap_or_else(|| "#000000".to_string());

        let default_stroke_width = temp_line_ref
            .default_channel_value("stroke_width")
            .and_then(|scalar| scalar.as_f32().ok())
            .unwrap_or(2.0);

        // Initialize config with defaults
        // Use longer line length for better dash pattern visibility
        let mut config = LineLegendConfig {
            title: params.legend.title.clone(),
            text: ScalarOrArray::new_array(text_values),
            inner_width: 0.0,
            inner_height: 100.0,
            line_length: 20.0,
            ..Default::default()
        };

        // Analyze mark encodings to determine how to set each channel
        // We'll look at all marks to find line marks and check their encodings
        let mut mark_encodings = HashMap::new();
        for mark in &self.plot.marks {
            // Check if this is a line mark by checking the mark type
            let mark_type = mark.mark_type();
            if mark_type == "line" {
                let encodings = mark.data_context().encodings();
                for (channel, value) in encodings {
                    mark_encodings.insert(channel.clone(), value.clone());
                }
            }
        }

        // Get the expression being used for the legend channel
        let legend_channel_expr = mark_encodings.get(params.channel).map(|v| v.expr().clone());

        // Set stroke color based on whether it varies with the legend channel
        if params.channel == "stroke" {
            // Legend is for stroke itself - vary stroke color
            // Always map through the scale for the legend channel
            let colors = self
                .map_values_through_scale(legend_scale, &domain_values)
                .await?;
            config.stroke = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );
        } else if let Some(channel_value) = mark_encodings.get("stroke") {
            // Check if stroke uses the same expression as the legend channel
            if let Some(legend_expr) = &legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through stroke scale if available
                    if let Some(stroke_scale) = params.scales.get("stroke") {
                        if stroke_scale.has_explicit_domain() {
                            let colors = self
                                .map_values_through_scale(stroke_scale, &domain_values)
                                .await?;
                            config.stroke = ScalarOrArray::new_array(
                                colors
                                    .into_iter()
                                    .map(avenger_common::types::ColorOrGradient::Color)
                                    .collect(),
                            );
                        }
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Constant expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(ScalarValue::Utf8(Some(color_str))) = scalars.into_iter().next()
                        {
                            if let Some(color) = parse_color_string(&color_str) {
                                config.stroke = ScalarOrArray::new_scalar(color);
                            }
                        }
                    }
                }
            }
        } else {
            // Use default stroke color
            if let Some(color) = parse_color_string(&default_stroke) {
                config.stroke = ScalarOrArray::new_scalar(color);
            }
        }

        // Set stroke width based on whether it varies with the legend channel
        if params.channel == "stroke_width" {
            // Legend is for stroke_width itself - vary width
            // Always map through the scale for the legend channel
            let widths = self
                .map_values_through_scale_numeric(legend_scale, &domain_values)
                .await?;
            config.stroke_width = ScalarOrArray::new_array(widths);
        } else if let Some(channel_value) = mark_encodings.get("stroke_width") {
            // Check if stroke_width uses the same expression as the legend channel
            if let Some(legend_expr) = &legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through stroke_width scale if available
                    let stroke_width_scale =
                        params.scales.get("stroke_width").unwrap_or(legend_scale);
                    if stroke_width_scale.has_explicit_domain() {
                        let widths = self
                            .map_values_through_scale_numeric(stroke_width_scale, &domain_values)
                            .await?;
                        config.stroke_width = ScalarOrArray::new_array(widths);
                    }
                } else if !Self::references_columns(channel_value.expr()) {
                    // Constant expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(value) =
                            scalars.into_iter().next().and_then(|s| s.as_f32().ok())
                        {
                            config.stroke_width = ScalarOrArray::new_scalar(value);
                        }
                    }
                }
            }
        } else {
            // Use default stroke width
            config.stroke_width = ScalarOrArray::new_scalar(default_stroke_width);
        }

        // Set stroke dash based on whether it varies with the legend channel
        if params.channel == "stroke_dash" {
            // Legend is for stroke_dash itself - vary dash pattern
            let dash_patterns = self.map_dash_patterns(&domain_values, legend_scale).await?;
            config.stroke_dash = ScalarOrArray::new_array(dash_patterns);
        } else if let Some(channel_value) = mark_encodings.get("stroke_dash") {
            // Check if stroke_dash uses the same expression as the legend channel
            if let Some(legend_expr) = &legend_channel_expr {
                if channel_value.expr() == legend_expr {
                    // Same expression as legend channel - map through stroke_dash scale if available
                    let stroke_dash_scale =
                        params.scales.get("stroke_dash").unwrap_or(legend_scale);
                    let dash_patterns = self
                        .map_dash_patterns(&domain_values, stroke_dash_scale)
                        .await?;
                    config.stroke_dash = ScalarOrArray::new_array(dash_patterns);
                } else if !Self::references_columns(channel_value.expr()) {
                    // Constant expression - evaluate it
                    if let Ok(scalars) = crate::utils::eval_to_scalars(
                        vec![channel_value.expr().clone()],
                        None,
                        None,
                    )
                    .await
                    {
                        if let Some(ScalarValue::Utf8(Some(pattern_str))) =
                            scalars.into_iter().next()
                        {
                            let dash = Self::convert_dash_pattern(&pattern_str);
                            config.stroke_dash = ScalarOrArray::new_scalar(dash);
                        }
                    }
                }
            }
        } else {
            // Use default (solid)
            config.stroke_dash = ScalarOrArray::new_scalar(None);
        }

        let legend_group = make_line_legend(&config)?;
        let x = params.padding.left + params.plot_width + params.legend_margin;
        let y = params.padding.top + params.y_offset;
        Ok(Some(SceneGroup {
            origin: [x, y],
            marks: legend_group.marks,
            zindex: Some(10),
            ..Default::default()
        }))
    }

    /// Create a colorbar legend
    async fn create_colorbar_legend(
        &self,
        params: LegendParams<'_>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_guides::legend::colorbar::{
            ColorbarConfig, ColorbarOrientation, make_colorbar_marks,
        };

        // Create a ConfiguredScale from our Scale
        let legend_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;
        let configured_scale = legend_scale
            .create_configured_scale(100.0, params.plot_height)
            .await?;

        // Determine colorbar dimensions
        let colorbar_height = params
            .legend
            .gradient_length
            .unwrap_or((params.plot_height * 0.5) as f64)
            .min(200.0) as f32;
        let colorbar_width = params.legend.gradient_thickness.unwrap_or(10.0) as f32;

        let config = ColorbarConfig {
            orientation: ColorbarOrientation::Right,
            dimensions: [params.plot_width, params.plot_height],
            colorbar_width: Some(colorbar_width),
            colorbar_height: Some(colorbar_height),
            colorbar_margin: Some(params.legend_margin),
        };

        // Create the colorbar marks
        // Pass the plot area origin - the colorbar will position itself relative to this
        let plot_origin = [params.padding.left, params.padding.top];
        let title = params.legend.title.as_deref().unwrap_or("");

        let mut colorbar_group =
            make_colorbar_marks(&configured_scale, title, plot_origin, &config)?;

        // Adjust vertical position for multiple legends
        if params.y_offset > 0.0 {
            // Offset the colorbar marks vertically
            colorbar_group.origin[1] += params.y_offset;
        }

        // Set z-index
        colorbar_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(colorbar_group))
    }

    /// Extract domain values from a scale for legend entries
    async fn extract_scale_domain_values(
        &self,
        scale: &Scale,
    ) -> Result<Vec<datafusion_common::ScalarValue>, AvengerChartError> {
        use crate::scales::ScaleDefaultDomain;

        match &scale.get_domain().default_domain {
            ScaleDefaultDomain::Discrete(exprs) => {
                // Evaluate discrete expressions to get values
                let values = crate::utils::eval_to_scalars(exprs.clone(), None, None).await?;
                Ok(values)
            }
            ScaleDefaultDomain::Interval(start, end) => {
                // For interval domains, we'll sample some values
                // For now, just return the min/max
                let values = crate::utils::eval_to_scalars(
                    vec![start.clone(), end.as_ref().clone()],
                    None,
                    None,
                )
                .await?;
                Ok(values)
            }
            ScaleDefaultDomain::DomainExprs(_) => {
                // Domain not yet inferred from data
                // Return empty for now
                Ok(Vec::new())
            }
        }
    }

    /// Map values through a scale to get color arrays
    async fn map_values_through_scale(
        &self,
        scale: &Scale,
        values: &[datafusion_common::ScalarValue],
    ) -> Result<Vec<[f32; 4]>, AvengerChartError> {
        use datafusion::arrow::array::{Array, AsArray};
        use datafusion::arrow::datatypes::Float32Type;

        // Create a ConfiguredScale
        let configured_scale = scale.create_configured_scale(100.0, 100.0).await?;

        // Convert scalar values to an arrow array
        let domain_array = ScalarValue::iter_to_array(values.iter().cloned())?;

        // Apply the scale transformation
        let scaled_array = configured_scale.scale(&domain_array)?;

        // The scaled result for color scales should be a list array containing [f32; 4] arrays
        // Try to extract colors from the scaled result
        if let Some(list_array) = scaled_array.as_list_opt::<i32>() {
            let mut colors = Vec::new();
            for i in 0..list_array.len() {
                if list_array.is_null(i) {
                    colors.push([0.0, 0.0, 0.0, 0.0]);
                } else {
                    let color_array = list_array.value(i);
                    if let Some(float_array) = color_array.as_primitive_opt::<Float32Type>() {
                        if float_array.len() >= 4 {
                            colors.push([
                                float_array.value(0),
                                float_array.value(1),
                                float_array.value(2),
                                float_array.value(3),
                            ]);
                        } else {
                            colors.push([0.0, 0.0, 0.0, 1.0]);
                        }
                    } else {
                        colors.push([0.0, 0.0, 0.0, 1.0]);
                    }
                }
            }
            Ok(colors)
        } else {
            // The scale might return colors directly without wrapping in a list array
            // This is expected for ordinal scales - just use the coercer to parse
            use avenger_scales::scales::coerce::Coercer;
            let coercer = Coercer::default();

            match coercer.to_color(&scaled_array, None) {
                Ok(color_or_gradient) => {
                    // Extract colors from the result and convert to [f32; 4]
                    let color_or_gradients = color_or_gradient.as_vec(scaled_array.len(), None);
                    let colors: Vec<[f32; 4]> = color_or_gradients
                        .into_iter()
                        .map(|cog| match cog {
                            avenger_common::types::ColorOrGradient::Color(c) => c,
                            avenger_common::types::ColorOrGradient::GradientIndex(_) => {
                                [0.0, 0.0, 0.0, 1.0]
                            }
                        })
                        .collect();
                    Ok(colors)
                }
                Err(_) => {
                    // If we can't parse as colors, return an error
                    Err(AvengerChartError::InternalError(
                        "Scale did not return valid color data".to_string(),
                    ))
                }
            }
        }
    }

    /// Extract shape names from a scale's range
    async fn extract_shape_range_from_scale(
        &self,
        scale: &Scale,
    ) -> Result<Vec<String>, AvengerChartError> {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        // // Check if the scale has an explicit range
        // if scale.has_explicit_range() {
        //     // Try to extract shape names from the range
        //     if let ScaleRange::Enum(values) = scale.get_range() {
        //         let mut shape_names = Vec::new();
        //         for value in values {
        //             if let ScalarValue::Utf8(Some(s)) = value {
        //                 shape_names.push(s.clone());
        //             }
        //         }
        //         if !shape_names.is_empty() {
        //             return Ok(shape_names);
        //         }
        //     }
        // }
        // Try to extract shape names from the range
        if let ScaleRange::Enum(values) = scale.get_range() {
            let mut shape_names = Vec::new();
            for value in values {
                if let ScalarValue::Utf8(Some(s)) = value {
                    shape_names.push(s.clone());
                }
            }
            if !shape_names.is_empty() {
                return Ok(shape_names);
            }
        }
        Err(AvengerChartError::InternalError(format!(
            "Shape scale range is not supported for legend: {:?}",
            scale
        )))

        // // Fall back to default shapes
        // Ok(crate::scales::shape_defaults::DEFAULT_SHAPES
        //     .iter()
        //     .map(|&s| s.to_string())
        //     .collect())
    }

    /// Map values through a scale to get numeric values
    async fn map_values_through_scale_numeric(
        &self,
        scale: &Scale,
        values: &[datafusion_common::ScalarValue],
    ) -> Result<Vec<f32>, AvengerChartError> {
        use datafusion::arrow::array::{Array, AsArray};
        use datafusion::arrow::compute::cast;
        use datafusion::arrow::datatypes::{DataType, Float32Type};

        // Create a ConfiguredScale
        // For non-position scales like size, we pass NaN to indicate the range should not be overridden
        let configured_scale = scale.create_configured_scale(f32::NAN, f32::NAN).await?;

        // Convert scalar values to an arrow array
        let domain_array = ScalarValue::iter_to_array(values.iter().cloned())?;

        // Apply the scale transformation
        let scaled_array = configured_scale.scale(&domain_array)?;

        // Cast to Float32Array - this handles all numeric types and dictionary arrays with numeric values
        let float_array = cast(&scaled_array, &DataType::Float32).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to cast scale result to Float32: {}",
                e
            ))
        })?;

        // Extract the values
        if let Some(float_array) = float_array.as_primitive_opt::<Float32Type>() {
            let mut result = Vec::new();
            for i in 0..float_array.len() {
                if float_array.is_null(i) {
                    result.push(0.0);
                } else {
                    result.push(float_array.value(i));
                }
            }
            Ok(result)
        } else {
            // This shouldn't happen after a successful cast
            Err(AvengerChartError::InternalError(
                "Cast to Float32 succeeded but result is not a Float32Array".to_string(),
            ))
        }
    }

    /// Create title mark if configured
    fn create_title(
        &self,
        _total_width: f32,
        _padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // TODO: Create title text mark if plot.title is set
        // When implemented, wrap in a group with zindex: Some(20)
        Ok(Vec::new())
    }

    /// Infer and apply domain for a scale if not explicitly set
    async fn infer_scale_domain(
        &self,
        scale: &mut Scale,
        name: &str,
        plot_area_width: f32,
        plot_area_height: f32,
        all_scales: &std::collections::HashMap<String, Scale>,
    ) -> Result<(), AvengerChartError> {
        if !scale.has_explicit_domain() {
            // Only use radius-aware gathering for linear positional scales
            if scale.get_scale_impl().scale_type() == "linear"
                && matches!(name, "x" | "y" | "x2" | "y2")
            {
                // Use the new method that gathers radius information
                let data_expressions_with_radius = self
                    .plot
                    .gather_scale_domain_expressions_with_radius(name, all_scales);

                // Check if any expressions actually have radius
                let has_radius = data_expressions_with_radius
                    .iter()
                    .any(|(_, _, radius)| radius.is_some());

                if !data_expressions_with_radius.is_empty() && has_radius {
                    // Use the internal method that accepts radius
                    *scale = scale
                        .clone()
                        .domain_data_fields_with_radius_internal(data_expressions_with_radius);
                } else if !data_expressions_with_radius.is_empty() {
                    // Convert to standard expressions (without radius)
                    let data_expressions: Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)> =
                        data_expressions_with_radius
                            .into_iter()
                            .map(|(df, expr, _)| (df, expr))
                            .collect();
                    *scale = scale.clone().domain_data_fields_internal(data_expressions);
                }
            } else {
                // Use standard domain gathering for non-linear scales
                let data_expressions = self.plot.gather_scale_domain_expressions(name);
                if !data_expressions.is_empty() {
                    *scale = scale.clone().domain_data_fields_internal(data_expressions);
                }
            }
        }

        // Infer domain from data if needed
        if matches!(
            &scale.domain.default_domain,
            crate::scales::ScaleDefaultDomain::DomainExprs(_)
        ) {
            // Compute range hint for positional scales
            let range_hint = match name {
                "x" => Some((0.0, plot_area_width as f64)),
                "y" => Some((plot_area_height as f64, 0.0)), // Y is flipped
                _ => None,
            };

            *scale = scale.clone().infer_domain_from_data(range_hint).await?;
        }

        Ok(())
    }

    /// Process a scale with common workflow
    async fn process_scale<F>(
        &self,
        scale: &mut Scale,
        name: &str,
        plot_area_width: f32,
        plot_area_height: f32,
        all_scales: &std::collections::HashMap<String, Scale>,
        apply_scale_specific: F,
    ) -> Result<(), AvengerChartError>
    where
        F: FnOnce(&mut Scale),
    {
        let mut scale_copy = scale.clone();

        // Apply domain inference
        self.infer_scale_domain(
            &mut scale_copy,
            name,
            plot_area_width,
            plot_area_height,
            all_scales,
        )
        .await?;

        // Apply default range
        self.plot.apply_default_range(
            &mut scale_copy,
            name,
            plot_area_width as f64,
            plot_area_height as f64,
        );

        // Apply scale-specific logic
        apply_scale_specific(&mut scale_copy);

        // Normalize the scale
        scale_copy = scale_copy
            .normalize_domain(plot_area_width, plot_area_height)
            .await?;

        // Update the scale
        *scale = scale_copy;

        Ok(())
    }
}

/// Extension trait for Canvas to render Plot objects
#[allow(async_fn_in_trait)]
pub trait CanvasExt {
    /// Render a plot to this canvas
    async fn render_plot<C: CoordinateSystem>(
        &mut self,
        plot: &Plot<C>,
    ) -> Result<(), AvengerChartError>;
}

// Implement CanvasExt for PngCanvas
impl CanvasExt for PngCanvas {
    async fn render_plot<C: CoordinateSystem>(
        &mut self,
        plot: &Plot<C>,
    ) -> Result<(), AvengerChartError> {
        // Create a renderer for the plot
        let renderer = PlotRenderer::new(plot);

        // Just call render directly since we're already async
        let render_result = renderer.render().await?;

        // Set the entire scene graph at once (handles zindex sorting)
        self.set_scene(&render_result.scene_graph)?;

        Ok(())
    }
}
