//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

mod guide;

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use crate::render_context::RenderContext;
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
    pub(crate) plot: &'a Plot<C>,
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

        // Create initial RenderContext with estimated dimensions
        let theme = self.plot.get_theme();
        let initial_context =
            RenderContext::new(theme.clone(), estimated_plot_width, estimated_plot_height);

        let (initial_scales, configured_non_positional, configured_positional) =
            self.build_initial_scales(&initial_context).await?;

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
        // Create final RenderContext with actual plot dimensions
        let final_context = RenderContext::new(theme.clone(), plot_area_width, plot_area_height);

        let final_configured_scales = self
            .rebuild_scales_with_final_dimensions(
                &initial_scales,
                &configured_non_positional,
                &final_context,
            )
            .await?;

        // STAGE 4: RENDER ALL COMPONENTS WITH FINAL SCALES
        let all_component_marks = self
            .render_all_components(&final_configured_scales, &layout, width, height)
            .await?;

        let (mark_groups, guide_marks, legend_marks, title_marks, subtitle_marks) =
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

        // Add background rect if theme specifies one
        let theme = self.plot.get_theme();
        if let Some(bg_color) = theme.canvas_background() {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            // Parse the color string to RGBA - fail if color is invalid
            let color = crate::utils::parse_color_to_array_strict(&bg_color)?;

            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(width.into()),
                height: Some(height.into()),
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
                crate::cartesian::axis::AxisPosition::Left => {
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
                crate::cartesian::axis::AxisPosition::Right => {
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
                crate::cartesian::axis::AxisPosition::Top => {
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
                crate::cartesian::axis::AxisPosition::Bottom => {
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

    /// Render a single mark to scene marks using the new Mark trait
    /// Build initial scales with estimated dimensions
    /// Returns (raw_scales, configured_non_positional, configured_positional)
    async fn build_initial_scales(
        &self,
        context: &RenderContext,
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
                .build_configured_scale_with_radius_context(scale.clone(), name, context, None)
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
                    context,
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
        // Create guide with all configurations applied
        let guide = self.create_configured_guide(scales);

        // Call the dynamic layout helper with the guide
        self.compute_layout_with_dynamic_guide(width, height, scales, guide)
            .await
    }

    /// Helper method for dynamic layout with guide
    async fn compute_layout_with_dynamic_guide(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        guide: C::Guide,
    ) -> Result<LayoutSolution, AvengerChartError> {
        // Check for required positional scales before measuring overflow
        // This ensures we provide proper error messages for literal values
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the guide needs
        let width_estimate = width * INITIAL_PLOT_AREA_RATIO;
        let height_estimate = height * INITIAL_PLOT_AREA_RATIO;
        let overflow = self
            .measure_guide_overflow(&guide, scales, width_estimate, height_estimate)
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
        context: &RenderContext,
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
                        context,
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

        // Create guide marks (axes, grids, backgrounds)
        let guide_marks = self
            .create_guide_marks(scales, plot_area_width, plot_area_height, &layout.padding)
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
            guide_marks,
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
        let channels = crate::channel::resolution::resolve_all_channel_refs(channels)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                !expr.column_refs().is_empty()
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    !condition.column_refs().is_empty() || !value.expr().column_refs().is_empty()
                }) || !otherwise.expr().column_refs().is_empty()
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

        // Check if mark has a sorting channel and apply sorting if needed
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    // Apply sorting transformation
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales)?;

                    // Sort the DataFrame by the sorting expression
                    let sorted_df = df_ref.clone().sort(vec![sort_expr.sort(true, false)])?;
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

        // Create render context with theme and dimensions
        let theme = self.plot.get_theme();
        let context = RenderContext::new(theme, plot_width, plot_height);

        // Call the mark's render_from_data method with context and coordinate system
        mark.render_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            self.plot.coord_system(),
        )
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
        use crate::channel::value::strip_trailing_numbers;

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
                use crate::channel::{ConditionalValue, value::strip_trailing_numbers};
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

    /// Create guide marks (axes, grids, backgrounds, and other visual guides)
    async fn create_guide_marks(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Create guide with all configurations applied
        let guide = self.create_configured_guide(scales);

        // Render the guide using the coordinate system
        let theme = self.plot.get_theme();
        self.plot
            .coord_system()
            .render_guide(&guide, scales, plot_width, plot_height, padding, &theme)
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
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales);

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
                    let mark_opt = self.plot.marks.get(primary_channel.mark_index);

                    mark_opt
                        .and_then(|mark| mark.preferred_merged_legend_renderer(&channels, &scales))
                } else {
                    // Single channel - use the standard renderer selection
                    scales.get(&primary_channel.name).and_then(
                        |scale| -> Option<Arc<dyn crate::legend::LegendRenderer>> {
                            if let Some(ref renderer) = legend.renderer {
                                // Use explicitly configured renderer
                                Some(renderer.clone())
                            } else {
                                // Find the mark and get its preference
                                self.plot
                                    .marks
                                    .get(primary_channel.mark_index)
                                    .and_then(|mark| {
                                        mark.preferred_legend_renderer(
                                            &primary_channel.channel_type,
                                            scale,
                                        )
                                    })
                            }
                        },
                    )
                };

                // Skip this legend group if no renderer is available
                let Some(renderer) = renderer_opt else {
                    continue;
                };

                // Render the legend with all channels in the group
                if let Some(group) = renderer.render(
                    &channels,
                    legend,
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                )? {
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

        // Add channels that marks indicate shouldn't have legends
        // (by returning None from preferred_legend_renderer)
        for mark in &self.plot.marks {
            for (channel, scale) in scales {
                if mark.preferred_legend_renderer(channel, scale).is_none() {
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
            if self.plot.legends.contains_key(channel) {
                continue;
            }

            // ConfiguredScale always has resolved domain, so always create a legend
            // for channels that have scales
            let theme = self.plot.get_theme();
            let mut legend = Legend::new()
                .title(self.infer_legend_title(channel))
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
            legend.title_color = Some(theme.legend_title_color());
            legend.label_color = Some(theme.legend_label_color());
            legend.title_font_family = Some(theme.legend_title_font_family());
            legend.title_font_size = Some(theme.legend_title_font_size());
            legend.title_font_weight = Some(theme.legend_title_font_weight());
            legend.label_font_family = Some(theme.legend_label_font_family());
            legend.label_font_size = Some(theme.legend_label_font_size());
            legend.label_font_weight = Some(theme.legend_label_font_weight());
            legend.tick_font_family = Some(theme.legend_tick_font_family());
            legend.tick_font_size = Some(theme.legend_tick_font_size());
            legend.tick_font_weight = Some(theme.legend_tick_font_weight());
            legend.tick_color = Some(theme.legend_tick_color());

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(&self, channel: &str) -> String {
        // First try to extract from marks (like we do for axes)
        use crate::coords::extract_channel_title_from_marks;
        if let Some(title) = extract_channel_title_from_marks(&self.plot.marks, channel) {
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

        // Get theme for typography
        let theme = self.plot.get_theme();

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
            font: title
                .font_family
                .clone()
                .unwrap_or_else(|| theme.title_font_family().to_string())
                .into(),
            font_size: title.font_size.unwrap_or(theme.title_font_size()).into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.title_font_weight()).into(),
            color: crate::utils::parse_color_string(&theme.title_color())
                .unwrap_or(ColorOrGradient::Color([0.102, 0.102, 0.102, 1.0]))
                .into(),
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

        // Get theme for typography
        let theme = self.plot.get_theme();

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
            font: subtitle
                .font_family
                .clone()
                .unwrap_or_else(|| theme.subtitle_font_family().to_string())
                .into(),
            font_size: subtitle
                .font_size
                .unwrap_or(theme.subtitle_font_size())
                .into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.subtitle_font_weight())
                .into(),
            color: crate::utils::parse_color_string(&theme.subtitle_color())
                .unwrap_or(ColorOrGradient::Color([0.290, 0.290, 0.290, 1.0]))
                .into(),
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
        Vec<Vec<crate::legend::LegendChannel>>,
        IndexMap<String, crate::legend::Legend>,
    ) {
        use crate::legend::{LegendChannel, MergeKey};
        use std::collections::HashMap;

        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for (mark_index, mark) in self.plot.marks.iter().enumerate() {
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
                use crate::legend::ChannelInfo;
                let mut related_channels = HashMap::new();
                for (other_name, other_value) in mark.data_context().channels() {
                    if other_name != channel_name {
                        // Check if this channel has a scale or is constant
                        let channel_info =
                            if let Some(other_scale) = configured_scales.get(other_name) {
                                // Channel has a scale
                                ChannelInfo::Scaled {
                                    expr: other_value.expr().cloned(),
                                    scale: other_scale.clone(),
                                }
                            } else if let Some(expr) = other_value.expr() {
                                // Channel has a constant expression
                                ChannelInfo::Constant { expr: expr.clone() }
                            } else {
                                // Skip channels without expressions
                                continue;
                            };
                        related_channels.insert(other_name.clone(), channel_info);
                    }
                }

                let legend_channel = LegendChannel {
                    name: channel_name.clone(),
                    expression: channel_value.expr().cloned(),
                    scale: scale.clone(),
                    channel_type: channel_name.clone(),
                    mark_type: mark.mark_type().to_string(),
                    mark_index,
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

    /// Get legends with theme applied - used for both layout measurement and rendering
    /// This ensures consistency between measurement and actual rendering
    fn get_legends_with_theme(
        &self,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> indexmap::IndexMap<String, crate::legend::Legend> {
        // Get default legends for channels with ConfiguredScale
        let default_legends = self.create_default_legends(configured_scales);
        let mut all_legends = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legends.entry(channel).or_insert(default_legend);
        }

        // Get theme for legend text colors and typography
        let theme = self.plot.get_theme();

        // Apply theme colors, typography and mark defaults to all legends
        // This ensures accurate text measurement during layout and consistent rendering
        for legend in all_legends.values_mut() {
            // Only set colors if not already explicitly set by user
            if legend.title_color.is_none() {
                legend.title_color = Some(theme.legend_title_color());
            }
            if legend.label_color.is_none() {
                legend.label_color = Some(theme.legend_label_color());
            }
            // Set typography from theme
            if legend.title_font_family.is_none() {
                legend.title_font_family = Some(theme.legend_title_font_family());
            }
            if legend.title_font_size.is_none() {
                legend.title_font_size = Some(theme.legend_title_font_size());
            }
            if legend.title_font_weight.is_none() {
                legend.title_font_weight = Some(theme.legend_title_font_weight());
            }
            if legend.label_font_family.is_none() {
                legend.label_font_family = Some(theme.legend_label_font_family());
            }
            if legend.label_font_size.is_none() {
                legend.label_font_size = Some(theme.legend_label_font_size());
            }
            if legend.label_font_weight.is_none() {
                legend.label_font_weight = Some(theme.legend_label_font_weight());
            }
            // Set tick label typography (for colorbar legends)
            if legend.tick_font_family.is_none() {
                legend.tick_font_family = Some(theme.legend_tick_font_family());
            }
            if legend.tick_font_size.is_none() {
                legend.tick_font_size = Some(theme.legend_tick_font_size());
            }
            if legend.tick_font_weight.is_none() {
                legend.tick_font_weight = Some(theme.legend_tick_font_weight());
            }
            if legend.tick_color.is_none() {
                legend.tick_color = Some(theme.legend_tick_color());
            }
            // Always pass theme mark defaults for legend rendering
            if legend.theme_mark_defaults.is_none() {
                legend.theme_mark_defaults = Some(theme.mark_defaults_map());
            }
        }

        all_legends
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

        // Get legends with theme applied (ensures measurement uses correct fonts)
        let all_legends = self.get_legends_with_theme(configured_scales);

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
            &self.plot.get_theme(),
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
        context: &RenderContext,
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
            // Only use radius-aware gathering for positional scales that support it
            if let Some(configured_non_positional) = configured_non_positional {
                // Check if this is a positional channel (including interval variants)
                let is_positional = self
                    .plot
                    .coord_system()
                    .required_channels()
                    .iter()
                    .any(|&ch| name == ch || name == format!("{}2", ch));

                if scale.get_scale_impl().supports_radius_expansion() && is_positional {
                    // Use the method that gathers radius information
                    let data_expressions_with_radius =
                        self.plot.gather_scale_domain_expressions_with_radius(
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
            context.plot_width as f64,
            context.plot_height as f64,
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
                .infer_domain_from_data(context.plot_width, context.plot_height)
                .await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(context.plot_width, context.plot_height)
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
            crate::scales::ScaleRange::Discrete(_) => true, // Custom discrete values
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
                        crate::channel::resolution::resolve_all_channel_refs(channels)
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
                        let theme = self.plot.get_theme();
                        // Convert domain to ResolvedDomain
                        let resolved_domain = scale.get_domain().to_resolved()?;
                        if let Some(mark_range) = mark.default_channel_range(
                            name,
                            scale.scale_impl.as_ref(),
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

        // Step 6: Create ConfiguredScale
        scale
            .create_configured_scale(context.plot_width, context.plot_height)
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
