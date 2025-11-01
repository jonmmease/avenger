//! Space requirements for guide overflow

use serde::{Deserialize, Serialize};

/// Space requirements for guide overflow beyond plot area
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}
