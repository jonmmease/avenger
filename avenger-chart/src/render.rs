//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::axis::AxisTrait;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::PlotTrait;
use crate::marks::MarkConfig;
use crate::plot::Plot;
use avenger_common::types::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::array::{Array, Float32Array, Float64Array, StringArray};
use datafusion::logical_expr::lit;
use std::collections::HashMap;
use std::sync::Arc;

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

impl<'a, C: CoordinateSystem> PlotRenderer<'a, C> {
    pub fn new(plot: &'a Plot<C>) -> Self {
        Self { plot }
    }

    /// Render the plot to a scene graph
    pub async fn render(&self) -> Result<RenderResult, AvengerChartError> {
        // Get plot dimensions from preferred size or default
        let (width, height) = self.plot.get_preferred_size().unwrap_or((800.0, 600.0));

        // Calculate padding for axes, title, etc.
        let padding = self.plot.measure_padding(width as f32, height as f32);

        // Calculate plot area (inside padding)
        let plot_area_x = padding.left;
        let plot_area_y = padding.top;
        let plot_area_width = width as f32 - padding.left - padding.right;
        let plot_area_height = height as f32 - padding.top - padding.bottom;

        // Create scale registry with default ranges applied
        let mut scales = crate::scales::ScaleRegistry::new();

        // Copy scales from plot and apply default ranges
        for (name, scale) in &self.plot.scales {
            let mut scale_copy = scale.clone();
            self.plot.apply_default_range(
                &mut scale_copy,
                name,
                plot_area_width as f64,
                plot_area_height as f64,
            );
            scales.add(name.clone(), scale_copy);
        }

        // Calculate the actual data bounds based on scale ranges
        // The scales may have padding, so the data area is slightly smaller than plot area
        let (clip_y_offset, clip_height) = if let Some(y_scale) = scales.get("y") {
            // Get the actual y range from the scale domain
            let configured_y_scale =
                self.create_configured_scale(y_scale, "y", plot_area_width, plot_area_height)?;

            // Get domain bounds from the scale
            let domain = configured_y_scale.domain();
            if domain.len() >= 2 {
                // Scale the domain min and max to screen coordinates
                let first = domain.slice(0, 1);
                let last = domain.slice(domain.len() - 1, 1);
                let domain_refs: &[&dyn datafusion::arrow::array::Array] = &[&*first, &*last];
                let domain_array = datafusion::arrow::compute::concat(domain_refs)?;
                let scaled_domain = configured_y_scale
                    .scale_to_numeric(&domain_array)?
                    .as_vec(2, None);

                if scaled_domain.len() >= 2 {
                    let y_min = scaled_domain[0];
                    let y_max = scaled_domain[1];
                    let clip_height = (y_min - y_max).abs();
                    (y_max.min(y_min), clip_height)
                } else {
                    (0.0, plot_area_height)
                }
            } else {
                (0.0, plot_area_height)
            }
        } else {
            (0.0, plot_area_height)
        };

        // Process each mark
        let mut mark_groups = Vec::new();

        // Debug rects removed for cleaner output

        for mark_config in &self.plot.marks {
            let mark_group = self
                .render_mark(mark_config, &scales, plot_area_width, plot_area_height)
                .await?;
            mark_groups.push(mark_group);
        }

        // Create axes
        let axis_marks = self.create_axes(&scales, plot_area_width, plot_area_height, &padding)?;

        // Create title if present
        let title_marks = self.create_title(width as f32, &padding)?;

        // Compose all elements into a scene graph
        let mut all_marks = Vec::new();

        // Create a group for data marks with adjusted clipping
        // Note: clip coordinates are relative to the group's origin
        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip: Clip::Rect {
                x: plot_area_x,
                y: clip_y_offset + plot_area_y,
                width: plot_area_width,
                height: clip_height,
            },
            ..Default::default()
        };

        // Add marks in proper z-order:
        // 1. Clipped data marks (background)
        all_marks.push(SceneMark::Group(data_marks_group));

        // 2. Axes (can overflow the plot area)
        all_marks.extend(axis_marks);

        // 3. Title (can overflow, rendered on top)
        all_marks.extend(title_marks);

