use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray, StrokeCap};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneRuleMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    pub x0: ScalarOrArray<f32>,
    pub y0: ScalarOrArray<f32>,
    pub x1: ScalarOrArray<f32>,
    pub y1: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke_cap: ScalarOrArray<StrokeCap>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SceneRuleMark {
    pub fn x0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x0.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y0.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x1.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y1.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_cap_iter(&self) -> Box<dyn Iterator<Item = &StrokeCap> + '_> {
        self.stroke_cap
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_dash_iter(&self) -> Option<Box<dyn Iterator<Item = &Vec<f32>> + '_>> {
        if let Some(stroke_dash) = &self.stroke_dash {
            Some(stroke_dash.as_iter(self.len as usize, self.indices.as_ref()))
        } else {
            None
        }
    }
}

impl Default for SceneRuleMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_dash: None,
            x0: ScalarOrArray::Scalar { value: 0.0 },
            y0: ScalarOrArray::Scalar { value: 0.0 },
            x1: ScalarOrArray::Scalar { value: 0.0 },
            y1: ScalarOrArray::Scalar { value: 0.0 },
            stroke: ScalarOrArray::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            },
            stroke_width: ScalarOrArray::Scalar { value: 1.0 },
            stroke_cap: ScalarOrArray::Scalar {
                value: StrokeCap::Butt,
            },
            indices: None,
            zindex: None,
        }
    }
}
