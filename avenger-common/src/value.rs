use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ScalarOrArray<T: Sync + Clone> {
    Scalar { value: T },
    Array { values: Vec<T> },
}

impl<T: Sync + Clone> ScalarOrArray<T> {
    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Vec<usize>>,
    ) -> Box<dyn Iterator<Item = &T> + '_> {
        match self {
            ScalarOrArray::Scalar { value } => Box::new(std::iter::repeat(value).take(scalar_len)),
            ScalarOrArray::Array { values } => match indices {
                None => Box::new(values.iter()),
                Some(indices) => Box::new(indices.iter().map(|i| &values[*i])),
            },
        }
    }

    pub fn as_iter_owned<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Vec<usize>>,
    ) -> Box<dyn Iterator<Item = T> + '_> {
        match self {
            ScalarOrArray::Scalar { value } => {
                Box::new(std::iter::repeat(value.clone()).take(scalar_len))
            }
            ScalarOrArray::Array { values } => match indices {
                None => Box::new(values.iter().cloned()),
                Some(indices) => Box::new(indices.iter().map(|i| values[*i].clone())),
            },
        }
    }

    pub fn as_vec(&self, scalar_len: usize, indices: Option<&Vec<usize>>) -> Vec<T> {
        self.as_iter(scalar_len, indices)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn map<U: Sync + Clone>(&self, f: impl Fn(&T) -> U) -> ScalarOrArray<U> {
        match self {
            ScalarOrArray::Scalar { value } => ScalarOrArray::Scalar { value: f(value) },
            ScalarOrArray::Array { values } => ScalarOrArray::Array {
                values: values.iter().map(f).collect(),
            },
        }
    }
}

impl ScalarOrArray<f32> {
    pub fn equals_scalar(&self, v: f32) -> bool {
        match self {
            ScalarOrArray::Scalar { value } => v == *value,
            _ => false,
        }
    }
}

impl<T: Sync + Clone> From<Vec<T>> for ScalarOrArray<T> {
    fn from(values: Vec<T>) -> Self {
        ScalarOrArray::Array { values }
    }
}

impl<T: Sync + Clone> From<T> for ScalarOrArray<T> {
    fn from(value: T) -> Self {
        ScalarOrArray::Scalar { value }
    }
}

#[derive(Debug, Clone)]
pub enum ScalarOrArrayRef<'a, T: Sync + Clone> {
    Scalar { value: T },
    Array { values: &'a [T] },
}

impl<'a, T: Sync + Clone> ScalarOrArrayRef<'a, T> {
    pub fn from_slice(values: &'a [T]) -> Self {
        ScalarOrArrayRef::Array { values }
    }

    pub fn to_owned(self) -> ScalarOrArray<T> {
        match self {
            ScalarOrArrayRef::Scalar { value } => ScalarOrArray::Scalar {
                value: value.clone(),
            },
            ScalarOrArrayRef::Array { values } => ScalarOrArray::Array {
                values: values.to_vec(),
            },
        }
    }

    pub fn map<U: Sync + Clone>(self, f: impl Fn(&T) -> U) -> ScalarOrArray<U> {
        match self {
            ScalarOrArrayRef::Scalar { value } => ScalarOrArray::Scalar { value: f(&value) },
            ScalarOrArrayRef::Array { values } => ScalarOrArray::Array {
                values: values.iter().map(f).collect(),
            },
        }
    }
}

impl<'a, T: Sync + Clone> From<&'a [T]> for ScalarOrArrayRef<'a, T> {
    fn from(values: &'a [T]) -> Self {
        ScalarOrArrayRef::Array { values }
    }
}

impl<'a, T: Sync + Clone> From<&'a Vec<T>> for ScalarOrArrayRef<'a, T> {
    fn from(values: &'a Vec<T>) -> Self {
        ScalarOrArrayRef::Array {
            values: values.as_slice(),
        }
    }
}

impl<'a, T: Sync + Clone> From<&'a T> for ScalarOrArrayRef<'a, T> {
    fn from(value: &'a T) -> Self {
        ScalarOrArrayRef::Scalar {
            value: value.clone(),
        }
    }
}

impl<'a, T: Sync + Clone> From<T> for ScalarOrArrayRef<'a, T> {
    fn from(value: T) -> Self {
        ScalarOrArrayRef::Scalar { value }
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
