use serde::{Deserialize, Serialize};

/// Coordinate frame used for mark geometry construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeometrySpace {
    /// Build geometry in the coordinate system's encoded/scaled channel space,
    /// then project the result into display coordinates.
    #[default]
    Coordinate,
    /// Project encoded/scaled channel values to display coordinates first, then
    /// build geometry directly in display space.
    Display,
}
