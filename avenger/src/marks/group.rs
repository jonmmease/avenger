use crate::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GroupBounds {
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl Default for GroupBounds {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGroup {
    pub bounds: GroupBounds,
    pub marks: Vec<SceneMark>,
}
