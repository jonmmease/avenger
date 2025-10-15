//! Polar coordinate system guide implementation

use crate::coords::extract_channel_title_from_marks;
use crate::error::AvengerChartError;
use crate::guide::{CompiledGuide, CoordinateGuide, GuideUpdate, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::maybe::{Maybe, MaybeOptionalExpr};
use crate::plot::compiled::expr_eval::evaluate_string_expr;
use crate::polar::{PolarAxis, PolarAxisType};
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::collections::HashMap;
use std::sync::Arc;

/// Options for polar coordinate system
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolarOptions {
    /// Background color for the plot area
    #[serde_as(as = "MaybeOptionalExpr")]
    pub plot_background_color: Maybe<Option<LogicalExprNode>>,
}

/// Guide for Polar coordinate system
///
/// Combines:
/// - Axes configured at the channel level (r, theta)
/// - Coordinate-level options (start angle, clockwise, inner radius)
#[derive(Clone, Serialize, Deserialize)]
pub struct PolarGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, PolarAxis>,
    /// Coordinate-system-level options
    pub options: PolarOptions,
    /// Channel titles extracted from mark renderers
    pub channel_titles: HashMap<String, String>,
}

impl std::fmt::Debug for PolarGuide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolarGuide")
            .field("axes", &self.axes)
            .field("options", &self.options)
            .field("channel_titles", &self.channel_titles)
            .finish()
    }
}

impl PolarGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: PolarOptions::default(),
            channel_titles: HashMap::new(),
        }
    }

    /// Configure coordinate-level options
    pub fn with_options(mut self, options: PolarOptions) -> Self {
        self.options = options;
        self
    }

    /// Set the plot background color
    pub fn plot_background_color(mut self, color: impl crate::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = color.into_expr();
        self.options.plot_background_color = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr)
                .expect("Failed to serialize plot_background_color expr"),
        ));
        self
    }
}

impl Default for PolarGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl PolarGuide {
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

impl GuideUpdate for PolarGuide {
    fn update(self, other: Self) -> Self {
        self.update(other)
    }
}

impl CoordinateGuide for PolarGuide {
    type Axis = PolarAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn crate::marks::CompiledMark>>,
        session_context: &datafusion::prelude::SessionContext,
    ) {
        // Extract titles from mark renderers immediately
        for channel in ["r", "theta"] {
            if let Some(title) =
                extract_channel_title_from_marks(&compiled_marks, channel, session_context)
            {
                self.channel_titles.insert(channel.to_string(), title);
            }
        }
    }

    fn update(&mut self, other: Self) {
        *self = PolarGuide::update(self.clone(), other);
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for PolarGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        use avenger_geometry::marks::MarkGeometryUtils;

        // For overflow measurement, we can place the plot at origin
        let initial_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        };

