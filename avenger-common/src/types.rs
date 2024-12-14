use crate::impl_hash_for_scalar_or_array;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeCap {
    #[default]
    Butt,
    Round,
    Square,
}
impl_hash_for_scalar_or_array!(StrokeCap);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}
impl_hash_for_scalar_or_array!(StrokeJoin);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageAlign {
    #[default]
    Left,
    Center,
    Right,
}
impl_hash_for_scalar_or_array!(ImageAlign);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageBaseline {
    #[default]
    Top,
    Middle,
    Bottom,
}
impl_hash_for_scalar_or_array!(ImageBaseline);

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
impl_hash_for_scalar_or_array!(ColorOrGradient);

impl ColorOrGradient {
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
impl_hash_for_scalar_or_array!(Gradient);

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

#[derive(Default, Debug, Copy, Clone, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}
impl_hash_for_scalar_or_array!(AreaOrientation);
