//! Cartesian coordinate system guide implementation

use crate::cartesian::axis::{AxisPosition, CartesianAxis};
use crate::coords::extract_channel_title_from_marks;
use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuide, GuideUpdate, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::Mark;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;
use std::sync::Arc;

/// Options for Cartesian coordinate system (beyond axes)
#[derive(Clone, Debug)]
pub struct CartesianOptions {
    /// Background color for the plot area
    pub plot_background_color: Option<[f32; 4]>,
}

impl Default for CartesianOptions {
    fn default() -> Self {
        Self {
            plot_background_color: None,
        }
    }
}

/// Guide for Cartesian coordinate system
///
/// Combines:
/// - Axes configured at the channel level (x, y)
/// - Coordinate-level options (background color)
#[derive(Clone, Debug)]
pub struct CartesianGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, CartesianAxis>,
    /// Coordinate-system-level options
    pub options: CartesianOptions,
}

impl CartesianGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: CartesianOptions::default(),
        }
    }

    /// Configure coordinate-level options
    pub fn with_options(mut self, options: CartesianOptions) -> Self {
        self.options = options;
        self
    }

    /// Set the plot background color
    pub fn plot_background_color(mut self, color: [f32; 4]) -> Self {
        self.options.plot_background_color = Some(color);
        self
    }

    /// Create default axes for channels that have scales
    /// This is called during plot construction to create axes for channels
    /// that don't have explicit axis configuration
    pub fn create_default_axes<C>(
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn Mark<C>>],
    ) -> HashMap<String, CartesianAxis>
    where
        C: crate::coords::CoordinateSystem,
    {
        let mut axes = HashMap::new();

        // Create default axes for all position channels that have scales
        for (channel_name, _scale) in scales {
            if channel_name == "x" || channel_name == "y" {
                // Set default position based on channel
                let position = match channel_name.as_str() {
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => AxisPosition::Bottom,
                };

                let mut axis = CartesianAxis::new().position(position);

                // Try to extract a title from the marks
                if let Some(title) = extract_channel_title_from_marks(marks, channel_name) {
                    axis = axis.title(title);
                }

                axes.insert(channel_name.clone(), axis);
            }
        }

        axes
    }
}

impl Default for CartesianGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl CartesianGuide {
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
        if other.options.plot_background_color.is_some() {
            self.options.plot_background_color = other.options.plot_background_color;
        }

        self
    }
}

impl GuideUpdate for CartesianGuide {
    fn update(self, other: Self) -> Self {
        self.update(other)
    }
}

#[async_trait::async_trait]
impl CoordinateGuide for CartesianGuide {
    type Axis = CartesianAxis;

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
        let initial_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        };

        // Render axes to measure their bounding box
        let axis_marks = self
            .render(scales, plot_width, plot_height, &initial_bounds, theme)
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

    async fn render(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &dyn Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Render background if specified (behind everything else)
        if let Some(bg_color) = self.options.plot_background_color {
            use avenger_common::types::ColorOrGradient;
            use avenger_common::value::ScalarOrArray;
            use avenger_scenegraph::marks::rect::SceneRectMark;

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

        // Render each axis
        for (channel, axis) in &self.axes {
            if let Some(scale) = scales.get(channel) {
                let axis_mark =
                    axis.render(channel, scale, plot_width, plot_height, plot_bounds, theme)?;
                marks.push(axis_mark);
            }
        }

        Ok(marks)
    }
}
