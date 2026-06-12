//! Band vocabulary: flow direction and per-boundary main-axis demand for
//! one-dimensional child arrangements (concat stacks, facet bands).

/// Flow direction for a one-dimensional band arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// Main-axis rendered demand outside one child boundary
/// (leading/trailing edge totals projected onto the main axis).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoundaryDemand {
    pub before: f32,
    pub after: f32,
}
