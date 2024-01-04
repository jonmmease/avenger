use crate::specs::mark::MarkItemSpec;
use serde::{Deserialize, Serialize};
use crate::scene::value::StrokeCap;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleItemSpec {
    pub x: f32,
    pub y: f32,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
}

impl MarkItemSpec for RuleItemSpec {}
