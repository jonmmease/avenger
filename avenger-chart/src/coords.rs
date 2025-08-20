use crate::axis::{AxisPosition, AxisTrait, CartesianAxis};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::Expr;
use std::collections::HashMap;
use std::sync::Arc;

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
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();

        // Create default axes for r and theta channels if they have scales
        for channel in ["r", "theta"] {
            if scales.get(channel).is_some() {
                // Extract title from mark encodings
                let title = extract_axis_title_from_marks(marks, channel)
                    .unwrap_or_else(|| channel.to_string());

                // Create axis with appropriate defaults
                let axis_type = match channel {
                    "r" => crate::axis::PolarAxisType::Radial,
                    "theta" => crate::axis::PolarAxisType::Angular,
                    _ => crate::axis::PolarAxisType::Radial,
                };

                let axis = PolarAxis {
                    visible: true,
                    axis_type,
                    title: Some(title),
                    grid: true, // Enable grid by default for polar
                    tick_count: None, // Will use scale's default
                    format_number: None,
                    grid_levels: None,
                    start_angle: 0.0,
                    direction: crate::axis::PolarDirection::Clockwise,
                };

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
        use avenger_scenegraph::marks::arc::SceneArcMark;
        use avenger_scenegraph::marks::rule::SceneRuleMark;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_common::value::ScalarOrArray;
        use avenger_text::types::{TextAlign, TextBaseline, FontWeight, FontWeightNameSpec, FontStyle};

        let mut axis_marks = Vec::new();

        // Calculate center of plot area
        let center_x = padding.left + plot_width / 2.0;
        let center_y = padding.top + plot_height / 2.0;
        let max_radius = f32::min(plot_width, plot_height) / 2.0;

        // Render radial grid (concentric circles) if r axis is visible and has grid enabled
        if let Some(r_axis) = axes.get("r") {
            if r_axis.visible && r_axis.grid {
                if let Some(r_scale) = scales.get("r") {
                    // Get tick values from the scale
                    let tick_count = r_axis.tick_count.map(|c| c as f32);
                    let ticks = r_scale.ticks(tick_count.or(Some(5.0)))?;
                    let num_ticks = ticks.len();
                    
                    if num_ticks > 0 {
                        // Create concentric circles for each tick value
                        let mut radii = Vec::with_capacity(num_ticks);
                        
                        // Handle different array types for ticks
                        use datafusion::arrow::datatypes::DataType;
                        use datafusion::arrow::array::{Float32Array, Float64Array};
                        
                        let tick_values: Vec<f64> = match ticks.data_type() {
                            DataType::Float64 => {
                                ticks.as_any()
                                    .downcast_ref::<Float64Array>()
                                    .unwrap()
                                    .iter()
                                    .filter_map(|v| v)
                                    .collect()
                            }
                            DataType::Float32 => {
                                ticks.as_any()
                                    .downcast_ref::<Float32Array>()
                                    .unwrap()
                                    .iter()
                                    .filter_map(|v| v.map(|f| f as f64))
                                    .collect()
                            }
                            _ => vec![],
                        };
                        
                        for value in tick_values {
                            // Transform tick value through scale to get radius
                            let tick_array = Arc::new(Float64Array::from(vec![value])) as datafusion::arrow::array::ArrayRef;
                            let scaled_values = r_scale.scale(&tick_array)?;
                            if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float64Array>() {
                                if scaled_array.len() > 0 {
                                    let radius = scaled_array.value(0) as f32;
                                    if radius.is_finite() && radius > 0.0 {
                                        radii.push(radius);
                                    }
                                }
                            } else if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float32Array>() {
                                if scaled_array.len() > 0 {
                                    let radius = scaled_array.value(0);
                                    if radius.is_finite() && radius > 0.0 {
                                        radii.push(radius);
                                    }
                                }
                            }
                        }
                        
                        // Create arc marks for concentric circles if we have any radii
                        if !radii.is_empty() {
                            let grid_arcs = SceneArcMark {
                            name: "polar-radial-grid".to_string(),
                            clip: false,
                            len: radii.len() as u32,
                            gradients: vec![],
                            x: ScalarOrArray::new_scalar(center_x),
                            y: ScalarOrArray::new_scalar(center_y),
                            start_angle: ScalarOrArray::new_scalar(0.0),
                            end_angle: ScalarOrArray::new_scalar(2.0 * std::f32::consts::PI),
                            outer_radius: ScalarOrArray::new_array(radii.clone()),
                            inner_radius: ScalarOrArray::new_array(radii.iter().map(|r| (r - 0.5).max(0.0)).collect()), // Thin circles
                            pad_angle: ScalarOrArray::new_scalar(0.0),
                            corner_radius: ScalarOrArray::new_scalar(0.0),
                            fill: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.8, 0.8, 0.8, 0.3])),
                            stroke: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                            stroke_width: ScalarOrArray::new_scalar(0.0),
                            indices: None,
                            zindex: Some(-1), // Render behind data
                        };
                            
                            axis_marks.push(SceneMark::Arc(grid_arcs));
                        }
                    }
                }
            }
            
            // Add radial axis tick labels
            if r_axis.visible {
                if let Some(r_scale) = scales.get("r") {
                    let tick_count = r_axis.tick_count.map(|c| c as f32);
                    let ticks = r_scale.ticks(tick_count.or(Some(5.0)))?;
                    
                    // Format tick values as strings
                    let formatted_ticks = r_scale.format(&ticks)?;
                    
                    // Position labels along the bottom vertical line (at theta = 3π/2)
                    let label_angle = 3.0 * std::f32::consts::PI / 2.0; // Bottom
                    let label_offset = 5.0; // Small offset from the tick circle
                    
                    let mut label_x = Vec::new();
                    let mut label_y = Vec::new();
                    let mut label_texts = Vec::new();
                    
                    // Handle different array types for ticks
                    use datafusion::arrow::datatypes::DataType;
                    use datafusion::arrow::array::{Float32Array, Float64Array};
                    
                    let tick_values: Vec<f64> = match ticks.data_type() {
                        DataType::Float64 => {
                            ticks.as_any()
                                .downcast_ref::<Float64Array>()
                                .unwrap()
                                .iter()
                                .filter_map(|v| v)
                                .collect()
                        }
                        DataType::Float32 => {
                            ticks.as_any()
                                .downcast_ref::<Float32Array>()
                                .unwrap()
                                .iter()
                                .filter_map(|v| v.map(|f| f as f64))
                                .collect()
                        }
                        _ => vec![],
                    };
                    
                    // Get formatted strings and positions
                    for (i, value) in tick_values.iter().enumerate() {
                        // Transform tick value through scale to get radius
                        let tick_array = Arc::new(Float64Array::from(vec![*value])) as datafusion::arrow::array::ArrayRef;
                        let scaled_values = r_scale.scale(&tick_array)?;
                        
                        let radius = if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float64Array>() {
                            scaled_array.value(0) as f32
                        } else if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float32Array>() {
                            scaled_array.value(0)
                        } else {
                            continue;
                        };
                        
                        if radius.is_finite() && radius > 0.0 {
                            // Position label slightly below the grid circle
                            let x = center_x;
                            let y = center_y + radius + label_offset;
                            
                            label_x.push(x);
                            label_y.push(y);
                            
                            // Get formatted text for this tick
                            match formatted_ticks.value() {
                                avenger_common::value::ScalarOrArrayValue::Array(texts) => {
                                    if i < texts.len() {
                                        label_texts.push(texts[i].clone());
                                    }
                                }
                                avenger_common::value::ScalarOrArrayValue::Scalar(text) => {
                                    label_texts.push(text.clone());
                                }
                            }
                        }
                    }
                    
                    if !label_texts.is_empty() {
                        let labels = SceneTextMark {
                            name: "polar-r-labels".to_string(),
                            clip: false,
                            len: label_texts.len() as u32,
                            text: ScalarOrArray::new_array(label_texts),
                            x: ScalarOrArray::new_array(label_x),
                            y: ScalarOrArray::new_array(label_y),
                            align: ScalarOrArray::new_scalar(TextAlign::Center),
                            baseline: ScalarOrArray::new_scalar(TextBaseline::Top),
                            angle: ScalarOrArray::new_scalar(0.0),
                            color: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                            font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                            font_size: ScalarOrArray::new_scalar(10.0),
                            font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Normal)),
                            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                            limit: ScalarOrArray::new_scalar(0.0),
                            indices: None,
                            zindex: Some(0),
                        };
                        
                        axis_marks.push(SceneMark::Text(Arc::new(labels)));
                    }
                }
            }
        }

        // Render angular grid (radial lines) if theta axis is visible and has grid enabled
        if let Some(theta_axis) = axes.get("theta") {
            if theta_axis.visible && theta_axis.grid {
                if let Some(theta_scale) = scales.get("theta") {
                    // Get tick values from the scale
                    let tick_count = theta_axis.tick_count.map(|c| c as f32);
                    let ticks = theta_scale.ticks(tick_count.or(Some(8.0)))?;
                    let num_ticks = ticks.len();
                    
                    if num_ticks > 0 {
                        // Create radial lines for each tick value
                        let mut x_values = Vec::with_capacity(num_ticks);
                        let mut y_values = Vec::with_capacity(num_ticks);
                        let mut x2_values = Vec::with_capacity(num_ticks);
                        let mut y2_values = Vec::with_capacity(num_ticks);
                        
                        // Handle different array types for ticks
                        use datafusion::arrow::datatypes::DataType;
                        use datafusion::arrow::array::{Float32Array, Float64Array};
                        
                        let angle_values: Vec<f64> = match ticks.data_type() {
                            DataType::Float64 => {
                                ticks.as_any()
                                    .downcast_ref::<Float64Array>()
                                    .unwrap()
                                    .iter()
                                    .filter_map(|v| v)
                                    .collect()
                            }
                            DataType::Float32 => {
                                ticks.as_any()
                                    .downcast_ref::<Float32Array>()
                                    .unwrap()
                                    .iter()
                                    .filter_map(|v| v.map(|f| f as f64))
                                    .collect()
                            }
                            _ => vec![],
                        };
                        
                        for angle in angle_values {
                            // Start from center
                            x_values.push(center_x);
                            y_values.push(center_y);
                            
                            // End at max radius
                            let cos_angle = (angle as f32).cos();
                            let sin_angle = (angle as f32).sin();
                            x2_values.push(center_x + max_radius * cos_angle);
                            y2_values.push(center_y + max_radius * sin_angle);
                        }
                        
                        // Create rule marks for radial lines
                        let grid_lines = SceneRuleMark {
                            name: "polar-angular-grid".to_string(),
                            clip: false,
                            len: x_values.len() as u32,
                            gradients: vec![],
                            x: ScalarOrArray::new_array(x_values),
                            y: ScalarOrArray::new_array(y_values),
                            x2: ScalarOrArray::new_array(x2_values),
                            y2: ScalarOrArray::new_array(y2_values),
                            stroke: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.8, 0.8, 0.8, 0.3])),
                            stroke_width: ScalarOrArray::new_scalar(1.0),
                            stroke_cap: ScalarOrArray::new_scalar(avenger_common::types::StrokeCap::Butt),
                            stroke_dash: None,
                            indices: None,
                            zindex: Some(-1), // Render behind data
                        };
                        
                        axis_marks.push(SceneMark::Rule(grid_lines));
                    }
                }
            }
            
            // Add angular axis tick labels
            if theta_axis.visible {
                if let Some(theta_scale) = scales.get("theta") {
                    let tick_count = theta_axis.tick_count.map(|c| c as f32);
                    let ticks = theta_scale.ticks(tick_count.or(Some(8.0)))?;
                    
                    // Format tick values as strings
                    let formatted_ticks = theta_scale.format(&ticks)?;
                    
                    let label_offset = 15.0; // Offset from the outer circle
                    let label_radius = max_radius + label_offset;
                    
                    let mut label_x = Vec::new();
                    let mut label_y = Vec::new();
                    let mut label_texts = Vec::new();
                    let mut label_aligns = Vec::new();
                    let mut label_baselines = Vec::new();
                    
                    // Handle different array types for ticks
                    use datafusion::arrow::datatypes::DataType;
                    use datafusion::arrow::array::{Float32Array, Float64Array};
                    
                    let angle_values: Vec<f64> = match ticks.data_type() {
                        DataType::Float64 => {
                            ticks.as_any()
                                .downcast_ref::<Float64Array>()
                                .unwrap()
                                .iter()
                                .filter_map(|v| v)
                                .collect()
                        }
                        DataType::Float32 => {
                            ticks.as_any()
                                .downcast_ref::<Float32Array>()
                                .unwrap()
                                .iter()
                                .filter_map(|v| v.map(|f| f as f64))
                                .collect()
                        }
                        _ => vec![],
                    };
                    
                    // Get formatted strings and positions
                    for (i, angle) in angle_values.iter().enumerate() {
                        let cos_angle = (*angle as f32).cos();
                        let sin_angle = (*angle as f32).sin();
                        
                        // Position label outside the plot circle
                        let x = center_x + label_radius * cos_angle;
                        let y = center_y + label_radius * sin_angle;
                        
                        label_x.push(x);
                        label_y.push(y);
                        
                        // Determine text alignment based on angle
                        let align = if cos_angle.abs() < 0.1 {
                            TextAlign::Center
                        } else if cos_angle > 0.0 {
                            TextAlign::Left
                        } else {
                            TextAlign::Right
                        };
                        
                        let baseline = if sin_angle.abs() < 0.1 {
                            TextBaseline::Middle
                        } else if sin_angle > 0.0 {
                            TextBaseline::Top
                        } else {
                            TextBaseline::Bottom
                        };
                        
                        label_aligns.push(align);
                        label_baselines.push(baseline);
                        
                        // Get formatted text and convert to degrees if appropriate
                        let text = match formatted_ticks.value() {
                            avenger_common::value::ScalarOrArrayValue::Array(texts) => {
                                if i < texts.len() {
                                    texts[i].clone()
                                } else {
                                    continue;
                                }
                            }
                            avenger_common::value::ScalarOrArrayValue::Scalar(text) => text.clone(),
                        };
                        
                        // Convert radians to degrees for display if the value looks like radians
                        let display_text = if *angle >= 0.0 && *angle <= 2.0 * std::f64::consts::PI {
                            // It's in radians, convert to degrees
                            let degrees = (*angle * 180.0 / std::f64::consts::PI).round() as i32;
                            format!("{}°", degrees)
                        } else {
                            text
                        };
                        label_texts.push(display_text);
                    }
                    
                    if !label_texts.is_empty() {
                        let labels = SceneTextMark {
                            name: "polar-theta-labels".to_string(),
                            clip: false,
                            len: label_texts.len() as u32,
                            text: ScalarOrArray::new_array(label_texts),
                            x: ScalarOrArray::new_array(label_x),
                            y: ScalarOrArray::new_array(label_y),
                            align: ScalarOrArray::new_array(label_aligns),
                            baseline: ScalarOrArray::new_array(label_baselines),
                            angle: ScalarOrArray::new_scalar(0.0),
                            color: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                            font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                            font_size: ScalarOrArray::new_scalar(10.0),
                            font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Normal)),
                            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                            limit: ScalarOrArray::new_scalar(0.0),
                            indices: None,
                            zindex: Some(0),
                        };
                        
                        axis_marks.push(SceneMark::Text(Arc::new(labels)));
                    }
                }
            }
        }
        
        // Add axis titles
        let mut title_marks = Vec::new();
        
        // Radial axis title
        if let Some(r_axis) = axes.get("r") {
            if let Some(ref title) = r_axis.title {
                if !title.is_empty() {
                    // Position title at the bottom-left of the plot
                    let title_x = padding.left;
                    let title_y = padding.top + plot_height + 30.0; // Below the plot
                    
                    let title_mark = SceneTextMark {
                        name: "polar-r-title".to_string(),
                        clip: false,
                        len: 1,
                        text: ScalarOrArray::new_scalar(title.clone()),
                        x: ScalarOrArray::new_scalar(title_x),
                        y: ScalarOrArray::new_scalar(title_y),
                        align: ScalarOrArray::new_scalar(TextAlign::Left),
                        baseline: ScalarOrArray::new_scalar(TextBaseline::Top),
                        angle: ScalarOrArray::new_scalar(0.0),
                        color: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                        font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                        font_size: ScalarOrArray::new_scalar(12.0),
                        font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Bold)),
                        font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                        limit: ScalarOrArray::new_scalar(0.0),
                        indices: None,
                        zindex: Some(0),
                    };
                    
                    title_marks.push(SceneMark::Text(Arc::new(title_mark)));
                }
            }
        }
        
        // Angular axis title
        if let Some(theta_axis) = axes.get("theta") {
            if let Some(ref title) = theta_axis.title {
                if !title.is_empty() {
                    // Position title at the top of the plot
                    let title_x = center_x;
                    let title_y = padding.top - 10.0; // Above the plot
                    
                    let title_mark = SceneTextMark {
                        name: "polar-theta-title".to_string(),
                        clip: false,
                        len: 1,
                        text: ScalarOrArray::new_scalar(title.clone()),
                        x: ScalarOrArray::new_scalar(title_x),
                        y: ScalarOrArray::new_scalar(title_y),
                        align: ScalarOrArray::new_scalar(TextAlign::Center),
                        baseline: ScalarOrArray::new_scalar(TextBaseline::Bottom),
                        angle: ScalarOrArray::new_scalar(0.0),
                        color: ScalarOrArray::new_scalar(avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                        font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                        font_size: ScalarOrArray::new_scalar(12.0),
                        font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Bold)),
                        font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                        limit: ScalarOrArray::new_scalar(0.0),
                        indices: None,
                        zindex: Some(0),
                    };
                    
                    title_marks.push(SceneMark::Text(Arc::new(title_mark)));
                }
            }
        }
        
        axis_marks.extend(title_marks);

        Ok(axis_marks)
    }
}
