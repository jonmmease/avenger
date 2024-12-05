use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray, StrokeCap, StrokeJoin};
use lyon_path::geom::euclid::{Transform2D, UnknownUnit};
use serde::{Deserialize, Serialize};

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
    pub indices: Option<Vec<usize>>,
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
            path: ScalarOrArray::Scalar(lyon_path::Path::default()),
            fill: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            transform: ScalarOrArray::Scalar(PathTransform::identity()),
            indices: None,
            zindex: None,
        }
    }
}
