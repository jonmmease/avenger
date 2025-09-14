//! Space requirements for guide overflow

/// Space requirements for guide overflow beyond plot area
#[derive(Debug, Clone, Default)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}
