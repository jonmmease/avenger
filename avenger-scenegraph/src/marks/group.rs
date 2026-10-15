use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{
    hash::hash_floats,
    lyon::{hash_lyon_path, lyon_paths_equal},
    types::PathTransform,
    value::ScalarOrArray,
};
use educe::Educe;
use lyon_path::{
    geom::{euclid::Point2D, Box2D},
    Winding,
};
use serde::{Deserialize, Serialize};

use crate::marks::{mark::SceneMark, path::ScenePathMark};

#[derive(Debug, Clone, Default, Educe, Serialize, Deserialize)]
#[educe(Hash, PartialEq)]
pub enum Clip {
    #[default]
    None,
    Rect {
        #[educe(Hash(method(hash_floats)))]
        x: f32,
        #[educe(Hash(method(hash_floats)))]
        y: f32,
        #[educe(Hash(method(hash_floats)))]
        width: f32,
        #[educe(Hash(method(hash_floats)))]
        height: f32,
    },
    Path(
        #[educe(Hash(method(hash_lyon_path)), PartialEq(method(lyon_paths_equal)))] lyon_path::Path,
    ),
}

impl Clip {
    pub fn maybe_clip(&self, should_clip: bool) -> Self {
        if !should_clip {
            Self::None
        } else {
            self.clone()
        }
    }

    pub fn translate(&self, translate_x: f32, translate_y: f32) -> Self {
        match self {
            Clip::None => Clip::None,
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => Clip::Rect {
                x: *x + translate_x,
                y: *y + translate_y,
                width: *width,
                height: *height,
            },
            Clip::Path(path) => Clip::Path(
                path.clone()
                    .transformed(&PathTransform::translation(translate_x, translate_y)),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
pub struct SceneGroup {
    pub name: String,
    /// Whether hits find the group's marks, for each mark that doesn't set its own. `None` takes
    /// the enclosing group's setting, and groups at the scene root are interactive.
    pub interactive: Option<bool>,
    #[educe(Hash(method(hash_floats)))]
    pub origin: [f32; 2],
    /// Where group-anchored patterns start for the marks in this group and its descendants, in
    /// the group's coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[educe(Hash(method(hash_floats)))]
    pub pattern_origin: Option<[f32; 2]>,
    pub clip: Clip,
    pub marks: Vec<SceneMark>,
    pub gradients: Vec<Gradient>,
    pub fill: Option<ColorOrGradient>,
    pub stroke: Option<ColorOrGradient>,
    #[educe(Hash(method(hash_floats)))]
    pub stroke_width: Option<f32>,
    #[educe(Hash(method(hash_floats)))]
    pub stroke_offset: Option<f32>,
    pub zindex: Option<i32>,
}

impl SceneGroup {
    pub fn make_path_mark(&self) -> Option<ScenePathMark> {
        if self.fill.is_none() && self.stroke.is_none() {
            return None;
        }
        let stroke_width =
            self.stroke_width
                .unwrap_or(if self.stroke.is_some() { 1.0 } else { 0.0 });
        let stroke_offset = if let Some(stroke_offset) = self.stroke_offset {
            stroke_offset
        } else {
            // From Vega's default stroke offset logic
            if self.stroke.is_some() && stroke_width > 0.5 && stroke_width < 1.5 {
                0.5 - (stroke_width - 1.0).abs()
            } else {
                0.0
            }
        };

        // Convert clip region to path
        let path = match &self.clip {
            Clip::None => return None,
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                let mut builder = lyon_path::Path::builder();
                let x = self.origin[0] + *x + stroke_offset;
                let y = self.origin[1] + *y + stroke_offset;
                builder.add_rectangle(
                    &Box2D::new(Point2D::new(x, y), Point2D::new(x + width, y + height)),
                    Winding::Positive,
                );
                builder.build()
            }
            Clip::Path(path) => path.clone().transformed(&PathTransform::translation(
                self.origin[0] + stroke_offset,
                self.origin[1] + stroke_offset,
            )),
        };

        Some(ScenePathMark {
            name: format!("path_{}", self.name),
            interactive: self.interactive,
            clip: false,
            len: 1,
            gradients: self.gradients.clone(),
            path: ScalarOrArray::new_scalar(path),
            fill: ScalarOrArray::new_scalar(
                self.fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            ),
            fill_pattern: Default::default(),
            stroke: ScalarOrArray::new_scalar(
                self.stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            ),
            stroke_width: Some(stroke_width),
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            transform: ScalarOrArray::new_scalar(PathTransform::identity()),
            indices: None,
            zindex: self.zindex,
        })
    }

    pub fn group_paths(&self) -> Vec<Vec<usize>> {
        let mut paths = vec![];
        for (index, mark) in self.marks.iter().enumerate() {
            let SceneMark::Group(group) = mark else {
                continue;
            };
            paths.push(vec![index]);
            for sub_path in group.group_paths() {
                let mut path = vec![index];
                path.extend(sub_path);
                paths.push(path);
            }
        }
        paths
    }
}

impl Default for SceneGroup {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            interactive: None,
            origin: [0.0, 0.0],
            pattern_origin: None,
            clip: Default::default(),
            marks: vec![],
            gradients: vec![],
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        }
    }
}

impl From<SceneGroup> for SceneMark {
    fn from(mark: SceneGroup) -> Self {
        SceneMark::Group(mark)
    }
}