        let scene_graph = SceneGraph {
            marks: all_marks,
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

    /// Render a single mark to scene marks
    async fn render_mark(
        &self,
        mark_config: &MarkConfig<C>,
        scales: &crate::scales::ScaleRegistry,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<SceneMark, AvengerChartError> {
        match mark_config.mark_type.as_str() {
            "rect" => {
                self.render_rect_mark(mark_config, scales, plot_width, plot_height)
                    .await
            }
            "line" => todo!("Implement line mark rendering"),
            "symbol" => todo!("Implement symbol mark rendering"),
            _ => Err(AvengerChartError::MarkTypeLookupError(
                mark_config.mark_type.clone(),
            )),
        }
    }

    /// Render a rect mark (for bar charts, heatmaps, etc.)
    async fn render_rect_mark(
        &self,
        mark_config: &MarkConfig<C>,
        scales: &crate::scales::ScaleRegistry,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<SceneMark, AvengerChartError> {
        // Get the data - either from mark or inherit from plot
        let df = match &mark_config.data_source {
            crate::marks::DataSource::Explicit => mark_config.data.dataframe(),
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
        let encodings = mark_config.data.encodings();

        // Extract expressions and band parameters for rect channels
        let x_channel = encodings.get("x");
        let x2_channel = encodings.get("x2");
        let y_channel = encodings.get("y");
        let y2_channel = encodings.get("y2");
        let x_expr = x_channel.map(|ch| &ch.expr);
        let x2_expr = x2_channel.map(|ch| &ch.expr);
        let x2_band = x2_channel.and_then(|ch| ch.band);
        let y_expr = y_channel.map(|ch| &ch.expr);
        let y2_expr = y2_channel.map(|ch| &ch.expr);
        let fill_expr = encodings.get("fill").map(|ch| &ch.expr);
        let stroke_expr = encodings.get("stroke").map(|ch| &ch.expr);
        let stroke_width_expr = encodings.get("stroke_width").map(|ch| &ch.expr);

        // Build expressions with scale transformations applied
        let mut select_exprs = vec![];
        let mut channel_names = vec![];

        // For position channels, apply scales if available
        if let Some(expr) = x_expr {
            let scaled_expr = if let Some(x_scale) = scales.get("x") {
                x_scale.to_expr(expr.clone())?
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("x"));
            channel_names.push("x");
        }

        if let Some(expr) = x2_expr {
            let scaled_expr = if let Some(x_scale) = scales.get("x") {
                // For x2 with band scale, we need to handle band parameter
                if x_scale.get_scale_impl().scale_type() == "band" {
                    // Add band parameter to scale options for x2
                    let mut x2_scale = x_scale.clone();
                    if let Some(band) = x2_band {
                        x2_scale = x2_scale.option("band", lit(band));
                    } else {
                        x2_scale = x2_scale.option("band", lit(1.0));
                    }
                    x2_scale.to_expr(expr.clone())?
                } else {
                    x_scale.to_expr(expr.clone())?
                }
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("x2"));
            channel_names.push("x2");
        }

        if let Some(expr) = y_expr {
            let scaled_expr = if let Some(y_scale) = scales.get("y") {
                y_scale.to_expr(expr.clone())?
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("y"));
            channel_names.push("y");
        }

        if let Some(expr) = y2_expr {
            let scaled_expr = if let Some(y_scale) = scales.get("y") {
                y_scale.to_expr(expr.clone())?
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("y2"));
            channel_names.push("y2");
        }

        // Color and size channels typically don't use plot-level scales in our current design
        if let Some(expr) = fill_expr {
            select_exprs.push(expr.clone().alias("fill"));
            channel_names.push("fill");
        }
        if let Some(expr) = stroke_expr {
            select_exprs.push(expr.clone().alias("stroke"));
            channel_names.push("stroke");
        }
        if let Some(expr) = stroke_width_expr {
            select_exprs.push(expr.clone().alias("stroke_width"));
            channel_names.push("stroke_width");
        }

        // Execute the query to get the data
        let result_df = df.clone().select(select_exprs)?;
        let batches = result_df.collect().await?;

        if batches.is_empty() {
            // Return empty rect mark
            return Ok(SceneMark::Rect(SceneRectMark {
                name: "rect".to_string(),
                clip: true,
                len: 0,
                gradients: vec![],
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: None,
                height: None,
                x2: None,
                y2: None,
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                corner_radius: ScalarOrArray::new_scalar(0.0),
                indices: None,
                zindex: mark_config.zindex,
            }));
        }

        // Get the first batch (assuming single batch for now)
        let batch = &batches[0];
        let num_rows = batch.num_rows();

        // Extract scaled values from the DataFrame results
        // The scale UDFs have already transformed the values to pixel coordinates
        let mut x_values: Vec<f32> = vec![];
        let mut x2_values: Vec<f32> = vec![];
        let mut y_values: Vec<f32> = vec![];
        let mut y2_values: Vec<f32> = vec![];

        // Extract x values if present
        if channel_names.contains(&"x") {
            let col_idx = channel_names.iter().position(|&c| c == "x").unwrap();
            let x_array = batch.column(col_idx);
            if let Some(float_array) = x_array.as_any().downcast_ref::<Float32Array>() {
                x_values = (0..num_rows).map(|i| float_array.value(i)).collect();
            } else if let Some(float64_array) = x_array.as_any().downcast_ref::<Float64Array>() {
                x_values = (0..num_rows)
                    .map(|i| float64_array.value(i) as f32)
                    .collect();
            }
        }

        // Extract x2 values if present
        if channel_names.contains(&"x2") {
            let col_idx = channel_names.iter().position(|&c| c == "x2").unwrap();
            let x2_array = batch.column(col_idx);
            if let Some(float_array) = x2_array.as_any().downcast_ref::<Float32Array>() {
                x2_values = (0..num_rows).map(|i| float_array.value(i)).collect();
            } else if let Some(float64_array) = x2_array.as_any().downcast_ref::<Float64Array>() {
                x2_values = (0..num_rows)
                    .map(|i| float64_array.value(i) as f32)
                    .collect();
            }
        }

        // Extract y values if present
        if channel_names.contains(&"y") {
            let col_idx = channel_names.iter().position(|&c| c == "y").unwrap();
            let y_array = batch.column(col_idx);
            if let Some(float_array) = y_array.as_any().downcast_ref::<Float32Array>() {
                y_values = (0..num_rows).map(|i| float_array.value(i)).collect();
            } else if let Some(float64_array) = y_array.as_any().downcast_ref::<Float64Array>() {
                y_values = (0..num_rows)
                    .map(|i| float64_array.value(i) as f32)
                    .collect();
            }
        }

        // Extract y2 values if present
        if channel_names.contains(&"y2") {
            let col_idx = channel_names.iter().position(|&c| c == "y2").unwrap();
            let y2_array = batch.column(col_idx);
            if let Some(float_array) = y2_array.as_any().downcast_ref::<Float32Array>() {
                y2_values = (0..num_rows).map(|i| float_array.value(i)).collect();
            } else if let Some(float64_array) = y2_array.as_any().downcast_ref::<Float64Array>() {
                y2_values = (0..num_rows)
                    .map(|i| float64_array.value(i) as f32)
                    .collect();
            }
        }

        // Get fill color
        let fill_value = if channel_names.contains(&"fill") {
            let col_idx = channel_names.iter().position(|&c| c == "fill").unwrap();
            let fill_array = batch.column(col_idx);
            if let Some(str_array) = fill_array.as_any().downcast_ref::<StringArray>() {
                if str_array.len() == 1 && !str_array.is_null(0) {
                    let color_str = str_array.value(0);
                    if let Ok(color) = self.parse_color(color_str) {
                        ScalarOrArray::new_scalar(ColorOrGradient::Color(color))
                    } else {
                        ScalarOrArray::new_scalar(ColorOrGradient::Color([0.27, 0.51, 0.71, 1.0])) // Default steel blue
                    }
                } else {
                    ScalarOrArray::new_scalar(ColorOrGradient::Color([0.27, 0.51, 0.71, 1.0]))
                }
            } else {
                ScalarOrArray::new_scalar(ColorOrGradient::Color([0.27, 0.51, 0.71, 1.0]))
            }
        } else {
            ScalarOrArray::new_scalar(ColorOrGradient::Color([0.27, 0.51, 0.71, 1.0]))
        };

        // Get stroke color
        let stroke_value = if channel_names.contains(&"stroke") {
            let col_idx = channel_names.iter().position(|&c| c == "stroke").unwrap();
            let stroke_array = batch.column(col_idx);
            if let Some(str_array) = stroke_array.as_any().downcast_ref::<StringArray>() {
                if str_array.len() == 1 && !str_array.is_null(0) {
                    let color_str = str_array.value(0);
                    if let Ok(color) = self.parse_color(color_str) {
                        ScalarOrArray::new_scalar(ColorOrGradient::Color(color))
                    } else {
                        ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])) // Default black
                    }
                } else {
                    ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]))
                }
            } else {
                ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]))
            }
        } else {
            ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]))
        };

        // Get stroke width
        let stroke_width_value = if channel_names.contains(&"stroke_width") {
            let col_idx = channel_names
                .iter()
                .position(|&c| c == "stroke_width")
                .unwrap();
            let sw_array = batch.column(col_idx);
            if let Some(f64_array) = sw_array.as_any().downcast_ref::<Float64Array>() {
                if f64_array.len() == 1 && !f64_array.is_null(0) {
                    ScalarOrArray::new_scalar(f64_array.value(0) as f32)
                } else {
                    ScalarOrArray::new_scalar(1.0)
                }
            } else {
                ScalarOrArray::new_scalar(1.0)
            }
        } else {
            ScalarOrArray::new_scalar(1.0)
        };

        // Create SceneRectMark
        let rect_mark = SceneRectMark {
            name: "rect".to_string(),
            clip: true,
            len: num_rows as u32,
            gradients: vec![],
            x: ScalarOrArray::new_array(x_values),
            y: ScalarOrArray::new_array(y_values),
            width: None,
            height: None,
            x2: Some(ScalarOrArray::new_array(x2_values)),
            y2: Some(ScalarOrArray::new_array(y2_values)),
            fill: fill_value,
            stroke: stroke_value,
            stroke_width: stroke_width_value,
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: mark_config.zindex,
        };

        Ok(SceneMark::Rect(rect_mark))
    }

    /// Parse a color string to RGBA values
    fn parse_color(&self, color_str: &str) -> Result<[f32; 4], AvengerChartError> {
        // Simple hex color parser
        if color_str.starts_with('#') && color_str.len() == 7 {
            let r = u8::from_str_radix(&color_str[1..3], 16).map_err(|_| {
                AvengerChartError::InvalidArgument(format!("Invalid color: {}", color_str))
            })?;
            let g = u8::from_str_radix(&color_str[3..5], 16).map_err(|_| {
                AvengerChartError::InvalidArgument(format!("Invalid color: {}", color_str))
            })?;
            let b = u8::from_str_radix(&color_str[5..7], 16).map_err(|_| {
                AvengerChartError::InvalidArgument(format!("Invalid color: {}", color_str))
            })?;

            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        } else {
            Err(AvengerChartError::InvalidArgument(format!(
                "Unsupported color format: {}",
                color_str
            )))
        }
    }

    /// Create axis marks based on configured axes
    fn create_axes(
        &self,
        scales: &crate::scales::ScaleRegistry,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::layout::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut axis_marks = Vec::new();

        // For Cartesian coordinates, we can downcast and access the axes
        if let Some(cartesian_axes) = self
            .plot
            .axes
            .iter()
            .map(|(ch, axis)| {
                // Try to downcast to CartesianAxis
                axis.as_any()
                    .downcast_ref::<crate::axis::CartesianAxis>()
                    .map(|a| (ch, a))
            })
            .collect::<Option<Vec<_>>>()
        {
            // Process each Cartesian axis
            for (channel, axis) in cartesian_axes {
                // Processing axis for channel

                // Skip invisible axes
                if !axis.visible {
                    // Skipping invisible axis
                    continue;
                }

                // Get the scale for this axis
                let scale = scales.get(channel).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "No scale found for axis channel: {}",
                        channel
                    ))
                })?;

                // Determine axis position
                let position = axis.position.unwrap_or_else(|| {
                    // Default positions based on channel name
                    match channel.as_ref() {
                        "x" => crate::axis::AxisPosition::Bottom,
                        "y" => crate::axis::AxisPosition::Left,
                        _ => crate::axis::AxisPosition::Bottom,
                    }
                });

                // Convert position to orientation
                let orientation = match position {
                    crate::axis::AxisPosition::Top => AxisOrientation::Top,
                    crate::axis::AxisPosition::Bottom => AxisOrientation::Bottom,
                    crate::axis::AxisPosition::Left => AxisOrientation::Left,
                    crate::axis::AxisPosition::Right => AxisOrientation::Right,
                };

                // Calculate axis origin based on position
                // For axes, dimensions represent the axis space, not the plot area
                let (axis_origin, dimensions) = match position {
                    crate::axis::AxisPosition::Bottom => {
                        // Bottom axis origin calculation
                        (
                            [padding.left, padding.top + plot_height],
                            [plot_width, 0.0], // Bottom axis has 0 height
                        )
                    }
                    crate::axis::AxisPosition::Top => (
                        [padding.left, padding.top],
                        [plot_width, 0.0], // Top axis has 0 height
                    ),
                    crate::axis::AxisPosition::Left => (
                        [padding.left, padding.top],
                        [0.0, plot_height], // Left axis has 0 width
                    ),
                    crate::axis::AxisPosition::Right => (
                        [padding.left + plot_width, padding.top],
                        [0.0, plot_height], // Right axis has 0 width
                    ),
                };

                // Create axis config
                let axis_config = AxisConfig {
                    orientation,
                    dimensions,
                    grid: axis.grid,
                };

                // Create configured scale for avenger-guides
                let configured_scale =
                    self.create_configured_scale(scale, channel, plot_width, plot_height)?;

                // Generate axis marks based on scale type
                let scale_type = scale.get_scale_impl().scale_type();
                // Scale type determined

                let axis_group = match scale_type {
                    "band" => {
                        // Creating band axis
                        let result = make_band_axis_marks(
                            &configured_scale,
                            axis.title.as_deref().unwrap_or(""),
                            axis_origin,
                            &axis_config,
                        );
                        match result {
                            Ok(group) => {
                                // Band axis created successfully
                                group
                            }
                            Err(e) => {
                                // Error creating band axis
                                return Err(e.into());
                            }
                        }
                    }
                    _ => {
                        // Default to numeric axis for linear and other continuous scales
                        make_numeric_axis_marks(
                            &configured_scale,
                            axis.title.as_deref().unwrap_or(""),
                            axis_origin,
                            &axis_config,
                        )?
                    }
                };

                axis_marks.push(SceneMark::Group(axis_group));
            }
        }

        Ok(axis_marks)
    }

    /// Create a ConfiguredScale for avenger-guides
    fn create_configured_scale(
        &self,
        scale: &crate::scales::Scale,
        channel: &str,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};

        // Get domain and range from scale
        let domain = match &scale.domain.default_domain {
            crate::scales::ScaleDefaultDomain::Interval(start, end) => {
                // Try to evaluate literal expressions
                if let (
                    datafusion::logical_expr::Expr::Literal(start_val, _),
                    datafusion::logical_expr::Expr::Literal(end_val, _),
                ) = (start, end.as_ref())
                {
                    // Convert literals to f32 values
                    let start_f32 = match start_val {
                        datafusion_common::ScalarValue::Float64(Some(v)) => *v as f32,
                        datafusion_common::ScalarValue::Float32(Some(v)) => *v,
                        datafusion_common::ScalarValue::Int64(Some(v)) => *v as f32,
                        datafusion_common::ScalarValue::Int32(Some(v)) => *v as f32,
                        _ => {
                            return Err(AvengerChartError::InternalError(
                                "Scale domain must be numeric literals or expressions".to_string(),
                            ));
                        }
                    };
                    let end_f32 = match end_val {
                        datafusion_common::ScalarValue::Float64(Some(v)) => *v as f32,
                        datafusion_common::ScalarValue::Float32(Some(v)) => *v,
                        datafusion_common::ScalarValue::Int64(Some(v)) => *v as f32,
                        datafusion_common::ScalarValue::Int32(Some(v)) => *v as f32,
                        _ => {
                            return Err(AvengerChartError::InternalError(
                                "Scale domain must be numeric literals or expressions".to_string(),
                            ));
                        }
                    };
                    Arc::new(Float32Array::from(vec![start_f32, end_f32]))
                        as datafusion::arrow::array::ArrayRef
                } else {
                    // TODO: Evaluate expressions against data
                    return Err(AvengerChartError::InternalError(
                        "Scale domain expressions not yet implemented. Please use literal values."
                            .to_string(),
                    ));
                }
            }
            crate::scales::ScaleDefaultDomain::Discrete(values) => {
                // For discrete domains, evaluate the expressions to get string values
                // Extract string literals from the expressions
                let mut strings = Vec::new();
                for expr in values {
                    if let datafusion::logical_expr::Expr::Literal(
                        datafusion_common::ScalarValue::Utf8(Some(s)),
                        _,
                    ) = expr
                    {
                        strings.push(s.clone());
                    }
                }
                Arc::new(StringArray::from(strings)) as datafusion::arrow::array::ArrayRef
            }
            _ => {
                // Return error for undefined domain
                return Err(AvengerChartError::InternalError(
                    "Scale domain must be explicitly set".to_string(),
                ));
            }
        };

        // Set range based on channel and plot dimensions
        let range = match channel {
            "x" => Arc::new(Float32Array::from(vec![0.0, plot_width]))
                as datafusion::arrow::array::ArrayRef,
            "y" => Arc::new(Float32Array::from(vec![plot_height, 0.0]))
                as datafusion::arrow::array::ArrayRef, // Inverted for screen coords
            _ => {
                Arc::new(Float32Array::from(vec![0.0, 100.0])) as datafusion::arrow::array::ArrayRef
            }
        };

        // Configure band scale options if this is a band scale
        let mut options = HashMap::new();
        if scale.get_scale_impl().scale_type() == "band" {
            // Set default band scale options
            options.insert(
                "padding_inner".to_string(),
                avenger_scales::scalar::Scalar::from_f32(0.1),
            );
            options.insert(
                "padding_outer".to_string(),
                avenger_scales::scalar::Scalar::from_f32(0.1),
            ); // Increase outer padding
            options.insert(
                "align".to_string(),
                avenger_scales::scalar::Scalar::from_f32(0.5),
            );
        }

        let config = ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        };

        Ok(ConfiguredScale {
            scale_impl: scale.get_scale_impl().clone(),
            config,
        })
    }

    /// Create title mark if configured
    fn create_title(
        &self,
        _total_width: f32,
        _padding: &crate::layout::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // TODO: Create title text mark if plot.title is set
        Ok(Vec::new())
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

        // Add scene graph to canvas
        self.clear_mark_renderer();

        // Recursively add marks from the scene graph
        add_marks_to_canvas(self, &render_result.scene_graph.marks, [0.0, 0.0])?;

        Ok(())
    }
}

