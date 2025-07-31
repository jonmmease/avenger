use crate::axis::{AxisPosition, AxisTrait, CartesianAxis};
use crate::error::AvengerChartError;
use crate::scales::ScaleRange;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::{Expr, lit};
use std::collections::HashMap;

/// Result of coordinate transformation
#[derive(Debug, Clone)]
pub struct TransformResult {
    /// X coordinate expression in screen space
    pub x: Expr,
    /// Y coordinate expression in screen space
    pub y: Expr,
    /// Optional depth/z-order expression for 3D effects or layering
    pub depth: Option<Expr>,
}

pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The axis type for this coordinate system
    type Axis: AxisTrait + Clone + 'static;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Get default range for a specific position channel based on inner plot dimensions
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange>;

    /// Transform position channel expressions to screen coordinates
    ///
    /// # Arguments
    /// * `channels` - Map from channel name (e.g., "r", "theta") to expressions
    ///   that compute the scaled values for those channels
    ///
    /// # Returns
    /// Result containing TransformResult or error if required channels are missing
    fn transform_expressions(
        &self,
        channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError>;

    /// Get default axis configuration for a channel
    /// The index parameter indicates which instance of this channel type this is
    /// (e.g., 0 for primary y-axis, 1 for first alternative y-axis, etc.)
    fn default_axis(channel: &str, index: usize) -> Option<Self::Axis>;

    /// Create default axes for channels that have scales but no explicit axis configuration
    ///
    /// # Arguments
    /// * `scales` - The scale registry containing all configured scales
    /// * `existing_axes` - Map of channel names to existing axis configurations
    /// * `marks` - The marks in the plot, used to extract column names for titles
    ///
    /// # Returns
    /// A map of channel names to default axis configurations
    fn create_default_axes(
        &self,
        scales: &HashMap<String, crate::scales::Scale>,
        existing_axes: &HashMap<String, Self::Axis>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized;

    /// Render all axes for this coordinate system
    ///
    /// # Arguments
    /// * `axes` - Map of all axes (configured + defaults) to render
    /// * `scales` - The scale registry containing all configured scales
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    /// * `padding` - Padding around the plot area
    ///
    /// # Returns
    /// A vector of SceneMark objects representing the rendered axes
    fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, crate::scales::Scale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::layout::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

pub struct Cartesian;

impl CoordinateSystem for Cartesian {
    type Axis = CartesianAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange> {
        match channel {
            "x" => Some(ScaleRange::new_interval(lit(0.0f32), lit(width as f32))),
            "y" => Some(ScaleRange::new_interval(lit(height as f32), lit(0.0f32))), // Inverted for screen coords
            _ => None,
        }
    }

    fn transform_expressions(
        &self,
        mut channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError> {
        // Cartesian is identity transform - just pass through x and y
        let x = channels
            .remove("x")
            .ok_or_else(|| AvengerChartError::MissingChannelError("x".to_string()))?;

        let y = channels
            .remove("y")
            .ok_or_else(|| AvengerChartError::MissingChannelError("y".to_string()))?;

        Ok(TransformResult { x, y, depth: None })
    }

    fn default_axis(channel: &str, index: usize) -> Option<Self::Axis> {
        match channel {
            "x" => Some(
                CartesianAxis::new()
                    .position(if index % 2 == 0 {
                        AxisPosition::Bottom
                    } else {
                        AxisPosition::Top
                    })
                    .label_angle(0.0),
            ),
            "y" => Some(
                CartesianAxis::new()
                    .position(if index % 2 == 0 {
                        AxisPosition::Left
                    } else {
                        AxisPosition::Right
                    })
                    .label_angle(0.0),
            ),
            _ => None,
        }
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, crate::scales::Scale>,
        existing_axes: &HashMap<String, Self::Axis>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();

        // Create default axes for x and y channels if they have scales but no explicit axis
        for channel in ["x", "y"] {
            if scales.get(channel).is_some() && !existing_axes.contains_key(channel) {
                // Extract title from mark encodings
                let title = extract_axis_title_from_marks(marks, channel)
                    .unwrap_or_else(|| channel.to_string());

                // Determine if grid should be enabled based on scale type
                let grid = if let Some(scale) = scales.get(channel) {
                    matches!(
                        scale.get_scale_type(),
                        "linear" | "log" | "pow" | "sqrt" | "time"
                    )
                } else {
                    false
                };

                let axis = CartesianAxis::new().title(title).grid(grid);

                default_axes.insert(channel.to_string(), axis);
            }
        }

        default_axes
    }

    fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, crate::scales::Scale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::layout::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_guides::axis::{
            band::make_band_axis_marks,
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
        };

        let mut axis_marks = Vec::new();

        for (channel, axis) in axes {
            // Skip invisible axes
            if !axis.visible {
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
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => AxisPosition::Bottom,
                }
            });

            // Convert position to orientation
            let orientation = match position {
                AxisPosition::Top => AxisOrientation::Top,
                AxisPosition::Bottom => AxisOrientation::Bottom,
                AxisPosition::Left => AxisOrientation::Left,
                AxisPosition::Right => AxisOrientation::Right,
            };

            // Calculate axis origin based on position
            let axis_origin = match position {
                AxisPosition::Bottom => [padding.left, padding.top + plot_height],
                AxisPosition::Top => [padding.left, padding.top],
                AxisPosition::Left => [padding.left, padding.top],
                AxisPosition::Right => [padding.left + plot_width, padding.top],
            };

            // Create axis config with plot dimensions
            let axis_config = AxisConfig {
                orientation,
                dimensions: [plot_width, plot_height],
                grid: axis.grid,
            };

            // Create configured scale for avenger-guides
            let configured_scale = scale.create_configured_scale(plot_width, plot_height)?;

            // Generate axis marks based on scale type
            let scale_type = scale.get_scale_impl().scale_type();

            let axis_group = match scale_type {
                "band" | "point" => make_band_axis_marks(
                    &configured_scale,
                    axis.title.as_deref().unwrap_or(""),
                    axis_origin,
                    &axis_config,
                )?,
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

        Ok(axis_marks)
    }
}

/// Helper function to extract axis title from mark encodings
fn extract_axis_title_from_marks<C: CoordinateSystem>(
    marks: &[Box<dyn crate::marks::Mark<C>>],
    channel: &str,
) -> Option<String> {
    // Look through marks to find a column name for this channel
    for mark in marks {
        if let Some(channel_value) = mark.data_context().encodings().get(channel) {
            // Try to get column name
            if let Some(col_name) = channel_value.as_column_name() {
                return Some(col_name);
            }
        }
    }
    None
}

pub struct Polar;

impl CoordinateSystem for Polar {
    type Axis = CartesianAxis; // TODO: Implement PolarAxis for proper polar coordinate support

    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange> {
        match channel {
            "theta" => Some(ScaleRange::new_interval(
                lit(0.0f32),
                lit((2.0 * std::f64::consts::PI) as f32),
            )),
            "r" => {
                let max_radius = f64::min(width, height) / 2.0;
                Some(ScaleRange::new_interval(
                    lit(0.0f32),
                    lit(max_radius as f32),
                ))
            }
            _ => None,
        }
    }

    fn transform_expressions(
        &self,
        mut channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError> {
        // Get required channels
        let r = channels
            .remove("r")
            .ok_or_else(|| AvengerChartError::MissingChannelError("r".to_string()))?;

        let theta = channels
            .remove("theta")
            .ok_or_else(|| AvengerChartError::MissingChannelError("theta".to_string()))?;

        // Transform to cartesian coordinates
        // x = r * cos(theta)
        // y = r * sin(theta)
        let x = r.clone() * cos(theta.clone());
        let y = r * sin(theta);

        Ok(TransformResult { x, y, depth: None })
    }

    fn default_axis(channel: &str, _index: usize) -> Option<Self::Axis> {
        match channel {
            "r" => Some(CartesianAxis::new()), // TODO: Implement PolarAxis for radial axis
            "theta" => Some(CartesianAxis::new()), // TODO: Implement PolarAxis for angular axis
            _ => None,
        }
    }

    fn create_default_axes(
        &self,
        _scales: &HashMap<String, crate::scales::Scale>,
        _existing_axes: &HashMap<String, Self::Axis>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        // TODO: Implement default polar axes when PolarAxis is available
        // For now, return empty map to disable automatic axis creation for polar plots
        HashMap::new()
    }

    fn render_axes(
        &self,
        _axes: &HashMap<String, Self::Axis>,
        _scales: &HashMap<String, crate::scales::Scale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &crate::layout::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // TODO: Implement polar axis rendering when PolarAxis is available
        // Polar axes would include:
        // - Circular grid lines for theta
        // - Radial lines from center for r
        // - Labels around the circumference
        Ok(Vec::new())
    }
}
