use crate::coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult};
use crate::error::AvengerChartError;
use crate::polar::{PolarAxis, PolarAxisType, PolarDirection};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::Expr;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Polar {
    // Fields for center injection from renderer
    center_x: Option<Expr>,
    center_y: Option<Expr>,
}

impl Polar {
    pub fn new() -> Self {
        Self {
            center_x: None,
            center_y: None,
        }
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

    async fn measure_guide_overflow(
        &self,
        axes: HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width: f32,
        height: f32,
        plot_area_ratio: f32,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        use crate::render::Padding;
        use avenger_geometry::marks::MarkGeometryUtils;

        // The scales were configured with the given ratio of canvas dimensions
        // We need to use consistent dimensions for measuring overflow
        let plot_width = width * plot_area_ratio;
        let plot_height = height * plot_area_ratio;

        // Calculate padding that centers this plot area in the canvas
        let initial_padding = Padding {
            left: (width - plot_width) / 2.0,
            right: (width - plot_width) / 2.0,
            top: (height - plot_height) / 2.0,
            bottom: (height - plot_height) / 2.0,
        };

        // Render axes to measure their bounding box
        let axis_marks = self
            .render_axes(&axes, scales, plot_width, plot_height, &initial_padding)
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

        // Calculate overflow on each side and add margin only if there's overflow
        let margin = 5.0;
        let left_overflow = (initial_padding.left - min_x).max(0.0);
        let right_overflow = (max_x - (width - initial_padding.right)).max(0.0);
        let top_overflow = (initial_padding.top - min_y).max(0.0);
        let bottom_overflow = (max_y - (height - initial_padding.bottom)).max(0.0);

        // Only add margin if there's actual overflow
        let left = if left_overflow > 0.0 {
            left_overflow + margin
        } else {
            0.0
        };
        let right = if right_overflow > 0.0 {
            right_overflow + margin
        } else {
            0.0
        };
        let top = if top_overflow > 0.0 {
            top_overflow + margin
        } else {
            0.0
        };
        let bottom = if bottom_overflow > 0.0 {
            bottom_overflow + margin
        } else {
            0.0
        };

        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        })
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();

