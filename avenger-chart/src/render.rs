//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::PlotTrait;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use crate::scales::Scale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::prelude::DataFrame;
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

        // 3. Title (can overflow, rendered on top)
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
        padding: &crate::layout::Padding,
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

    /// Create title mark if configured
    fn create_title(
        &self,
        _total_width: f32,
        _padding: &crate::layout::Padding,
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
