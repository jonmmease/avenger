use crate::marks::value::{ColorOrGradient, Gradient, ScalarOrArray, StrokeCap, StrokeJoin};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneAreaMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub orientation: AreaOrientation,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub x2: ScalarOrArray<f32>,
    pub y2: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub fill: ColorOrGradient,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,
}

impl SceneAreaMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn x2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x2.as_iter(self.len as usize, None)
    }

    pub fn y2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y2.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }
}

impl Default for SceneAreaMark {
    fn default() -> Self {
        Self {
            name: "area_mark".to_string(),
            clip: true,
            len: 1,
            orientation: Default::default(),
            gradients: vec![],
            x: ScalarOrArray::Scalar { value: 0.0 },
            y: ScalarOrArray::Scalar { value: 0.0 },
            x2: ScalarOrArray::Scalar { value: 0.0 },
            y2: ScalarOrArray::Scalar { value: 0.0 },
            defined: ScalarOrArray::Scalar { value: true },
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
            zindex: None,
        }
    }
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}
