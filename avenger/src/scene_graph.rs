use crate::marks::group::SceneGroup;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGraph {
    pub groups: Vec<SceneGroup>,
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
}
