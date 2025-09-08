use crate::axis::AxisPosition;
use crate::cartesian::CartesianAxis;
use crate::coords::{CoordinateSystem, OverflowSpaceRequirement};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Cartesian coordinate system with concrete axis type
#[derive(Clone, Default)]
pub struct Cartesian;

#[async_trait::async_trait]
impl CoordinateSystem for Cartesian {
    type Axis = CartesianAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "x" => Some((0.0, width)),
            "y" => Some((height, 0.0)), // Inverted for screen coords
            _ => None,
        }
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
                    scale.ticks(None).is_ok()
                } else {
                    false
                };

                // Create axis with appropriate default position
                let position = match channel {
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => AxisPosition::Bottom,
                };

                // Create a default axis instance with concrete type
                let axis = CartesianAxis::default()
                    .position(position)
                    .label_angle(0.0)
                    .title(title)
                    .grid(grid);

                default_axes.insert(channel.to_string(), axis);
            }
        }

        default_axes
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

        // Get the actual scale ranges to measure overflow against
        // The scale ranges define where data can be plotted
        let x_scale = scales
            .get("x")
            .ok_or_else(|| AvengerChartError::InternalError("No x scale found".to_string()))?;
        let y_scale = scales
            .get("y")
            .ok_or_else(|| AvengerChartError::InternalError("No y scale found".to_string()))?;

        let x_range = x_scale.numeric_interval_range()?;
        let y_range = y_scale.numeric_interval_range()?;

        // Calculate scale boundaries (plot is at origin for measurement)
        let scale_left = x_range.0.min(x_range.1);
        let scale_right = x_range.0.max(x_range.1);
        let scale_top = y_range.0.min(y_range.1);
        let scale_bottom = y_range.0.max(y_range.1);

        // Calculate overflow relative to scale boundaries
        // Use a threshold to ignore tiny overflows from anti-aliasing/rounding
        const THRESHOLD: f32 = 1.0; // Ignore overflows less than 1px as they're likely rounding errors
        let left = (scale_left - min_x).max(0.0);
        let right = (max_x - scale_right).max(0.0);
        let top = (scale_top - min_y).max(0.0);
        let bottom = (max_y - scale_bottom).max(0.0);

        // Round very small overflows to zero
        let left = if left < THRESHOLD { 0.0 } else { left };
        let right = if right < THRESHOLD { 0.0 } else { right };
        let top = if top < THRESHOLD { 0.0 } else { top };
        let bottom = if bottom < THRESHOLD { 0.0 } else { bottom };

        let result = OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        };

        Ok(result)
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

        for (channel, axis) in axes {
            // Get the scale for this axis
            let scale = scales.get(channel).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "No scale found for axis channel: {}",
                    channel
                ))
            })?;

            // Let the axis implementation handle all rendering logic
            let axis_mark = axis.render(channel, scale, plot_width, plot_height, padding, theme)?;

            // Only add non-empty marks
            if let SceneMark::Group(ref group) = axis_mark {
                if !group.marks.is_empty() {
                    axis_marks.push(axis_mark);
                }
            } else {
                axis_marks.push(axis_mark);
            }
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
        let is_position = matches!(channel, "x" | "x2" | "y" | "y2");
        let is_y_axis = matches!(channel, "y" | "y2");

        if is_position {
            // For continuous numeric scales
            if scale_impl.domain_kind() == DomainKind::Numeric
                && scale_impl.range_kind() == RangeKind::Continuous
            {
                // Y-axis scales typically include zero, X-axis scales don't necessarily
                if is_y_axis && scale_impl.scale_type() == "linear" {
                    options.insert("zero".to_string(), lit(true));
                }

                // Nice domain for better tick values
                options.insert("nice".to_string(), lit(true));

                // Pixel-aligned positions for crisp rendering
                options.insert("round".to_string(), lit(true));
            }
            // For temporal scales
            else if scale_impl.domain_kind() == DomainKind::Temporal
                && scale_impl.range_kind() == RangeKind::Continuous
            {
                // Pixel-aligned positions
                options.insert("round".to_string(), lit(true));
            }
        }

        options
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // Cartesian uses rectangular clipping
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn transform_to_plot_coords(
        &self,
        position_channels: &std::collections::HashMap<
            &str,
            avenger_common::value::ScalarOrArray<f32>,
        >,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<
        (
            avenger_common::value::ScalarOrArray<f32>,
            avenger_common::value::ScalarOrArray<f32>,
        ),
        AvengerChartError,
    > {
        use avenger_common::value::ScalarOrArray;

        // For Cartesian coordinates, x and y are already in plot coordinates
        // (scales map directly to plot area)
        let x = position_channels
            .get("x")
            .or_else(|| position_channels.get("x2"))
            .cloned()
            .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0));

        let y = position_channels
            .get("y")
            .or_else(|| position_channels.get("y2"))
            .cloned()
            .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0));

        Ok((x, y))
    }
}

/// Helper function to extract axis title from mark encodings
fn extract_axis_title_from_marks<C: crate::coords::CoordinateSystem>(
    marks: &[Box<dyn crate::marks::Mark<C>>],
    channel: &str,
) -> Option<String> {
    use datafusion::logical_expr::Expr;
    
    // Look through marks to find a column name for this channel
    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(channel) {
            // Skip literal values - they don't represent data dimensions
            if let Some(expr) = channel_value.expr() {
                if matches!(expr, Expr::Literal(_, _)) {
                    continue;
                }
            }
            
            // Try to get column name
            if let Some(col_name) = channel_value.as_column_name() {
                return Some(col_name);
            }
        }
    }
    None
}
