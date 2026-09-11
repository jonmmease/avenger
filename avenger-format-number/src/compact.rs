use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactTier {
    pub exponent: i32,
    pub one: Option<String>,
    pub other: String,
}

impl CompactTier {
    pub fn pattern_for_value(&self, formatted_value: &str) -> &str {
        if formatted_value == "1" {
            self.one.as_deref().unwrap_or(&self.other)
        } else {
            &self.other
        }
    }
}
