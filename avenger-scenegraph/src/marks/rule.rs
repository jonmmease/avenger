use super::mark::SceneMark;
use crate::marks::mark::default_interactive;
use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::StrokeCap;
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_path::{geom::Point, Path};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneRuleMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub x2: ScalarOrArray<f32>,
    pub y2: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke_cap: ScalarOrArray<StrokeCap>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl SceneRuleMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x2.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y2.as_iter(self.len as usize, self.indices.as_ref())
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

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }

    pub fn undashed_path_iter(&self, origin: [f32; 2]) -> impl Iterator<Item = Path> + '_ {
        izip!(self.x_iter(), self.y_iter(), self.x2_iter(), self.y2_iter()).map(
            move |(x0, y0, x1, y1)| {
                let mut builder = Path::builder();
                builder.begin(Point::new(*x0 + origin[0], *y0 + origin[1]));
                builder.line_to(Point::new(*x1 + origin[0], *y1 + origin[1]));
                builder.end(false);
                builder.build()
            },
        )
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        let paths = self.undashed_path_iter(origin);
        match self.stroke_dash_iter() {
            Some(dashes) => {
                Box::new(paths.zip(dashes).map(|(path, dash)| {
                    super::stroke_dash::dash_paths(std::iter::once(&path), dash)
                }))
            }
            None => Box::new(paths),
        }
    }
}

impl Default for SceneRuleMark {
    fn default() -> Self {
        Self {
            interactive: true,
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_dash: None,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            x2: ScalarOrArray::new_scalar(0.0),
            y2: ScalarOrArray::new_scalar(0.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke_width: ScalarOrArray::new_scalar(1.0),
            stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneRuleMark> for SceneMark {
    fn from(mark: SceneRuleMark) -> Self {
        SceneMark::Rule(mark)
    }
}
