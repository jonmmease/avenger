use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattingContext {
    pub number_locale: Option<String>,
    #[serde(default)]
    pub number_locale_specs: avenger_text::NumberLocaleSpecs,
}

impl FormattingContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn number_locale(mut self, locale: impl Into<String>) -> Self {
        self.number_locale = Some(locale.into());
        self
    }

    pub fn number_locale_spec(
        mut self,
        id: impl Into<String>,
        spec: avenger_text::NumberLocaleSpec,
    ) -> Self {
        self.number_locale_specs.insert(id.into(), spec);
        self
    }

    pub fn number_locale_specs(&self) -> &avenger_text::NumberLocaleSpecs {
        &self.number_locale_specs
    }

    pub fn number_locale_registry(
        &self,
    ) -> Result<Option<std::sync::Arc<avenger_format_number::NumberLocaleRegistry>>, String> {
        avenger_text::number_locale_registry_from_specs(Some(&self.number_locale_specs))
    }

    pub fn resolved_number_locale(&self) -> &str {
        self.number_locale.as_deref().unwrap_or("en-US")
    }

    pub fn resolved_with_parent(&self, parent: &FormattingContext) -> FormattingContext {
        let mut number_locale_specs = parent.number_locale_specs.clone();
        number_locale_specs.extend(self.number_locale_specs.clone());
        FormattingContext {
            number_locale: self
                .number_locale
                .clone()
                .or_else(|| parent.number_locale.clone()),
            number_locale_specs,
        }
    }
}
