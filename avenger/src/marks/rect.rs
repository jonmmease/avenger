use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RectMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub width: EncodingValue<f32>,
    pub height: EncodingValue<f32>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub stroke_width: EncodingValue<f32>,
    pub corner_radius: EncodingValue<f32>,
}

impl RectMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = RectMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();
        Box::new(
            izip!(
                self.x.as_iter(n, inds),
                self.y.as_iter(n, inds),
                self.width.as_iter(n, inds),
                self.height.as_iter(n, inds),
                self.fill.as_iter(n, inds),
                self.stroke.as_iter(n, inds),
                self.stroke_width.as_iter(n, inds),
                self.corner_radius.as_iter(n, inds)
            )
            .map(
                |(x, y, width, height, fill, stroke, stroke_width, corner_radius)| {
                    RectMarkInstance {
                        x: *x,
                        y: *y,
                        width: *width,
                        height: *height,
                        fill: fill.clone(),
                        stroke: stroke.clone(),
                        stroke_width: *stroke_width,
                        corner_radius: *corner_radius,
                    }
                },
            ),
        )
    }
}

impl Default for RectMark {
    fn default() -> Self {
        let default_instance = RectMarkInstance::default();
        Self {
            name: "rect_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            width: EncodingValue::Scalar {
                value: default_instance.width,
            },
            height: EncodingValue::Scalar {
                value: default_instance.height,
            },
            fill: EncodingValue::Scalar {
                value: default_instance.fill,
            },
            stroke: EncodingValue::Scalar {
                value: default_instance.stroke,
            },
            stroke_width: EncodingValue::Scalar {
                value: default_instance.stroke_width,
            },
            corner_radius: EncodingValue::Scalar {
                value: default_instance.corner_radius,
            },
            indices: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RectMarkInstance {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub fill: ColorOrGradient,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub corner_radius: f32,
}

impl Default for RectMarkInstance {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke_width: 0.0,
            corner_radius: 0.0,
        }
    }
}