        // Create default axes for r and theta channels if they have scales
        for channel in ["r", "theta"] {
            if scales.get(channel).is_some() {
                // Create axis with appropriate defaults
                let axis_type = match channel {
                    "r" => PolarAxisType::Radial,
                    "theta" => PolarAxisType::Angular,
                    _ => PolarAxisType::Radial,
                };

                let axis = PolarAxis {
                    visible: true,
                    axis_type,
                    title: None,      // No titles for polar axes for now
                    grid: true,       // Enable grid by default for polar
                    tick_count: None, // Will use scale's default
                    format_number: None,
                    grid_levels: None,
                    start_angle: 0.0,
                    direction: PolarDirection::Clockwise,
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
        use avenger_common::value::ScalarOrArray;
        use avenger_scenegraph::marks::arc::SceneArcMark;
        use avenger_scenegraph::marks::rule::SceneRuleMark;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{
            FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline,
        };

        let mut axis_marks = Vec::new();

        // Calculate center of plot area in canvas coordinates
        // The axes need to be positioned in absolute canvas coordinates
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
                        use datafusion::arrow::array::{Float32Array, Float64Array};
                        use datafusion::arrow::datatypes::DataType;

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
                            let scaled_values = r_scale.scale(&tick_array)?;
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
            }

            // Add radial axis tick labels
            if r_axis.visible {
                if let Some(r_scale) = scales.get("r") {
                    let tick_count = r_axis.tick_count.map(|c| c as f32);
                    let ticks = r_scale.ticks(tick_count.or(Some(5.0)))?;

                    // Format tick values as strings
                    let formatted_ticks = r_scale.format(&ticks)?;

                    // Position labels along the bottom vertical line (at theta = 3π/2)
                    let _label_angle = 3.0 * std::f32::consts::PI / 2.0; // Bottom
                    let _label_offset = 2.0; // Small offset from the tick circle

                    let mut label_x = Vec::new();
                    let mut label_y = Vec::new();
                    let mut label_texts = Vec::new();

                    // Handle different array types for ticks
                    use datafusion::arrow::array::{Float32Array, Float64Array};
                    use datafusion::arrow::datatypes::DataType;

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
                        let scaled_values = r_scale.scale(&tick_array)?;

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
                            font: ScalarOrArray::new_scalar(
                                "Atkinson Hyperlegible Next".to_string(),
                            ),
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
                        // This should match the outermost concentric circle, not extend beyond it
                        let outer_radius = if let Some(r_scale) = scales.get("r") {
                            // Get tick values to find the outermost grid circle
                            let tick_count = if let Some(r_axis) = axes.get("r") {
                                r_axis.tick_count.map(|c| c as f32)
                            } else {
                                None
                            };

                            let ticks = r_scale.ticks(tick_count.or(Some(5.0)))?;

                            // Get the last tick value which represents the outermost circle
                            use datafusion::arrow::datatypes::DataType;
                            let last_tick = match ticks.data_type() {
                                DataType::Float64 => {
                                    let array =
                                        ticks.as_any().downcast_ref::<Float64Array>().unwrap();
                                    if !array.is_empty() {
                                        array.value(array.len() - 1)
                                    } else {
                                        120.0 // fallback
                                    }
                                }
                                DataType::Float32 => {
                                    let array =
                                        ticks.as_any().downcast_ref::<Float32Array>().unwrap();
                                    if !array.is_empty() {
                                        array.value(array.len() - 1) as f64
                                    } else {
                                        120.0 // fallback
                                    }
                                }
                                _ => 120.0, // fallback
                            };

                            // Scale this tick value to get the actual radius
                            let tick_array = Arc::new(Float64Array::from(vec![last_tick]))
                                as datafusion::arrow::array::ArrayRef;
                            if let Ok(scaled) = r_scale.scale(&tick_array) {
                                if let Some(scaled_f64) =
                                    scaled.as_any().downcast_ref::<Float64Array>()
                                {
                                    scaled_f64.value(0) as f32
                                } else if let Some(scaled_f32) =
                                    scaled.as_any().downcast_ref::<Float32Array>()
                                {
                                    scaled_f32.value(0)
                                } else {
                                    max_radius
                                }
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

                            // End at outer radius (matches outermost circle)
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
                        let display_text = if *angle >= 0.0 && *angle <= 2.0 * std::f64::consts::PI
                        {
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
                            color: ScalarOrArray::new_scalar(
                                avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                            ),
                            font: ScalarOrArray::new_scalar(
                                "Atkinson Hyperlegible Next".to_string(),
                            ),
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
        }

        // Skip axis titles for polar axes for now - we'll focus on tick label spacing

        Ok(axis_marks)
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // For polar plots, create a circular clipping path
        if let Some(r_scale) = scales.get("r") {
            // Get the maximum radius from the scale's range
            if let Ok(r_range) = r_scale.numeric_interval_range() {
                let max_radius = r_range.1.max(r_range.0);

                // Create a circular path centered in the plot area
                let center_x = plot_width / 2.0;
                let center_y = plot_height / 2.0;

                // Build a circular path using lyon
                let mut builder = lyon_path::Path::builder();

                // Start at the rightmost point
                builder.begin(lyon_path::math::point(center_x + max_radius, center_y));

                // Create a circle using bezier curves
                // We'll use 4 arcs to make a complete circle
                let control_dist = max_radius * 0.552_284_8; // Magic number for circle approximation with bezier curves

                // Top-right quadrant
                builder.cubic_bezier_to(
                    lyon_path::math::point(center_x + max_radius, center_y - control_dist),
                    lyon_path::math::point(center_x + control_dist, center_y - max_radius),
                    lyon_path::math::point(center_x, center_y - max_radius),
                );

                // Top-left quadrant
                builder.cubic_bezier_to(
                    lyon_path::math::point(center_x - control_dist, center_y - max_radius),
                    lyon_path::math::point(center_x - max_radius, center_y - control_dist),
                    lyon_path::math::point(center_x - max_radius, center_y),
                );

                // Bottom-left quadrant
                builder.cubic_bezier_to(
                    lyon_path::math::point(center_x - max_radius, center_y + control_dist),
                    lyon_path::math::point(center_x - control_dist, center_y + max_radius),
                    lyon_path::math::point(center_x, center_y + max_radius),
                );

                // Bottom-right quadrant
                builder.cubic_bezier_to(
                    lyon_path::math::point(center_x + control_dist, center_y + max_radius),
                    lyon_path::math::point(center_x + max_radius, center_y + control_dist),
                    lyon_path::math::point(center_x + max_radius, center_y),
                );

                builder.close();

                return Clip::Path(builder.build());
            }
        }

        // Fallback to no clipping if we can't determine the radius
        Clip::None
    }

    fn prepare_scalar_batch(
        &self,
        batch: datafusion::arrow::record_batch::RecordBatch,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<datafusion::arrow::record_batch::RecordBatch, AvengerChartError> {
        use datafusion::arrow::array::Float32Array;
        use datafusion::arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        // Calculate center of plot area
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Get existing schema and columns
        let schema = batch.schema();
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        let mut columns: Vec<Arc<dyn datafusion::arrow::array::Array>> = batch.columns().to_vec();

        // Add polar_center_x column
        let center_x_array = Float32Array::from(vec![center_x; batch.num_rows()]);
        columns.push(Arc::new(center_x_array));
        fields.push(Field::new("polar_center_x", DataType::Float32, false));

        // Add polar_center_y column
        let center_y_array = Float32Array::from(vec![center_y; batch.num_rows()]);
        columns.push(Arc::new(center_y_array));
        fields.push(Field::new("polar_center_y", DataType::Float32, false));

        // Create new batch with additional columns
        let new_schema = Arc::new(Schema::new(fields));
        datafusion::arrow::record_batch::RecordBatch::try_new(new_schema, columns)
            .map_err(AvengerChartError::ArrowError)
    }
}
