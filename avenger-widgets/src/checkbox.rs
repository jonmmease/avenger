use crate::{Options, WidgetId, WidgetSpec};

/// A boolean choice with a clickable label.
#[derive(Clone, Debug)]
pub struct Checkbox {
    pub(crate) options: Options,
    pub(crate) label: String,
    pub(crate) checked: bool,
}
impl Checkbox {
    /// Create a checkbox with its current application value.
    pub fn new(id: impl Into<WidgetId>, label: impl Into<String>, checked: bool) -> Self {
        Self {
            options: Options::new(id),
            label: label.into(),
            checked,
        }
    }
    /// Enable or disable toggling and focus.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.options.enabled = enabled;
        self
    }
    /// Supply a semantic name distinct from the visible label.
    pub fn semantic_name(mut self, name: impl Into<String>) -> Self {
        self.options.semantic_name = Some(name.into());
        self
    }
}
impl From<Checkbox> for WidgetSpec {
    fn from(value: Checkbox) -> Self {
        Self::Checkbox(value)
    }
}
