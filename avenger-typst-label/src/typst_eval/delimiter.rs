#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DelimiterDisplayHint {
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DelimiterInfo {
    pub full_range: std::ops::Range<usize>,
    pub opening_range: std::ops::Range<usize>,
    pub closing_range: std::ops::Range<usize>,
    pub display_hint: DelimiterDisplayHint,
}
