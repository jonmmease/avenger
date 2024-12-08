use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{lyon_to_geo::IntoGeoType, GeometryInstance};
use itertools::izip;
use lyon_extra::euclid::Vector2D;
use lyon_path::{
    geom::euclid::{Transform2D, UnknownUnit},
    Path,
};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

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

    pub fn geometry_iter(
        &self,
        mark_index: usize,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let half_stroke_width = self.stroke_width.unwrap_or(0.0) / 2.0;
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter([0.0, 0.0]))
                .enumerate()
                .map(move |(z_index, (id, path))| {
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        mark_index,
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width,
                    }
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
            path: ScalarOrArray::Scalar(lyon_path::Path::default()),
            fill: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            transform: ScalarOrArray::Scalar(PathTransform::identity()),
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
