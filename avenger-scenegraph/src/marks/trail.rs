use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTrailMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke: ColorOrGradient,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub size: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub zindex: Option<i32>,
}

impl SceneTrailMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }
}

impl Default for SceneTrailMark {
    fn default() -> Self {
        Self {
            name: "trail_mark".to_string(),
            clip: true,
            len: 1,
            x: ScalarOrArray::Scalar(0.0),
            y: ScalarOrArray::Scalar(0.0),
            size: ScalarOrArray::Scalar(1.0),
            defined: ScalarOrArray::Scalar(true),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            gradients: vec![],
            zindex: None,
        }
    }
}

impl From<SceneTrailMark> for SceneMark {
    fn from(mark: SceneTrailMark) -> Self {
        SceneMark::Trail(mark)
    }
}
