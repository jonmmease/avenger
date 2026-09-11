use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{
    types::{AreaOrientation, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use itertools::izip;
use lyon_path::{builder::WithSvg, geom::point, BuilderImpl, Path};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use super::{
    mark::SceneMark,
    pattern::PatternFill,
    stroke_dash::{combine_paths, dash_paths},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneAreaMark {
    pub name: String,
    /// Whether hits find the mark. `None` takes the enclosing group's setting, and marks at the
    /// scene root are interactive.
    pub interactive: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_pattern: Option<PatternFill>,
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
        self.interactive.hash(state);
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
        self.fill_pattern.hash(state);
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
        combine_paths(&self.run_paths(origin))
    }

    /// The outline of each run of defined points, as its own path.
    fn run_paths(&self, origin: [f32; 2]) -> Vec<Path> {
        let mut runs = Vec::new();
        let mut path_builder = Path::builder().with_svg();
        let mut tail: Vec<(f32, f32)> = Vec::new();

        fn close_area(
            b: &mut WithSvg<BuilderImpl>,
            tail: &mut Vec<(f32, f32)>,
            runs: &mut Vec<Path>,
        ) {
            if tail.is_empty() {
                return;
            }
            for (x, y) in tail.iter().rev() {
                b.line_to(point(*x, *y));
            }

            tail.clear();
            b.close();
            runs.push(std::mem::replace(b, Path::builder().with_svg()).build());
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
                    close_area(&mut path_builder, &mut tail, &mut runs);
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
                    close_area(&mut path_builder, &mut tail, &mut runs);
                }
            }
        }

        close_area(&mut path_builder, &mut tail, &mut runs);
        runs
    }

    /// The outline to stroke. Each run's dashes start at the run's first point, as a line's do.
    pub fn transformed_stroke_path(&self, origin: [f32; 2]) -> Path {
        let runs = self.run_paths(origin);
        if let Some(stroke_dash) = &self.stroke_dash {
            dash_paths(&runs, stroke_dash)
        } else {
            combine_paths(&runs)
        }
    }
}

impl Default for SceneAreaMark {
    fn default() -> Self {
        Self {
            name: "area_mark".to_string(),
            interactive: None,
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
            fill_pattern: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_path::Event;

    #[test]
    fn default_fill_pattern_is_skipped_when_serializing() {
        let value = serde_json::to_value(SceneAreaMark::default()).unwrap();

        assert!(value.get("fill-pattern").is_none());
    }

    #[test]
    fn missing_fill_pattern_deserializes_as_no_overlay() {
        let mut value = serde_json::to_value(SceneAreaMark::default()).unwrap();
        value.as_object_mut().unwrap().remove("fill-pattern");

        let mark: SceneAreaMark = serde_json::from_value(value).unwrap();

        assert!(mark.fill_pattern.is_none());
    }

    #[test]
    fn transformed_stroke_path_applies_dash_pattern() {
        let mark = SceneAreaMark {
            len: 2,
            x: ScalarOrArray::new_array(vec![0.0, 10.0]),
            y: ScalarOrArray::new_array(vec![0.0, 0.0]),
            x2: ScalarOrArray::new_array(vec![0.0, 10.0]),
            y2: ScalarOrArray::new_array(vec![10.0, 10.0]),
            stroke_dash: Some(vec![2.0, 2.0]),
            ..Default::default()
        };

        let fill_path = mark.transformed_path([0.0, 0.0]);
        let stroke_path = mark.transformed_stroke_path([0.0, 0.0]);

        assert!(begin_count(&stroke_path) > begin_count(&fill_path));
        assert!(stroke_path.iter().count() > fill_path.iter().count());
    }

    /// The dashes of a run after an undefined point start the pattern over.
    #[test]
    fn each_defined_run_starts_its_own_dash_pattern() {
        let mark = SceneAreaMark {
            len: 5,
            x: ScalarOrArray::new_array(vec![0.0, 20.0, 30.0, 40.0, 63.0]),
            y: ScalarOrArray::new_array(vec![0.0, 0.0, 0.0, 0.0, 0.0]),
            x2: ScalarOrArray::new_array(vec![0.0, 20.0, 30.0, 40.0, 63.0]),
            y2: ScalarOrArray::new_array(vec![10.0, 10.0, 10.0, 10.0, 10.0]),
            defined: ScalarOrArray::new_array(vec![true, true, false, true, true]),
            // A 60-unit outline before the gap ends partway through the 11-unit pattern, and the
            // 66-unit outline after it ends at a pattern boundary.
            stroke_dash: Some(vec![7.0, 4.0]),
            ..Default::default()
        };
        let dashes = mark.transformed_stroke_path([0.0, 0.0]);
        let second_run_start = lyon_path::math::point(40.0, 0.0);
        let mut first_dash_length = None;
        let mut current: Option<(lyon_path::math::Point, f32)> = None;
        for event in dashes.iter() {
            match event {
                Event::Begin { at } => current = Some((at, 0.0)),
                Event::Line { from, to } => {
                    if let Some((_, length)) = current.as_mut() {
                        *length += (to - from).length();
                    }
                }
                Event::End { .. } => {
                    if let Some((start, length)) = current.take() {
                        if start == second_run_start && first_dash_length.is_none() {
                            first_dash_length = Some(length);
                        }
                    }
                }
                _ => {}
            }
        }
        let length = first_dash_length.expect("a dash starts the second run");
        assert!((length - 7.0).abs() < 1e-3, "{length}");
    }

    fn begin_count(path: &Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Begin { .. }))
            .count()
    }
}
