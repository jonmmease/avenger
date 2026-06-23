use serde::{Deserialize, Serialize};

/// Cartesian coordinate option for fixed screen-length ratios between x and y
/// data units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CartesianUnitAspect {
    pub ratio: f64,
}

/// Coordinate-neutral unit-aspect constraint reported by a coordinate
/// transform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitAspectConstraint {
    pub x_channel: String,
    pub y_channel: String,
    pub ratio: f64,
    pub policy: UnitAspectPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitAspectPolicy {
    ExpandDomain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitAspectAdjustedAxis {
    X,
    Y,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitAspectAdjustment {
    pub x_scale: String,
    pub y_scale: String,
    pub adjusted_axis: UnitAspectAdjustedAxis,
    pub original_x_domain: (f64, f64),
    pub original_y_domain: (f64, f64),
    pub adjusted_x_domain: (f64, f64),
    pub adjusted_y_domain: (f64, f64),
}

/// Runtime-side scale pair for a coordinate-reported unit-aspect constraint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedUnitAspectConstraint {
    pub x_channel: String,
    pub y_channel: String,
    pub x_scale: String,
    pub y_scale: String,
    pub ratio: f64,
    pub policy: UnitAspectPolicy,
}
