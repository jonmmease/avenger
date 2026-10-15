use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{
    hash::hash_floats,
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use educe::Educe;
use lyon_path::{geom::point, Path};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;
use crate::render::stroke::dash_path;

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct SceneLineMark {
    pub name: String,
    /// Whether hits find the mark. `None` takes the enclosing group's setting, and marks at the
    /// scene root are interactive.
    pub interactive: Option<bool>,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub stroke: ColorOrGradient,
    #[educe(Hash(method(hash_floats)))]
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    #[educe(Hash(method(hash_floats)))]
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,
}

impl SceneLineMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }

    pub fn undashed_path(&self, origin: [f32; 2]) -> Path {
        let mut builder = Path::builder();
        let mut last = None;
        let mut count = 0;
        for (x, y, defined) in itertools::izip!(self.x_iter(), self.y_iter(), self.defined_iter()) {
            if *defined {
                let at = point(*x + origin[0], *y + origin[1]);
                if count == 0 {
                    builder.begin(at);
                } else {
                    builder.line_to(at);
                }
                last = Some(at);
                count += 1;
            } else if count > 0 {
                if count == 1 {
                    builder.line_to(last.unwrap());
                }
                builder.end(false);
                count = 0;
            }
        }
        if count > 0 {
            if count == 1 {
                builder.line_to(last.unwrap());
            }
            builder.end(false);
        }
        builder.build()
    }

    pub fn transformed_path(&self, origin: [f32; 2]) -> Path {
        let path = self.undashed_path(origin);
        match &self.stroke_dash {
            Some(dash) => dash_path(&path, dash),
            None => path,
        }
    }
}

impl Default for SceneLineMark {
    fn default() -> Self {
        Self {
            interactive: None,
            name: "line_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            defined: ScalarOrArray::new_scalar(true),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
            zindex: None,
        }
    }
}

impl From<SceneLineMark> for SceneMark {
    fn from(mark: SceneLineMark) -> Self {
        SceneMark::Line(mark)
    }
}
