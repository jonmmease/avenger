use crate::coords::{
    CoordinateSystem, CoordinateSystemTransform, PointGeometry, extract_channel_title_from_marks,
};
use crate::error::AvengerChartError;
use crate::guide::CoordinateGuideBuilder;
use crate::polar::{PolarAxis, PolarAxisType, PolarGuide};
use avenger_scenegraph::marks::group::Clip;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Polar coordinate system with radial and angular axes
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Polar {}

impl Polar {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl CoordinateSystem for Polar {
    type Guide = PolarGuide;
    type PlotGeometry = PointGeometry;

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

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn crate::marks::CompiledMark>],
    ) -> HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis> {
        let mut default_axes = HashMap::new();

        // Create default axes for r and theta channels if they have scales
        for channel in ["r", "theta"] {
            if scales.get(channel).is_some() {
                let axis_type = match channel {
                    "r" => PolarAxisType::Radial,
                    "theta" => PolarAxisType::Angular,
                    _ => continue,
                };

                let mut axis = PolarAxis::new().axis_type(axis_type).grid(true); // Both radial and angular axes should show grid by default

                // Extract title from mark encodings
                if let Some(title) = extract_channel_title_from_marks(marks, channel) {
                    axis = axis.title(title);
                }

                default_axes.insert(channel.to_string(), axis);
            }
        }

        default_axes
    }

    fn create_default_guide(
        &self,
        axes: HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Arc<dyn crate::marks::CompiledMark>],
    ) -> Self::Guide {
        let mut guide = PolarGuide::new();
        guide.set_axes(axes);
        guide
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // Circular clipping for polar coordinates
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;
        let radius = plot_width.min(plot_height) / 2.0;

        // Create a circular path for clipping
        let mut builder = lyon_path::Path::builder();
        builder.add_circle(
            lyon_path::geom::point(center_x, center_y),
            radius,
            lyon_path::Winding::Positive,
        );
        let path = builder.build();

        Clip::Path(path)
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<PointGeometry, AvengerChartError> {
        use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};

        // Get r and theta channels
        let r = position_channels.get("r").ok_or_else(|| {
            AvengerChartError::InternalError("Missing r position channel".to_string())
        })?;

        let theta = position_channels.get("theta").ok_or_else(|| {
            AvengerChartError::InternalError("Missing theta position channel".to_string())
        })?;

        // Calculate center point
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Convert polar to Cartesian coordinates
        // x = center_x + r * cos(theta)
        // y = center_y + r * sin(theta)
        let (x, y) = match (r.value(), theta.value()) {
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Scalar(theta_val)) => {
                let x = center_x + r_val * theta_val.cos();
                let y = center_y + r_val * theta_val.sin();
                (ScalarOrArray::new_scalar(x), ScalarOrArray::new_scalar(y))
            }
            (ScalarOrArrayValue::Array(r_vals), ScalarOrArrayValue::Array(theta_vals)) => {
                if r_vals.len() != theta_vals.len() {
                    return Err(AvengerChartError::InternalError(
                        "r and theta arrays must have the same length".to_string(),
                    ));
                }

                let x_vals: Vec<f32> = r_vals
                    .iter()
                    .zip(theta_vals.iter())
                    .map(|(r, theta)| center_x + r * theta.cos())
                    .collect();

                let y_vals: Vec<f32> = r_vals
                    .iter()
                    .zip(theta_vals.iter())
                    .map(|(r, theta)| center_y + r * theta.sin())
                    .collect();

                (
                    ScalarOrArray::new_array(x_vals),
                    ScalarOrArray::new_array(y_vals),
                )
            }
            _ => {
                return Err(AvengerChartError::InternalError(
                    "r and theta must both be scalars or both be arrays".to_string(),
                ));
            }
        };

        Ok(PointGeometry { x, y })
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for Polar {
    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let geom = <Self as CoordinateSystem>::transform(
            self,
            position_channels,
            plot_width,
            plot_height,
        )?;
        Ok(Box::new(geom))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        <Self as CoordinateSystem>::default_range(self, channel, plot_area_width, plot_area_height)
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        // Use the same logic as the CoordinateSystem implementation
        use crate::scales::infer_scale_type_from_name;
        use avenger_scales::scales::{DomainKind, RangeKind};
        use datafusion::scalar::ScalarValue;

        let mut options = HashMap::new();

        // Determine domain and range kinds from scale type
        let scale_spec = infer_scale_type_from_name(scale_type);
        let (domain_kind, range_kind) = (scale_spec.domain_kind(), scale_spec.range_kind());

        // Apply polar-specific defaults
        if channel == "r" {
            // Radial axis typically starts at zero unless explicitly overridden
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
        } else if channel == "theta" {
            // Angular values are typically in radians or degrees
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                // Nice domain for better tick values
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
            }
        }

        options
    }
}
