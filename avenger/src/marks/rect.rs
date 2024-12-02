use crate::marks::value::{ColorOrGradient, Gradient, ScalarOrArray};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RectMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub width: ScalarOrArray<f32>,
    pub height: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub corner_radius: ScalarOrArray<f32>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl RectMark {
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

    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn width_vec(&self) -> Vec<f32> {
        self.width.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn height_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.height
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn height_vec(&self) -> Vec<f32> {
        self.height.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_width_vec(&self) -> Vec<f32> {
        self.stroke_width
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn corner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.corner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn corner_radius_vec(&self) -> Vec<f32> {
        self.corner_radius
            .as_vec(self.len as usize, self.indices.as_ref())
    }
}

impl Default for RectMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::Scalar { value: 0.0 },
            y: ScalarOrArray::Scalar { value: 0.0 },
            width: ScalarOrArray::Scalar { value: 0.0 },
            height: ScalarOrArray::Scalar { value: 0.0 },
            fill: ScalarOrArray::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            stroke: ScalarOrArray::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            stroke_width: ScalarOrArray::Scalar { value: 0.0 },
            corner_radius: ScalarOrArray::Scalar { value: 0.0 },
            indices: None,
            zindex: None,
        }
    }
}
