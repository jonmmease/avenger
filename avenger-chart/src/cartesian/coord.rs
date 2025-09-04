use crate::axis::AxisPosition;
use crate::cartesian::CartesianAxis;
use crate::coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::Expr;
use std::collections::HashMap;
use std::sync::Arc;

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

        // Calculate scale boundaries in screen coordinates
        // Add initial padding to convert from plot-relative to screen coordinates
        let scale_left = initial_padding.left + x_range.0.min(x_range.1);
        let scale_right = initial_padding.left + x_range.0.max(x_range.1);
        let scale_top = initial_padding.top + y_range.0.min(y_range.1);
        let scale_bottom = initial_padding.top + y_range.0.max(y_range.1);

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

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Arc<dyn avenger_scales::scales::ScaleImpl>> {
        use avenger_scales::scales::{
            linear::LinearScale, point::PointScale, time::TimeScale,
        };
        use datafusion::arrow::datatypes::DataType;
        
        // Handle position channels specifically
        match channel {
            "x" | "y" | "x2" | "y2" => {
                match data_type {
                    // Categorical data uses point scale for positions
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean => {
                        Some(Arc::new(PointScale))
                    }
                    // Temporal data uses time scale
                    DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => {
                        Some(Arc::new(TimeScale))
                    }
                    // Numeric data uses linear scale
                    DataType::Float32 | DataType::Float64 |
                    DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 |
                    DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
                        Some(Arc::new(LinearScale))
                    }
                    // Default to linear for unknown types
                    _ => Some(Arc::new(LinearScale))
                }
            }
            // Not a position channel - let marks decide
            _ => None
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        let mut options = HashMap::new();

        match (channel, scale_type) {
            // Y-axis linear scales typically include zero
            ("y" | "y2", "linear") => {
                options.insert("zero".to_string(), lit(true));
                options.insert("nice".to_string(), lit(true));
                options.insert("round".to_string(), lit(true)); // Pixel-aligned for crisp grid lines
            }
            // X-axis linear scales don't necessarily need zero
            ("x" | "x2", "linear") => {
                options.insert("nice".to_string(), lit(true));
                options.insert("round".to_string(), lit(true)); // Pixel-aligned for crisp grid lines
            }
            // For any numeric positional scale, enable rounding for pixel alignment
            ("x" | "x2" | "y" | "y2", "log" | "pow" | "sqrt" | "symlog" | "time") => {
                options.insert("round".to_string(), lit(true)); // Pixel-aligned positions
            }
            _ => {}
        }

        options
    }

    async fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
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
            let axis_mark = axis.render(channel, scale, plot_width, plot_height, padding)?;

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
}

/// Helper function to extract axis title from mark encodings
fn extract_axis_title_from_marks<C: crate::coords::CoordinateSystem>(
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
