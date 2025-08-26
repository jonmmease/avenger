use crate::coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult};
use crate::error::AvengerChartError;
use crate::polar::axis::DefaultPolarAxis;
use crate::polar::{PolarAxis, PolarAxisType, PolarDirection};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::Expr;
use std::collections::HashMap;

pub struct PolarGeneral<A: PolarAxis = DefaultPolarAxis> {
    // Fields for center injection from renderer
    center_x: Option<Expr>,
    center_y: Option<Expr>,
    _phantom: std::marker::PhantomData<A>,
}

impl<A: PolarAxis> PolarGeneral<A> {
    pub fn new() -> Self {
        Self {
            center_x: None,
            center_y: None,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<A: PolarAxis> Default for PolarGeneral<A> {
    fn default() -> Self {
        Self::new()
    }
}

pub type Polar = PolarGeneral<DefaultPolarAxis>;

#[async_trait::async_trait]
impl<A: PolarAxis> CoordinateSystem for PolarGeneral<A> {
    type Axis = A;

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

                let axis = A::default()
                    .with_axis_type(axis_type)
                    .with_visible(true)
                    .with_grid(true)
                    .with_start_angle(0.0)
                    .with_direction(PolarDirection::Clockwise);

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
