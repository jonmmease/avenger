use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ArcMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub start_angle: EncodingValue<f32>,
    pub end_angle: EncodingValue<f32>,
    pub outer_radius: EncodingValue<f32>,
    pub inner_radius: EncodingValue<f32>,
    pub pad_angle: EncodingValue<f32>,
    pub corner_radius: EncodingValue<f32>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub stroke_width: EncodingValue<f32>,
}

impl ArcMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = ArcMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();
        Box::new(
            izip!(
                self.x.as_iter(n, inds),
                self.y.as_iter(n, inds),
                self.start_angle.as_iter(n, inds),
                self.end_angle.as_iter(n, inds),
                self.outer_radius.as_iter(n, inds),
                self.inner_radius.as_iter(n, inds),
                self.pad_angle.as_iter(n, inds),
                self.corner_radius.as_iter(n, inds),
                self.fill.as_iter(n, inds),
                self.stroke.as_iter(n, inds),
                self.stroke_width.as_iter(n, inds),
            )
            .map(
                |(
                    x,
                    y,
                    start_angle,
                    end_angle,
                    outer_radius,
                    inner_radius,
                    pad_angle,
                    corner_radius,
                    fill_color,
                    stroke_color,
                    stroke_width,
                )| ArcMarkInstance {
                    x: *x,
                    y: *y,
                    start_angle: *start_angle,
                    end_angle: *end_angle,
                    outer_radius: *outer_radius,
                    inner_radius: *inner_radius,
                    pad_angle: *pad_angle,
                    corner_radius: *corner_radius,
                    fill_color: fill_color.clone(),
                    stroke_color: stroke_color.clone(),
                    stroke_width: *stroke_width,
                },
            ),
        )
    }
}

impl Default for ArcMark {
    fn default() -> Self {
        let default_instance = ArcMarkInstance::default();
        Self {
            name: "arc_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            start_angle: EncodingValue::Scalar {
                value: default_instance.start_angle,
            },
            end_angle: EncodingValue::Scalar {
                value: default_instance.end_angle,
            },
            outer_radius: EncodingValue::Scalar {
                value: default_instance.outer_radius,
            },
            inner_radius: EncodingValue::Scalar {
                value: default_instance.inner_radius,
            },
            pad_angle: EncodingValue::Scalar {
                value: default_instance.pad_angle,
            },
            corner_radius: EncodingValue::Scalar {
                value: default_instance.corner_radius,
            },
            fill: EncodingValue::Scalar {
                value: default_instance.fill_color,
            },
            stroke: EncodingValue::Scalar {
                value: default_instance.stroke_color,
            },
            stroke_width: EncodingValue::Scalar {
                value: default_instance.stroke_width,
            },
            indices: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArcMarkInstance {
    pub x: f32,
    pub y: f32,
    pub start_angle: f32,
    pub end_angle: f32,
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub pad_angle: f32,
    pub corner_radius: f32,
    pub fill_color: ColorOrGradient,
    pub stroke_color: ColorOrGradient,
    pub stroke_width: f32,
}

impl Default for ArcMarkInstance {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            start_angle: 0.0,
            end_angle: 0.0,
            outer_radius: 0.0,
            inner_radius: 0.0,
            pad_angle: 0.0,
            corner_radius: 0.0,
            fill_color: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_color: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke_width: 0.0,
        }
    }
}
