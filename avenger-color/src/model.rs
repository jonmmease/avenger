use std::hash::{Hash, Hasher};

use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorOrGradient {
    Color([f32; 4]),
    GradientIndex(u32),
}

impl Hash for ColorOrGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ColorOrGradient::Color(c) => [
                OrderedFloat::from(c[0]),
                OrderedFloat::from(c[1]),
                OrderedFloat::from(c[2]),
                OrderedFloat::from(c[3]),
            ]
            .hash(state),
            ColorOrGradient::GradientIndex(i) => i.hash(state),
        }
    }
}

impl ColorOrGradient {
    pub fn transparent() -> Self {
        ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
    }

    pub fn color_or_transparent(&self) -> [f32; 4] {
        match self {
            ColorOrGradient::Color(c) => *c,
            _ => [0.0, 0.0, 0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Gradient {
    // Radial gradients include every linear field, so Serde must try them first.
    RadialGradient(RadialGradient),
    LinearGradient(LinearGradient),
}

impl Gradient {
    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Gradient::LinearGradient(grad) => grad.stops.as_slice(),
            Gradient::RadialGradient(grad) => grad.stops.as_slice(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stops: Vec<GradientStop>,
}

impl Hash for LinearGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        [self.x0, self.y0, self.x1, self.y1]
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));

        self.stops.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r0: f32,
    pub r1: f32,
    pub stops: Vec<GradientStop>,
}

impl Hash for RadialGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        [self.x0, self.y0, self.x1, self.y1, self.r0, self.r1]
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));

        self.stops.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: [f32; 4],
}

impl Hash for GradientStop {
    fn hash<H: Hasher>(&self, state: &mut H) {
        OrderedFloat::from(self.offset).hash(state);
        self.color
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));
    }
}

/// Apply opacity to a solid color. Gradient references are passed through
/// because gradient opacity is represented by individual gradient stops.
/// The opacity is clamped to `[0, 1]` before multiplying the existing alpha.
pub fn apply_opacity_to_color(color: &ColorOrGradient, opacity: f32) -> ColorOrGradient {
    match color {
        ColorOrGradient::Color(color) => {
            let mut color = *color;
            color[3] *= opacity.clamp(0.0, 1.0);
            ColorOrGradient::Color(color)
        }
        ColorOrGradient::GradientIndex(_) => color.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gradient_json_roundtrips_preserve_shape() {
        let linear = json!({
            "x0": 0.0, "y0": 0.25, "x1": 1.0, "y1": 0.75,
            "stops": [{"offset": 0.0, "color": [1.0, 0.0, 0.0, 0.5]}]
        });
        let mut radial = linear.clone();
        radial["r0"] = json!(0.25);
        radial["r1"] = json!(0.75);
        for (input, is_radial) in [(linear, false), (radial, true)] {
            let gradient: Gradient = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(matches!(gradient, Gradient::RadialGradient(_)), is_radial);
            assert_eq!(serde_json::to_value(&gradient).unwrap(), input);
        }
    }

    #[test]
    fn opacity_scales_solid_alpha_and_preserves_gradient_references() {
        let color = ColorOrGradient::Color([0.2, 0.4, 0.6, 0.5]);
        for (opacity, alpha) in [(-0.5, 0.0), (0.25, 0.125), (1.5, 0.5)] {
            assert_eq!(
                apply_opacity_to_color(&color, opacity),
                ColorOrGradient::Color([0.2, 0.4, 0.6, alpha])
            );
        }
        assert_eq!(
            apply_opacity_to_color(&ColorOrGradient::GradientIndex(2), 0.25),
            ColorOrGradient::GradientIndex(2)
        );
    }
}
