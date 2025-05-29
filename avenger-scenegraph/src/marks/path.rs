use super::mark::SceneMark;
use avenger_common::lyon::hash_lyon_path;
use avenger_common::types::{ColorOrGradient, Gradient, PathTransform, StrokeCap, StrokeJoin};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use itertools::izip;
use lyon_extra::euclid::Vector2D;
use lyon_path::Path;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hasher};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ScenePathMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_width: Option<f32>,
    pub path: ScalarOrArray<lyon_path::Path>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub transform: ScalarOrArray<PathTransform>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl std::hash::Hash for ScenePathMark {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.gradients.hash(state);
        self.stroke_cap.hash(state);
        self.stroke_join.hash(state);
        if let Some(stroke_width) = self.stroke_width {
            OrderedFloat(stroke_width).hash(state);
        } else {
            OrderedFloat(0.0).hash(state);
        }
        self.path.hash(state);
        self.fill.hash(state);
        self.stroke.hash(state);
        self.transform.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
    }
}

impl PartialEq for ScenePathMark {
    fn eq(&self, other: &Self) -> bool {
        if self.name != other.name || 
           self.clip != other.clip || 
           self.len != other.len || 
           self.gradients != other.gradients ||
           self.stroke_cap != other.stroke_cap ||
           self.stroke_join != other.stroke_join ||
           self.stroke_width != other.stroke_width ||
           self.fill != other.fill ||
           self.stroke != other.stroke ||
           self.transform != other.transform ||
           self.indices != other.indices ||
           self.zindex != other.zindex {
            return false;
        }
                
        match (&self.path.value(), &other.path.value()) {
            (ScalarOrArrayValue::Scalar(path1), ScalarOrArrayValue::Scalar(path2)) => {
                let mut hash_a = DefaultHasher::new();
                let mut hash_b = DefaultHasher::new();
                hash_lyon_path(path1, &mut hash_a);
                hash_lyon_path(path2, &mut hash_b);
                hash_a.finish() == hash_b.finish()
            }
            (ScalarOrArrayValue::Array(paths1), ScalarOrArrayValue::Array(paths2)) => {
                if paths1.len() != paths2.len() {
                    return false;
                }
                
                for (p1, p2) in paths1.iter().zip(paths2.iter()) {
                    let mut hash_a = DefaultHasher::new();
                    let mut hash_b = DefaultHasher::new();
                    hash_lyon_path(p1, &mut hash_a);
                    hash_lyon_path(p2, &mut hash_b);
                    if hash_a.finish() != hash_b.finish() {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }
}

impl ScenePathMark {
    pub fn path_iter(&self) -> Box<dyn Iterator<Item = &lyon_path::Path> + '_> {
        self.path.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn path_vec(&self) -> Vec<lyon_path::Path> {
        self.path.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn transform_iter(&self) -> Box<dyn Iterator<Item = &PathTransform> + '_> {
        self.transform
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn transform_vec(&self) -> Vec<PathTransform> {
        self.transform
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(
            izip!(self.path_iter(), self.transform_iter()).map(move |(path, transform)| {
                path.clone()
                    .transformed(&transform.then_translate(Vector2D::new(origin[0], origin[1])))
            }),
        )
    }
}

impl Default for ScenePathMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            stroke_width: Some(0.0),
            path: ScalarOrArray::new_scalar(lyon_path::Path::default()),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            transform: ScalarOrArray::new_scalar(PathTransform::identity()),
            indices: None,
            zindex: None,
        }
    }
}

impl From<ScenePathMark> for SceneMark {
    fn from(mark: ScenePathMark) -> Self {
        SceneMark::Path(mark)
    }
}

