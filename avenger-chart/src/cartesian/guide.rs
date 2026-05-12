//! Cartesian coordinate system guide implementation
use std::{collections::HashMap, sync::Arc};

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark, rect::SceneRectMark};
use datafusion::{
    dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
    scalar::ScalarValue as DFScalarValue,
};
use datafusion_common::ScalarValue;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{
    cartesian::axis::{AxisPosition, CartesianAxis},
    coords::{CoordMeasurement, EmptyCoordMeasurement, extract_channel_title_from_marks},
    error::AvengerChartError,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    guide::{
        CompiledGuide, CoordinateGuide, FacetDirection, GuideUpdate, OverflowSpaceRequirement,
        UnifiableChannelInfo,
    },
    layout::LayoutBounds,
    marks::CompiledMark,
    maybe::{Maybe, MaybeOptionalExpr},
    plot::{IntoExpr, compiled::expr_eval::evaluate_string_expr},
    serialization::LogicalExprNodeExt,
    theme::{Theme, ThemeContext},
    utils::parse_color_to_array_strict,
};

/// Options for Cartesian coordinate system (beyond axes)
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CartesianOptions {
    /// Background color for the plot area
    #[serde_as(as = "MaybeOptionalExpr")]
    pub plot_background_color: Maybe<Option<LogicalExprNode>>,
}

/// Guide for Cartesian coordinate system
///
/// Combines:
/// - Axes configured at the channel level (x, y)
/// - Coordinate-level options (background color)
#[derive(Clone, Serialize, Deserialize)]
pub struct CartesianGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, CartesianAxis>,
    /// Coordinate-system-level options
    pub options: CartesianOptions,
    /// Channel titles extracted from mark renderers
    pub channel_titles: HashMap<String, String>,
}

impl std::fmt::Debug for CartesianGuide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CartesianGuide")
            .field("axes", &self.axes)
            .field("options", &self.options)
            .field("channel_titles", &self.channel_titles)
            .finish()
    }
}

impl CartesianGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: CartesianOptions::default(),
            channel_titles: HashMap::new(),
        }
    }

    /// Configure coordinate-level options
    pub fn with_options(mut self, options: CartesianOptions) -> Self {
        self.options = options;
        self
    }

    /// Set the plot background color
    pub fn plot_background_color(mut self, color: impl IntoExpr) -> Self {
        let expr = color.into_expr();
        self.options.plot_background_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr)
                .expect("Failed to serialize plot_background_color expr"),
        ));
        self
    }
}

impl Default for CartesianGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl CartesianGuide {
    /// Get plot background color from options or theme
    async fn get_background_color(
        &self,
        theme: &Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Option<[f32; 4]> {
        // Try to evaluate expression if set
        if let Some(color_node) = self
            .options
            .plot_background_color
            .as_option()
            .and_then(|o| o.as_ref())
            && let Ok(color_expr) = color_node.to_expr(ctx)
        {
            // Evaluate the expression to get color string
            if let Ok(color_str) = evaluate_string_expr(&color_expr, ctx, params).await {
                // Parse the color string
                if let Ok(color) = parse_color_to_array_strict(&color_str) {
                    return Some(color);
                }
            }
        }

        // Fallback to theme
        let guide_ctx = ThemeContext::new("guide", params.clone()).with_subtype("cartesian");
        theme
            .query(&guide_ctx, "background-color")
            .and_then(|v| v.as_color_array())
    }

    /// Update this guide with values from another guide
    pub fn update(mut self, other: Self) -> Self {
        // Merge axes - other's axes take precedence
        for (channel, axis) in other.axes {
            match self.axes.get(&channel) {
                Some(existing) => {
                    // Update existing axis with new configuration
                    let updated = existing.clone().update(axis);
                    self.axes.insert(channel, updated);
                }
                None => {
                    // Add new axis
                    self.axes.insert(channel, axis);
                }
            }
        }

        // Update options - other's options take precedence when set
        if other.options.plot_background_color.is_set() {
            self.options.plot_background_color = other.options.plot_background_color;
        }

        self
    }
}

impl GuideUpdate for CartesianGuide {
    fn update(self, other: Self) -> Self {
        CartesianGuide::update(self, other)
    }
}

impl CoordinateGuide for CartesianGuide {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        session_context: &SessionContext,
    ) {
        // Extract titles from mark renderers immediately
        for channel in ["x", "y"] {
            if let Some(title) =
                extract_channel_title_from_marks(&compiled_marks, channel, session_context)
            {
                self.channel_titles.insert(channel.to_string(), title);
            }
        }
    }

