use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AreaMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub orientation: AreaOrientation,
    pub gradients: Vec<Gradient>,
    pub fill: ColorOrGradient,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,

    // Encodings
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub x2: EncodingValue<f32>,
    pub y2: EncodingValue<f32>,
    pub defined: EncodingValue<bool>,
}

impl AreaMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = AreaMarkInstance> + '_> {
        let n = self.len as usize;
        Box::new(
            izip!(
                self.x.as_iter(n, None),
                self.y.as_iter(n, None),
                self.x2.as_iter(n, None),
                self.y2.as_iter(n, None),
                self.defined.as_iter(n, None)
            )
            .map(|(x, y, x2, y2, defined)| AreaMarkInstance {
                x: *x,
                y: *y,
                x2: *x2,
                y2: *y2,
                defined: *defined,
            }),
        )
    }
}

impl Default for AreaMark {
    fn default() -> Self {
        let default_instance = AreaMarkInstance::default();
        Self {
            name: "area_mark".to_string(),
            clip: true,
            len: 1,
            orientation: Default::default(),
            gradients: vec![],
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            x2: EncodingValue::Scalar {
                value: default_instance.x2,
            },
            y2: EncodingValue::Scalar {
                value: default_instance.y2,
            },
            defined: EncodingValue::Scalar {
                value: default_instance.defined,
            },
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
            zindex: None,
        }
    }
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreaMarkInstance {
    pub x: f32,
    pub y: f32,
    pub x2: f32,
    pub y2: f32,
    pub defined: bool,
}

impl Default for AreaMarkInstance {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            x2: 0.0,
            y2: 0.0,
            defined: true,
        }
    }
}
