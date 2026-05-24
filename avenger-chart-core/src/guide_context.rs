use std::collections::HashSet;

/// Context passed to guides during rendering and measurement.
///
/// This separates internal guide state from user-provided parameters, avoiding
/// pollution of the params namespace.
#[derive(Debug, Clone, Default)]
pub struct GuideContext {
    /// Channels whose axes should be suppressed during rendering.
    pub suppressed_axes: HashSet<String>,
}

impl GuideContext {
    /// Create an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context that suppresses specific axis channels.
    pub fn with_suppressed_axes(channels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            suppressed_axes: channels.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Check if an axis channel should be suppressed.
    pub fn is_axis_suppressed(&self, channel: &str) -> bool {
        self.suppressed_axes.contains(channel)
    }
}
