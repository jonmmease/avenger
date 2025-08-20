use crate::axis::{AxisPosition, AxisTrait, CartesianAxis};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::Expr;
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

#[async_trait::async_trait]
pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The axis type for this coordinate system
    type Axis: AxisTrait + Clone + 'static;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Whether this coordinate system supports dynamic layout (e.g., Taffy)
    /// Default is false for backward compatibility
    fn supports_dynamic_layout(&self) -> bool {
        false
    }

    /// Get default range for a specific position channel based on inner plot dimensions
    /// Returns the range as a tuple of (start, end) values
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)>;

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

    /// Create default axes for all channels that have scales
    ///
    /// # Arguments
    /// * `scales` - The scale registry containing all configured scales
    /// * `marks` - The marks in the plot, used to extract column names for titles
    ///
    /// # Returns
    /// A map of channel names to default axis configurations
    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized;

    /// Convert axes to CartesianAxis if this is a Cartesian coordinate system
    /// Returns None for non-Cartesian systems
    fn axes_as_cartesian(
        _axes: HashMap<String, Self::Axis>,
    ) -> Option<HashMap<String, CartesianAxis>> {
        None // Default implementation returns None
    }

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
    async fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

pub struct Cartesian;

#[async_trait::async_trait]
impl CoordinateSystem for Cartesian {
    type Axis = CartesianAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn supports_dynamic_layout(&self) -> bool {
        true
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "x" => Some((0.0, width)),
            "y" => Some((height, 0.0)), // Inverted for screen coords
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

    fn axes_as_cartesian(
        axes: HashMap<String, Self::Axis>,
    ) -> Option<HashMap<String, CartesianAxis>> {
        // For Cartesian, Self::Axis = CartesianAxis, so we can just return the axes
        Some(axes)
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();

        // Always create default axes for x and y channels if they have scales
        // User axes will be merged with these defaults later
        for channel in ["x", "y"] {
            if scales.get(channel).is_some() {
                // Extract title from mark encodings
                let title = extract_axis_title_from_marks(marks, channel)
                    .unwrap_or_else(|| channel.to_string());

                // Determine if grid should be enabled based on scale type
                let grid = if let Some(scale) = scales.get(channel) {
                    matches!(
                        scale.scale_impl.scale_type(),
                        "linear" | "log" | "pow" | "sqrt" | "time"
                    )
                } else {
                    false
                };

                // Create axis with appropriate default position
                let position = match channel {
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => AxisPosition::Bottom,
                };

                let axis = CartesianAxis::new()
                    .position(position)
                    .label_angle(0.0)
                    .title(title)
                    .grid(grid);

                default_axes.insert(channel.to_string(), axis);
            }
        }

        default_axes
    }

    async fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
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

            // Axis origin is always the top-left corner of the plot area
            let axis_origin = [padding.left, padding.top];

            // Create axis config with plot dimensions
            let axis_config = AxisConfig {
                orientation,
                dimensions: [plot_width, plot_height],
                grid: axis.grid,
                format_number: axis.format_number.clone(),
                title_font_size: None, // Use default for regular axes
            };

            // Use the already configured scale
            let configured_scale = scale;

            // Generate axis marks based on scale type
            let scale_type = configured_scale.scale_impl.scale_type();

            let axis_group = match scale_type {
                "band" | "point" => make_band_axis_marks(
                    configured_scale,
                    axis.title.as_deref().unwrap_or(""),
                    axis_origin,
                    &axis_config,
                )?,
                _ => {
                    // Default to numeric axis for linear and other continuous scales
                    make_numeric_axis_marks(
                        configured_scale,
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
        if let Some(channel_value) = mark.data_context().channels().get(channel) {
            // Try to get column name
            if let Some(col_name) = channel_value.as_column_name() {
                return Some(col_name);
            }
        }
    }
    None
}

use crate::axis::PolarAxis;

pub struct Polar {
    // Fields for center injection from renderer
    pub(crate) center_x: Option<Expr>,
    pub(crate) center_y: Option<Expr>,
}

impl Polar {
    pub fn new() -> Self {
        Self {
            center_x: None,
            center_y: None,
        }
    }

    /// Set the center coordinates for polar transformation
    /// This is called by the renderer to inject the calculated center
    pub(crate) fn with_center(mut self, cx: Expr, cy: Expr) -> Self {
        self.center_x = Some(cx);
        self.center_y = Some(cy);
        self
    }
}

impl Default for Polar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CoordinateSystem for Polar {
    type Axis = PolarAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn supports_dynamic_layout(&self) -> bool {
        false // Will be enabled in Milestone 4
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "theta" => Some((0.0, 2.0 * std::f64::consts::PI)),
            "r" => {
                let max_radius = f64::min(width, height) / 2.0;
                Some((0.0, max_radius))
            }
            _ => None,
        }
    }

    fn transform_expressions(
        &self,
        mut channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError> {
        use datafusion::logical_expr::lit;
        
        // Get required channels
        let r = channels
            .remove("r")
            .ok_or_else(|| AvengerChartError::MissingChannelError("r".to_string()))?;

        let theta = channels
            .remove("theta")
            .ok_or_else(|| AvengerChartError::MissingChannelError("theta".to_string()))?;

        // Use injected center if available, otherwise use defaults
        // The renderer will provide proper center based on plot dimensions
        let cx = self.center_x.clone().unwrap_or_else(|| lit(250.0));
        let cy = self.center_y.clone().unwrap_or_else(|| lit(250.0));

        // Transform to cartesian coordinates with center offset
        // x = cx + r * cos(theta)
        // y = cy + r * sin(theta)
        let x = cx + r.clone() * cos(theta.clone());
        let y = cy + r * sin(theta);

        Ok(TransformResult { x, y, depth: None })
    }

    fn create_default_axes(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        // For Milestone 1, return empty - no axes rendered yet
        // This will be implemented in Milestone 2
        HashMap::new()
    }

    async fn render_axes(
        &self,
        _axes: &HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // For Milestone 1, no axis rendering
        // Will be implemented in Milestone 2 for grid and Milestone 3 for full axes
        Ok(Vec::new())
    }
}
