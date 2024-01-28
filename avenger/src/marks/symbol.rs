use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use serde::{Deserialize, Serialize};

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
}

impl SymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn shape_index_iter(&self) -> Box<dyn Iterator<Item = &usize> + '_> {
        self.shape_index
            .as_iter(self.len as usize, self.indices.as_ref())
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
