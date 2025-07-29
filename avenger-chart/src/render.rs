//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::axis::AxisTrait;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::PlotTrait;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
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

        for mark in &self.plot.marks {
            let scene_marks = self
                .render_mark(mark.as_ref(), &scales, plot_area_width, plot_area_height)
                .await?;
            mark_groups.extend(scene_marks);
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
        scales: &crate::scales::ScaleRegistry,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get the data - either from mark or inherit from plot
        let df = match mark.data_source() {
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

            let batch = df.clone().select(select_exprs)?.collect().await?;

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
                // Create empty batch with single row
                RecordBatch::try_new(Arc::new(Schema::new(vec![] as Vec<Field>)), vec![]).unwrap()
            } else {
                // Execute query to get scalar values
                let batches = df
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
        scales: &crate::scales::ScaleRegistry,
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
                    "band" | "point" => {
                        // Creating band/point axis
                        let result = make_band_axis_marks(
                            &configured_scale,
                            axis.title.as_deref().unwrap_or(""),
                            axis_origin,
                            &axis_config,
                        );
                        match result {
                            Ok(group) => {
                                // Band/point axis created successfully
                                group
                            }
                            Err(e) => {
                                // Error creating band/point axis
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
                canvas.add_group_mark(group, origin, parent_clip)?;
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
