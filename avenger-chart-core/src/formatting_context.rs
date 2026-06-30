use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattingContext {
    pub number_locale: Option<String>,
}

impl FormattingContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn number_locale(mut self, locale: impl Into<String>) -> Self {
        self.number_locale = Some(locale.into());
        self
    }

    pub fn resolved_number_locale(&self) -> &str {
        self.number_locale.as_deref().unwrap_or("en-US")
    }

    pub fn resolved_with_parent(&self, parent: &FormattingContext) -> FormattingContext {
        FormattingContext {
            number_locale: self
                .number_locale
                .clone()
                .or_else(|| parent.number_locale.clone()),
        }
    }
}
