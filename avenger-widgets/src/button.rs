use crate::{Options, WidgetId, WidgetSpec};

/// Visual emphasis of a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Neutral,
    Accent,
}

/// A labeled button that emits an activation event.
#[derive(Clone, Debug)]
pub struct Button {
    pub(crate) options: Options,
    pub(crate) label: String,
    pub(crate) variant: ButtonVariant,
}
impl Button {
    /// Create an enabled neutral button.
    pub fn new(id: impl Into<WidgetId>, label: impl Into<String>) -> Self {
        Self {
            options: Options::new(id),
            label: label.into(),
            variant: ButtonVariant::Neutral,
        }
    }
    /// Select neutral or accent emphasis.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    /// Enable or disable activation and focus.
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
impl From<Button> for WidgetSpec {
    fn from(value: Button) -> Self {
        Self::Button(value)
    }
}
