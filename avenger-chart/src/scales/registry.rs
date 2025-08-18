use crate::scales::Scale;
use std::collections::HashMap;

/// Registry for accessing scales in a plot
#[derive(Debug, Clone, Default)]
pub struct ScaleRegistry {
    scales: HashMap<String, Scale>,
}

impl ScaleRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            scales: HashMap::new(),
        }
    }

    /// Add a scale to the registry
    pub fn add(&mut self, name: impl Into<String>, scale: Scale) -> &mut Self {
        self.scales.insert(name.into(), scale);
        self
    }

    /// Get a scale by name
    pub fn get(&self, name: &str) -> Option<&Scale> {
        self.scales.get(name)
    }

    /// Get all scales
    pub fn scales(&self) -> &HashMap<String, Scale> {
        &self.scales
    }

    /// Get scales for specific channels
    pub fn get_scales_for_channels<'a>(
        &'a self,
        channels: &'a [&'a str],
    ) -> Vec<(&'a str, &'a Scale)> {
        channels
            .iter()
            .filter_map(|&channel| self.scales.get(channel).map(|scale| (channel, scale)))
            .collect()
    }
}
