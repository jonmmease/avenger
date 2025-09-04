use crate::coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult};
use crate::error::AvengerChartError;
use crate::polar::{PolarAxis, PolarAxisType, PolarDirection};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::Expr;
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

        // Use center coordinates from scalar batch columns
        // These are injected by prepare_scalar_batch
        use datafusion::logical_expr::col;
        let cx = col("center_x");
        let cy = col("center_y");

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

                let axis = PolarAxis::default()
                    .axis_type(axis_type)
                    .visible(true)
                    .grid(true)
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
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut axis_marks = Vec::new();

        // Render each axis using its render method
        for (channel, axis) in axes {
            let scale = scales.get(channel);
            if let Some(scale) = scale {
                let marks =
                    axis.render(channel, scale, scales, plot_width, plot_height, padding)?;
                axis_marks.extend(marks);
            }
        }

        Ok(axis_marks)
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        let mut options = HashMap::new();

        match (channel, scale_type) {
            // Radial scales often start at zero
            ("r", "linear") => {
                options.insert("zero".to_string(), lit(true));
                options.insert("nice".to_string(), lit(true));
            }
            // Angular scales for continuous data
            ("theta", "linear") => {
                options.insert("nice".to_string(), lit(true));
            }
            // Nice for other numeric scales
            ("r" | "theta", "log" | "pow" | "sqrt" | "symlog") => {
                options.insert("nice".to_string(), lit(true));
            }
            _ => {}
        }

        options
    }

    fn prepare_scalar_batch(
        &self,
        batch: datafusion::arrow::record_batch::RecordBatch,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<datafusion::arrow::record_batch::RecordBatch, AvengerChartError> {
        use datafusion::arrow::array::Float32Array;
        use datafusion::arrow::datatypes::{DataType, Field};
        use std::sync::Arc;

        // Calculate the center of the plot area
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Create arrays with the center coordinates
        let center_x_array = Float32Array::from(vec![center_x]);
        let center_y_array = Float32Array::from(vec![center_y]);

        // Get the existing columns from the batch
        let mut columns: Vec<Arc<dyn datafusion::arrow::array::Array>> = batch.columns().to_vec();
        let mut fields: Vec<Arc<Field>> = batch.schema().fields().to_vec();

        // Add polar_center_x column if not present (mark expects this name)
        if batch.column_by_name("polar_center_x").is_none() {
            columns.push(Arc::new(center_x_array));
            fields.push(Arc::new(Field::new(
                "polar_center_x",
                DataType::Float32,
                false,
            )));
        }

        // Add polar_center_y column if not present (mark expects this name)
        if batch.column_by_name("polar_center_y").is_none() {
            columns.push(Arc::new(center_y_array));
            fields.push(Arc::new(Field::new(
                "polar_center_y",
                DataType::Float32,
                false,
            )));
        }

        // Create new schema and batch with center coordinates
        let new_schema = Arc::new(datafusion::arrow::datatypes::Schema::new(fields));
        Ok(datafusion::arrow::record_batch::RecordBatch::try_new(
            new_schema, columns,
        )?)
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
}
