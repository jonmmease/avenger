use serde::{Deserialize, Serialize};

/// Cartesian coordinate option for fixed screen-length ratios between x and y
/// data units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CartesianUnitAspect {
    pub ratio: f64,
}
