//! Localized number formatting with ICU number skeletons.
//!
//! ```
//! use avenger_format::NumberFormatProvider;
//! use avenger_format_number_icu::IcuNumberFormatProvider;
//!
//! let formatter = IcuNumberFormatProvider::new()
//!     .with_locale("en-US")
//!     .prepare(".00 group-off")?;
//! assert_eq!(formatter.format(1234.5).text, "1234.50");
//! # Ok::<(), avenger_format::NumberFormatError>(())
//! ```

mod arithmetic;
mod currency;
mod data;
mod locale;
mod notation;
mod percent;
mod precision;
mod prepared;
mod skeleton;
mod units;

use avenger_format::{NumberFormatError, NumberFormatProvider, PreparedNumberFormatter};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Prepare ICU number skeletons for one locale using compiled locale data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IcuNumberFormatProvider {
    /// BCP 47 locale identifier. Omission selects `en-US`.
    pub locale: Option<String>,
}

impl IcuNumberFormatProvider {
    /// Use US English number conventions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a locale, accepting hyphens or underscores as separators.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }
}

impl NumberFormatProvider for IcuNumberFormatProvider {
    fn prepare(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        let mut skeleton = skeleton::Skeleton::parse(pattern)?;
        currency::resolve_precision(&mut skeleton)?;
        let name = self.locale.as_deref().unwrap_or("en-US");
        let context = data::Context::new(name, skeleton.numbering_system.as_deref())?;
        let locale = locale::LocaleData::new(&context, &skeleton)?;
        Ok(Arc::new(prepared::Prepared { skeleton, locale }))
    }
}
