use crate::NumberFormatError;
use avenger_format_locales_d3::{LocaleId, NumberLocaleSpec};
use std::sync::Arc;

/// A validated locale with an immutable definition shared across clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNumberLocale {
    id: LocaleId,
    definition: Arc<NumberLocaleSpec>,
}
impl std::ops::Deref for ResolvedNumberLocale {
    type Target = NumberLocaleSpec;
    fn deref(&self) -> &Self::Target {
        &self.definition
    }
}
impl ResolvedNumberLocale {
    /// Return the name assigned when this locale was resolved.
    pub fn id(&self) -> &LocaleId {
        &self.id
    }

    /// Borrow the validated D3 number definition.
    pub fn definition(&self) -> &NumberLocaleSpec {
        &self.definition
    }

    /// Retain a named locale definition, rejecting zero-sized digit groups.
    pub fn new(
        id: impl Into<String>,
        definition: NumberLocaleSpec,
    ) -> Result<Self, NumberFormatError> {
        let id = id.into();
        definition.validate(&id)?;
        Ok(Self {
            id: LocaleId::new(id),
            definition: Arc::new(definition),
        })
    }
    /// Resolve the bundled U.S. English locale.
    pub fn en_us() -> Self {
        Self::new("en-US", NumberLocaleSpec::en_us()).expect("bundled D3 number locale")
    }
}
