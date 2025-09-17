//! Polar coordinate system guide implementation

use crate::coords::extract_channel_title_from_marks;
use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuide, OverflowSpaceRequirement};
use crate::marks::Mark;
use crate::polar::{PolarAxis, PolarAxisType};
use crate::render::Padding;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Options for polar coordinate system
#[derive(Clone, Debug)]
pub struct PolarOptions {
    /// Background color for the plot area
    pub plot_background_color: Option<[f32; 4]>,
}

impl Default for PolarOptions {
    fn default() -> Self {
        Self {
            plot_background_color: None,
        }
    }
}

/// Guide for Polar coordinate system
///
/// Combines:
/// - Axes configured at the channel level (r, theta)
/// - Coordinate-level options (start angle, clockwise, inner radius)
#[derive(Clone, Debug)]
pub struct PolarGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, PolarAxis>,
    /// Coordinate-system-level options
    pub options: PolarOptions,
}

impl PolarGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: PolarOptions::default(),
        }
    }

    /// Configure coordinate-level options
    pub fn with_options(mut self, options: PolarOptions) -> Self {
        self.options = options;
        self
    }

    /// Set the plot background color
    pub fn plot_background_color(mut self, color: [f32; 4]) -> Self {
        self.options.plot_background_color = Some(color);
        self
    }

    /// Create default axes for channels that have scales
    pub fn create_default_axes<C>(
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Box<dyn Mark<C>>],
    ) -> HashMap<String, PolarAxis>
    where
        C: crate::coords::CoordinateSystem,
    {
        let mut axes = HashMap::new();

        // Create default axes for polar position channels that have scales
        for (channel_name, _scale) in scales {
            if channel_name == "r" || channel_name == "theta" {
                let axis_type = match channel_name.as_str() {
                    "r" => PolarAxisType::Radial,
                    "theta" => PolarAxisType::Angular,
                    _ => continue,
                };

                let mut axis = PolarAxis::new().axis_type(axis_type);

                // Try to extract a title from the marks
                if let Some(title) = extract_channel_title_from_marks(marks, channel_name) {
                    axis.title = Some(title);
                }

                axes.insert(channel_name.clone(), axis);
            }
        }

        axes
    }
}

impl Default for PolarGuide {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CoordinateGuide for PolarGuide {
    type Axis = PolarAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn axes(&self) -> &HashMap<String, Self::Axis> {
        &self.axes
    }

    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &dyn Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        use avenger_geometry::marks::MarkGeometryUtils;

        // For overflow measurement, we can place the plot at origin
        let initial_padding = Padding {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };

        // Render axes to measure their bounding box
        let axis_marks = self
            .render(scales, plot_width, plot_height, &initial_padding, theme)
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

    async fn render(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
        theme: &dyn Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Render background circle if specified (behind everything else)
        if let Some(bg_color) = self.options.plot_background_color {
            use avenger_common::types::ColorOrGradient;
            use avenger_common::value::ScalarOrArray;
            use avenger_scenegraph::marks::arc::SceneArcMark;

            // Calculate center and radius
            let center_x = padding.left + plot_width / 2.0;
            let center_y = padding.top + plot_height / 2.0;
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

        // Render each axis using existing render method
        // Note: The polar axis render method needs the channel name and all scales
        for (channel, axis) in &self.axes {
            if let Some(scale) = scales.get(channel) {
                let axis_marks = axis.render(
                    channel,
                    scale,
                    scales,
                    plot_width,
                    plot_height,
                    padding,
                    theme,
                )?;
                marks.extend(axis_marks);
            }
        }

        Ok(marks)
    }
}
