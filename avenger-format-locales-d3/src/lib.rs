#![doc = include_str!("../README.md")]

use std::collections::BTreeMap;

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

/// D3 time locale JSON, with optional fields that GoogleSQL's formats read and the d3 provider
/// ignores. Array lengths are enforced during deserialization.
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
    /// The month names in a date, which some languages decline, such as Russian `марта`.
    pub months: [String; 12],
    pub short_months: [String; 12],
    /// The month names that stand alone, such as Russian `март`, when they differ from `months`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone_months: Option<[String; 12]>,
    /// The short month names that stand alone, when they differ from `short_months`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone_short_months: Option<[String; 12]>,
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

    /// The month names that stand alone, or the month names in a date without them.
    pub fn standalone_months(&self) -> &[String; 12] {
        self.standalone_months.as_ref().unwrap_or(&self.months)
    }

    /// The short month names that stand alone, or the short month names without them.
    pub fn standalone_short_months(&self) -> &[String; 12] {
        (self.standalone_short_months.as_ref()).unwrap_or(&self.short_months)
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

/// The bundled definition named `id`, with hyphens or underscores, without resolving the tag.
pub fn bundled(id: &str) -> Option<DateTimeLocaleSpec> {
    let id = id.replace('_', "-");
    BUNDLED
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(name, json)| {
            DateTimeLocaleSpec::from_json(name, json).expect("bundled D3 time locale")
        })
}

/// The region of each language of the bundled locales that CLDR's likely subtags give it, such as
/// `pt` to `BR`: the region of the language's entry in `supplemental/likelySubtags.json` of
/// CLDR 49, such as `pt-Latn-BR`. A new bundled language takes its region from the same file.
const LIKELY_REGIONS: &[(&str, &str)] = &[
    ("ar", "EG"),
    ("ca", "ES"),
    ("cs", "CZ"),
    ("da", "DK"),
    ("de", "DE"),
    ("en", "US"),
    ("es", "ES"),
    ("fa", "IR"),
    ("fi", "FI"),
    ("fr", "FR"),
    ("he", "IL"),
    ("hr", "HR"),
    ("hu", "HU"),
    ("it", "IT"),
    ("ja", "JP"),
    ("ko", "KR"),
    ("mk", "MK"),
    ("nb", "NO"),
    ("nl", "NL"),
    ("pl", "PL"),
    ("pt", "BR"),
    ("ru", "RU"),
    ("sv", "SE"),
    ("tr", "TR"),
    ("uk", "UA"),
    ("zh", "CN"),
];

/// The name among `available` that the BCP 47 tag `tag` selects: the name with the same language
/// and region, without regard to case, or the language's likely region, or the first name of the
/// language. A tag may write `_` for `-`. `None` when no name has the tag's language.
pub fn resolve_tag<'a>(tag: &str, available: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let tag = tag.replace('_', "-");
    let available: Vec<&str> = available.into_iter().collect();
    if let Some(exact) = available
        .iter()
        .find(|name| name.replace('_', "-").eq_ignore_ascii_case(&tag))
    {
        return Some(exact);
    }
    let language = tag.split('-').next()?.to_ascii_lowercase();
    let of_language = |name: &&&str| {
        name.split(['-', '_'])
            .next()
            .is_some_and(|l| l.eq_ignore_ascii_case(&language))
    };
    let likely = (LIKELY_REGIONS.iter())
        .find(|(l, _)| *l == language)
        .map(|(l, region)| format!("{l}-{region}"));
    if let Some(likely) = likely {
        if let Some(name) = available
            .iter()
            .find(|name| name.replace('_', "-").eq_ignore_ascii_case(&likely))
        {
            return Some(name);
        }
    }
    let mut candidates: Vec<&&str> = available.iter().filter(of_language).collect();
    candidates.sort();
    candidates.first().map(|name| **name)
}

/// Named definitions of d3 time locales: the bundled ones and the ones that a host registers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeLocales {
    locales: BTreeMap<String, DateTimeLocaleSpec>,
}

impl DateTimeLocales {
    /// The bundled locales.
    pub fn with_bundled() -> Self {
        let locales = (BUNDLED.iter())
            .map(|(id, json)| {
                let spec = DateTimeLocaleSpec::from_json(id, json).expect("bundled D3 time locale");
                (id.to_string(), spec)
            })
            .collect();
        Self { locales }
    }

    /// Adds or replaces the definition named `id`.
    pub fn register(&mut self, id: impl Into<String>, spec: DateTimeLocaleSpec) {
        self.locales.insert(id.into(), spec);
    }

    /// Adds or replaces the definition named `id` from d3 time locale JSON.
    pub fn register_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), DateTimeFormatError> {
        let id = id.into();
        let spec = DateTimeLocaleSpec::from_json(&id, json)?;
        self.register(id, spec);
        Ok(())
    }

    /// The names of the definitions.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.locales.keys().map(String::as_str)
    }

    /// The definition named exactly `id`.
    pub fn get(&self, id: &str) -> Option<&DateTimeLocaleSpec> {
        self.locales.get(id)
    }

    /// The name and the definition that the BCP 47 tag `tag` selects, by [`resolve_tag`], or an
    /// error that lists the names.
    pub fn resolve(&self, tag: &str) -> Result<(&str, &DateTimeLocaleSpec), DateTimeFormatError> {
        let name = resolve_tag(tag, self.names()).ok_or_else(|| {
            let names: Vec<&str> = self.names().collect();
            DateTimeFormatError::LocaleUnavailable {
                locale: tag.to_string(),
                message: format!("the locales are {}", names.join(", ")),
            }
        })?;
        let (name, spec) = self.locales.get_key_value(name).expect("a resolved name");
        Ok((name.as_str(), spec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["en-GB", "en-US", "fr-CA", "fr-FR", "pt-BR", "de_CH"];

    #[test]
    fn tags_resolve_to_their_region_or_their_languages_likely_region() {
        let resolve = |tag: &str| resolve_tag(tag, NAMES.iter().copied());
        assert_eq!(resolve("fr-CA"), Some("fr-CA"));
        assert_eq!(resolve("FR_ca"), Some("fr-CA"));
        assert_eq!(resolve("fr"), Some("fr-FR"));
        assert_eq!(resolve("fr-BE"), Some("fr-FR"));
        assert_eq!(resolve("pt-PT"), Some("pt-BR"));
        assert_eq!(resolve("en"), Some("en-US"));
        assert_eq!(resolve("de-CH"), Some("de_CH"));
        // No likely region among the names: the first name of the language.
        assert_eq!(resolve("de"), Some("de_CH"));
        assert_eq!(resolve("xx"), None);
    }

    #[test]
    fn standalone_months_default_to_the_months_of_a_date() {
        let mut spec = DateTimeLocaleSpec::default();
        assert_eq!(spec.standalone_months(), &spec.months);
        let standalone = spec.months.clone().map(|month| month.to_uppercase());
        spec.standalone_months = Some(standalone.clone());
        assert_eq!(spec.standalone_months(), &standalone);
        let json = serde_json::to_string(&DateTimeLocaleSpec::default()).unwrap();
        assert!(!json.contains("standalone"), "{json}");
    }
}
