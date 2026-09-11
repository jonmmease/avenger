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
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
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
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    use super::*;

    #[test]
    fn transparent_returns_zero_alpha_black() {
        assert_eq!(
            ColorOrGradient::transparent(),
            ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn gradient_stops_are_exposed_for_both_gradient_types() {
        let stops = vec![
            GradientStop {
                offset: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            GradientStop {
                offset: 1.0,
                color: [0.0, 0.0, 1.0, 1.0],
            },
        ];
        let linear = Gradient::LinearGradient(LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 0.0,
            stops: stops.clone(),
        });
        let radial = Gradient::RadialGradient(RadialGradient {
            x0: 0.5,
            y0: 0.5,
            x1: 0.5,
            y1: 0.5,
            r0: 0.0,
            r1: 0.5,
            stops: stops.clone(),
        });

        assert_eq!(linear.stops(), stops.as_slice());
        assert_eq!(radial.stops(), stops.as_slice());
    }

    #[test]
    fn hashing_handles_float_components() {
        let color = ColorOrGradient::Color([0.1, 0.2, 0.3, 0.4]);
        let mut first = DefaultHasher::new();
        let mut second = DefaultHasher::new();

        color.hash(&mut first);
        color.hash(&mut second);

        assert_eq!(first.finish(), second.finish());
    }

    #[test]
    fn opacity_applies_to_solid_color_only() {
        assert_eq!(
            apply_opacity_to_color(&ColorOrGradient::Color([0.2, 0.4, 0.6, 0.5]), 0.25),
            ColorOrGradient::Color([0.2, 0.4, 0.6, 0.125])
        );
        assert_eq!(
            apply_opacity_to_color(&ColorOrGradient::GradientIndex(2), 0.25),
            ColorOrGradient::GradientIndex(2)
        );
    }
}
