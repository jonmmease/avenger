use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use lyon_path::Winding;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub shapes: Vec<SymbolShape>,
    pub stroke_width: Option<f32>,
    pub shape_index: EncodingValue<usize>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub size: EncodingValue<f32>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub angle: EncodingValue<f32>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn x_vec(&self) -> Vec<f32> {
        self.x.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_vec(&self) -> Vec<f32> {
        self.y.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn size_vec(&self) -> Vec<f32> {
        self.size.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_vec(&self) -> Vec<f32> {
        self.angle.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_iter(&self) -> Box<dyn Iterator<Item = &usize> + '_> {
        self.shape_index
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_vec(&self) -> Vec<usize> {
        self.shape_index
            .as_vec(self.len as usize, self.indices.as_ref())
    }
}

impl Default for SymbolMark {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            clip: true,
            shapes: vec![Default::default()],
            stroke_width: None,
            len: 1,
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            shape_index: EncodingValue::Scalar { value: 0 },
            fill: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            size: EncodingValue::Scalar { value: 20.0 },
            stroke: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            angle: EncodingValue::Scalar { value: 0.0 },
            indices: None,
            gradients: vec![],
            zindex: None,
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolShape {
    #[default]
    Circle,
    /// Path with origin top-left
    Path(lyon_path::Path),
}

impl SymbolShape {
    pub fn as_path(&self) -> Cow<lyon_path::Path> {
        match self {
            SymbolShape::Circle => {
                let mut builder = lyon_path::Path::builder();
                builder.add_circle(lyon_path::geom::point(0.0, 0.0), 0.5, Winding::Positive);
                Cow::Owned(builder.build())
            }
            SymbolShape::Path(path) => Cow::Borrowed(path),
        }
    }
}