    fn update(&mut self, other: Self) {
        *self = std::mem::take(self).update(other);
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for CartesianGuide {
    /// Measure how much space this guide needs outside the plot area
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Use provided coord_measurement or default empty one (CartesianGuide doesn't use it)
        let empty_coord = EmptyCoordMeasurement;
        let coord_measurement = coord_measurement.unwrap_or(&empty_coord);

        // For overflow measurement, we can place the plot at origin
        let initial_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        };
        let guide_overflow = OverflowSpaceRequirement::default();

        // Evaluate axes to measure their bounding box with actual params
        // Use facet_tree and facet_path for visibility-aware overflow measurement
        let axis_marks = self
            .evaluate(
                scales,
                plot_width,
                plot_height,
                &initial_bounds,
                &guide_overflow,
                theme,
                params,
                ctx,
                data_override,
                facet_tree,
                facet_path,
                coord_measurement,
            )
            .await?;

        // Calculate bounding box of all axis marks
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        for mark in &axis_marks {
            let bbox = mark.bounding_box();
            let lower = bbox.lower();
            let upper = bbox.upper();
            min_x = min_x.min(lower[0]);
            max_x = max_x.max(upper[0]);
            min_y = min_y.min(lower[1]);
            max_y = max_y.max(upper[1]);
        }

        // Get the actual scale ranges to measure overflow against
        let x_scale = scales.get("x");
        let y_scale = scales.get("y");

        // Calculate scale boundaries (plot is at origin for measurement)
        let (scale_left, scale_right) = if let Some(x_scale) = x_scale {
            let x_range = x_scale.numeric_interval_range()?;
            (x_range.0.min(x_range.1), x_range.0.max(x_range.1))
        } else {
            (0.0, plot_width)
        };

        let (scale_top, scale_bottom) = if let Some(y_scale) = y_scale {
            let y_range = y_scale.numeric_interval_range()?;
            (y_range.0.min(y_range.1), y_range.0.max(y_range.1))
        } else {
            (0.0, plot_height)
        };

        // Calculate overflow relative to scale boundaries
        const THRESHOLD: f32 = 1.0; // Ignore overflows less than 1px
        let left = (scale_left - min_x).max(0.0);
        let right = (max_x - scale_right).max(0.0);
        let top = (scale_top - min_y).max(0.0);
        let bottom = (max_y - scale_bottom).max(0.0);

        // Round very small overflows to zero
        let left = if left < THRESHOLD { 0.0 } else { left };
        let right = if right < THRESHOLD { 0.0 } else { right };
        let top = if top < THRESHOLD { 0.0 } else { top };
        let bottom = if bottom < THRESHOLD { 0.0 } else { bottom };

        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        })
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Render background if specified (behind everything else)
        if let Some(bg_color) = self.get_background_color(theme, params, ctx).await {
            let bg_rect = SceneRectMark {
                name: "plot-background".to_string(),
                clip: false,
                len: 1,
                gradients: Vec::new(),
                x: ScalarOrArray::new_scalar(plot_bounds.x),
                y: ScalarOrArray::new_scalar(plot_bounds.y),
                width: Some(ScalarOrArray::new_scalar(plot_width)),
                height: Some(ScalarOrArray::new_scalar(plot_height)),
                x2: None,
                y2: None,
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(bg_color)),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                stroke_width: ScalarOrArray::new_scalar(0.0),
                corner_radius: ScalarOrArray::new_scalar(0.0),
                indices: None,
                zindex: Some(-2), // Behind grid lines (which are at -1)
            };
            marks.push(SceneMark::Rect(bg_rect));
        }

        // Create default axes for all channels with scales at render time
        let mut default_axes = HashMap::new();
        for channel_name in scales.keys() {
            if channel_name == "x" || channel_name == "y" {
                // Set default position based on channel
                let position = if channel_name == "x" {
                    AxisPosition::Bottom
                } else {
                    AxisPosition::Left
                };

                // Determine if grid should be enabled based on scale type
                let grid = scales
                    .get(channel_name)
                    .is_some_and(|s| s.ticks(None).is_ok());

                let mut axis = CartesianAxis::new()
                    .position(position)
                    .visible(true)
                    .grid(grid);

                // Use previously extracted title if available
                // Note: unified channel checking removed - visibility will be redesigned
                if let Some(title) = self.channel_titles.get(channel_name) {
                    axis = axis.title(title.clone());
                }

                default_axes.insert(channel_name.clone(), axis);
            }
        }

        // Merge with user-configured axes
        let mut all_axes = default_axes;

        // Apply user configurations on top of defaults
        // Note: unified channel checking removed - visibility will be redesigned
        for (channel, user_axis) in &self.axes {
            let axis_to_apply = user_axis.clone();

            if let Some(default_axis) = all_axes.get_mut(channel) {
                *default_axis = std::mem::take(default_axis).update(axis_to_apply);
            } else {
                all_axes.insert(channel.clone(), axis_to_apply);
            }
        }

        // Render each axis
        for (channel, axis) in &all_axes {
            if let Some(scale) = scales.get(channel) {
                // Get sharing level for this channel from the facet tree.
                // The facet tree stores sharing levels extracted from innermost marks.
                // Keep this value intact for ownership/title calculations.
                let sharing_level = facet_tree.channel_domain_sharing_level_typed(channel);
                let axis_mark = axis
                    .evaluate(
                        channel,
                        scale,
                        plot_width,
                        plot_height,
                        plot_bounds,
                        theme,
                        params,
                        ctx,
                        facet_tree,
                        facet_path,
                        sharing_level.raw(),
                    )
                    .await?;
                marks.push(axis_mark);
            }
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        // Cartesian coordinates use a rectangular clip
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn facet_unifiable_channel(
        &self,
        facet_direction: FacetDirection,
        marks: &[Arc<dyn CompiledMark>],
        session_context: &SessionContext,
    ) -> Option<UnifiableChannelInfo> {
        // Cartesian can unify y-axis in row faceting, x-axis in column faceting
        let channel = match facet_direction {
            FacetDirection::Row => "y",
            FacetDirection::Column => "x",
        };

        // Extract the title from the marks
        let title = extract_channel_title_from_marks(marks, channel, session_context);

        Some(UnifiableChannelInfo {
            channel: channel.to_string(),
            title,
        })
    }

    fn axis_position(&self, channel: &str) -> Option<AxisPosition> {
        // Check if we have an axis configured for this channel
        if let Some(axis) = self.axes.get(channel) {
            // If axis has explicit position expression, try to extract it if it's a simple literal
            if let Some(position_node) = axis.position.as_option().and_then(|o| o.as_ref()) {
                // Try to convert to datafusion Expr and check if it's a literal
                if let Ok(expr) = position_node.to_expr(&SessionContext::new())
                    && let Expr::Literal(DFScalarValue::Utf8(Some(pos_str)), _) = expr
                {
                    // Got a literal string, parse it as an axis position
                    return match pos_str.to_lowercase().as_str() {
                        "top" => Some(AxisPosition::Top),
                        "bottom" => Some(AxisPosition::Bottom),
                        "left" => Some(AxisPosition::Left),
                        "right" => Some(AxisPosition::Right),
                        _ => None,
                    };
                }
                // Has position expression but can't extract it - return None for fallback
                None
            } else {
                // No explicit position, use defaults based on channel name
                match channel {
                    "x" => Some(AxisPosition::Bottom),
                    "y" => Some(AxisPosition::Left),
                    _ => Some(AxisPosition::Bottom),
                }
            }
        } else {
            // No axis configured for this channel, use defaults
            match channel {
                "x" => Some(AxisPosition::Bottom),
                "y" => Some(AxisPosition::Left),
                _ => None,
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
