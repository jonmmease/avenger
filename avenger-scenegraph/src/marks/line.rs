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
    stroke_dash::{combine_paths, dash_paths},
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

    pub fn transformed_path(&self, origin: [f32; 2]) -> Path {
        let mut defined_paths: Vec<Path> = Vec::new();

        // Build path for each defined line segment
        let mut path_builder = Path::builder().with_svg();
        let mut path_len = 0;
        for (x, y, defined) in itertools::izip!(self.x_iter(), self.y_iter(), self.defined_iter()) {
            if *defined {
                if path_len > 0 {
                    // Continue path
                    path_builder.line_to(point(*x + origin[0], *y + origin[1]));
                } else {
                    // New path
                    path_builder.move_to(point(*x + origin[0], *y + origin[1]));
                }
                path_len += 1;
            } else {
                if path_len == 1 {
                    // Finishing single point line. Add extra point at the same location
                    // so that stroke caps are drawn
                    path_builder.close();
                }
                defined_paths.push(path_builder.build());
                path_builder = Path::builder().with_svg();
                path_len = 0;
            }
        }
        defined_paths.push(path_builder.build());

        if let Some(stroke_dash) = &self.stroke_dash {
            dash_paths(&defined_paths, stroke_dash)
        } else {
            combine_paths(&defined_paths)
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
