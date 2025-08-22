//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

use crate::cartesian::Cartesian;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelValue, Mark};
use crate::plot::Plot;
use crate::scales::Scale;
use crate::utils::ScalarValueHelpers;
use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::arrow::array::ArrayRef;
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::prelude::DataFrame;
use datafusion_common::ScalarValue;
use indexmap::IndexMap;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, trace};

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

/// Result of layout computation, containing padding and optional Taffy layout
#[derive(Debug, Clone)]
pub struct LayoutSolution {
    /// Padding around the plot area
    pub padding: Padding,
    /// The actual plot area rectangle (x, y, width, height)
    pub plot_area: (f32, f32, f32, f32),
    /// Optional Taffy layout result for dynamic positioning
    pub taffy_layout: Option<crate::chart_layout::LayoutResult>,
    /// Cache of pre-created legend groups for exact sizing
    pub legend_cache: Option<LegendCache>,
}

impl LayoutSolution {
    /// Create a simple layout solution with just padding
    pub fn from_padding(padding: Padding, total_width: f32, total_height: f32) -> Self {
        let plot_area = (
            padding.left,
            padding.top,
            total_width - padding.left - padding.right,
            total_height - padding.top - padding.bottom,
        );
        Self {
            padding,
            plot_area,
            taffy_layout: None,
            legend_cache: None,
        }
    }

    /// Check if this solution uses dynamic layout
    pub fn has_dynamic_layout(&self) -> bool {
        self.taffy_layout.is_some()
    }

    /// Get the plot area dimensions
    pub fn plot_area_bounds(&self) -> (f32, f32, f32, f32) {
        self.plot_area
    }
}

/// Cache for pre-created legend scene groups
/// Ensures exact matching between measurement and rendering
#[derive(Debug, Clone, Default)]
pub struct LegendCache {
    /// Maps channel names to their pre-created legend scene groups
    legends: HashMap<String, SceneGroup>,
}

impl LegendCache {
    /// Create a new empty legend cache
    pub fn new() -> Self {
        Self {
            legends: HashMap::new(),
        }
    }

    /// Add a legend to the cache
    pub fn insert(&mut self, channel: String, legend: SceneGroup) {
        self.legends.insert(channel, legend);
    }

    /// Get a legend from the cache
    pub fn get(&self, channel: &str) -> Option<&SceneGroup> {
        self.legends.get(channel)
    }

