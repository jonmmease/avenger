use crate::coords::{CoordinateSystem, OverflowSpaceRequirement};
use crate::error::AvengerChartError;
use crate::polar::{PolarAxis, PolarAxisType, PolarDirection};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Polar coordinate system with concrete axis type
#[derive(Clone, Default)]
pub struct Polar {}

impl Polar {
    pub fn new() -> Self {
        Self {}
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

    async fn measure_guide_overflow(
        &self,
        axes: HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width_estimate: f32,
        height_estimate: f32,
        theme: &crate::theme::Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        use crate::render::Padding;
        use avenger_geometry::marks::MarkGeometryUtils;

        // Use the estimated plot dimensions directly
        let plot_width = width_estimate;
        let plot_height = height_estimate;

        // For overflow measurement, we can place the plot at origin
        // The relative overflow is what matters, not the absolute position
        let initial_padding = Padding {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        };

        // Render axes to measure their bounding box
        let axis_marks = self
            .render_axes(
                &axes,
                scales,
                plot_width,
                plot_height,
                &initial_padding,
                theme,
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

        // Calculate overflow relative to plot boundaries
        // Since we placed the plot at origin, boundaries are simple
        // Use a threshold to ignore tiny overflows from anti-aliasing/rounding
        const THRESHOLD: f32 = 1.0; // Ignore overflows less than 1px as they're likely rounding errors
        let left = (0.0 - min_x).max(0.0);
        let right = (max_x - plot_width).max(0.0);
        let top = (0.0 - min_y).max(0.0);
        let bottom = (max_y - plot_height).max(0.0);

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

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();

        // Create default axes for r and theta channels if they have scales
        for channel in ["r", "theta"] {
            if scales.get(channel).is_some() {
                // Determine if grid should be enabled based on scale type
                let grid = if let Some(scale) = scales.get(channel) {
                    scale.ticks(None).is_ok()
                } else {
                    false
                };

                // Create axis with appropriate defaults
                let axis_type = match channel {
                    "r" => PolarAxisType::Radial,
                    "theta" => PolarAxisType::Angular,
                    _ => PolarAxisType::Radial,
                };

                let axis = PolarAxis::default()
                    .axis_type(axis_type)
                    .visible(true)
                    .grid(grid)
                    .start_angle(0.0)
                    .direction(PolarDirection::Clockwise);

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
        theme: &crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut axis_marks = Vec::new();

        // Render each axis using its render method
        for (channel, axis) in axes {
            let scale = scales.get(channel).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "No scale found for axis channel: {}",
                    channel
                ))
            })?;

            let marks = axis.render(
                channel,
                scale,
                scales,
                plot_width,
                plot_height,
                padding,
                theme,
            )?;
            axis_marks.extend(marks);
        }

        Ok(axis_marks)
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        use avenger_scales::scales::{DomainKind, RangeKind};
        use datafusion::logical_expr::lit;
        let mut options = HashMap::new();

        // Check if this is a position channel
        let is_radial = channel == "r";
        let is_angular = channel == "theta";

        if is_radial || is_angular {
            // For continuous numeric scales
            if scale_impl.domain_kind() == DomainKind::Numeric
                && scale_impl.range_kind() == RangeKind::Continuous
            {
                // Radial scales typically start at zero
                if is_radial && scale_impl.scale_type() == "linear" {
                    options.insert("zero".to_string(), lit(true));
                }

                // Nice domain for better tick values
                options.insert("nice".to_string(), lit(true));
            }
        }

        options
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

                // Bottom-right quadrant (back to start)
                builder.cubic_bezier_to(
                    lyon_path::math::point(center_x + control_dist, center_y + max_radius),
                    lyon_path::math::point(center_x + max_radius, center_y + control_dist),
                    lyon_path::math::point(center_x + max_radius, center_y),
                );

                builder.close();
                let path = builder.build();

                return Clip::Path(path);
            }
        }

        // Fallback to rectangular clipping if scales are not configured properly
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn transform_to_plot_coords(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<
        (
            avenger_common::value::ScalarOrArray<f32>,
            avenger_common::value::ScalarOrArray<f32>,
        ),
        AvengerChartError,
    > {
        use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};

        // Get r and theta from position channels
        let r = position_channels
            .get("r")
            .cloned()
            .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0));

        let theta = position_channels
            .get("theta")
            .cloned()
            .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0));

        // Calculate center of plot area
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Transform r/theta to x/y
        let (x, y) = match (r.value(), theta.value()) {
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Scalar(theta_val)) => {
                // Both are scalars
                let x_val = center_x + r_val * theta_val.cos();
                let y_val = center_y + r_val * theta_val.sin();
                (
                    ScalarOrArray::new_scalar(x_val),
                    ScalarOrArray::new_scalar(y_val),
                )
            }
            (ScalarOrArrayValue::Array(r_arr), ScalarOrArrayValue::Array(theta_arr)) => {
                // Both are arrays
                let mut x_values = Vec::with_capacity(r_arr.len());
                let mut y_values = Vec::with_capacity(r_arr.len());

                for i in 0..r_arr.len() {
                    let r_val = r_arr[i];
                    let theta_val = theta_arr[i];
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (
                    ScalarOrArray::new_array(x_values),
                    ScalarOrArray::new_array(y_values),
                )
            }
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Array(theta_arr)) => {
                // r is scalar, theta is array
                let mut x_values = Vec::with_capacity(theta_arr.len());
                let mut y_values = Vec::with_capacity(theta_arr.len());

                for theta_val in theta_arr.iter() {
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (
                    ScalarOrArray::new_array(x_values),
                    ScalarOrArray::new_array(y_values),
                )
            }
            (ScalarOrArrayValue::Array(r_arr), ScalarOrArrayValue::Scalar(theta_val)) => {
                // r is array, theta is scalar
                let mut x_values = Vec::with_capacity(r_arr.len());
                let mut y_values = Vec::with_capacity(r_arr.len());

                for r_val in r_arr.iter() {
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (
                    ScalarOrArray::new_array(x_values),
                    ScalarOrArray::new_array(y_values),
                )
            }
        };

        Ok((x, y))
    }
}
