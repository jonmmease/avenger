//! The custom scales that a renderer adds, which the chart names by type.

use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::ScaleImpl;

/// The custom scales that a chart's `CustomScale`s name by their `type`.
#[derive(Clone, Default)]
pub struct Registry {
    scales: HashMap<String, Arc<dyn ScaleImpl>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry with the custom scale `name`.
    pub fn with_scale(mut self, name: impl Into<String>, scale: Arc<dyn ScaleImpl>) -> Self {
        self.scales.insert(name.into(), scale);
        self
    }

    /// The custom scale `name`, or `None` when the registry has none.
    pub fn scale(&self, name: &str) -> Option<&Arc<dyn ScaleImpl>> {
        self.scales.get(name)
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.scales.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("Registry").field("scales", &names).finish()
    }
}