    /// Iterate over all cached legends
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SceneGroup)> {
        self.legends.iter()
    }

    /// Get a mutable reference to a legend from the cache
    pub fn get_mut(&mut self, channel: &str) -> Option<&mut SceneGroup> {
        self.legends.get_mut(channel)
    }

    /// Remove and return a legend from the cache
    pub fn take(&mut self, channel: &str) -> Option<SceneGroup> {
        self.legends.remove(channel)
    }
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
    scales: &'a HashMap<String, avenger_scales::scales::ConfiguredScale>, // All available scales
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
            if let Some(taffy_layout) = &layout.taffy_layout {
                all_marks.extend(Self::create_debug_layout_rects(taffy_layout));
            }
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
            let scale = self.plot.get_scale(channel);
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
                    if channel_value.scale_name(channel_name).is_none() {
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
            ChannelValue::Identity { expr } => {
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

        // Call the specialized Cartesian layout helper
        // This will only work if the axes are actually CartesianAxis
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
        // Apply user axis customizations before getting layout axes
        let mut customized_axes = axes;
        for (channel, axis_spec) in &self.plot.axis_specs {
            if let Some(base_axis) = customized_axes.get(channel).cloned() {
                match axis_spec {
                    crate::plot::AxisSpec::Local(f) => {
                        let customized = f(base_axis);
                        customized_axes.insert(channel.clone(), customized);
                    }
                    crate::plot::AxisSpec::Reference(_) => {
                        // Reference axes not yet supported
                    }
                }
            }
        }

        // Check for required positional scales before measuring overflow
        // This ensures we provide proper error messages for literal values
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the coordinate system's guides need
        let overflow = self
            .plot
            .coord_system()
            .measure_guide_overflow(customized_axes, scales, width, height)
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
        let (padding, layout_bundle) = self
            .compute_layout_with_overflow(width, height, scales, overflow)
            .await?;

        Ok(LayoutSolution {
            padding,
            plot_area: Self::calculate_plot_area_from_padding(&padding, width, height),
            taffy_layout: layout_bundle.as_ref().map(|(layout, _)| layout.clone()),
            legend_cache: layout_bundle.map(|(_, cache)| cache),
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
        let legend_marks = if let (Some(taffy_layout), Some(legend_cache)) =
            (&layout.taffy_layout, &layout.legend_cache)
        {
            self.create_legends_with_layout(
                scales,
                taffy_layout,
                legend_cache,
                plot_area_width,
                plot_area_height,
            )
            .await?
        } else {
            self.create_legends(scales, plot_area_width, plot_area_height, &layout.padding)
                .await?
        };

        // Create title
        let title_marks = if let Some(taffy_layout) = &layout.taffy_layout {
            if let Some(title_bounds) = &taffy_layout.title {
                self.create_title(
                    width,
                    &layout.padding,
                    Some(*title_bounds),
                    Some(taffy_layout.plot_area),
                )?
            } else {
                Vec::new()
            }
        } else {
            self.create_title(width, &layout.padding, None, None)?
        };

        // Create subtitle
        let subtitle_marks = if let Some(taffy_layout) = &layout.taffy_layout {
            if let Some(subtitle_bounds) = &taffy_layout.subtitle {
                self.create_subtitle(
                    width,
                    &layout.padding,
                    Some(*subtitle_bounds),
                    Some(taffy_layout.plot_area),
                )?
            } else {
                Vec::new()
            }
        } else {
            self.create_subtitle(width, &layout.padding, None, None)?
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
        // Get the data - either from mark or inherit from plot
        let df_ref = match mark.data_source() {
            crate::marks::DataSource::Explicit => mark.data_context().dataframe(),
            crate::marks::DataSource::Inherited => {
                // Get plot-level data
                Some(self.plot.data.as_ref().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Mark expects inherited data but plot has no data".to_string(),
                    )
                })?)
            }
        };

        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references (e.g., ":x" -> actual x expression)
        let channels = crate::channel_resolution::resolve_all_channel_refs(channels)?;

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
            // No data - return error
            return Err(AvengerChartError::InternalError(
                "Mark requires data but none available".to_string(),
            ));
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
        legend_cache: &LegendCache,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get default legends for channels with data-driven scales
        let default_legends = self.create_default_legends(scales);

        // Combine existing legends with defaults
        let mut all_legends = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legends.entry(channel).or_insert(default_legend);
        }

        // Create legend marks positioned according to layout
        let mut legend_marks = Vec::new();

        for (channel, legend) in &all_legends {
            if !legend.visible {
                continue;
            }

            // Get layout bounds for this legend
            if let Some(bounds) = layout.legends.get(channel) {
                // Determine legend type and create it
                let scale = scales.get(channel).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Scale for channel '{}' not found",
                        channel
                    ))
                })?;
                let legend_type = self.determine_legend_type(channel, scale);

                let params = LegendParams {
                    channel,
                    legend,
                    scales,
                    plot_width: bounds.width,
                    plot_height: bounds.height,
                    padding: &Padding {
                        left: 0.0,
                        right: 0.0,
                        top: 0.0,
                        bottom: 0.0,
                    },
                    legend_margin: 0.0,
                    y_offset: 0.0,
                };

                // Prefer cached legend to guarantee measurement/render match
                if let Some(group) = legend_cache.get(channel) {
                    legend_marks.push(SceneMark::Group(group.clone()));
                } else {
                    let legend_group = match legend_type {
                        LegendType::Symbol => self.create_symbol_legend(params).await?,
                        LegendType::Line => self.create_line_legend(params).await?,
                        LegendType::Colorbar => self.create_colorbar_legend(params).await?,
                    };
                    if let Some(mut group) = legend_group {
                        // For symbol legends, shift down slightly to account for stroke extending beyond bounds
                        let y_offset = if matches!(legend_type, LegendType::Symbol) {
                            // The stroke width is 1.0 by default for symbol legends
                            1.0
                        } else {
                            0.0
                        };
                        group.origin = [bounds.x, bounds.y + y_offset];
                        legend_marks.push(SceneMark::Group(group));
                    }
                }
            }
        }

        Ok(legend_marks)
    }

    async fn create_legends(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
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
            let position = legend
                .position
                .unwrap_or(crate::legend::LegendPosition::Right);
            match position {
                crate::legend::LegendPosition::Right => right_legends.push((channel, legend)),
                crate::legend::LegendPosition::Left => left_legends.push((channel, legend)),
                crate::legend::LegendPosition::Top => top_legends.push((channel, legend)),
                crate::legend::LegendPosition::Bottom => bottom_legends.push((channel, legend)),
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

    /// Determine the type of legend to create based on mark type, channel, and scale
    fn determine_legend_type(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> LegendType {
        // Check if scale is continuous (for colorbar)
        let scale_type = scale.scale_impl.scale_type();
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

        // Extract domain values from scale using extension trait
        use crate::scales::{ConfiguredScaleLegendExt, DomainValues};

        let legend_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;
        let domain_values = match legend_scale.domain_values()? {
            DomainValues::Discrete(values) => {
                // Debug: log discrete values
                debug!(
                    channel = params.channel,
                    scale_type = ?legend_scale.scale_impl.scale_type(),
                    values = ?values,
                    "Symbol legend domain values"
                );
                values
            }
            DomainValues::Interval(min, max) => vec![min, max],
        };
        if domain_values.is_empty() {
            return Ok(None);
        }

        // Create text labels - use special labels for threshold scales
        let text_values: Vec<String> = if legend_scale.scale_impl.scale_type() == "threshold" {
            // Use interval labels for threshold scales
            let labels = legend_scale.domain_labels()?;
            debug!(
                channel = params.channel,
                labels = ?labels,
                "Threshold scale legend labels"
            );
            labels
        } else {
            // Regular labels from domain values
            domain_values
                .iter()
                .map(|v| match v {
                    datafusion_common::ScalarValue::Utf8(Some(s)) => s.clone(),
                    datafusion_common::ScalarValue::Float64(Some(f)) => {
                        // Format float nicely - remove trailing zeros
                        if f.fract() == 0.0 && f.abs() < 1e10 {
                            format!("{:.0}", f)
                        } else {
                            format!("{}", f)
                        }
                    }
                    datafusion_common::ScalarValue::Float32(Some(f)) => {
                        if f.fract() == 0.0 && f.abs() < 1e10 {
                            format!("{:.0}", f)
                        } else {
                            format!("{}", f)
                        }
                    }
                    datafusion_common::ScalarValue::Int64(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int32(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int16(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int8(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt64(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt32(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt16(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt8(Some(i)) => i.to_string(),
                    _ => format!("{:?}", v), // Fallback for other types
                })
                .collect()
        };

        // Check if any mark is a rect mark
        let has_rect_mark = self
            .plot
            .marks
            .iter()
            .any(|mark| mark.mark_type() == "rect");

        // Get mark defaults - use rect defaults if we have rect marks, otherwise symbol defaults
        // Cartesian specialization only for type clarity; no direct use
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
            // Use smaller size for legend to match symbol legends
            (64.0, "square".to_string(), 0.0, fill, stroke, stroke_width)
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
        debug!(
            channel = params.channel,
            text_values = ?text_values,
            default_size = default_size,
            scale_type = ?legend_scale.scale_impl.scale_type(),
            "Creating symbol legend with inner_width: 0.0, inner_height: 100.0, outer_margin: 0.0, text_padding: 2.0"
        );
        let mut config = SymbolLegendConfig {
            title: params.legend.title.clone(),
            text: ScalarOrArray::new_array(text_values),
            inner_width: 0.0, // Don't offset internally, we'll position the whole group
            inner_height: 100.0, // Will be calculated by legend
            outer_margin: 0.0, // Don't offset legend entries
            text_padding: 2.0, // Consistent padding
            ..Default::default()
        };

        // Apply legend background styling if provided
        if let Some(pad) = params.legend.background_padding {
            config.background_padding = Some(pad);
            trace!(padding = pad, "Symbol legend padding set");
        } else {
            trace!("Symbol legend padding: None (will use default)");
        }
        if let Some(r) = params.legend.background_corner_radius {
            config.background_corner_radius = Some(r);
        }
        if let Some(ref fill_str) = params.legend.background_fill {
            if let Some(color) = parse_color_string(fill_str) {
                config.background_fill = Some(color);
            }
        }
        if let Some(ref stroke_str) = params.legend.background_stroke {
            if let Some(color) = parse_color_string(stroke_str) {
                config.background_stroke = Some(color);
            }
        }

        // Analyze mark encodings to determine how to set each channel
        // We'll look at all marks to find symbol or rect marks and check their encodings
        let mut mark_encodings = HashMap::new();
        for mark in &self.plot.marks {
            // Check if this is a symbol or rect mark by checking the mark type
            let mark_type = mark.mark_type();
            let is_relevant = mark_type == "symbol" || mark_type == "rect";
            if is_relevant {
                let channels = mark.data_context().channels();
                for (channel, value) in channels {
                    mark_encodings.insert(channel.clone(), value.clone());
                }
            }
        }

        // Each legend only shows its own channel varying - no cross-channel variation

        // Shape channel
        let default_shape_parsed = parse_shape(&default_shape)?;
        config.shape = ScalarOrArray::new_scalar(default_shape_parsed);

        if params.channel == "shape" {
            // Shape is the legend channel - map domain values to shapes
            // Try to get shapes from the scale's range; if not present, fall back to default shapes
            let shape_names = {
                let names = legend_scale.extract_shape_range();
                if !names.is_empty() {
                    names
                } else {
                    crate::scales::shape_defaults::DEFAULT_SHAPES
                        .iter()
                        .map(|&s| s.to_string())
                        .collect()
                }
            };

            let shapes: Result<Vec<_>, _> = domain_values
                .iter()
                .enumerate()
                .map(|(i, _)| parse_shape(&shape_names[i % shape_names.len()]))
                .collect();
            config.shape = ScalarOrArray::new_array(shapes?);
        } else if let Some(channel_value) = mark_encodings.get("shape") {
            // Shape channel exists but this legend is not for shape
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(ScalarValue::Utf8(Some(s))) = scalars.into_iter().next() {
                        config.shape = ScalarOrArray::new_scalar(parse_shape(&s)?);
                    }
                }
            }
            // Otherwise keep the default shape - don't vary it
        }

        // Size channel
        config.size = ScalarOrArray::new_scalar(default_size);
        if params.channel == "shape" {
            trace!(default_size = default_size, "Initial size set to default");
        }

        if params.channel == "size" {
            // Size is the legend channel - map through scale
            let sizes = legend_scale.map_values_numeric(&domain_values)?;
            config.size = ScalarOrArray::new_array(sizes);
        } else if let Some(channel_value) = mark_encodings.get("size") {
            // Size channel exists but this legend is not for size
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it and scale it through the size scale
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(value) = scalars.into_iter().next() {
                        // If there's a size scale, map the value through it
                        // Otherwise use the raw value (capped for shape legends)
                        let scaled_size = if let Some(size_scale) = params.scales.get("size") {
                            // Map the size value through the scale
                            let size_values = vec![value];
                            if let Ok(scaled) = size_scale.map_values_numeric(&size_values) {
                                scaled.first().copied().unwrap_or(default_size)
                            } else {
                                default_size
                            }
                        } else {
                            // No size scale, use the raw value
                            value.as_f32().ok().unwrap_or(default_size)
                        };

                        // For shape legends, cap the size to a reasonable maximum
                        let legend_size = if params.channel == "shape" {
                            scaled_size.min(49.0) // Use default size as max for shape legends
                        } else {
                            scaled_size
                        };
                        config.size = ScalarOrArray::new_scalar(legend_size);
                    }
                }
            }
            // Otherwise keep the default size - don't vary it
        }

        if params.channel == "shape" {
            let sizes = config.size.as_vec(3, None);
            trace!(sizes = ?sizes, "After size logic");
        }

        // Fill channel
        let default_fill_color = parse_color_string(&default_fill).unwrap_or(
            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
        );
        config.fill = ScalarOrArray::new_scalar(default_fill_color.clone());

        if params.channel == "fill" || params.channel == "color" {
            // Fill/color is the legend channel - map through scale
            let colors = if legend_scale.scale_impl.scale_type() == "threshold" {
                // For threshold scales, get the range colors directly
                // The range has one more color than the domain has thresholds
                legend_scale.range_colors()?
            } else {
                legend_scale.map_values_colors(&domain_values)?
            };
            config.fill = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );
        } else if let Some(channel_value) = mark_encodings.get("fill") {
            // Fill channel exists but this legend is not for fill
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Ok(color_array) = ScalarValue::iter_to_array(scalars.iter().cloned()) {
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

        // Stroke channel
        let default_stroke_color = parse_color_string(&default_stroke).unwrap_or(
            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
        );
        config.stroke = ScalarOrArray::new_scalar(default_stroke_color.clone());

        if params.channel == "stroke" {
            // Stroke is the legend channel - map through scale
            let colors = if legend_scale.scale_impl.scale_type() == "threshold" {
                // For threshold scales, get the range colors directly
                legend_scale.range_colors()?
            } else {
                legend_scale.map_values_colors(&domain_values)?
            };
            config.stroke = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );
        } else if let Some(channel_value) = mark_encodings.get("stroke") {
            // Stroke channel exists but this legend is not for stroke
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Ok(color_array) = ScalarValue::iter_to_array(scalars.iter().cloned()) {
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

        // Stroke width channel - start with default
        config.stroke_width = Some(default_stroke_width);
        if params.channel == "shape" {
            trace!(stroke_width = default_stroke_width, "Stroke width set");
        }

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
            let angles = legend_scale.map_values_numeric(&domain_values)?;
            config.angle = ScalarOrArray::new_array(angles);
        } else if let Some(channel_value) = mark_encodings.get("angle") {
            // Angle channel exists but this legend is not for angle
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(value) = scalars.into_iter().next().and_then(|s| s.as_f32().ok()) {
                        config.angle = ScalarOrArray::new_scalar(value);
                    }
                }
            }
        }

        // Create the legend marks
        if params.channel == "shape" {
            let sizes = config.size.as_vec(3, None);
            trace!(sizes = ?sizes, "Final config.size for shape");
        }
        let mut legend_group = make_symbol_legend(&config)?;

        // Position the legend
        let x = params.padding.left + params.plot_width + params.legend_margin;
        let y = params.padding.top + params.y_offset;

        // Update position
        legend_group.origin = [x, y];
        legend_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(legend_group))
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

    /// Create a line legend
    async fn create_line_legend(
        &self,
        params: LegendParams<'_>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_common::value::ScalarOrArray;
        use avenger_guides::legend::line::{LineLegendConfig, make_line_legend};
        use std::collections::HashMap;

        // Extract domain values from scale using extension trait
        use crate::scales::{ConfiguredScaleLegendExt, DomainValues};

        let legend_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;

        let domain_values = match legend_scale.domain_values()? {
            DomainValues::Discrete(values) => values,
            DomainValues::Interval(min, max) => vec![min, max],
        };
        if domain_values.is_empty() {
            return Ok(None);
        }

        // Create text labels - use special labels for threshold scales
        let text_values: Vec<String> = if legend_scale.scale_impl.scale_type() == "threshold" {
            // Use interval labels for threshold scales
            let labels = legend_scale.domain_labels()?;
            debug!(
                channel = params.channel,
                labels = ?labels,
                "Threshold scale legend labels"
            );
            labels
        } else {
            // Regular labels from domain values
            domain_values
                .iter()
                .map(|v| match v {
                    datafusion_common::ScalarValue::Utf8(Some(s)) => s.clone(),
                    datafusion_common::ScalarValue::Float64(Some(f)) => {
                        // Format float nicely - remove trailing zeros
                        if f.fract() == 0.0 && f.abs() < 1e10 {
                            format!("{:.0}", f)
                        } else {
                            format!("{}", f)
                        }
                    }
                    datafusion_common::ScalarValue::Float32(Some(f)) => {
                        if f.fract() == 0.0 && f.abs() < 1e10 {
                            format!("{:.0}", f)
                        } else {
                            format!("{}", f)
                        }
                    }
                    datafusion_common::ScalarValue::Int64(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int32(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int16(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::Int8(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt64(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt32(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt16(Some(i)) => i.to_string(),
                    datafusion_common::ScalarValue::UInt8(Some(i)) => i.to_string(),
                    _ => format!("{:?}", v), // Fallback for other types
                })
                .collect()
        };

        // Get mark defaults from a default line mark instance
        use crate::cartesian::Cartesian;
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

        // Get stroke_cap and stroke_join from line marks
        let (stroke_cap, stroke_join) = {
            let mut cap = avenger_common::types::StrokeCap::Round; // Default to round
            let mut join = avenger_common::types::StrokeJoin::Round; // Default to round

            // Find the first line mark and use its stroke_cap/stroke_join settings
            for mark in &self.plot.marks {
                if mark.mark_type() == "line" {
                    // Try to get stroke_cap from mark's default channel values
                    if let Some(cap_value) = mark.default_channel_value("stroke_cap") {
                        if let Ok(cap_str) = cap_value.as_scalar_string() {
                            cap = match cap_str.as_str() {
                                "butt" => avenger_common::types::StrokeCap::Butt,
                                "round" => avenger_common::types::StrokeCap::Round,
                                "square" => avenger_common::types::StrokeCap::Square,
                                _ => cap,
                            };
                        }
                    }
                    // Try to get stroke_join from mark's default channel values
                    if let Some(join_value) = mark.default_channel_value("stroke_join") {
                        if let Ok(join_str) = join_value.as_scalar_string() {
                            join = match join_str.as_str() {
                                "miter" => avenger_common::types::StrokeJoin::Miter,
                                "round" => avenger_common::types::StrokeJoin::Round,
                                "bevel" => avenger_common::types::StrokeJoin::Bevel,
                                _ => join,
                            };
                        }
                    }
                    break;
                }
            }
            (cap, join)
        };

        // Initialize config with defaults
        // Use longer line length for better dash pattern visibility
        let mut config = LineLegendConfig {
            title: params.legend.title.clone(),
            text: ScalarOrArray::new_array(text_values),
            stroke_cap,
            stroke_join: Some(stroke_join), // Add stroke_join to config
            inner_width: 0.0,
            inner_height: 100.0,
            outer_margin: 0.0, // Don't offset legend entries
            line_length: ScalarOrArray::new_scalar(16.0), // Default, will be adjusted for dash patterns
            text_padding: 4.0,                            // Consistent with symbol legend
            ..Default::default()
        };

        // Apply legend background styling if provided
        if let Some(pad) = params.legend.background_padding {
            config.background_padding = Some(pad);
            trace!(
                channel = params.channel,
                padding = pad,
                "Line legend setting padding"
            );
        } else {
            trace!(
                channel = params.channel,
                "Line legend has no padding specified, will use default"
            );
        }
        if let Some(r) = params.legend.background_corner_radius {
            config.background_corner_radius = Some(r);
        }
        if let Some(ref fill_str) = params.legend.background_fill {
            if let Some(color) = parse_color_string(fill_str) {
                config.background_fill = Some(color);
            }
        }
        if let Some(ref stroke_str) = params.legend.background_stroke {
            if let Some(color) = parse_color_string(stroke_str) {
                config.background_stroke = Some(color);
            }
        }

        // Analyze mark encodings to determine how to set each channel
        // We'll look at all marks to find line marks and check their encodings
        let mut mark_encodings = HashMap::new();
        for mark in &self.plot.marks {
            // Check if this is a line mark by checking the mark type
            let mark_type = mark.mark_type();
            if mark_type == "line" {
                let channels = mark.data_context().channels();
                for (channel, value) in channels {
                    mark_encodings.insert(channel.clone(), value.clone());
                }
            }
        }

        // Each legend only shows its own channel varying - no cross-channel variation

        // Set stroke color based on whether it varies with the legend channel
        if params.channel == "stroke" {
            // Legend is for stroke itself - vary stroke color
            // Always map through the scale for the legend channel
            let colors = legend_scale.map_values_colors(&domain_values)?;
            config.stroke = ScalarOrArray::new_array(
                colors
                    .into_iter()
                    .map(avenger_common::types::ColorOrGradient::Color)
                    .collect(),
            );

            // Don't vary dash patterns in stroke legend - each legend only shows its own channel
        } else if let Some(channel_value) = mark_encodings.get("stroke") {
            // Stroke channel exists but this legend is not for stroke
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Constant expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(ScalarValue::Utf8(Some(color_str))) = scalars.into_iter().next() {
                        if let Some(color) = parse_color_string(&color_str) {
                            config.stroke = ScalarOrArray::new_scalar(color);
                        }
                    }
                }
            } else {
                // Stroke references columns, use a default color for non-stroke legends
                // For stroke_width and stroke_dash legends, we need a visible default color
                if params.channel == "stroke_width" || params.channel == "stroke_dash" {
                    let color = avenger_common::types::ColorOrGradient::Color([0.2, 0.2, 0.2, 1.0]); // Dark gray
                    config.stroke = ScalarOrArray::new_scalar(color);
                } else {
                    // For other legends, use the parsed default
                    let color = parse_color_string(&default_stroke).unwrap_or(
                        avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                    );
                    config.stroke = ScalarOrArray::new_scalar(color);
                }
            }
        } else {
            // No stroke channel in mark encodings - use default stroke color
            let color = parse_color_string(&default_stroke).unwrap_or(
                avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            );
            config.stroke = ScalarOrArray::new_scalar(color);
        }

        // Set stroke width based on whether it varies with the legend channel
        if params.channel == "stroke_width" {
            // Legend is for stroke_width itself - vary width
            // Always map through the scale for the legend channel
            let widths = legend_scale.map_values_numeric(&domain_values)?;
            config.stroke_width = ScalarOrArray::new_array(widths);
        } else if let Some(channel_value) = mark_encodings.get("stroke_width") {
            // Stroke width channel exists but this legend is not for stroke_width
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Constant expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(value) = scalars.into_iter().next().and_then(|s| s.as_f32().ok()) {
                        config.stroke_width = ScalarOrArray::new_scalar(value);
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
            let dash_patterns = legend_scale.map_dash_patterns(&domain_values);

            debug!(
                channel = params.channel,
                domain_values = ?domain_values,
                dash_patterns = ?dash_patterns,
                "Line legend dash patterns"
            );

            // Don't vary stroke colors - each legend only shows its own channel varying

            // Use 32 as the target legend length - all patterns are designed to align at this length
            let max_legend_length = 32.0;

            // Now calculate optimal length for each pattern
            let mut individual_lengths = Vec::new();

            for (i, pattern) in dash_patterns.iter().enumerate() {
                let optimal_length = if let Some(pattern) = pattern.as_ref() {
                    if pattern.is_empty() {
                        // Solid line - should be exactly the same as max length
                        max_legend_length
                    } else {
                        // Calculate how many complete dash segments fit within max_legend_length
                        let mut current_pos = 0.0;
                        let mut last_valid_length = 0.0;
                        let mut is_dash = true; // Start with a dash segment
                        let mut pattern_idx = 0;

                        // Simulate drawing the pattern
                        while current_pos < max_legend_length {
                            let segment_length = pattern[pattern_idx];
                            let next_pos = current_pos + segment_length;

                            if next_pos > max_legend_length {
                                // This segment would exceed our limit
                                break;
                            }

                            if is_dash {
                                // This is a dash segment - update our valid length
                                last_valid_length = next_pos;
                            }

                            current_pos = next_pos;
                            is_dash = !is_dash;
                            pattern_idx = (pattern_idx + 1) % pattern.len();
                        }

                        // Make sure we show at least some pattern
                        if last_valid_length == 0.0 && !pattern.is_empty() {
                            last_valid_length = pattern[0]; // At least show first dash
                        }

                        last_valid_length
                    }
                } else {
                    // No pattern (solid line)
                    max_legend_length
                };

                individual_lengths.push(optimal_length);

                trace!(
                    index = i,
                    pattern = ?pattern,
                    length = optimal_length,
                    max_length = max_legend_length,
                    "Dash pattern"
                );
            }

            trace!(max_legend_length = max_legend_length, "Max legend length");

            // Add some extra for rounded caps if used
            let cap_extension = if stroke_cap == avenger_common::types::StrokeCap::Round {
                default_stroke_width // Add stroke width for rounded caps at both ends
            } else {
                0.0
            };

            // Set individual lengths for each pattern
            config.line_length = ScalarOrArray::new_array(
                individual_lengths
                    .into_iter()
                    .map(|l| l + cap_extension)
                    .collect(),
            );

            // Keep the same stroke width as the chart lines for consistency
            // The default is already set to match the chart

            config.stroke_dash = ScalarOrArray::new_array(dash_patterns);
        } else if let Some(channel_value) = mark_encodings.get("stroke_dash") {
            // Stroke dash channel exists but this legend is not for stroke_dash
            // Only use it if it's a scalar (constant) expression
            if !Self::references_columns(channel_value.expr()) {
                // Constant expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![channel_value.expr().clone()], None, None)
                        .await
                {
                    if let Some(ScalarValue::Utf8(Some(pattern_str))) = scalars.into_iter().next() {
                        let dash = Self::convert_dash_pattern(&pattern_str);
                        config.stroke_dash = ScalarOrArray::new_scalar(dash);
                    }
                }
            }
        } else {
            // Use default (solid)
            config.stroke_dash = ScalarOrArray::new_scalar(None);
        }

        debug!(
            channel = params.channel,
            stroke = ?config.stroke.as_vec(8, None),
            stroke_width = ?config.stroke_width.as_vec(8, None),
            stroke_dash = ?config.stroke_dash.as_vec(8, None),
            line_length = ?config.line_length.as_vec(8, None),
            "Line legend config"
        );

        let mut legend_group = make_line_legend(&config)?;
        let x = params.padding.left + params.plot_width + params.legend_margin;
        let y = params.padding.top + params.y_offset;

        // Update position and add debug stroke
        legend_group.origin = [x, y];
        legend_group.zindex = Some(10);

        Ok(Some(legend_group))
    }

    /// Create a colorbar legend
    async fn create_colorbar_legend(
        &self,
        params: LegendParams<'_>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_guides::legend::colorbar::{
            ColorbarConfig, ColorbarOrientation, make_colorbar_marks,
        };

        // Get the ConfiguredScale for this channel
        let configured_scale = params.scales.get(params.channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Scale for channel '{}' not found",
                params.channel
            ))
        })?;

        // Determine colorbar dimensions
        // When using Taffy layout, params.plot_height is the allocated height
        // We should use this directly as the total colorbar height
        let colorbar_height = params.plot_height;
        let colorbar_width = params.legend.gradient_thickness.unwrap_or(15.0) as f32;

        let mut config = ColorbarConfig {
            orientation: ColorbarOrientation::Right,
            dimensions: [params.plot_width, params.plot_height], // Available space for the colorbar
            colorbar_width: Some(colorbar_width),
            colorbar_height: Some(colorbar_height),
            colorbar_margin: Some(0.0), // No margin - align exactly with axis
            format_number: params.legend.format_number.clone(),
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
        };

        // Apply legend background styling if provided
        if let Some(pad) = params.legend.background_padding {
            config.background_padding = Some(pad);
        }
        if let Some(r) = params.legend.background_corner_radius {
            config.background_corner_radius = Some(r);
        }
        if let Some(ref fill_str) = params.legend.background_fill {
            if let Some(color) = parse_color_string(fill_str) {
                config.background_fill = Some(color);
            }
        }
        if let Some(ref stroke_str) = params.legend.background_stroke {
            if let Some(color) = parse_color_string(stroke_str) {
                config.background_stroke = Some(color);
            }
        }

        // Create the colorbar marks at origin [0, 0] (will be positioned by group origin)
        let plot_origin = [0.0, 0.0];
        let title = params.legend.title.as_deref().unwrap_or("");

        let mut colorbar_group =
            make_colorbar_marks(configured_scale, title, plot_origin, &config)?;

        // Adjust vertical position for multiple legends
        if params.y_offset > 0.0 {
            // Offset the colorbar marks vertically
            colorbar_group.origin[1] += params.y_offset;
        }

        // Set z-index
        colorbar_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(colorbar_group))
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
            // Fallback: left-aligned at top with small margin
            (10.0, 16.0)
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

    /// Compute layout using Taffy for Cartesian coordinate system
    /// Convert overflow requirements to pseudo-axes for Taffy layout
    async fn compute_layout_with_overflow(
        &self,
        width: f32,
        height: f32,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        overflow: crate::coords::OverflowSpaceRequirement,
    ) -> Result<
        (
            Padding,
            Option<(crate::chart_layout::LayoutResult, LegendCache)>,
        ),
        AvengerChartError,
    > {
        use crate::chart_layout::ChartLayout;

        // Get default legends for channels with ConfiguredScale
        let default_legends = self.create_default_legends(configured_scales);
        let mut all_legends = self.plot.legends.clone();
        for (channel, default_legend) in default_legends {
            all_legends.entry(channel).or_insert(default_legend);
        }

        // Filter to visible legends that have corresponding scales
        let visible_legends: Vec<_> = all_legends
            .iter()
            .filter(|(channel, legend)| legend.visible && configured_scales.contains_key(*channel))
            .map(|(channel, legend)| (channel.clone(), legend.clone()))
            .collect();

        // Create legends map preserving order
        let legends_map: IndexMap<String, crate::legend::Legend> =
            visible_legends.clone().into_iter().collect();

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

        // Build legend cache - for now return empty cache
        // TODO: Implement proper legend creation in the overflow-based layout
        let legend_cache = LegendCache::new();

        Ok((padding, Some((layout_result, legend_cache))))
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
            // Compute range hint for positional scales using coordinate system
            let range_hint = self.plot.coord_system().default_range(
                name,
                plot_area_width as f64,
                plot_area_height as f64,
            );

            scale = scale.infer_domain_from_data(range_hint).await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(plot_area_width, plot_area_height)
            .await?;

        // Step 5: Create ConfiguredScale
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
