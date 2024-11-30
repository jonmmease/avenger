use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LineMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,

    // Encodings
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub defined: EncodingValue<bool>,
}

impl LineMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = LineMarkInstance> + '_> {
        let n = self.len as usize;
        Box::new(
            izip!(
                self.x.as_iter(n, None),
                self.y.as_iter(n, None),
                self.defined.as_iter(n, None)
            )
            .map(|(x, y, defined)| LineMarkInstance {
                x: *x,
                y: *y,
                defined: *defined,
            }),
        )
    }
}

impl Default for LineMark {
    fn default() -> Self {
        let default_instance = LineMarkInstance::default();
        Self {
            name: "line_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            defined: EncodingValue::Scalar {
                value: default_instance.defined,
            },
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineMarkInstance {
    pub x: f32,
    pub y: f32,
    pub defined: bool,
}

impl Default for LineMarkInstance {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            defined: true,
        }
    }
}
