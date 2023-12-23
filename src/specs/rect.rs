use serde::{Deserialize, Serialize};
use crate::specs::mark::MarkItemSpec;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectItemSpec {
    x: f32,
    y: f32,
    width: Option<f32>,
    height: Option<f32>,
    x2: Option<f32>,
    y2: Option<f32>,
    fill: Option<String>,
    fill_opacity: Option<f32>,
}

impl MarkItemSpec for RectItemSpec {}
