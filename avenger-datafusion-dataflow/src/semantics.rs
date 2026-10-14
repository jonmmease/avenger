use std::collections::BTreeMap;

/// Result-affecting requirements preserved when a definition is serialized.
/// Custom function keys are `scalar:name`, `aggregate:name`, `window:name`, or `higher_order:name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticConfig {
    pub time_zone: String,
    pub function_versions: BTreeMap<String, String>,
}
impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            time_zone: "UTC".into(),
            function_versions: BTreeMap::new(),
        }
    }
}
