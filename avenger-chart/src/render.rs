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

    /// Prepare data for padding calculation, extracting only the required channels
    async fn prepare_padding_data(
        &self,
        mark: &dyn Mark<C>,
        padding_channels: &[&str],
        scales: &HashMap<String, Scale>,
    ) -> Result<(Option<RecordBatch>, RecordBatch), AvengerChartError> {
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

        let df = Arc::new(df_ref.clone());

        // Get channel mappings from DataContext
        let encodings = mark.data_context().encodings();

        // Get supported channels from the mark
        let supported_channels = mark.supported_channels();

        // Create a map of channel names to their descriptors for quick lookup
        let channel_desc_map: std::collections::HashMap<&str, &crate::marks::ChannelDescriptor> =
            supported_channels
                .iter()
                .map(|desc| (desc.name, desc))
                .collect();

        // Separate channels into those that need array data vs scalar data
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;

        for &channel_name in padding_channels {
            if let Some(channel_value) = encodings.get(channel_name) {
                // Get channel descriptor
                let channel_desc = channel_desc_map.get(channel_name).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Padding channel '{}' not in supported channels",
                        channel_name
                    ))
                })?;

                // Apply scaling to get the final expression
                let scaled_expr = self.apply_channel_scale(channel_name, channel_value, scales)?;

                // Check if this channel references columns (needs array data)
                if channel_desc.allow_column_ref && Self::references_columns(&scaled_expr) {
                    array_channels.push((channel_name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_name, scaled_expr));
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

        // Build scalar batch
        let scalar_batch = {
            let mut select_exprs = vec![];
            for (name, expr) in &scalar_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            if select_exprs.is_empty() {
                // Create empty batch with single row
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

        Ok((data_batch, scalar_batch))
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
        for (name, scale) in &mut non_positional_scales {
            self.process_scale(
                scale,
                name,
                plot_area_width,
                plot_area_height,
                |scale_copy| {
                    // Apply default color range for color channels
                    if matches!(name.as_str(), "fill" | "stroke" | "color") {
                        self.plot.apply_default_color_range(scale_copy, name);
                    }

                    // Apply default shape range for shape channel
                    if name == "shape" {
                        self.plot.apply_default_shape_range(scale_copy);
                    }
                },
            )
            .await?;
        }

        // Stage 2: Compute padding requirements from marks
        // Now that non-positional scales are ready, we can compute padding
        #[derive(Default)]
        struct AsymmetricPadding {
            lower: f64,
            upper: f64,
        }
        let mut mark_paddings: std::collections::HashMap<String, AsymmetricPadding> =
            std::collections::HashMap::new();

        // Merge all scales for padding calculation
        let mut scales_for_padding = non_positional_scales.clone();
        scales_for_padding.extend(positional_scales.clone());

        // Process positional scales first to get clip bounds, but only if domains aren't explicit
        // We need to clone and process separately to avoid marking domains as explicit
        let mut temp_positional_scales = HashMap::new();
        for (name, scale) in &positional_scales {
            if !scale.has_explicit_domain() {
                // Clone and process to get domain bounds without modifying original
                let mut scale_copy = scale.clone();
                self.infer_scale_domain(&mut scale_copy, name).await?;
                temp_positional_scales.insert(name.clone(), scale_copy);
            } else {
                temp_positional_scales.insert(name.clone(), scale.clone());
            }
        }

        // Extract current clip bounds from positional scales (data domain bounds)
        let mut x_bounds = (0.0, plot_area_width as f64);
        let mut y_bounds = (0.0, plot_area_height as f64);

        if let Some(scale) = temp_positional_scales.get("x") {
            // Try to extract actual domain bounds from scale
            if let Ok(configured_scale) = scale.create_configured_scale(plot_area_width, plot_area_height).await {
                if let Ok((min, max)) = configured_scale.numeric_interval_domain() {
                    x_bounds = (min as f64, max as f64);
                }
            }
        }

        if let Some(scale) = temp_positional_scales.get("y") {
            // Try to extract actual domain bounds from scale
            if let Ok(configured_scale) = scale.create_configured_scale(plot_area_width, plot_area_height).await {
                if let Ok((min, max)) = configured_scale.numeric_interval_domain() {
                    y_bounds = (min as f64, max as f64);
                }
            }
        }

        let clip_bounds = crate::marks::ClipBounds {
            x_min: x_bounds.0,
            x_max: x_bounds.1,
            y_min: y_bounds.0,
            y_max: y_bounds.1,
        };

        // Now compute padding for marks that don't have explicit domains
        for mark in &self.plot.marks {
            // Skip if positional scales have explicit domains
            let x_scale = positional_scales.get("x");
            let y_scale = positional_scales.get("y");
            if let (Some(x), Some(y)) = (x_scale, y_scale) {
                if x.has_explicit_domain() && y.has_explicit_domain() {
                    continue;
                }
            }
            
            let padding_channels = mark.padding_channels();
            if !padding_channels.is_empty() {
                // Prepare data for padding calculation using all scales
                let mut all_scales = scales_for_padding.clone();
                all_scales.extend(positional_scales.clone());
                
                let (padding_data, padding_scalars) = self
                    .prepare_padding_data(mark.as_ref(), &padding_channels, &all_scales)
                    .await?;

                // Compute padding for this mark with clip bounds
                let mark_padding = mark.compute_padding(
                    padding_data.as_ref(), 
                    &padding_scalars,
                    &clip_bounds,
                    plot_area_width,
                    plot_area_height
                )?;

                // Track maximum padding needed for each positional scale
                if let Some(x_lower) = mark_padding.x_lower {
                    let entry = mark_paddings.entry("x".to_string()).or_default();
                    entry.lower = entry.lower.max(x_lower);
                }
                if let Some(x_upper) = mark_padding.x_upper {
                    let entry = mark_paddings.entry("x".to_string()).or_default();
                    entry.upper = entry.upper.max(x_upper);
                }
                if let Some(y_lower) = mark_padding.y_lower {
                    let entry = mark_paddings.entry("y".to_string()).or_default();
                    entry.lower = entry.lower.max(y_lower);
                }
                if let Some(y_upper) = mark_padding.y_upper {
                    let entry = mark_paddings.entry("y".to_string()).or_default();
                    entry.upper = entry.upper.max(y_upper);
                }
            }
        }

        // Stage 3: Process positional scales with padding
        for (name, scale) in &mut positional_scales {
            let mark_paddings_ref = &mark_paddings;
            self.process_scale(
                scale,
                name,
                plot_area_width,
                plot_area_height,
                |scale_copy| {
                    // Apply mark-computed padding if available and not explicitly set
                    if !scale_copy.has_explicit_padding() && !scale_copy.has_explicit_domain() {
                        if let Some(padding) = mark_paddings_ref.get(name) {
                            if padding.lower > 0.0 || padding.upper > 0.0 {
                                // Apply asymmetric padding
                                *scale_copy = scale_copy
                                    .clone()
                                    .clip_padding_lower(datafusion::prelude::lit(padding.lower))
                                    .clip_padding_upper(datafusion::prelude::lit(padding.upper));
                            }
                        }
                    }
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
    ) -> Result<(), AvengerChartError> {
        if !scale.has_explicit_domain() {
            let data_expressions = self.plot.gather_scale_domain_expressions(name);
            if !data_expressions.is_empty() {
                // Convert expressions to data fields
                let mut data_fields = Vec::new();

                for (df, expr) in data_expressions {
                    match &expr {
                        Expr::Column(col) => {
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
                    *scale = scale.clone().domain_data_fields_internal(data_fields);
                }
            }
        }

        // Infer domain from data if needed
        if matches!(
            &scale.domain.default_domain,
            crate::scales::ScaleDefaultDomain::DataFields(_)
        ) {
            *scale = scale.clone().infer_domain_from_data().await?;
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
        apply_scale_specific: F,
    ) -> Result<(), AvengerChartError>
    where
        F: FnOnce(&mut Scale),
    {
        let mut scale_copy = scale.clone();

        // Apply domain inference
        self.infer_scale_domain(&mut scale_copy, name).await?;

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
