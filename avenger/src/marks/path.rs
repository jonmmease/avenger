use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use itertools::izip;
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
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub path: EncodingValue<lyon_path::Path>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub transform: EncodingValue<PathTransform>,
}

impl PathMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = PathMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();
        Box::new(
            izip!(
                self.path.as_iter(n, inds),
                self.fill.as_iter(n, inds),
                self.stroke.as_iter(n, inds),
                self.transform.as_iter(n, inds)
            )
            .map(|(path, fill, stroke, transform)| PathMarkInstance {
                path: path.clone(),
                fill: fill.clone(),
                stroke: stroke.clone(),
                transform: *transform,
            }),
        )
    }
}

impl Default for PathMark {
    fn default() -> Self {
        let default_instance = PathMarkInstance::default();
        Self {
            name: "path_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            stroke_width: Some(0.0),
            path: EncodingValue::Scalar {
                value: default_instance.path,
            },
            fill: EncodingValue::Scalar {
                value: default_instance.fill,
            },
            stroke: EncodingValue::Scalar {
                value: default_instance.stroke,
            },
            transform: EncodingValue::Scalar {
                value: default_instance.transform,
            },
            indices: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMarkInstance {
    pub path: lyon_path::Path,
    pub fill: ColorOrGradient,
    pub stroke: ColorOrGradient,
    pub transform: PathTransform,
}

impl Default for PathMarkInstance {
    fn default() -> Self {
        Self {
            path: lyon_path::Path::default(),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            transform: PathTransform::identity(),
        }
    }
}
