use crate::value::{EncodingValue, StrokeCap, StrokeJoin};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PathMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_width: Option<f32>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub path: EncodingValue<lyon_path::Path>,
    pub scale_x: EncodingValue<f32>,
    pub scale_y: EncodingValue<f32>,
    pub fill: EncodingValue<[f32; 4]>,
    pub stroke: EncodingValue<[f32; 4]>,
    pub angle: EncodingValue<f32>,
    pub indices: Option<Vec<usize>>,
}

impl PathMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn path_iter(&self) -> Box<dyn Iterator<Item = &lyon_path::Path> + '_> {
        self.path.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn scale_x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.scale_x
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn scale_y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.scale_y
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
}

impl Default for PathMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            stroke_width: Some(0.0),
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            path: EncodingValue::Scalar {
                value: lyon_path::Path::default(),
            },
            scale_x: EncodingValue::Scalar { value: 1.0 },
            scale_y: EncodingValue::Scalar { value: 1.0 },
            fill: EncodingValue::Scalar {
                value: [0.0, 0.0, 0.0, 0.0],
            },
            stroke: EncodingValue::Scalar {
                value: [0.0, 0.0, 0.0, 0.0],
            },
            angle: EncodingValue::Scalar { value: 0.0 },
            indices: None,
        }
    }
}
