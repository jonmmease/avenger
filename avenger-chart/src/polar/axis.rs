use crate::axis::Axis as AxisBase;
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use std::any::Any;

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

/// Trait for axes that can be used with Polar coordinates
/// This trait is NOT object-safe due to the setter methods returning Self,
/// but that's fine since we always use it with concrete types via generics.
pub trait PolarAxis: AxisBase + Clone + Send + Sync + Default + 'static {
    // === Getters ===

    /// Get axis visibility
    fn visible(&self) -> bool;

    /// Get axis type (Radial or Angular)
    fn axis_type(&self) -> PolarAxisType;

    /// Get axis title
    fn title(&self) -> Option<&str>;

    /// Whether to show grid lines
    fn grid(&self) -> bool;

    /// Get tick count hint
    fn tick_count(&self) -> Option<usize>;

    /// Get number format pattern
    fn format_number(&self) -> Option<&str>;

    /// Get grid levels for radial axis
    fn grid_levels(&self) -> Option<usize>;

    /// Get start angle for angular axis
    fn start_angle(&self) -> f32;

    /// Get direction for angular axis
    fn direction(&self) -> PolarDirection;

    // === Setters (make trait non-object-safe) ===

    /// Set axis visibility
    fn with_visible(self, visible: bool) -> Self;

    /// Set axis type
    fn with_axis_type(self, axis_type: PolarAxisType) -> Self;

    /// Set axis title
    fn with_title(self, title: impl Into<String>) -> Self;

    /// Set whether to show grid lines
    fn with_grid(self, grid: bool) -> Self;

    /// Set tick count hint
    fn with_tick_count(self, count: usize) -> Self;

    /// Set number format pattern
    fn with_format_number(self, format: impl Into<String>) -> Self;

    /// Set grid levels for radial axis
    fn with_grid_levels(self, levels: usize) -> Self;

    /// Set start angle for angular axis
    fn with_start_angle(self, angle: f32) -> Self;

    /// Set direction for angular axis
    fn with_direction(self, direction: PolarDirection) -> Self;

    // === Rendering ===

    /// Render this axis to scene marks
    /// Each axis implementation is responsible for its complete rendering logic
    fn render(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        scales: &std::collections::HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

/// Default implementation of PolarAxis
#[derive(Clone, Debug)]
pub struct DefaultPolarAxis {
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

impl DefaultPolarAxis {
    pub fn new(axis_type: PolarAxisType) -> Self {
        Self {
            axis_type,
            ..Default::default()
        }
    }

    pub fn radial() -> Self {
        Self::new(PolarAxisType::Radial)
    }

    pub fn angular() -> Self {
        Self::new(PolarAxisType::Angular)
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
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
}

impl Default for DefaultPolarAxis {
    fn default() -> Self {
        Self {
            visible: true,
            axis_type: PolarAxisType::Radial,
            title: None,
            grid: true,
            tick_count: None,
            format_number: None,
            grid_levels: None,
            start_angle: 0.0,
            direction: PolarDirection::Clockwise,
        }
    }
}

impl AxisBase for DefaultPolarAxis {
    fn clone_box(&self) -> Box<dyn AxisBase> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl PolarAxis for DefaultPolarAxis {
    // === Getters ===
    fn visible(&self) -> bool {
        self.visible
    }

    fn axis_type(&self) -> PolarAxisType {
        self.axis_type
    }

    fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn grid(&self) -> bool {
        self.grid
    }

    fn tick_count(&self) -> Option<usize> {
        self.tick_count
    }

    fn format_number(&self) -> Option<&str> {
        self.format_number.as_deref()
    }

    fn grid_levels(&self) -> Option<usize> {
        self.grid_levels
    }

    fn start_angle(&self) -> f32 {
        self.start_angle
    }

    fn direction(&self) -> PolarDirection {
        self.direction
    }

    // === Setters ===
    fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    fn with_axis_type(mut self, axis_type: PolarAxisType) -> Self {
        self.axis_type = axis_type;
        self
    }

    fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    fn with_grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    fn with_tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    fn with_format_number(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    fn with_grid_levels(mut self, levels: usize) -> Self {
        self.grid_levels = Some(levels);
        self
    }

    fn with_start_angle(mut self, angle: f32) -> Self {
        self.start_angle = angle;
        self
    }

    fn with_direction(mut self, direction: PolarDirection) -> Self {
        self.direction = direction;
        self
    }

    // === Rendering ===
    fn render(
        &self,
        _channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        scales: &std::collections::HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::value::ScalarOrArray;
        use avenger_scenegraph::marks::arc::SceneArcMark;
        use avenger_scenegraph::marks::rule::SceneRuleMark;
        use datafusion::arrow::array::{Float32Array, Float64Array};
        use datafusion::arrow::datatypes::DataType;
        use std::sync::Arc;

        let mut axis_marks = Vec::new();

        // Skip if invisible
        if !self.visible {
            return Ok(axis_marks);
        }

        // Calculate center of plot area in canvas coordinates
        let center_x = padding.left + plot_width / 2.0;
        let center_y = padding.top + plot_height / 2.0;
        let max_radius = f32::min(plot_width, plot_height) / 2.0;

        match self.axis_type {
            PolarAxisType::Radial => {
                // Render radial grid (concentric circles) and labels
                if self.grid {
                    // Get tick values from the scale
                    let tick_count = self.tick_count.map(|c| c as f32);
                    let ticks = scale.ticks(tick_count.or(Some(5.0)))?;
                    let num_ticks = ticks.len();

                    if num_ticks > 0 {
                        // Create concentric circles for each tick value
                        let mut radii = Vec::with_capacity(num_ticks);

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
                            let tick_array = Arc::new(Float64Array::from(vec![value]))
                                as datafusion::arrow::array::ArrayRef;
                            let scaled_values = scale.scale(&tick_array)?;
                            if let Some(scaled_array) =
                                scaled_values.as_any().downcast_ref::<Float64Array>()
                            {
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
                                inner_radius: ScalarOrArray::new_array(
                                    radii.iter().map(|r| (r - 0.5).max(0.0)).collect(),
                                ), // Thin circles
                                pad_angle: ScalarOrArray::new_scalar(0.0),
                                corner_radius: ScalarOrArray::new_scalar(0.0),
                                fill: ScalarOrArray::new_scalar(
                                    avenger_common::types::ColorOrGradient::Color([
                                        0.8, 0.8, 0.8, 0.3,
                                    ]),
                                ),
                                stroke: ScalarOrArray::new_scalar(
                                    avenger_common::types::ColorOrGradient::Color([
                                        0.0, 0.0, 0.0, 0.0,
                                    ]),
                                ),
                                stroke_width: ScalarOrArray::new_scalar(0.0),
                                indices: None,
                                zindex: Some(-1), // Render behind data
                            };

                            axis_marks.push(SceneMark::Arc(grid_arcs));
                        }
                    }
                }

                // Add radial axis tick labels
                let tick_count = self.tick_count.map(|c| c as f32);
                let ticks = scale.ticks(tick_count.or(Some(5.0)))?;

                // Format tick values as strings
                let formatted_ticks = scale.format(&ticks)?;

                // Position labels along the bottom vertical line (at theta = 3π/2)
                let _label_angle = 3.0 * std::f32::consts::PI / 2.0; // Bottom
                let _label_offset = 2.0; // Small offset from the tick circle

                let mut label_x = Vec::new();
                let mut label_y = Vec::new();
                let mut label_texts = Vec::new();

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
                    // Transform tick value through scale to get radius
                    let tick_array = Arc::new(Float64Array::from(vec![*value]))
                        as datafusion::arrow::array::ArrayRef;
                    let scaled_values = scale.scale(&tick_array)?;

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
                        // Position label exactly on the grid circle, centered
                        let x = center_x;
                        let y = center_y + radius;

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
                    use avenger_scenegraph::marks::text::SceneTextMark;
                    use avenger_text::types::{
                        FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline,
                    };

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
                        color: ScalarOrArray::new_scalar(
                            avenger_common::types::ColorOrGradient::Color([0.4, 0.4, 0.4, 1.0]),
                        ), // Lighter gray
                        font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                        font_size: ScalarOrArray::new_scalar(8.0), // Smaller font
                        font_weight: ScalarOrArray::new_scalar(FontWeight::Name(
                            FontWeightNameSpec::Normal,
                        )),
                        font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                        limit: ScalarOrArray::new_scalar(0.0),
                        indices: None,
                        zindex: Some(0),
                    };

                    axis_marks.push(SceneMark::Text(Arc::new(labels)));
                }
            }
            PolarAxisType::Angular => {
                // Render angular grid (radial lines) and labels
                if self.grid {
                    // Get tick values from the scale
                    let tick_count = self.tick_count.map(|c| c as f32);
                    let ticks = scale.ticks(tick_count.or(Some(8.0)))?;
                    let num_ticks = ticks.len();

                    if num_ticks > 0 {
                        // Create radial lines for each tick value
                        let mut x_values = Vec::with_capacity(num_ticks);
                        let mut y_values = Vec::with_capacity(num_ticks);
                        let mut x2_values = Vec::with_capacity(num_ticks);
                        let mut y2_values = Vec::with_capacity(num_ticks);

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
                            stroke: ScalarOrArray::new_scalar(
                                avenger_common::types::ColorOrGradient::Color([0.8, 0.8, 0.8, 0.3]),
                            ),
                            stroke_width: ScalarOrArray::new_scalar(1.0),
                            stroke_cap: ScalarOrArray::new_scalar(
                                avenger_common::types::StrokeCap::Butt,
                            ),
                            stroke_dash: None,
                            indices: None,
                            zindex: Some(-1), // Render behind data
                        };

                        axis_marks.push(SceneMark::Rule(grid_lines));
                    }
                }

                // Add angular axis tick labels
                let tick_count = self.tick_count.map(|c| c as f32);
                let ticks = scale.ticks(tick_count.or(Some(8.0)))?;

                // Format tick values as strings
                let formatted_ticks = scale.format(&ticks)?;

                let label_offset = 15.0; // Offset from the outer circle
                let label_radius = max_radius + label_offset;

                let mut label_x = Vec::new();
                let mut label_y = Vec::new();
                let mut label_texts = Vec::new();
                let mut label_aligns = Vec::new();
                let mut label_baselines = Vec::new();

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
                        avenger_text::types::TextAlign::Center
                    } else if cos_angle > 0.0 {
                        avenger_text::types::TextAlign::Left
                    } else {
                        avenger_text::types::TextAlign::Right
                    };

                    let baseline = if sin_angle.abs() < 0.1 {
                        avenger_text::types::TextBaseline::Middle
                    } else if sin_angle > 0.0 {
                        avenger_text::types::TextBaseline::Top
                    } else {
                        avenger_text::types::TextBaseline::Bottom
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
                    use avenger_scenegraph::marks::text::SceneTextMark;
                    use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec};

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
                        color: ScalarOrArray::new_scalar(
                            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                        ),
                        font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                        font_size: ScalarOrArray::new_scalar(10.0),
                        font_weight: ScalarOrArray::new_scalar(FontWeight::Name(
                            FontWeightNameSpec::Normal,
                        )),
                        font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                        limit: ScalarOrArray::new_scalar(0.0),
                        indices: None,
                        zindex: Some(0),
                    };

                    axis_marks.push(SceneMark::Text(Arc::new(labels)));
                }
            }
        }

        Ok(axis_marks)
    }
}
