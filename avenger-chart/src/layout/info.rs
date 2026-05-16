//! Layout information structures for cross-subplot alignment
//!
//! This module defines layout information structures used during the rendering process.

/// Legend position info for a single subplot used for cross-subplot alignment
///
/// Stores the X/Y positions of legends at each position (right, left, top, bottom).
/// These positions are populated from frame layout bounds after layout computation.
#[derive(Debug, Clone, Default)]
pub struct LegendLayoutInfo {
    /// X position of right legends (for alignment across subplots)
    pub right_x: f32,
    /// X position of left legends (for alignment across subplots)
    pub left_x: f32,
    /// Y position of top legends (for alignment across subplots)
    pub top_y: f32,
    /// Y position of bottom legends (for alignment across subplots)
    pub bottom_y: f32,
}
