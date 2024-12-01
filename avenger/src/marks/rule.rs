use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RuleMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub x0: EncodingValue<f32>,
    pub y0: EncodingValue<f32>,
    pub x1: EncodingValue<f32>,
    pub y1: EncodingValue<f32>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub stroke_width: EncodingValue<f32>,
    pub stroke_cap: EncodingValue<StrokeCap>,
    pub stroke_dash: Option<EncodingValue<Vec<f32>>>,
}

impl RuleMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = RuleMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();

        let stroke_dash_iter = self.stroke_dash.as_ref().map_or_else(
            || {
                Box::new(std::iter::repeat(None))
                    as Box<dyn Iterator<Item = Option<&Vec<f32>>> + '_>
            },
            |dash| Box::new(dash.as_iter(n, inds).map(Some)),
        );

        Box::new(
            izip!(
                self.x0.as_iter(n, inds),
                self.y0.as_iter(n, inds),
                self.x1.as_iter(n, inds),
                self.y1.as_iter(n, inds),
                self.stroke.as_iter(n, inds),
                self.stroke_width.as_iter(n, inds),
                self.stroke_cap.as_iter(n, inds),
                stroke_dash_iter,
            )
            .map(
                |(x0, y0, x1, y1, stroke, stroke_width, stroke_cap, stroke_dash)| {
                    RuleMarkInstance {
                        x0: *x0,
                        y0: *y0,
                        x1: *x1,
                        y1: *y1,
                        stroke: stroke.clone(),
                        stroke_width: *stroke_width,
                        stroke_cap: *stroke_cap,
                        stroke_dash: stroke_dash.cloned(),
                    }
                },
            ),
        )
    }
}

impl Default for RuleMark {
    fn default() -> Self {
        let default_instance = RuleMarkInstance::default();
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_dash: None,
            x0: EncodingValue::Scalar {
                value: default_instance.x0,
            },
            y0: EncodingValue::Scalar {
                value: default_instance.y0,
            },
            x1: EncodingValue::Scalar {
                value: default_instance.x1,
            },
            y1: EncodingValue::Scalar {
                value: default_instance.y1,
            },
            stroke: EncodingValue::Scalar {
                value: default_instance.stroke,
            },
            stroke_width: EncodingValue::Scalar {
                value: default_instance.stroke_width,
            },
            stroke_cap: EncodingValue::Scalar {
                value: default_instance.stroke_cap,
            },
            indices: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMarkInstance {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_dash: Option<Vec<f32>>,
}

impl Default for RuleMarkInstance {
    fn default() -> Self {
        Self {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 1.0,
            stroke_cap: StrokeCap::Butt,
            stroke_dash: None,
        }
    }
}
