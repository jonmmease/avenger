use crate::value::EncodingValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolMark {
    pub name: String,
    pub clip: bool,
    pub shape: SymbolShape,
    pub stroke_width: Option<f32>,
    pub len: u32,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub fill: EncodingValue<[f32; 4]>,
    pub size: EncodingValue<f32>,
    pub stroke: EncodingValue<[f32; 4]>,
    pub angle: EncodingValue<f32>,
}

impl SymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize)
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.fill.as_iter(self.len as usize)
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize)
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.stroke.as_iter(self.len as usize)
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize)
    }
}

impl Default for SymbolMark {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            clip: true,
            shape: Default::default(),
            stroke_width: None,
            len: 1,
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            fill: EncodingValue::Scalar {
                value: [0.0, 0.0, 0.0, 0.0],
            },
            size: EncodingValue::Scalar { value: 20.0 },
            stroke: EncodingValue::Scalar {
                value: [0.0, 0.0, 0.0, 0.0],
            },
            angle: EncodingValue::Scalar { value: 0.0 },
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
