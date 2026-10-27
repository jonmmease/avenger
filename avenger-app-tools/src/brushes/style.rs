//! How brushes look.

/// How a [`Brushes`](super::Brushes) tool draws its selections.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushStyle {
    /// The fill inside boxes and lassos.
    pub fill: [f32; 4],
    /// The color of the outlines of boxes, lassos, and handles.
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    /// The dash pattern of the outlines of boxes and lassos: the lengths of alternating dashes and
    /// gaps, in pixels, or `None` for solid outlines. Handles keep solid outlines.
    pub stroke_dash: Option<Vec<f32>>,
    /// The side of the square handles on a box's edges and corners, which show while the pointer
    /// is over the box or the box is changing.
    pub handle_size: f32,
    pub handle_fill: [f32; 4],
}

impl Default for BrushStyle {
    fn default() -> Self {
        Self {
            fill: [0.16, 0.42, 0.85, 0.12],
            stroke: [0.16, 0.42, 0.85, 0.9],
            stroke_width: 1.0,
            stroke_dash: None,
            handle_size: 6.0,
            handle_fill: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
