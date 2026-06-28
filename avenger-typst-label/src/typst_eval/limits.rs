#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelLimits {
    pub max_source_bytes: usize,
    pub max_math_spans: usize,
    pub max_math_depth: usize,
}

impl Default for LabelLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024,
            max_math_spans: 64,
            max_math_depth: 64,
        }
    }
}
