use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EncodingValue<T: Sync + Clone> {
    Scalar { value: T },
    Array { values: Vec<T> },
}

impl<T: Sync + Clone> EncodingValue<T> {
    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Vec<usize>>,
    ) -> Box<dyn Iterator<Item = &T> + '_> {
        match self {
            EncodingValue::Scalar { value } => Box::new(std::iter::repeat(value).take(scalar_len)),
            EncodingValue::Array { values } => match indices {
                None => Box::new(values.iter()),
                Some(indices) => Box::new(indices.iter().map(|i| &values[*i])),
            },
        }
    }

    pub fn as_vec(&self, scalar_len: usize, indices: Option<&Vec<usize>>) -> Vec<T> {
        self.as_iter(scalar_len, indices)
            .cloned()
            .collect::<Vec<_>>()
    }
}

impl EncodingValue<f32> {
    pub fn equals_scalar(&self, v: f32) -> bool {
        match self {
            EncodingValue::Scalar { value } => v == *value,
            _ => false,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageBaseline {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorOrGradient {
    Color([f32; 4]),
    GradientIndex(u32),
}

impl ColorOrGradient {
    pub fn color_or_transparent(&self) -> [f32; 4] {
        match self {
            ColorOrGradient::Color(c) => *c,
            _ => [0.0, 0.0, 0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: [f32; 4],
}
