use super::mark::SceneMark;
use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_extra::euclid::Vector2D;
use lyon_path::{
    geom::euclid::{Transform2D, UnknownUnit},
    Path,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type PathTransform = Transform2D<f32, UnknownUnit, UnknownUnit>;

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
            Box::new((0..self.len as usize).into_iter())
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
