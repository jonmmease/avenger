use crate::specs::mark::MarkItemSpec;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectItemSpec {
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
}

impl MarkItemSpec for RectItemSpec {}