        // Evaluate axes to measure their bounding box with actual params
        let axis_marks = self
            .evaluate(
                scales,
                plot_width,
                plot_height,
                &initial_bounds,
                theme,
                params,
                ctx,
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

        // Polar coordinate systems are centered, so calculate overflow from center
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;
        let radius = plot_width.min(plot_height) / 2.0;

        // Calculate the circular bounds
        let circle_left = center_x - radius;
        let circle_right = center_x + radius;
        let circle_top = center_y - radius;
        let circle_bottom = center_y + radius;

        // Calculate overflow relative to circular bounds
        const THRESHOLD: f32 = 1.0;
        let left = (circle_left - min_x).max(0.0);
        let right = (max_x - circle_right).max(0.0);
        let top = (circle_top - min_y).max(0.0);
        let bottom = (max_y - circle_bottom).max(0.0);

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
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        _ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Render background circle if specified (behind everything else)
        // First try to evaluate expression if set
        use crate::serialization::LogicalExprNodeExt;

        let bg_color = if let Some(color_node) = self
            .options
            .plot_background_color
            .as_option()
            .and_then(|o| o.as_ref())
        {
            if let Ok(color_expr) = color_node.to_expr(_ctx) {
                // Evaluate the expression to get color string
                if let Ok(color_str) = evaluate_string_expr(&color_expr, _ctx, params).await {
                    // Parse the color string
                    crate::utils::parse_color_to_array_strict(&color_str).ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            // Fallback to theme
            let guide_ctx =
                crate::theme::ThemeContext::new("guide", params.clone()).with_subtype("polar");
            theme
                .query(&guide_ctx, "background-color")
                .and_then(|v| v.as_color_array())
        };

        if let Some(bg_color) = bg_color {
            use avenger_common::types::ColorOrGradient;
            use avenger_common::value::ScalarOrArray;
            use avenger_scenegraph::marks::arc::SceneArcMark;

            // Calculate center and radius
            let center_x = plot_bounds.x + plot_width / 2.0;
            let center_y = plot_bounds.y + plot_height / 2.0;
            let radius = plot_width.min(plot_height) / 2.0;

            // Create a full circle for background
            let bg_circle = SceneArcMark {
                name: "plot-background".to_string(),
                clip: false,
                len: 1,
                gradients: Vec::new(),
                x: ScalarOrArray::new_scalar(center_x),
                y: ScalarOrArray::new_scalar(center_y),
                start_angle: ScalarOrArray::new_scalar(0.0),
                end_angle: ScalarOrArray::new_scalar(2.0 * std::f32::consts::PI),
                outer_radius: ScalarOrArray::new_scalar(radius),
                inner_radius: ScalarOrArray::new_scalar(0.0),
                pad_angle: ScalarOrArray::new_scalar(0.0),
                corner_radius: ScalarOrArray::new_scalar(0.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(bg_color)),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                stroke_width: ScalarOrArray::new_scalar(0.0),
                indices: None,
                zindex: Some(-2), // Behind grid lines (which are at -1)
            };
            marks.push(SceneMark::Arc(bg_circle));
        }

        // Create default axes at render time with full scale information
        let mut default_axes = HashMap::new();
        for (channel_name, _scale) in scales {
            if channel_name == "r" || channel_name == "theta" {
                let axis_type = match channel_name.as_str() {
                    "r" => PolarAxisType::Radial,
                    "theta" => PolarAxisType::Angular,
                    _ => continue,
                };

                let mut axis = PolarAxis::new().axis_type(axis_type).grid(true); // Polar axes should show grid by default

                // Use previously extracted title if available
                if let Some(title) = self.channel_titles.get(channel_name) {
                    axis = axis.title(title.clone());
                }

                default_axes.insert(channel_name.clone(), axis);
            }
        }

        // Merge with user-configured axes
        let mut all_axes = default_axes;

        // Apply user configurations on top of defaults
        for (channel, user_axis) in &self.axes {
            if let Some(default_axis) = all_axes.get_mut(channel) {
                *default_axis = default_axis.clone().update(user_axis.clone());
            } else {
                all_axes.insert(channel.clone(), user_axis.clone());
            }
        }

        // Render each axis
        for (channel, axis) in &all_axes {
            if let Some(scale) = scales.get(channel) {
                let axis_marks = axis
                    .evaluate(
                        channel,
                        scale,
                        scales,
                        plot_width,
                        plot_height,
                        plot_bounds,
                        theme,
                        params,
                        _ctx,
                    )
                    .await?;
                marks.extend(axis_marks);
            }
        }

        Ok(marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        use avenger_scenegraph::marks::group::Clip;

        // Polar coordinates use circular clipping
        let radius = plot_width.min(plot_height) / 2.0;
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Create a circular clip path
        let mut builder = lyon_path::Path::builder();
        builder.add_circle(
            lyon_path::geom::point(center_x, center_y),
            radius,
            lyon_path::Winding::Positive,
        );
        let path = builder.build();

        Clip::Path(path)
    }
}
