//! Shared axis-side position types.

use serde::{Deserialize, Serialize};

/// Position for Cartesian-style frame sides and axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisPosition {
    Top,
    Right,
    Bottom,
    Left,
}
