//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use crate::scales::Scale;
use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::prelude::DataFrame;
use indexmap::IndexMap;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// Estimated proportion of plot area relative to total size for initial scale computation.
/// This is used before layout is calculated to build scales with approximate dimensions.
/// The actual plot area is typically 70-85% of total size after padding for axes/legends.
pub(crate) const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

/// Padding around a plot area
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Result of layout computation, containing padding and Taffy layout
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Padding around the plot area
    pub padding: Padding,
    /// The actual plot area rectangle (x, y, width, height)
    pub plot_area: (f32, f32, f32, f32),
    /// Taffy layout result for dynamic positioning
    pub taffy_layout: crate::chart_layout::LayoutResult,
}

impl LayoutSolution {
    /// Get the plot area bounds as a tuple
    pub fn plot_area_bounds(&self) -> (f32, f32, f32, f32) {
        self.plot_area
    }
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
#[allow(dead_code)]
fn parse_shape(s: &str) -> Result<avenger_common::types::SymbolShape, AvengerChartError> {
    avenger_common::types::SymbolShape::from_vega_str(s)
        .map_err(|_| AvengerChartError::InternalError(format!("Invalid shape name: '{}'", s)))
}

impl<'a, C: CoordinateSystem + Any> PlotRenderer<'a, C> {
    pub fn new(plot: &'a Plot<C>) -> Self {
        Self { plot }
    }

    /// Render the plot to a scene graph
    pub async fn render(&self) -> Result<RenderResult, AvengerChartError> {
        // Get plot dimensions from preferred size or default
        let (width, height) = self.plot.get_preferred_size().unwrap_or((400.0, 300.0));

        // STAGE 1: BUILD INITIAL SCALES WITH ESTIMATED DIMENSIONS
        // Use estimated dimensions for initial scale construction
        let estimated_plot_width = width * INITIAL_PLOT_AREA_RATIO;
        let estimated_plot_height = height * INITIAL_PLOT_AREA_RATIO;

        let (initial_scales, configured_non_positional, configured_positional) = self
            .build_initial_scales(estimated_plot_width, estimated_plot_height)
            .await?;

        // Merge configured scales for layout computation
        let mut initial_configured_scales = configured_non_positional.clone();
        initial_configured_scales.extend(configured_positional.clone());

        // STAGE 2: COMPUTE LAYOUT USING INITIAL SCALES
        let layout = self
            .compute_layout(width, height, &initial_configured_scales)
            .await?;
        let (plot_area_x, plot_area_y, plot_area_width, plot_area_height) =
            layout.plot_area_bounds();

        // STAGE 3: REBUILD POSITIONAL SCALES WITH FINAL DIMENSIONS
        let final_configured_scales = self
            .rebuild_scales_with_final_dimensions(
                &initial_scales,
                &configured_non_positional,
                plot_area_width,
                plot_area_height,
            )
            .await?;

        // STAGE 4: RENDER ALL COMPONENTS WITH FINAL SCALES
        let all_component_marks = self
            .render_all_components(&final_configured_scales, &layout, width, height)
            .await?;

        let (mark_groups, axis_marks, legend_marks, title_marks, subtitle_marks) =
            all_component_marks;

        // Compose all elements into a scene graph
        // A single Plot should produce a single top-level group
        let mut all_marks = Vec::new();

        // Get the appropriate clipping region from the coordinate system
        let clip = self.plot.coord_system().get_clip(
            plot_area_width,
            plot_area_height,
            &final_configured_scales,
        );

        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip,
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

        // 5. Subtitle (can overflow, rendered on top)
        all_marks.extend(subtitle_marks);

        // 6. Debug: Add Taffy layout bounds visualization if AVENGER_CHART_DEBUG_LAYOUT is set
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            all_marks.extend(Self::create_debug_layout_rects(&layout.taffy_layout));
        }

        // Wrap everything in a single root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width,
            height,
            origin: [0.0, 0.0],
        };

