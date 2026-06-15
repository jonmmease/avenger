//! Chart-owned helpers shared by child-frame layout code.

use avenger_layout::Edges;

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

/// Coordinated edge targets granted to a child region by its parent.
///
/// `inner` is the interior edge between the content rectangle and any outer
/// content; `total` is the full rendered edge envelope. The difference
/// matters for local outer-content anchoring: outer content starts after the
/// coordinated inner edge, while sibling spacing uses the coordinated total.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeTargets {
    pub guide: Edges<f32>,
    pub total: Edges<f32>,
}
