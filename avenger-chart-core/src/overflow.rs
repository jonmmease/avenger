//! Space requirements for guide overflow and measurement results.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Constants for spacing coordination keys to avoid typos.
pub mod spacing_keys {
    /// Gap between adjacent rows in a facet
    pub const INTER_ROW_GAP: &str = "inter_row_gap";
    /// Gap between adjacent columns in a facet
    pub const INTER_COL_GAP: &str = "inter_col_gap";
    /// Space needed for legend on right side
    pub const LEGEND_RIGHT: &str = "legend_right";
    /// Space needed for legend on left side
    pub const LEGEND_LEFT: &str = "legend_left";
    /// Space needed for legend on top
    pub const LEGEND_TOP: &str = "legend_top";
    /// Space needed for legend on bottom
    pub const LEGEND_BOTTOM: &str = "legend_bottom";
    /// Marker indicating we're in nested measure_with_coordination call.
    /// When this is set, inner guides should skip 2-pass and use single-pass.
    pub const NESTED_MEASUREMENT: &str = "nested_measurement";
}

/// Space requirements for guide overflow beyond plot area.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl OverflowSpaceRequirement {
    /// Create overflow with max of each component from self and other.
    pub fn max_components(&self, other: &Self) -> Self {
        Self {
            top: self.top.max(other.top),
            bottom: self.bottom.max(other.bottom),
            left: self.left.max(other.left),
            right: self.right.max(other.right),
        }
    }
}

/// Result of self-coordinating measurement from a guide.
///
/// Contains both the overflow space requirement and any spacing needs that
/// should be coordinated with sibling subplots.
#[derive(Debug, Clone, Default)]
pub struct MeasurementResult {
    /// Final overflow after internal coordination.
    pub overflow: OverflowSpaceRequirement,

    /// Named spacing values computed during measurement.
    pub spacing_needs: HashMap<String, f32>,
}

impl MeasurementResult {
    /// Create a new MeasurementResult with the given overflow and empty spacing_needs.
    pub fn new(overflow: OverflowSpaceRequirement) -> Self {
        Self {
            overflow,
            spacing_needs: HashMap::new(),
        }
    }

    /// Add a spacing need and return self for chaining.
    pub fn with_spacing(mut self, key: impl Into<String>, value: f32) -> Self {
        self.spacing_needs.insert(key.into(), value);
        self
    }

    /// Merge child spacing_needs using max aggregation for each key.
    pub fn merge_spacing_needs(mut self, other: HashMap<String, f32>) -> Self {
        for (key, value) in other {
            self.spacing_needs
                .entry(key)
                .and_modify(|v| *v = v.max(value))
                .or_insert(value);
        }
        self
    }
}
