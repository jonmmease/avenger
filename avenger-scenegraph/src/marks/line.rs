use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use lyon_path::{geom::point, Path};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use super::{
    mark::{default_interactive, SceneMark},
    stroke_dash::dash_paths,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneLineMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,
}

impl std::hash::Hash for SceneLineMark {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.interactive.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.gradients.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.defined.hash(state);
        self.stroke.hash(state);
        OrderedFloat(self.stroke_width).hash(state);
        self.stroke_cap.hash(state);
        self.stroke_join.hash(state);
        if let Some(stroke_dash) = &self.stroke_dash {
            stroke_dash
                .iter()
                .for_each(|d| OrderedFloat(*d).hash(state));
        } else {
            OrderedFloat(0.0).hash(state);
        }
        self.zindex.hash(state);
    }
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
            Some(dash) => dash_paths(std::iter::once(&path), dash),
            None => path,
        }
    }
}

impl Default for SceneLineMark {
    fn default() -> Self {
        Self {
            name: "line_mark".to_string(),
            interactive: true,
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
