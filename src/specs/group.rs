use crate::specs::mark::{MarkItemSpec, MarkSpec};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupItemSpec {
    pub items: Vec<MarkSpec>,
    #[serde(default)]
    pub(crate) x: f32,
    #[serde(default)]
    pub(crate) y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl MarkItemSpec for GroupItemSpec {}
