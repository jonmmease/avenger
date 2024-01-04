use serde::{Deserialize, Serialize};

/// This struct is not part of the scene graph definition. This is the format
/// written by the gen-test-data crate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VegaSceneGraphDims {
    pub width: f32,
    pub height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}
