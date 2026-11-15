#![doc = include_str!("../README.md")]

use avenger_format::DateTimeFormatError;
use serde::{Deserialize, Serialize};

/// A named locale in an Avenger formatting context.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocaleId(pub String);

impl LocaleId {
    /// Construct a locale identifier without resolving it.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for LocaleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LocaleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// D3 time locale JSON. Array lengths are enforced during deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeLocaleSpec {
    /// The layout of `%c`.
    pub date_time: String,
    /// The layout of `%x`.
    pub date: String,
    /// The layout of `%X`.
    pub time: String,
    pub periods: [String; 2],
    pub days: [String; 7],
    pub short_days: [String; 7],
    pub months: [String; 12],
    pub short_months: [String; 12],
}

impl Default for DateTimeLocaleSpec {
    fn default() -> Self {
        serde_json::from_str(include_str!("../locales/en-US.json")).expect("bundled D3 time locale")
    }
}

impl DateTimeLocaleSpec {
    /// The definition of the JSON `json`, the locale `id` in its errors.
    pub fn from_json(id: &str, json: &str) -> Result<Self, DateTimeFormatError> {
        serde_json::from_str(json).map_err(|error| DateTimeFormatError::InvalidLocaleData {
            message: format!("locale `{id}`: {error}"),
        })
    }
}

/// The bundled locales: U.S. English, or all of d3's with the `all-locales` feature.
#[cfg(not(feature = "all-locales"))]
pub const BUNDLED: &[(&str, &str)] = &[("en-US", include_str!("../locales/en-US.json"))];

/// The bundled locales: U.S. English, or all of d3's with the `all-locales` feature.
#[cfg(feature = "all-locales")]
pub const BUNDLED: &[(&str, &str)] = &[
    ("ar-EG", include_str!("../locales/ar-EG.json")),
    ("ar-SY", include_str!("../locales/ar-SY.json")),
    ("ca-ES", include_str!("../locales/ca-ES.json")),
    ("cs-CZ", include_str!("../locales/cs-CZ.json")),
    ("da-DK", include_str!("../locales/da-DK.json")),
    ("de-CH", include_str!("../locales/de-CH.json")),
    ("de-DE", include_str!("../locales/de-DE.json")),
    ("en-CA", include_str!("../locales/en-CA.json")),
    ("en-GB", include_str!("../locales/en-GB.json")),
    ("en-US", include_str!("../locales/en-US.json")),
    ("es-ES", include_str!("../locales/es-ES.json")),
    ("es-MX", include_str!("../locales/es-MX.json")),
    ("fa-IR", include_str!("../locales/fa-IR.json")),
    ("fi-FI", include_str!("../locales/fi-FI.json")),
    ("fr-CA", include_str!("../locales/fr-CA.json")),
    ("fr-FR", include_str!("../locales/fr-FR.json")),
    ("he-IL", include_str!("../locales/he-IL.json")),
    ("hr-HR", include_str!("../locales/hr-HR.json")),
    ("hu-HU", include_str!("../locales/hu-HU.json")),
    ("it-IT", include_str!("../locales/it-IT.json")),
    ("ja-JP", include_str!("../locales/ja-JP.json")),
    ("ko-KR", include_str!("../locales/ko-KR.json")),
    ("mk-MK", include_str!("../locales/mk-MK.json")),
    ("nb-NO", include_str!("../locales/nb-NO.json")),
    ("nl-BE", include_str!("../locales/nl-BE.json")),
    ("nl-NL", include_str!("../locales/nl-NL.json")),
    ("pl-PL", include_str!("../locales/pl-PL.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
    ("ru-RU", include_str!("../locales/ru-RU.json")),
    ("sv-SE", include_str!("../locales/sv-SE.json")),
    ("tr-TR", include_str!("../locales/tr-TR.json")),
    ("uk-UA", include_str!("../locales/uk-UA.json")),
    ("zh-CN", include_str!("../locales/zh-CN.json")),
    ("zh-TW", include_str!("../locales/zh-TW.json")),
];

/// The bundled definition named `id`, with hyphens or underscores, such as `fr-FR` or `fr_FR`.
pub fn bundled(id: &str) -> Option<DateTimeLocaleSpec> {
    let id = id.replace('_', "-");
    BUNDLED
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(name, json)| {
            DateTimeLocaleSpec::from_json(name, json).expect("bundled D3 time locale")
        })
}