        // Build spatial index for hit testing
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(RenderResult {
            scene_graph,
            rtree: Some(rtree),
        })
    }

    /// Create debug rectangles to visualize Taffy layout bounds
    fn create_debug_layout_rects(layout: &crate::chart_layout::LayoutResult) -> Vec<SceneMark> {
        use avenger_common::types::ColorOrGradient;
        use avenger_scenegraph::marks::rect::SceneRectMark;
        use avenger_scenegraph::marks::text::SceneTextMark;

        let mut debug_marks = Vec::new();

        // Plot area - magenta outline
        let plot_rect = SceneRectMark {
            x: layout.plot_area.x.into(),
            y: layout.plot_area.y.into(),
            width: Some(layout.plot_area.width.into()),
            height: Some(layout.plot_area.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // Transparent
            stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
            stroke_width: 1.0.into(),                                  // Match other rectangles
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(plot_rect));

        // Plot area label
        let plot_label = SceneTextMark {
            text: "plot-area".into(),
            x: (layout.plot_area.x + 2.0).into(),
            y: (layout.plot_area.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(std::sync::Arc::new(plot_label)));

        // Axes - magenta outlines with labels
        for (position, bounds) in &layout.axes {
            let axis_rect = SceneRectMark {
                x: bounds.x.into(),
                y: bounds.y.into(),
                width: Some(bounds.width.into()),
                height: Some(bounds.height.into()),
                fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                stroke_width: 1.0.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Rect(axis_rect));

            // Add axis label - check if it's an overflow pseudo-axis
            let (axis_label, label_x, label_y, angle, align, baseline) = match position {
                crate::axis::AxisPosition::Left => {
                    // Rotate 90 degrees for left, position at top-left
                    (
                        "of-left",
                        bounds.x + 2.0,
                        bounds.y + 2.0,
                        -90.0,
                        avenger_text::types::TextAlign::Right, // Right align becomes top when rotated -90
                        avenger_text::types::TextBaseline::Top,
                    )
                }
                crate::axis::AxisPosition::Right => {
                    // Rotate 90 degrees for right, position at top-right
                    (
                        "of-right",
                        bounds.x + bounds.width - 2.0,
                        bounds.y + 2.0,
                        -90.0,
                        avenger_text::types::TextAlign::Right, // Right align becomes top when rotated -90
                        avenger_text::types::TextBaseline::Bottom,
                    ) // Bottom baseline becomes right when rotated
                }
                crate::axis::AxisPosition::Top => {
                    // Position at top of region
                    (
                        "of-top",
                        bounds.x + 2.0,
                        bounds.y,
                        0.0,
                        avenger_text::types::TextAlign::Left,
                        avenger_text::types::TextBaseline::Top,
                    )
                }
                crate::axis::AxisPosition::Bottom => {
                    // Position at bottom of region
                    (
                        "of-bottom",
                        bounds.x + 2.0,
                        bounds.y + bounds.height - 2.0,
                        0.0,
                        avenger_text::types::TextAlign::Left,
                        avenger_text::types::TextBaseline::Bottom,
                    )
                }
            };

            let label = SceneTextMark {
                text: axis_label.into(),
                x: label_x.into(),
                y: label_y.into(),
                font_size: 8.0.into(),
                color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                angle: angle.into(),
                align: align.into(),
                baseline: baseline.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Text(std::sync::Arc::new(label)));
        }

        // Legends - magenta outlines
        for (channel, bounds) in &layout.legends {
            let legend_rect = SceneRectMark {
                x: bounds.x.into(),
                y: bounds.y.into(),
                width: Some(bounds.width.into()),
                height: Some(bounds.height.into()),
                fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                stroke_width: 1.0.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Rect(legend_rect));

            // Add label for legend channel
            let label = SceneTextMark {
                text: channel.clone().into(),
                x: (bounds.x + 2.0).into(),
                y: (bounds.y + 10.0).into(),
                font_size: 8.0.into(),
                color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Text(std::sync::Arc::new(label)));
        }

        // Title - magenta outline
        if let Some(bounds) = &layout.title {
            let title_rect = SceneRectMark {
                x: bounds.x.into(),
                y: bounds.y.into(),
                width: Some(bounds.width.into()),
                height: Some(bounds.height.into()),
                fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                stroke_width: 1.0.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Rect(title_rect));

            // Add title label - right aligned to avoid overlapping with text
            let title_label = SceneTextMark {
                text: "title".into(),
                x: (bounds.x + bounds.width - 5.0).into(), // Right side with small padding
                y: (bounds.y + 10.0).into(),
                font_size: 8.0.into(),
                color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                align: avenger_text::types::TextAlign::Right.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Text(std::sync::Arc::new(title_label)));
        }

        // Subtitle - magenta outline
        if let Some(bounds) = &layout.subtitle {
            let subtitle_rect = SceneRectMark {
                x: bounds.x.into(),
                y: bounds.y.into(),
                width: Some(bounds.width.into()),
                height: Some(bounds.height.into()),
                fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                stroke_width: 1.0.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Rect(subtitle_rect));

            // Add subtitle label - right aligned to avoid overlapping with text
            let subtitle_label = SceneTextMark {
                text: "subtitle".into(),
                x: (bounds.x + bounds.width - 5.0).into(), // Right side with small padding
                y: (bounds.y + 10.0).into(),
                font_size: 8.0.into(),
                color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
                align: avenger_text::types::TextAlign::Right.into(),
                zindex: Some(20),
                ..Default::default()
            };
            debug_marks.push(SceneMark::Text(std::sync::Arc::new(subtitle_label)));
        }

        debug_marks
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
    /// Build initial scales with estimated dimensions
    /// Returns (raw_scales, configured_non_positional, configured_positional)
    async fn build_initial_scales(
        &self,
        estimated_plot_width: f32,
        estimated_plot_height: f32,
    ) -> Result<
        (
            HashMap<String, Scale>,
            HashMap<String, avenger_scales::scales::ConfiguredScale>,
            HashMap<String, avenger_scales::scales::ConfiguredScale>,
        ),
        AvengerChartError,
    > {
        // Collect all channels that need scales
        let mut channels_with_scales = self.plot.collect_channels_needing_scales();

        // Also include any channels with explicit scale specs
        // (even if they only have literal values, we need the scale for layout)
        for channel in self.plot.scale_specs.keys() {
            channels_with_scales.insert(channel.clone());
        }

        // Build initial scale definitions
        let mut initial_scales = HashMap::new();
        for channel in &channels_with_scales {
            let scale = self.plot.get_scale(channel)?;
            initial_scales.insert(channel.clone(), scale);
        }

        // Separate scales into positional and non-positional
        let mut positional_scales = HashMap::new();
        let mut non_positional_scales = HashMap::new();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .plot
            .coord_system()
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        for (name, scale) in &initial_scales {
            if required_channels.contains(name) {
                positional_scales.insert(name.clone(), scale.clone());
            } else {
                non_positional_scales.insert(name.clone(), scale.clone());
            }
        }

        // Build non-positional scales first (no radius context needed)
        let mut configured_non_positional = HashMap::new();
        for (name, scale) in &non_positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale.clone(),
                    name,
                    estimated_plot_width,
                    estimated_plot_height,
                    None,
                )
                .await?;
            configured_non_positional.insert(name.clone(), configured);
        }

        // Build positional scales with radius context
        let mut configured_positional = HashMap::new();
        for (name, scale) in &positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale.clone(),
                    name,
                    estimated_plot_width,
                    estimated_plot_height,
                    Some(&configured_non_positional),
                )
                .await?;
            configured_positional.insert(name.clone(), configured);
        }

        Ok((
            initial_scales,
            configured_non_positional,
            configured_positional,
        ))
    }

    /// Validate that required positional scales exist and provide proper error messages
    fn validate_positional_scales_exist(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<(), AvengerChartError> {
        // Get the required positional channels from the coordinate system
        let required_channels = self.plot.coord_system().required_channels();

        // Build a set of positional channel names to check
        // Include both base channels and their interval variants (e.g., "x" and "x2")
        let mut positional_channels = std::collections::HashSet::new();
        for &channel in required_channels {
            positional_channels.insert(channel.to_string());
            // Also check for interval variant (e.g., "x2" for "x")
            positional_channels.insert(format!("{}2", channel));
        }

        // Check if positional channels are being used with literal values
        for mark in &self.plot.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Check if this is a positional channel
                if positional_channels.contains(channel_name) {
                    // Check if this is a literal value (no scale needed)
                    if channel_value.get_scale_name(channel_name).is_none() {
                        // Get the base scale name (e.g., "x" from "x2")
                        let base_scale_name = channel_name.trim_end_matches('2');

                        // Check if we have an explicit scale spec for this channel or its base
                        if !self.plot.scale_specs.contains_key(channel_name)
                            && !self.plot.scale_specs.contains_key(base_scale_name)
                        {
                            // This is a literal value with no explicit scale - error
                            return self.create_positional_literal_error(
                                channel_name,
                                channel_value,
                                std::any::type_name::<C>(),
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Create error for literal values in positional scales
    fn create_positional_literal_error(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        coord_system_name: &str,
    ) -> Result<(), AvengerChartError> {
        use datafusion::logical_expr::Expr as DfExpr;

        // Extract the literal value description
        let literal_value = match channel_value {
            ChannelValue::Value { expr } => {
                // Check if the expression is a literal
                match expr {
                    DfExpr::Literal(scalar_value, _) => match scalar_value {
                        datafusion_common::ScalarValue::Utf8(Some(_))
                        | datafusion_common::ScalarValue::LargeUtf8(Some(_)) => {
                            "string literal".to_string()
                        }
                        datafusion_common::ScalarValue::Float32(Some(_))
                        | datafusion_common::ScalarValue::Float64(Some(_))
                        | datafusion_common::ScalarValue::Int32(Some(_))
                        | datafusion_common::ScalarValue::Int64(Some(_))
                        | datafusion_common::ScalarValue::Int8(Some(_))
                        | datafusion_common::ScalarValue::Int16(Some(_))
                        | datafusion_common::ScalarValue::UInt8(Some(_))
                        | datafusion_common::ScalarValue::UInt16(Some(_))
                        | datafusion_common::ScalarValue::UInt32(Some(_))
                        | datafusion_common::ScalarValue::UInt64(Some(_)) => {
                            "numeric literal".to_string()
                        }
                        _ => "literal value".to_string(),
                    },
                    _ => "expression".to_string(),
                }
            }
            _ => "literal value".to_string(),
        };

        // Create helpful suggestion based on the literal type
        let suggestion = if literal_value.contains("string") {
            "Did you mean to reference a column? Use col(\"column_name\") to reference a column."
                .to_string()
        } else {
            "To use a literal value, provide an explicit domain using .scale_x() or .scale_y().\n\
             Or use col(\"column_name\") to reference a data column."
                .to_string()
        };

        // Extract coordinate system name (remove module path)
        let coord_system = coord_system_name
            .split("::")
            .last()
            .unwrap_or(coord_system_name);

        Err(AvengerChartError::PositionalScaleLiteralError {
            scale_name: channel_name.to_string(),
            coord_system: coord_system.to_string(),
            literal_value,
            suggestion,
        })
    }

    /// Compute layout using the coordinate system's capabilities
    async fn compute_layout(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<LayoutSolution, AvengerChartError> {
        // All coordinate systems now use dynamic layout with Taffy

        // Get default axes for all channels with scales
        let default_axes = self
            .plot
            .coord_system()
            .create_default_axes(scales, &self.plot.marks);

        // Apply user axis customizations to defaults
        let mut all_axes = default_axes;
        for (channel, axis_spec) in &self.plot.axis_specs {
            // Only apply customizations if there's a default axis
            // Axes without scales won't be rendered anyway
            if let Some(base_axis) = all_axes.get(channel).cloned() {
                // Apply the customization function
                match axis_spec {
                    crate::plot::AxisSpec::Local(f) => {
                        let customized = f(base_axis);
                        all_axes.insert(channel.clone(), customized);
                    }
                    crate::plot::AxisSpec::Reference(_) => {
                        // Reference axes not yet supported, keep default
                    }
                }
            }
        }

        // Apply axis configurations from mark channels (last mark wins for conflicts)
        // This must happen BEFORE layout measurement so the layout system knows the actual axis titles
        for mark in &self.plot.marks {
            for (channel, axis_config) in mark.state().axis_configs.iter() {
                if let Some(base_axis) = all_axes.get(channel).cloned() {
                    let configured = axis_config(base_axis);
                    all_axes.insert(channel.clone(), configured);
                }
            }
        }

        // Call the dynamic layout helper with the coordinate system's axes
        self.compute_layout_with_dynamic_axes(width, height, scales, all_axes)
            .await
    }

    /// Helper method for dynamic layout with type-erased axes
    async fn compute_layout_with_dynamic_axes(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        axes: HashMap<String, C::Axis>,
    ) -> Result<LayoutSolution, AvengerChartError> {
        // The axes parameter already has all customizations applied from compute_layout
        // (both plot-level and mark-level configurations)
        let customized_axes = axes;

        // Check for required positional scales before measuring overflow
        // This ensures we provide proper error messages for literal values
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the coordinate system's guides need
        let overflow = self
            .plot
            .coord_system()
            .measure_guide_overflow(
                customized_axes,
                scales,
                width,
                height,
                INITIAL_PLOT_AREA_RATIO,
            )
            .await?;

        tracing::trace!(
            coord_system = std::any::type_name::<C>(),
            overflow_top = overflow.top,
            overflow_bottom = overflow.bottom,
            overflow_left = overflow.left,
            overflow_right = overflow.right,
            "Measured guide overflow"
        );

        // Use Taffy layout with the overflow requirements
        let (padding, taffy_layout) = self
            .compute_layout_with_overflow(width, height, scales, overflow)
            .await?;

        Ok(LayoutSolution {
            padding,
            plot_area: Self::calculate_plot_area_from_padding(&padding, width, height),
            taffy_layout,
        })
    }

    /// Rebuild scales with final dimensions from layout
    async fn rebuild_scales_with_final_dimensions(
        &self,
        initial_scales: &HashMap<String, Scale>,
        configured_non_positional: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<HashMap<String, avenger_scales::scales::ConfiguredScale>, AvengerChartError> {
        let mut final_configured_scales = HashMap::new();

        // Pass through non-positional scales unchanged
        for (name, configured_scale) in configured_non_positional {
            final_configured_scales.insert(name.clone(), configured_scale.clone());
        }

        // Rebuild positional scales with final dimensions
        for (name, scale) in initial_scales {
            // Check if this is a positional scale for the current coordinate system
            let is_positional = self
                .plot
                .coord_system()
                .required_channels()
                .iter()
                .any(|&ch| ch == name);

            if is_positional {
                let rebuilt_scale = self
                    .build_configured_scale_with_radius_context(
                        scale.clone(),
                        name,
                        plot_area_width,
                        plot_area_height,
                        Some(configured_non_positional),
                    )
                    .await?;
                final_configured_scales.insert(name.clone(), rebuilt_scale);
            }
        }

        Ok(final_configured_scales)
    }

    /// Render all components (marks, axes, legends, titles)
    async fn render_all_components(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        layout: &LayoutSolution,
        width: f32,
        _height: f32,
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
        let (_, _, plot_area_width, plot_area_height) = layout.plot_area_bounds();

        // Render marks
        let mut mark_groups = Vec::new();
        for mark in &self.plot.marks {
            let scene_marks = self
                .render_mark(mark.as_ref(), scales, plot_area_width, plot_area_height)
                .await?;
            mark_groups.extend(scene_marks);
        }

        // Create axes
        let axis_marks = self
            .create_axes(scales, plot_area_width, plot_area_height, &layout.padding)
            .await?;

        // Create legends
        let legend_marks = self
            .create_legends_with_layout(
                scales,
                &layout.taffy_layout,
                plot_area_width,
                plot_area_height,
            )
            .await?;

        // Create title
        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
            self.create_title(
                width,
                &layout.padding,
                Some(*title_bounds),
                Some(layout.taffy_layout.plot_area),
            )?
        } else {
            Vec::new()
        };

        // Create subtitle
        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
            self.create_subtitle(
                width,
                &layout.padding,
                Some(*subtitle_bounds),
                Some(layout.taffy_layout.plot_area),
            )?
        } else {
            Vec::new()
        };

        Ok((
            mark_groups,
            axis_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
        ))
    }

    async fn render_mark(
        &self,
        mark: &dyn Mark<C>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references (e.g., ":x" -> actual x expression)
        let channels = crate::channel_resolution::resolve_all_channel_refs(channels)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                crate::scales::validation::expr_references_columns(expr)
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    crate::scales::validation::expr_references_columns(condition)
                        || crate::scales::validation::expr_references_columns(value.expr())
                }) || crate::scales::validation::expr_references_columns(otherwise.expr())
            }
        });

        // Determine data source based on:
        // 1. If mark has explicit data, use it
        // 2. If no expressions reference columns, use unit (single row)
        // 3. Otherwise inherit from plot if available
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe() {
            // Mark has explicit data
            Some(mark_df)
        } else if !references_columns {
            // No column references - use unit data
            None
        } else if let Some(plot_data) = &self.plot.data {
            // Inherit from plot
            Some(plot_data)
        } else {
            // No data available but columns are referenced
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Check if mark supports order and has order encoding
        let df = if let Some(df_ref) = df_ref {
            if mark.supports_order() {
                if let Some(order_channel) = channels.get("order") {
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
            }
        } else {
            // Unit data source - create minimal DataFrame with single row
            // This allows scalar expressions to be evaluated
            use datafusion::prelude::*;
            let ctx = SessionContext::new();
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

        // Validate positional channel data types before rendering
        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        // Let the coordinate system prepare the scalar batch with any required columns
        let scalar_batch =
            self.plot
                .coord_system()
                .prepare_scalar_batch(scalar_batch, plot_width, plot_height)?;

        // Call the mark's render_from_data method
        mark.render_from_data(data_batch.as_ref(), &scalar_batch)
    }

    /// Validate that positional channels have numeric data types
    fn validate_positional_channel_types(
        &self,
        data_batch: &Option<RecordBatch>,
        scalar_batch: &RecordBatch,
    ) -> Result<(), AvengerChartError> {
        // Check each positional channel
        for channel_name in self.plot.coord_system().required_channels() {
            // Check in data batch first
            if let Some(data) = data_batch {
                if let Some(column) = data.column_by_name(channel_name) {
                    let dtype = column.data_type();
                    if !Self::is_numeric_type(dtype) {
                        return self.create_positional_type_error(channel_name, dtype);
                    }
                }
            }

            // Check in scalar batch
            if let Some(column) = scalar_batch.column_by_name(channel_name) {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(channel_name, dtype);
                }
            }
        }

        Ok(())
    }

    /// Check if a data type is numeric
    fn is_numeric_type(dtype: &datafusion::arrow::datatypes::DataType) -> bool {
        use datafusion::arrow::datatypes::DataType;
        matches!(
            dtype,
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
                | DataType::Float64
        )
    }

    /// Create error for non-numeric positional channel
    fn create_positional_type_error(
        &self,
        channel_name: &str,
        dtype: &datafusion::arrow::datatypes::DataType,
    ) -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Get coordinate system name
        let coord_system_name = std::any::type_name::<C>()
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
            coord_system: coord_system_name,
            literal_value,
            suggestion,
        })
    }

    /// Apply scaling transformation to a channel expression
    fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<datafusion::logical_expr::Expr, AvengerChartError> {
        use crate::marks::channel::strip_trailing_numbers;

        match channel_value {
            ChannelValue::Value { expr } => {
                // No scaling requested, return expression as-is
                Ok(expr.clone())
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build a CASE WHEN expression from the conditions
                use crate::marks::channel::{ConditionalValue, strip_trailing_numbers};
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

                // Get the scale if it exists (for applying to Field branches)
                let scale_opt = scales.get(&scale_key);

                // Process the otherwise value
                let otherwise_expr = match otherwise {
                    ConditionalValue::Scaled { expr } => {
                        // This needs scaling
                        if let Some(scale) = scale_opt {
                            use crate::scales::ConfiguredScaleDataFusionExt;
                            scale.to_expr(expr.clone())?
                        } else {
                            // No scale available, use expression as-is
                            expr.clone()
                        }
                    }
                    ConditionalValue::Value { expr } => {
                        // Literal value - convert if it's a color string
                        convert_color_literal(expr)
                    }
                };

                // Build CASE WHEN expression from conditions (evaluated in order added)
                // We iterate in reverse because we're building nested when() calls from inside out
                // The last condition in the array should be the innermost (evaluated last)
                let mut case_expr = otherwise_expr;
                for (test, value) in conditions.iter().rev() {
                    let value_expr = match value {
                        ConditionalValue::Scaled { expr } => {
                            // This needs scaling
                            if let Some(scale) = scale_opt {
                                use crate::scales::ConfiguredScaleDataFusionExt;
                                scale.to_expr(expr.clone())?
                            } else {
                                expr.clone()
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            // Literal value - convert if it's a color string
                            convert_color_literal(expr)
                        }
                    };

                    // Wrap in WHEN clause
                    case_expr = when(test.clone(), value_expr)
                        .otherwise(case_expr)?
                        .alias(channel_name);
                }

                Ok(case_expr)
            }
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
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

                // Use ConfiguredScale's extension methods
                use crate::scales::ConfiguredScaleDataFusionExt;

                // Apply the scale transformation with optional band parameter
                if let Some(band_value) = band {
                    scale.to_expr_with_band(expr.clone(), *band_value)
                } else {
                    scale.to_expr(expr.clone())
                }
            }
        }
    }

    /// Create axis marks based on configured axes
    async fn create_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get default axes for all channels with scales
        let default_axes = self
            .plot
            .coord_system()
            .create_default_axes(scales, &self.plot.marks);

        // Apply user axis customizations to defaults
        let mut all_axes = default_axes;
        for (channel, axis_spec) in &self.plot.axis_specs {
            // Only apply customizations if there's a default axis
            // Axes without scales won't be rendered anyway
            if let Some(base_axis) = all_axes.get(channel).cloned() {
                // Apply the customization function
                match axis_spec {
                    crate::plot::AxisSpec::Local(f) => {
                        let customized = f(base_axis);
                        all_axes.insert(channel.clone(), customized);
                    }
                    crate::plot::AxisSpec::Reference(_) => {
                        // Reference axes not yet supported, keep default
                    }
                }
            }
        }

        // Apply axis configurations from mark channels (same as in compute_layout)
        for mark in &self.plot.marks {
            for (channel, axis_config) in mark.state().axis_configs.iter() {
                if let Some(base_axis) = all_axes.get(channel).cloned() {
                    let configured = axis_config(base_axis);
                    all_axes.insert(channel.clone(), configured);
                }
            }
        }

        // Delegate all axis rendering to the coordinate system
        self.plot
            .coord_system()
            .render_axes(&all_axes, scales, plot_width, plot_height, padding)
            .await
    }

    /// Create legend marks based on configured legends
    /// Create legends using Taffy layout positions
    async fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        layout: &crate::chart_layout::LayoutResult,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get default legends for channels with data-driven scales
        let default_legends = self.create_default_legends(scales);

        // Combine existing legends with defaults
        let mut all_legend_configs = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legend_configs.entry(channel).or_insert(default_legend);
        }

        // Use the helper to merge legend channels - exactly the same as for layout
        let (sorted_channel_groups, _legends_map) =
            self.merge_legend_channels(&all_legend_configs, scales);

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
                    let mark_opt = self
                        .plot
                        .marks
                        .iter()
                        .find(|m| m.mark_id() == primary_channel.mark_id);

                    mark_opt
                        .and_then(|mark| mark.preferred_merged_legend_renderer(&channels, &scales))
                } else {
                    // Single channel - use the standard renderer selection
                    scales.get(&primary_channel.name).and_then(|scale| {
                        if let Some(ref renderer) = legend.renderer {
                            // Use explicitly configured renderer
                            Some(renderer.clone())
                        } else {
                            // Find the mark and get its preference
                            self.plot
                                .marks
                                .iter()
                                .find(|m| m.mark_id() == primary_channel.mark_id)
                                .and_then(|mark| {
                                    mark.preferred_legend_renderer(
                                        &primary_channel.channel_type,
                                        scale,
                                    )
                                })
                        }
                    })
                };

                // Skip this legend group if no renderer is available
                let Some(renderer) = renderer_opt else {
                    continue;
                };

                // Render the legend with all channels in the group
                if let Some(group) = renderer
                    .render(
                        &channels,
                        legend,
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    )
                    .await?
                {
                    legend_marks.push(SceneMark::Group(group));
                }
            }
        }

        Ok(legend_marks)
    }

    /// Create default legends for channels with ConfiguredScale
    fn create_default_legends(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> IndexMap<String, crate::legend::Legend> {
        use crate::legend::Legend;

        let mut default_legends = IndexMap::new();

        // Build set of channels to skip for legends
        let mut skip_channels = std::collections::HashSet::new();

        // Add positional channels from the coordinate system
        for &channel in self.plot.coord_system().required_channels() {
            skip_channels.insert(channel.to_string());
            // Also skip interval variants (e.g., "x2" for "x")
            skip_channels.insert(format!("{}2", channel));
        }

        // Add utility channels that don't need legends
        // These are non-positional but still shouldn't get legends
        skip_channels.insert("order".to_string());
        skip_channels.insert("defined".to_string());
        skip_channels.insert("angle".to_string());

        // Sort channels for deterministic ordering
        let mut sorted_channels: Vec<_> = scales.keys().collect();
        sorted_channels.sort();

        for channel in sorted_channels {
            // Skip channels that don't need legends
            if skip_channels.contains(channel) {
                continue;
            }

            // Skip if legend already configured
            if self.plot.legends.contains_key(channel) {
                continue;
            }

            // ConfiguredScale always has resolved domain, so always create a legend
            // for channels that have scales
            let legend = Legend::new()
                .title(self.infer_legend_title(channel))
                .position(self.default_legend_position(channel));

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(&self, channel: &str) -> String {
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
    fn default_legend_position(&self, _channel: &str) -> crate::legend::LegendPosition {
        // All legends default to the right
        crate::legend::LegendPosition::Right
    }

    /// Create title mark if configured
    fn create_title(
        &self,
        _total_width: f32,
        _padding: &Padding,
        layout_bounds: Option<crate::chart_layout::LayoutBounds>,
        _plot_area: Option<crate::chart_layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};

        let Some(title) = self.plot.get_title() else {
            return Ok(Vec::new());
        };

        // If we have layout bounds from Taffy, place the title left-aligned within its bounds
        let (x, y) = if let Some(bounds) = layout_bounds {
            // Use the title node's x position, not the plot area's
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            // Fallback: centered at top of canvas
            (10.0, 10.0)
        };

        let text = SceneTextMark {
            text: title.text.clone().into(),
            x: x.into(),
            y: y.into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            font: title.font_family.clone().into(),
            font_size: title.font_size.into(),
            font_weight: avenger_text::types::FontWeight::Number(500.0).into(),
            color: ColorOrGradient::Color([0.102, 0.102, 0.102, 1.0]).into(), // #1A1A1A
            ..Default::default()
        };

        let group = SceneGroup {
            marks: vec![SceneMark::Text(text.into())],
            zindex: Some(20),
            ..Default::default()
        };
        Ok(vec![SceneMark::Group(group)])
    }

    /// Create subtitle mark if configured
    fn create_subtitle(
        &self,
        _total_width: f32,
        _padding: &Padding,
        layout_bounds: Option<crate::chart_layout::LayoutBounds>,
        _plot_area: Option<crate::chart_layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{TextAlign, TextBaseline};

        let Some(subtitle) = self.plot.get_subtitle() else {
            return Ok(Vec::new());
        };

        // If we have layout bounds from Taffy, place the subtitle left-aligned within its bounds
        let (x, y) = if let Some(bounds) = layout_bounds {
            // Use the subtitle node's x position, not the plot area's
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            // Fallback: left-aligned below title
            (10.0, 30.0)
        };

        let text = SceneTextMark {
            text: subtitle.text.clone().into(),
            x: x.into(),
            y: y.into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            font: subtitle.font_family.clone().into(),
            font_size: subtitle.font_size.into(),
            font_weight: avenger_text::types::FontWeight::Number(200.0).into(),
            color: ColorOrGradient::Color([0.290, 0.290, 0.290, 1.0]).into(), // #4A4A4A
            ..Default::default()
        };

        let group = SceneGroup {
            marks: vec![SceneMark::Text(text.into())],
            zindex: Some(20),
            ..Default::default()
        };
        Ok(vec![SceneMark::Group(group)])
    }

    /// Helper function to merge legend channels based on MergeKey
    /// Returns a sorted list of channel groups and a legends map for layout
    fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, crate::legend::Legend>,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> (
        Vec<Vec<crate::legend_renderer::LegendChannel>>,
        IndexMap<String, crate::legend::Legend>,
    ) {
        use crate::legend_renderer::{LegendChannel, MergeKey};
        use std::collections::HashMap;

        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for mark in &self.plot.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Skip if no scale or no legend config
                if !configured_scales.contains_key(channel_name)
                    || !all_legends.contains_key(channel_name)
                {
                    continue;
                }

                let legend_config = &all_legends[channel_name];
                if !legend_config.visible {
                    continue;
                }

                let scale = &configured_scales[channel_name];

                // Collect related channels
                let mut related_channels = HashMap::new();
                for (other_name, other_value) in mark.data_context().channels() {
                    if other_name != channel_name {
                        let other_scale = configured_scales
                            .get(other_name)
                            .cloned()
                            .unwrap_or_else(|| {
                                // Create dummy scale for constants
                                use arrow::array::Float64Array;
                                use avenger_scales::scales::{
                                    ConfiguredScale, ScaleConfig, ScaleContext, linear::LinearScale,
                                };
                                use std::sync::Arc;

                                let scale_impl = Arc::new(LinearScale);
                                let domain = Arc::new(Float64Array::from(vec![0.0, 1.0]));
                                let range = Arc::new(Float64Array::from(vec![0.0, 1.0]));
                                let config = ScaleConfig {
                                    domain,
                                    range,
                                    options: HashMap::new(),
                                    context: ScaleContext::default(),
                                };
                                ConfiguredScale { scale_impl, config }
                            });
                        related_channels.insert(
                            other_name.clone(),
                            (other_value.expr().cloned(), other_scale),
                        );
                    }
                }

                let legend_channel = LegendChannel {
                    name: channel_name.clone(),
                    expression: channel_value.expr().cloned(),
                    scale: scale.clone(),
                    channel_type: channel_name.clone(),
                    mark_type: mark.mark_type().to_string(),
                    mark_id: mark.mark_id(),
                    related_channels,
                };

                all_channels.push(legend_channel);
            }
        }

        // Group channels by MergeKey - use a vector to preserve order
        // Channels without merge keys (continuous scales) are kept separate
        let mut channel_groups: Vec<Vec<LegendChannel>> = Vec::new();

        for channel in all_channels {
            let merge_key = MergeKey::from_channel(&channel);

            if merge_key.is_none() {
                // Continuous scales or channels without expressions should not be merged
                // Each gets its own group
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
        let mut groups_with_order: Vec<(Vec<LegendChannel>, i32)> = Vec::new();

        for channels in channel_groups {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    let order = legend_config.order.unwrap_or(i32::MAX);
                    groups_with_order.push((channels, order));
                }
            }
        }

        // Sort by order value
        groups_with_order.sort_by_key(|(_, order)| *order);

        // Extract sorted channel groups
        let sorted_channel_groups: Vec<Vec<LegendChannel>> = groups_with_order
            .iter()
            .map(|(channels, _)| channels.clone())
            .collect();

        // Create the legends map with merged channel info for layout
        let mut legends_map: IndexMap<String, crate::legend::Legend> = IndexMap::new();
        for (channels, _) in groups_with_order {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    if legend_config.visible {
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

    /// Compute layout using Taffy for the coordinate system
    /// Convert overflow requirements to pseudo-axes for Taffy layout
    async fn compute_layout_with_overflow(
        &self,
        width: f32,
        height: f32,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        overflow: crate::coords::OverflowSpaceRequirement,
    ) -> Result<(Padding, crate::chart_layout::LayoutResult), AvengerChartError> {
        use crate::chart_layout::ChartLayout;

        // Get default legends for channels with ConfiguredScale
        let default_legends = self.create_default_legends(configured_scales);
        let mut all_legends = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legends.entry(channel).or_insert(default_legend);
        }

        // Use the helper to merge legend channels - exactly the same as for rendering
        let (_channel_groups, legends_map) =
            self.merge_legend_channels(&all_legends, configured_scales);

        // Create ChartLayout with overflow directly
        let mut layout = ChartLayout::new_with_overflow::<C>(
            &overflow,
            &legends_map,
            configured_scales,
            Some((width, height)),
            self.plot.get_title(),
            self.plot.get_subtitle(),
            &self.plot.marks,
        )?;

        // Compute layout
        let layout_result = layout.compute(width, height)?;

        // Convert layout bounds to padding
        let padding = Padding {
            left: layout_result.plot_area.x,
            right: width - (layout_result.plot_area.x + layout_result.plot_area.width),
            top: layout_result.plot_area.y,
            bottom: height - (layout_result.plot_area.y + layout_result.plot_area.height),
        };

        Ok((padding, layout_result))
    }

    /// Calculate plot area from padding and total dimensions
    fn calculate_plot_area_from_padding(
        padding: &Padding,
        width: f32,
        height: f32,
    ) -> (f32, f32, f32, f32) {
        (
            padding.left,
            padding.top,
            width - padding.left - padding.right,
            height - padding.top - padding.bottom,
        )
    }

    /// Build a ConfiguredScale directly, handling domain processing with radius context
    /// This combines the functionality of process_scale_domain_with_radius and build_scale_with_context
    async fn build_configured_scale_with_radius_context(
        &self,
        scale: Scale,
        name: &str,
        plot_area_width: f32,
        plot_area_height: f32,
        configured_non_positional: Option<
            &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        >,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        let mut scale = scale;

        // Step 1: Process domain with radius if applicable
        if matches!(
            &scale.domain.default_domain,
            crate::scales::ScaleDefaultDomain::DomainExprs(_)
        ) {
            // Only use radius-aware gathering for linear positional scales with context
            if let Some(configured_non_positional) = configured_non_positional {
                // Check if this is a positional channel (including interval variants)
                let is_positional = self
                    .plot
                    .coord_system()
                    .required_channels()
                    .iter()
                    .any(|&ch| name == ch || name == format!("{}2", ch));

                if scale.get_scale_impl().scale_type() == "linear" && is_positional {
                    // Use the method that gathers radius information
                    let data_expressions_with_radius =
                        self.plot.gather_scale_domain_expressions_with_radius(
                            name,
                            configured_non_positional,
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
                    let data_expressions = self.plot.gather_scale_domain_expressions(name)?;
                    if !data_expressions.is_empty() {
                        scale = scale.domain_data_fields(data_expressions);
                    }
                }
            } else {
                // No radius context - use standard domain gathering
                let data_expressions = self.plot.gather_scale_domain_expressions(name)?;
                if !data_expressions.is_empty() {
                    scale = scale.domain_data_fields(data_expressions);
                }
            }
        }

        // Step 2: Apply default range if it's a coordinate channel
        if let Some((start, end)) = self.plot.get_coordinate_default_range(
            name,
            plot_area_width as f64,
            plot_area_height as f64,
        ) {
            scale = scale.range_interval(
                datafusion::logical_expr::lit(start),
                datafusion::logical_expr::lit(end),
            );
        }

        // Step 3: Infer domain from data if needed
        if matches!(
            &scale.domain.default_domain,
            crate::scales::ScaleDefaultDomain::DomainExprs(_)
        ) {
            // Infer domain from data
            scale = scale
                .infer_domain_from_data(plot_area_width, plot_area_height)
                .await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(plot_area_width, plot_area_height)
            .await?;

        // Step 5: Apply mark-specific range if not a position channel AND no range is set
        // Position channels already have their ranges set in Step 2
        let is_position = self
            .plot
            .coord_system()
            .required_channels()
            .contains(&name.as_ref());

        // Check if scale already has a user-specified range
        // The default range is [0, 1], so check if it's been customized from that
        let has_user_range = match scale.get_range() {
            crate::scales::ScaleRange::Color(_) => true, // Custom color range
            crate::scales::ScaleRange::Enum(_) => true,  // Custom discrete values
            crate::scales::ScaleRange::Numeric(start, end) => {
                // Check if it's not the default [0, 1] range
                use datafusion::logical_expr::Expr;
                use datafusion_common::ScalarValue;
                let is_default = match (start, end.as_ref()) {
                    (
                        Expr::Literal(ScalarValue::Float64(Some(v1)), _),
                        Expr::Literal(ScalarValue::Float64(Some(v2)), _),
                    ) => (*v1 - 0.0).abs() < 0.001 && (*v2 - 1.0).abs() < 0.001,
                    (
                        Expr::Literal(ScalarValue::Float32(Some(v1)), _),
                        Expr::Literal(ScalarValue::Float32(Some(v2)), _),
                    ) => (*v1 as f64 - 0.0).abs() < 0.001 && (*v2 as f64 - 1.0).abs() < 0.001,
                    _ => false,
                };
                !is_default
            }
        };

        if !is_position && !has_user_range {
            // Find the first mark that uses this channel
            for mark in &self.plot.marks {
                if mark.data_context().channels().contains_key(name) {
                    // Get data type from the channel expression
                    // First resolve channel references
                    let channels = mark.data_context().channels();
                    let resolved_channels =
                        crate::channel_resolution::resolve_all_channel_refs(channels)
                            .ok()
                            .unwrap_or_else(|| channels.clone());

                    let data_type = resolved_channels
                        .get(name)
                        .and_then(|channel_value| channel_value.expr())
                        .and_then(|expr| {
                            // Try to get data type from mark's dataframe
                            let df = mark
                                .data_context()
                                .dataframe()
                                .or(self.plot.data.as_ref())?;
                            use datafusion::logical_expr::ExprSchemable;
                            expr.get_type(df.schema()).ok()
                        });

                    if let Some(dt) = data_type {
                        if let Some(mark_range) =
                            mark.default_channel_range(name, scale.scale_impl.scale_type(), &dt)
                        {
                            scale = scale.range(mark_range);
                            break;
                        }
                    }
                }
            }
        }

        // Step 6: Create ConfiguredScale
        scale
            .create_configured_scale(plot_area_width, plot_area_height)
            .await
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
