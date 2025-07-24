use crate::axis::{AxisPosition, AxisTrait, CartesianAxis};
use crate::error::AvengerChartError;
use crate::scales::ScaleRange;
use datafusion::functions::math::expr_fn::{cos, sin};
use datafusion::logical_expr::{Expr, lit};
use std::collections::HashMap;

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

pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The axis type for this coordinate system
    type Axis: AxisTrait + Clone + 'static;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Get default range for a specific position channel based on inner plot dimensions
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange>;

    /// Transform position channel expressions to screen coordinates
    ///
    /// # Arguments
    /// * `channels` - Map from channel name (e.g., "r", "theta") to expressions
    ///                that compute the scaled values for those channels
    ///
    /// # Returns
    /// Result containing TransformResult or error if required channels are missing
    fn transform_expressions(
        &self,
        channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError>;

    /// Get default axis configuration for a channel
    fn default_axis(channel: &str) -> Option<Self::Axis>;
}

pub struct Cartesian;

impl CoordinateSystem for Cartesian {
    type Axis = CartesianAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange> {
        match channel {
            "x" => Some(ScaleRange::new_interval(lit(0.0), lit(width))),
            "y" => Some(ScaleRange::new_interval(lit(height), lit(0.0))), // Inverted for screen coords
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

    fn default_axis(channel: &str) -> Option<Self::Axis> {
        match channel {
            "x" => Some(
                CartesianAxis::new()
                    .position(AxisPosition::Bottom)
                    .label_angle(0.0),
            ),
            "y" => Some(
                CartesianAxis::new()
                    .position(AxisPosition::Left)
                    .label_angle(0.0),
            ),
            _ => None,
        }
    }
}

pub struct Polar;

impl CoordinateSystem for Polar {
    type Axis = CartesianAxis; // TODO: Implement PolarAxis for proper polar coordinate support

    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<ScaleRange> {
        match channel {
            "theta" => Some(ScaleRange::new_interval(
                lit(0.0),
                lit(2.0 * std::f64::consts::PI),
            )),
            "r" => {
                let max_radius = f64::min(width, height) / 2.0;
                Some(ScaleRange::new_interval(lit(0.0), lit(max_radius)))
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

        // Transform to cartesian coordinates
        // x = r * cos(theta)
        // y = r * sin(theta)
        let x = r.clone() * cos(theta.clone());
        let y = r * sin(theta);

        Ok(TransformResult { x, y, depth: None })
    }

    fn default_axis(channel: &str) -> Option<Self::Axis> {
        match channel {
            "r" => Some(CartesianAxis::new()), // TODO: Implement PolarAxis for radial axis
            "theta" => Some(CartesianAxis::new()), // TODO: Implement PolarAxis for angular axis
            _ => None,
        }
    }
}
