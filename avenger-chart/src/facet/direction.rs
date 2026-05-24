use serde::{Deserialize, Serialize};

/// Direction of a facet partition level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FacetDirection {
    /// Vertical stacking.
    Row,
    /// Horizontal arrangement.
    Column,
}