/// Helper function to recursively add marks to canvas
fn add_marks_to_canvas(
    canvas: &mut dyn Canvas,
    marks: &[SceneMark],
    origin: [f32; 2],
) -> Result<(), AvengerChartError> {
    add_marks_to_canvas_with_clip(
        canvas,
        marks,
        origin,
        &avenger_scenegraph::marks::group::Clip::None,
    )
}

/// Helper function to recursively add marks to canvas with inherited clip
fn add_marks_to_canvas_with_clip(
    canvas: &mut dyn Canvas,
    marks: &[SceneMark],
    origin: [f32; 2],
    parent_clip: &avenger_scenegraph::marks::group::Clip,
) -> Result<(), AvengerChartError> {
    for mark in marks {
        match mark {
            SceneMark::Arc(arc) => canvas.add_arc_mark(arc, origin, parent_clip)?,
            SceneMark::Area(area) => canvas.add_area_mark(area, origin, parent_clip)?,
            SceneMark::Group(group) => {
                let group_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
                // Groups define their own clip that applies to their children
                add_marks_to_canvas_with_clip(canvas, &group.marks, group_origin, &group.clip)?;
            }
            SceneMark::Image(image) => canvas.add_image_mark(image, origin, parent_clip)?,
            SceneMark::Line(line) => canvas.add_line_mark(line, origin, parent_clip)?,
            SceneMark::Path(path) => canvas.add_path_mark(path, origin, parent_clip)?,
            SceneMark::Rect(rect) => canvas.add_rect_mark(rect, origin, parent_clip)?,
            SceneMark::Rule(rule) => canvas.add_rule_mark(rule, origin, parent_clip)?,
            SceneMark::Symbol(symbol) => canvas.add_symbol_mark(symbol, origin, parent_clip)?,
            SceneMark::Text(text) => canvas.add_text_mark(text, origin, parent_clip)?,
            SceneMark::Trail(trail) => canvas.add_trail_mark(trail, origin, parent_clip)?,
        }
    }
    Ok(())
}
