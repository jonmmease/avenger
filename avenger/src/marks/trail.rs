use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TrailMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke: ColorOrGradient,
    pub zindex: Option<i32>,

    // Encodings
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub size: EncodingValue<f32>,
    pub defined: EncodingValue<bool>,
}

impl TrailMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = TrailMarkInstance> + '_> {
        let n = self.len as usize;
        Box::new(
            izip!(
                self.x.as_iter(n, None),
                self.y.as_iter(n, None),
                self.size.as_iter(n, None),
                self.defined.as_iter(n, None)
            )
            .map(|(x, y, size, defined)| TrailMarkInstance {
                x: *x,
                y: *y,
                size: *size,
                defined: *defined,
            }),
        )
    }
}

impl Default for TrailMark {
    fn default() -> Self {
        let default_instance = TrailMarkInstance::default();
        Self {
            name: "trail_mark".to_string(),
            clip: true,
            len: 1,
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            size: EncodingValue::Scalar {
                value: default_instance.size,
            },
            defined: EncodingValue::Scalar {
                value: default_instance.defined,
            },
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            gradients: vec![],
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailMarkInstance {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub defined: bool,
}

impl Default for TrailMarkInstance {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: 1.0,
            defined: true,
        }
    }
}
