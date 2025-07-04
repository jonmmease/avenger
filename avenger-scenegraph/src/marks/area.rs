use avenger_common::types::{AreaOrientation, ColorOrGradient, Gradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_path::{builder::WithSvg, geom::point, BuilderImpl, Path};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::hash::Hash for SceneAreaMark {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.orientation.hash(state);
        self.gradients.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.x2.hash(state);
        self.y2.hash(state);
        self.defined.hash(state);
        self.fill.hash(state);
        self.stroke.hash(state);
        self.stroke_cap.hash(state);
        self.stroke_join.hash(state);
        self.zindex.hash(state);

        if let Some(stroke_dash) = &self.stroke_dash {
            stroke_dash
                .iter()
                .for_each(|d| OrderedFloat(*d).hash(state));
        } else {
            OrderedFloat(0.0).hash(state);
        }
        OrderedFloat(self.stroke_width).hash(state);
    }
}

impl SceneAreaMark {
    pub fn x0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn x1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x2.as_iter(self.len as usize, None)
    }

    pub fn y1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y2.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }

    pub fn transformed_path(&self, origin: [f32; 2]) -> Path {
        let mut path_builder = Path::builder().with_svg();
        let mut tail: Vec<(f32, f32)> = Vec::new();

        fn close_area(b: &mut WithSvg<BuilderImpl>, tail: &mut Vec<(f32, f32)>) {
            if tail.is_empty() {
                return;
            }
            for (x, y) in tail.iter().rev() {
                b.line_to(point(*x, *y));
            }

            tail.clear();
            b.close();
        }

        if self.orientation == AreaOrientation::Vertical {
            for (x, y, y2, defined) in izip!(
                self.x0_iter(),
                self.y0_iter(),
                self.y1_iter(),
                self.defined_iter(),
            ) {
                if *defined {
                    if !tail.is_empty() {
                        // Continue path
                        path_builder.line_to(point(*x + origin[0], *y + origin[1]));
                    } else {
                        // New path
                        path_builder.move_to(point(*x + origin[0], *y + origin[1]));
                    }
                    tail.push((*x + origin[0], *y2 + origin[1]));
                } else {
                    close_area(&mut path_builder, &mut tail);
                }
            }
        } else {
            for (y, x, x2, defined) in izip!(
                self.y0_iter(),
                self.x0_iter(),
                self.x1_iter(),
                self.defined_iter(),
            ) {
                if *defined {
                    if !tail.is_empty() {
                        // Continue path
                        path_builder.line_to(point(*x + origin[0], *y + origin[1]));
                    } else {
                        // New path
                        path_builder.move_to(point(*x + origin[0], *y + origin[1]));
                    }
                    tail.push((*x2 + origin[0], *y + origin[1]));
                } else {
                    close_area(&mut path_builder, &mut tail);
                }
            }
        }

        close_area(&mut path_builder, &mut tail);
        path_builder.build()
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
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            x2: ScalarOrArray::new_scalar(0.0),
            y2: ScalarOrArray::new_scalar(0.0),
            defined: ScalarOrArray::new_scalar(true),
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

impl From<SceneAreaMark> for SceneMark {
    fn from(mark: SceneAreaMark) -> Self {
        SceneMark::Area(mark)
    }
}
