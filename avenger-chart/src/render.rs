//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::axis::AxisTrait;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::PlotTrait;
use crate::marks::{ChannelValue, MarkConfig};
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
use datafusion::arrow::array::{Array, Float32Array, Float64Array};
use datafusion::logical_expr::lit;
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

        // First, collect all channels that need scales
        let channels_with_scales = self.plot.collect_channels_needing_scales();

        // Add any missing scales with appropriate defaults
        let mut all_scales = self.plot.scales.clone();
        for channel in channels_with_scales.difference(&all_scales.keys().cloned().collect()) {
            if let Some(default_scale) = self.plot.create_default_scale_for_channel(channel).await {
                all_scales.insert(channel.clone(), default_scale);
            }
        }

        // Process scales: infer domains, apply defaults, and normalize
        for (name, scale) in &all_scales {
            let mut scale_copy = scale.clone();

            // Apply default domain if needed
            if !scale_copy.has_explicit_domain() {
                let data_expressions = self.plot.gather_scale_domain_expressions(name);
                if !data_expressions.is_empty() {
                    // Convert expressions to data fields
                    let mut data_fields = Vec::new();

                    for (df, expr) in data_expressions {
                        match &expr {
                            datafusion::logical_expr::Expr::Column(col) => {
                                data_fields.push((df, col.name.clone()));
                            }
                            _ => {
                                // For complex expressions, create a derived column
                                let expr_col_name = format!("__scale_domain_expr_{}", name);

                                if let Ok(df_with_expr) = df
                                    .as_ref()
                                    .clone()
                                    .with_column(&expr_col_name, expr.clone())
                                {
                                    data_fields.push((Arc::new(df_with_expr), expr_col_name));
                                }
                            }
                        }
                    }

                    if !data_fields.is_empty() {
                        scale_copy = scale_copy.domain_data_fields(data_fields);
                    }
                }
            }

            // Infer domain from data if needed
            if matches!(
                &scale_copy.domain.default_domain,
                crate::scales::ScaleDefaultDomain::DataFields(_)
            ) {
                scale_copy = scale_copy.infer_domain_from_data().await?;
            }

            // Apply default range
            self.plot.apply_default_range(
                &mut scale_copy,
                name,
                plot_area_width as f64,
                plot_area_height as f64,
            );

            // Normalize the scale to apply zero and nice transformations
            // This ensures data and axes use the same normalized domain
            scale_copy = scale_copy.normalize_domain(plot_area_width, plot_area_height)?;

            scales.add(name.clone(), scale_copy);
        }

        // Use the full plot area for clipping
        // Marks should be clipped to the plot area bounds, not the data bounds
        let (clip_y_offset, clip_height) = (0.0, plot_area_height);

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
        let x_expr = x_channel.map(|ch| ch.expr());
        let x2_expr = x2_channel.map(|ch| ch.expr());
        let x2_band = x2_channel.and_then(|ch| match ch {
            ChannelValue::Scaled { band, .. } => *band,
            _ => None,
        });
        let y_expr = y_channel.map(|ch| ch.expr());
        let y2_expr = y2_channel.map(|ch| ch.expr());
        let fill_expr = encodings.get("fill").map(|ch| ch.expr());
        let stroke_expr = encodings.get("stroke").map(|ch| ch.expr());
        let stroke_width_expr = encodings.get("stroke_width").map(|ch| ch.expr());

        // Build expressions with scale transformations applied
        let mut select_exprs = vec![];
        let mut channel_names = vec![];

        // For position channels, apply scales if requested
        if let Some(expr) = x_expr {
            let scaled_expr = if let Some(channel) = x_channel {
                if let Some(scale_name) = channel.scale_name("x") {
                    if let Some(x_scale) = scales.get(&scale_name) {
                        x_scale.to_expr(expr.clone())?
                    } else {
                        expr.clone()
                    }
                } else {
                    // Identity - use expression as-is
                    expr.clone()
                }
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("x"));
            channel_names.push("x");
        }

        if let Some(expr) = x2_expr {
            let scaled_expr = if let Some(channel) = x2_channel {
                if let Some(scale_name) = channel.scale_name("x2") {
                    if let Some(x_scale) = scales.get(&scale_name) {
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
                    }
                } else {
                    // Identity - use expression as-is
                    expr.clone()
                }
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("x2"));
            channel_names.push("x2");
        }

        if let Some(expr) = y_expr {
            let scaled_expr = if let Some(channel) = y_channel {
                if let Some(scale_name) = channel.scale_name("y") {
                    if let Some(y_scale) = scales.get(&scale_name) {
                        y_scale.to_expr(expr.clone())?
                    } else {
                        expr.clone()
                    }
                } else {
                    // Identity - use expression as-is
                    expr.clone()
                }
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("y"));
            channel_names.push("y");
        }

        if let Some(expr) = y2_expr {
            let scaled_expr = if let Some(channel) = y2_channel {
                if let Some(scale_name) = channel.scale_name("y2") {
                    if let Some(y_scale) = scales.get(&scale_name) {
                        y_scale.to_expr(expr.clone())?
                    } else {
                        expr.clone()
                    }
                } else {
                    // Identity - use expression as-is
                    expr.clone()
                }
            } else {
                expr.clone()
            };
            select_exprs.push(scaled_expr.alias("y2"));
            channel_names.push("y2");
        }

        // Apply scales to color and size channels if available
        if let Some(expr) = fill_expr {
            let fill_channel = encodings.get("fill").unwrap();
            if let Some(scale_name) = fill_channel.scale_name("fill") {
                if let Some(fill_scale) = scales.get(&scale_name) {
                    let scaled_expr = fill_scale.to_expr(expr.clone())?;
                    select_exprs.push(scaled_expr.alias("fill"));
                } else {
                    // No scale found, use raw expression
                    select_exprs.push(expr.clone().alias("fill"));
                }
            } else {
                // No scaling requested (ScaleSpec::None)
                select_exprs.push(expr.clone().alias("fill"));
            }
            channel_names.push("fill");
        }

        if let Some(expr) = stroke_expr {
            let stroke_channel = encodings.get("stroke").unwrap();
            if let Some(scale_name) = stroke_channel.scale_name("stroke") {
                if let Some(stroke_scale) = scales.get(&scale_name) {
                    let scaled_expr = stroke_scale.to_expr(expr.clone())?;
                    select_exprs.push(scaled_expr.alias("stroke"));
                } else {
                    // No scale found, use raw expression
                    select_exprs.push(expr.clone().alias("stroke"));
                }
            } else {
                // No scaling requested (ScaleSpec::None)
                select_exprs.push(expr.clone().alias("stroke"));
            }
            channel_names.push("stroke");
        }

        if let Some(expr) = stroke_width_expr {
            let stroke_width_channel = encodings.get("stroke_width").unwrap();
            if let Some(scale_name) = stroke_width_channel.scale_name("stroke_width") {
                if let Some(stroke_width_scale) = scales.get(&scale_name) {
                    let scaled_expr = stroke_width_scale.to_expr(expr.clone())?;
                    select_exprs.push(scaled_expr.alias("stroke_width"));
                } else {
                    // No scale found, use raw expression
                    select_exprs.push(expr.clone().alias("stroke_width"));
                }
            } else {
                // No scaling requested (ScaleSpec::None)
                select_exprs.push(expr.clone().alias("stroke_width"));
            }
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
        let fill_value = self.process_color_channel(
            batch,
            &channel_names,
            "fill",
            [0.27, 0.51, 0.71, 1.0], // Default steel blue
        );

        // Get stroke color
        let stroke_value = self.process_color_channel(
            batch,
            &channel_names,
            "stroke",
            [0.0, 0.0, 0.0, 1.0], // Default black
        );

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

    /// Process a color channel from the batch, handling both scalar and array cases
    fn process_color_channel(
        &self,
        batch: &datafusion::arrow::record_batch::RecordBatch,
        channel_names: &[&str],
        channel_name: &str,
        default_color: [f32; 4],
    ) -> ScalarOrArray<ColorOrGradient> {
        if channel_names.contains(&channel_name) {
            let col_idx = channel_names
                .iter()
                .position(|&c| c == channel_name)
                .unwrap();
            let color_array = batch.column(col_idx);

            if let Some(list_array) = color_array
                .as_any()
                .downcast_ref::<datafusion::arrow::array::ListArray>()
            {
                // Handle ListArray output from color scales (e.g., threshold scale)
                // Each element should be a Float32Array with RGBA values
                if list_array.len() == 1 && !list_array.is_null(0) {
                    // Single color for all instances
                    let rgba_array = list_array.value(0);
                    if let Some(f32_array) = rgba_array.as_any().downcast_ref::<Float32Array>() {
                        if f32_array.len() >= 4 {
                            let color = [
                                f32_array.value(0),
                                f32_array.value(1),
                                f32_array.value(2),
                                f32_array.value(3),
                            ];
                            ScalarOrArray::new_scalar(ColorOrGradient::Color(color))
                        } else {
                            ScalarOrArray::new_scalar(ColorOrGradient::Color(default_color))
                        }
                    } else {
                        ScalarOrArray::new_scalar(ColorOrGradient::Color(default_color))
                    }
                } else if list_array.len() > 1 {
                    // Multiple colors - one per instance
                    let mut colors = Vec::with_capacity(list_array.len());
                    for i in 0..list_array.len() {
                        if list_array.is_null(i) {
                            colors.push(default_color);
                        } else {
                            let rgba_array = list_array.value(i);
                            if let Some(f32_array) =
                                rgba_array.as_any().downcast_ref::<Float32Array>()
                            {
                                if f32_array.len() >= 4 {
                                    let color = [
                                        f32_array.value(0),
                                        f32_array.value(1),
                                        f32_array.value(2),
                                        f32_array.value(3),
                                    ];
                                    colors.push(color);
                                } else {
                                    colors.push(default_color);
                                }
                            } else {
                                colors.push(default_color);
                            }
                        }
                    }
                    // Convert to ColorOrGradient array
                    let color_gradients: Vec<ColorOrGradient> =
                        colors.into_iter().map(ColorOrGradient::Color).collect();
                    ScalarOrArray::new_array(color_gradients)
                } else {
                    // Empty array or all nulls
                    ScalarOrArray::new_scalar(ColorOrGradient::Color(default_color))
                }
            } else {
                // Not a string array or list array
                ScalarOrArray::new_scalar(ColorOrGradient::Color(default_color))
            }
        } else {
            // Channel not present
            ScalarOrArray::new_scalar(ColorOrGradient::Color(default_color))
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
                let configured_scale = scale.create_configured_scale(plot_width, plot_height)?;

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
