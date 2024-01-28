use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use lyon_path::geom::euclid::{Transform2D, UnknownUnit};
use serde::{Deserialize, Serialize};

pub type PathTransform = Transform2D<f32, UnknownUnit, UnknownUnit>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PathMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_width: Option<f32>,
    pub path: EncodingValue<lyon_path::Path>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub transform: EncodingValue<PathTransform>,
    pub indices: Option<Vec<usize>>,
}

impl PathMark {
    pub fn path_iter(&self) -> Box<dyn Iterator<Item = &lyon_path::Path> + '_> {
        self.path.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn transform_iter(&self) -> Box<dyn Iterator<Item = &PathTransform> + '_> {
        self.transform
            .as_iter(self.len as usize, self.indices.as_ref())
    }
}

impl Default for PathMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            stroke_width: Some(0.0),
            path: EncodingValue::Scalar {
                value: lyon_path::Path::default(),
            },
            fill: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            stroke: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            transform: EncodingValue::Scalar {
                value: PathTransform::identity(),
            },
            indices: None,
        }
    }
}
