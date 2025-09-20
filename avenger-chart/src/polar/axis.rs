use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;

/// Type of polar axis
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolarAxisType {
    Radial,
    Angular,
}

/// Direction for angular axis
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolarDirection {
    Clockwise,
    CounterClockwise,
}

/// Concrete struct for Polar axes
/// Using a struct instead of a trait enables type inference in closure parameters
#[derive(Clone, Debug)]
pub struct PolarAxis {
    pub visible: bool,
    pub axis_type: PolarAxisType,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub format_number: Option<String>,
    pub grid_levels: Option<usize>,
    pub start_angle: f32,
    pub direction: PolarDirection,
}

impl PolarAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn axis_type(mut self, axis_type: PolarAxisType) -> Self {
        self.axis_type = axis_type;
        self
    }

    pub fn title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    pub fn grid_levels(mut self, levels: usize) -> Self {
        self.grid_levels = Some(levels);
        self
    }

    pub fn start_angle(mut self, angle: f32) -> Self {
        self.start_angle = angle;
        self
    }

    pub fn direction(mut self, direction: PolarDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Render this axis to scene marks
    pub fn render(
        &self,
        _channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        scales: &std::collections::HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
        theme: &dyn crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Skip if invisible
        if !self.visible {
            return Ok(vec![]);
        }

        // Get center point
        let center_x = plot_bounds.x + plot_width / 2.0;
        let center_y = plot_bounds.y + plot_height / 2.0;
        let radius = plot_width.min(plot_height) / 2.0;

        match self.axis_type {
            PolarAxisType::Radial => {
                // Render radial axis (circles from center)
                self.render_radial_axis(scale, center_x, center_y, radius, theme)
            }
            PolarAxisType::Angular => {
                // Render angular axis (lines from center)
                self.render_angular_axis(scale, center_x, center_y, radius, scales, theme)
            }
        }
    }

    fn render_radial_axis(
        &self,
        _scale: &avenger_scales::scales::ConfiguredScale,
        center_x: f32,
        center_y: f32,
        _max_radius: f32,
        theme: &dyn crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::types::ColorOrGradient;
        use avenger_common::value::ScalarOrArray;
        use avenger_scenegraph::marks::arc::SceneArcMark;

        let mut marks = Vec::new();
        let _num_circles = self.grid_levels.unwrap_or(6);

        // Create concentric circles for the grid using actual scale ticks
        if self.grid {
            // Get tick values from the scale
            let tick_count = self.tick_count.map(|c| c as f32);
            let ticks = _scale.ticks(tick_count.or(Some(5.0)))?;

            let mut radii = Vec::new();

            // Convert ticks to radii through scale transformation
            use datafusion::arrow::array::{Float32Array, Float64Array};
            use datafusion::arrow::datatypes::DataType;
            use std::sync::Arc;

            let tick_values: Vec<f64> = match ticks.data_type() {
                DataType::Float64 => ticks
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap()
                    .iter()
                    .flatten()
                    .collect(),
                DataType::Float32 => ticks
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.map(|f| f as f64))
                    .collect(),
                _ => vec![],
            };

            for value in tick_values {
                // Transform tick value through scale to get radius
                let tick_array =
                    Arc::new(Float64Array::from(vec![value])) as datafusion::arrow::array::ArrayRef;
                let scaled_values = _scale.scale(&tick_array)?;
                if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float64Array>() {
                    if !scaled_array.is_empty() {
                        let radius = scaled_array.value(0) as f32;
                        if radius.is_finite() && radius > 0.0 {
                            radii.push(radius);
                        }
                    }
                } else if let Some(scaled_array) =
                    scaled_values.as_any().downcast_ref::<Float32Array>()
                {
                    if !scaled_array.is_empty() {
                        let radius = scaled_array.value(0);
                        if radius.is_finite() && radius > 0.0 {
                            radii.push(radius);
                        }
                    }
                }
            }

            if !radii.is_empty() {
                let grid_arc = SceneArcMark {
                    name: "polar-radial-grid".to_string(),
                    clip: false,
                    len: radii.len() as u32,
                    gradients: vec![],
                    x: ScalarOrArray::new_scalar(center_x),
                    y: ScalarOrArray::new_scalar(center_y),
                    start_angle: ScalarOrArray::new_scalar(0.0),
                    end_angle: ScalarOrArray::new_scalar(2.0 * std::f32::consts::PI),
                    outer_radius: ScalarOrArray::new_array(radii.clone()),
                    inner_radius: ScalarOrArray::new_array(
                        radii.iter().map(|r| (r - 0.5).max(0.0)).collect(),
                    ), // Thin circles
                    pad_angle: ScalarOrArray::new_scalar(0.0),
                    corner_radius: ScalarOrArray::new_scalar(0.0),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 0.8, 0.3])),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                    stroke_width: ScalarOrArray::new_scalar(0.0),
                    indices: None,
                    zindex: Some(-1),
                };

                marks.push(SceneMark::Arc(grid_arc));
            }
        }

        // Add radial tick labels
        if self.visible {
            use avenger_scenegraph::marks::text::SceneTextMark;
            use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

            // Get tick values from scale - use same as grid
            let tick_count = self.tick_count.map(|c| c as f32);
            let ticks = _scale.ticks(tick_count.or(Some(5.0)))?;

            // Format tick values as strings
            let formatted_ticks = _scale.format(&ticks)?;

            let mut x_vals = Vec::new();
            let mut y_vals = Vec::new();
            let mut text_vals = Vec::new();

            use datafusion::arrow::array::{Float32Array, Float64Array};
            use datafusion::arrow::datatypes::DataType;
            use std::sync::Arc;

            let tick_values: Vec<f64> = match ticks.data_type() {
                DataType::Float64 => ticks
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap()
                    .iter()
                    .flatten()
                    .collect(),
                DataType::Float32 => ticks
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.map(|f| f as f64))
                    .collect(),
                _ => vec![],
            };

            // Get formatted strings and positions
            for (i, value) in tick_values.iter().enumerate() {
                // Skip zero value
                if value.abs() < 0.001 {
                    continue;
                }

                // Transform tick value through scale to get radius (same as grid circles)
                let tick_array = Arc::new(Float64Array::from(vec![*value]))
                    as datafusion::arrow::array::ArrayRef;
                let scaled_values = _scale.scale(&tick_array)?;

                let radius = if let Some(scaled_array) =
                    scaled_values.as_any().downcast_ref::<Float64Array>()
                {
                    scaled_array.value(0) as f32
                } else if let Some(scaled_array) =
                    scaled_values.as_any().downcast_ref::<Float32Array>()
                {
                    scaled_array.value(0)
                } else {
                    continue;
                };

                if radius.is_finite() && radius > 0.0 {
                    // Position labels vertically below center (at 90 degrees)
                    x_vals.push(center_x);
                    y_vals.push(center_y + radius);

                    // Get formatted text for this tick
                    match formatted_ticks.value() {
                        avenger_common::value::ScalarOrArrayValue::Array(texts) => {
                            if i < texts.len() {
                                text_vals.push(texts[i].clone());
                            }
                        }
                        avenger_common::value::ScalarOrArrayValue::Scalar(text) => {
                            text_vals.push(text.clone());
                        }
                    }
                }
            }

            if !text_vals.is_empty() {
                let text_mark = SceneTextMark {
                    name: "polar-r-labels".to_string(),
                    clip: false,
                    len: text_vals.len() as u32,
                    x: ScalarOrArray::new_array(x_vals),
                    y: ScalarOrArray::new_array(y_vals),
                    text: ScalarOrArray::new_array(text_vals),
                    font: ScalarOrArray::new_scalar(theme.axis_label_font_family().to_string()),
                    font_weight: ScalarOrArray::new_scalar(
                        avenger_text::types::FontWeight::Number(theme.axis_label_font_weight()),
                    ),
                    font_size: ScalarOrArray::new_scalar(theme.axis_label_font_size()),
                    font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                    color: ScalarOrArray::new_scalar(ColorOrGradient::Color(
                        crate::utils::parse_color_to_array(&theme.axis_label_color()),
                    )),
                    align: ScalarOrArray::new_scalar(TextAlign::Center),
                    baseline: ScalarOrArray::new_scalar(TextBaseline::Top),
                    angle: ScalarOrArray::new_scalar(0.0),
                    limit: ScalarOrArray::new_scalar(200.0),
                    indices: None,
                    zindex: Some(0),
                };
                marks.push(SceneMark::Text(std::sync::Arc::new(text_mark)));
            }
        }

        // Note: Axis title rendering removed - placement needs design work

        Ok(marks)
    }

    fn render_angular_axis(
        &self,
        _scale: &avenger_scales::scales::ConfiguredScale,
        center_x: f32,
        center_y: f32,
        radius: f32,
        scales: &std::collections::HashMap<String, avenger_scales::scales::ConfiguredScale>,
        theme: &dyn crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::types::ColorOrGradient;
        use avenger_common::types::StrokeCap;
        use avenger_common::value::ScalarOrArray;
        use avenger_scenegraph::marks::rule::SceneRuleMark;

        let mut marks = Vec::new();

        // Get the radial scale to determine max radius
        let max_radius = if let Some(r_scale) = scales.get("r") {
            let r_range = r_scale.numeric_interval_range()?;
            r_range.1
        } else {
            radius
        };

        // Render angular grid (radial lines) based on scale ticks
        if self.grid {
            // Get tick values from the scale
            let tick_count = self.tick_count.map(|c| c as f32);
            let ticks = _scale.ticks(tick_count.or(Some(8.0)))?;

            let mut x_values = Vec::new();
            let mut y_values = Vec::new();
            let mut x2_values = Vec::new();
            let mut y2_values = Vec::new();

            use datafusion::arrow::array::{Float32Array, Float64Array};
            use datafusion::arrow::datatypes::DataType;

            let angle_values: Vec<f64> = match ticks.data_type() {
                DataType::Float64 => ticks
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap()
                    .iter()
                    .flatten()
                    .collect(),
                DataType::Float32 => ticks
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.map(|f| f as f64))
                    .collect(),
                _ => vec![],
            };

            // Get the outermost circle radius from the r scale if available
            let outer_radius = if let Some(r_scale) = scales.get("r") {
                // Get the maximum radius from the r scale's range
                if let Ok(r_range) = r_scale.numeric_interval_range() {
                    r_range.1.max(r_range.0)
                } else {
                    max_radius
                }
            } else {
                max_radius
            };

            for angle in angle_values {
                // Start from center
                x_values.push(center_x);
                y_values.push(center_y);

                // End at outer radius
                let cos_angle = (angle as f32).cos();
                let sin_angle = (angle as f32).sin();
                x2_values.push(center_x + outer_radius * cos_angle);
                y2_values.push(center_y + outer_radius * sin_angle);
            }

            let grid_rule = SceneRuleMark {
                name: "polar-angular-grid".to_string(),
                clip: false,
                len: x_values.len() as u32,
                gradients: vec![],
                x: ScalarOrArray::new_array(x_values),
                y: ScalarOrArray::new_array(y_values),
                x2: ScalarOrArray::new_array(x2_values),
                y2: ScalarOrArray::new_array(y2_values),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 0.8, 0.3])),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
                stroke_dash: None,
                indices: None,
                zindex: Some(-1),
            };

            marks.push(SceneMark::Rule(grid_rule));
        }

        // Add angular tick labels
        if self.visible {
            use avenger_scenegraph::marks::text::SceneTextMark;
            use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

            // Get tick values from scale
            let tick_values: Vec<f32> = if let Ok(ticks_array) = _scale.ticks(Some(8.0)) {
                // Convert arrow array to vec of f32
                use datafusion::arrow::array::Float32Array;
                if let Some(arr) = ticks_array.as_any().downcast_ref::<Float32Array>() {
                    (0..arr.len()).map(|i| arr.value(i)).collect()
                } else {
                    // Fallback if not float32
                    let num_ticks = self.tick_count.unwrap_or(8);
                    (0..num_ticks)
                        .map(|i| (i as f32 / num_ticks as f32) * 2.0 * std::f32::consts::PI)
                        .collect()
                }
            } else {
                // Fallback to uniform distribution
                let num_ticks = self.tick_count.unwrap_or(8);
                (0..num_ticks)
                    .map(|i| (i as f32 / num_ticks as f32) * 2.0 * std::f32::consts::PI)
                    .collect()
            };

            let mut x_vals = Vec::new();
            let mut y_vals = Vec::new();
            let mut text_vals = Vec::new();
            let mut label_aligns = Vec::new();
            let mut label_baselines = Vec::new();

            let label_radius = max_radius + 15.0; // Place labels outside the plot

            for tick_val in tick_values {
                // Use tick value directly as angle (it's already in radians)
                let cos_angle = tick_val.cos();
                let sin_angle = tick_val.sin();

                // Position label outside the plot circle
                let x = center_x + label_radius * cos_angle;
                let y = center_y + label_radius * sin_angle;

                x_vals.push(x);
                y_vals.push(y);

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

                // Convert radians to degrees for display
                let degrees = (tick_val * 180.0 / std::f32::consts::PI).round() as i32;
                text_vals.push(format!("{}°", degrees));
            }

            if !text_vals.is_empty() {
                let text_mark = SceneTextMark {
                    name: "polar-theta-labels".to_string(),
                    clip: false,
                    len: text_vals.len() as u32,
                    x: ScalarOrArray::new_array(x_vals),
                    y: ScalarOrArray::new_array(y_vals),
                    text: ScalarOrArray::new_array(text_vals),
                    align: ScalarOrArray::new_array(label_aligns),
                    baseline: ScalarOrArray::new_array(label_baselines),
                    font: ScalarOrArray::new_scalar(theme.axis_label_font_family().to_string()),
                    font_weight: ScalarOrArray::new_scalar(
                        avenger_text::types::FontWeight::Number(theme.axis_label_font_weight()),
                    ),
                    font_size: ScalarOrArray::new_scalar(theme.axis_label_font_size()),
                    font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                    color: ScalarOrArray::new_scalar(ColorOrGradient::Color(
                        crate::utils::parse_color_to_array(&theme.axis_label_color()),
                    )),
                    angle: ScalarOrArray::new_scalar(0.0),
                    limit: ScalarOrArray::new_scalar(200.0),
                    indices: None,
                    zindex: Some(0),
                };
                marks.push(SceneMark::Text(std::sync::Arc::new(text_mark)));
            }
        }

        Ok(marks)
    }
}

impl Default for PolarAxis {
    fn default() -> Self {
        Self {
            visible: true,
            axis_type: PolarAxisType::Radial,
            title: None,
            grid: false,
            tick_count: None,
            format_number: None,
            grid_levels: None,
            start_angle: 0.0,
            direction: PolarDirection::Clockwise,
        }
    }
}
