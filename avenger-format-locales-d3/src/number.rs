//! d3's number locales.

use avenger_format::NumberFormatError;
use serde::{Deserialize, Serialize};

/// A d3 number locale definition. Missing JSON fields use [`Self::default`].
/// The default has no grouping or currency affixes. Use [`Self::en_us`] for U.S. conventions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberLocaleSpec {
    /// Decimal separator, defaulting to `.`.
    pub decimal: String,
    /// Separator between digit groups, defaulting to an empty string.
    pub thousands: String,
    /// Positive group sizes, cycled from right to left. An empty list disables grouping.
    pub grouping: Vec<usize>,
    /// Prefix and suffix for `$`, independent of digit precision.
    pub currency: [String; 2],
    /// Replacement strings for ASCII digits 0 through 9, applied to the entire label.
    pub numerals: Option<[String; 10]>,
    /// Suffix for percent formats, defaulting to `%`.
    pub percent: String,
    /// Negative sign, defaulting to Unicode `−`.
    pub minus: String,
    /// Label for NaN, defaulting to `NaN`.
    pub nan: String,
}

impl Default for NumberLocaleSpec {
    fn default() -> Self {
        Self {
            decimal: ".".into(),
            thousands: String::new(),
            grouping: Vec::new(),
            currency: [String::new(), String::new()],
            numerals: None,
            percent: "%".into(),
            minus: "−".into(),
            nan: "NaN".into(),
        }
    }
}

impl NumberLocaleSpec {
    /// Load the bundled U.S. English d3 definition.
    pub fn en_us() -> Self {
        serde_json::from_str(include_str!("../locales/number/en-US.json"))
            .expect("bundled D3 number locale")
    }

    /// The definition of the JSON `json`, the locale `id` in its errors.
    pub fn from_json(id: &str, json: &str) -> Result<Self, NumberFormatError> {
        serde_json::from_str(json).map_err(|error| NumberFormatError::InvalidLocaleData {
            message: format!("locale `{id}`: {error}"),
        })
    }

    /// Rejects a group size of zero, the locale `id` in the error.
    pub fn validate(&self, id: &str) -> Result<(), NumberFormatError> {
        match self.grouping.contains(&0) {
            true => Err(NumberFormatError::InvalidLocaleData {
                message: format!("locale `{id}` grouping sizes must be positive"),
            }),
            false => Ok(()),
        }
    }
}

/// The bundled number locales: U.S. English, or all of d3's with the `all-locales` feature.
#[cfg(not(feature = "all-locales"))]
pub const BUNDLED_NUMBER: &[(&str, &str)] =
    &[("en-US", include_str!("../locales/number/en-US.json"))];

/// The bundled number locales: U.S. English, or all of d3's with the `all-locales` feature.
#[cfg(feature = "all-locales")]
pub const BUNDLED_NUMBER: &[(&str, &str)] = &[
    ("ar-001", include_str!("../locales/number/ar-001.json")),
    ("ar-AE", include_str!("../locales/number/ar-AE.json")),
    ("ar-BH", include_str!("../locales/number/ar-BH.json")),
    ("ar-DJ", include_str!("../locales/number/ar-DJ.json")),
    ("ar-DZ", include_str!("../locales/number/ar-DZ.json")),
    ("ar-EG", include_str!("../locales/number/ar-EG.json")),
    ("ar-EH", include_str!("../locales/number/ar-EH.json")),
    ("ar-ER", include_str!("../locales/number/ar-ER.json")),
    ("ar-IL", include_str!("../locales/number/ar-IL.json")),
    ("ar-IQ", include_str!("../locales/number/ar-IQ.json")),
    ("ar-JO", include_str!("../locales/number/ar-JO.json")),
    ("ar-KM", include_str!("../locales/number/ar-KM.json")),
    ("ar-KW", include_str!("../locales/number/ar-KW.json")),
    ("ar-LB", include_str!("../locales/number/ar-LB.json")),
    ("ar-LY", include_str!("../locales/number/ar-LY.json")),
    ("ar-MA", include_str!("../locales/number/ar-MA.json")),
    ("ar-MR", include_str!("../locales/number/ar-MR.json")),
    ("ar-OM", include_str!("../locales/number/ar-OM.json")),
    ("ar-PS", include_str!("../locales/number/ar-PS.json")),
    ("ar-QA", include_str!("../locales/number/ar-QA.json")),
    ("ar-SA", include_str!("../locales/number/ar-SA.json")),
    ("ar-SD", include_str!("../locales/number/ar-SD.json")),
    ("ar-SO", include_str!("../locales/number/ar-SO.json")),
    ("ar-SS", include_str!("../locales/number/ar-SS.json")),
    ("ar-SY", include_str!("../locales/number/ar-SY.json")),
    ("ar-TD", include_str!("../locales/number/ar-TD.json")),
    ("ar-TN", include_str!("../locales/number/ar-TN.json")),
    ("ar-YE", include_str!("../locales/number/ar-YE.json")),
    ("ca-ES", include_str!("../locales/number/ca-ES.json")),
    ("cs-CZ", include_str!("../locales/number/cs-CZ.json")),
    ("da-DK", include_str!("../locales/number/da-DK.json")),
    ("de-CH", include_str!("../locales/number/de-CH.json")),
    ("de-DE", include_str!("../locales/number/de-DE.json")),
    ("en-CA", include_str!("../locales/number/en-CA.json")),
    ("en-GB", include_str!("../locales/number/en-GB.json")),
    ("en-IE", include_str!("../locales/number/en-IE.json")),
    ("en-IN", include_str!("../locales/number/en-IN.json")),
    ("en-US", include_str!("../locales/number/en-US.json")),
    ("es-BO", include_str!("../locales/number/es-BO.json")),
    ("es-ES", include_str!("../locales/number/es-ES.json")),
    ("es-MX", include_str!("../locales/number/es-MX.json")),
    ("fi-FI", include_str!("../locales/number/fi-FI.json")),
    ("fr-CA", include_str!("../locales/number/fr-CA.json")),
    ("fr-FR", include_str!("../locales/number/fr-FR.json")),
    ("he-IL", include_str!("../locales/number/he-IL.json")),
    ("hu-HU", include_str!("../locales/number/hu-HU.json")),
    ("it-IT", include_str!("../locales/number/it-IT.json")),
    ("ja-JP", include_str!("../locales/number/ja-JP.json")),
    ("ko-KR", include_str!("../locales/number/ko-KR.json")),
    ("mk-MK", include_str!("../locales/number/mk-MK.json")),
    ("nl-NL", include_str!("../locales/number/nl-NL.json")),
    ("pl-PL", include_str!("../locales/number/pl-PL.json")),
    ("pt-BR", include_str!("../locales/number/pt-BR.json")),
    ("pt-PT", include_str!("../locales/number/pt-PT.json")),
    ("ru-RU", include_str!("../locales/number/ru-RU.json")),
    ("sl-SI", include_str!("../locales/number/sl-SI.json")),
    ("sv-SE", include_str!("../locales/number/sv-SE.json")),
    ("uk-UA", include_str!("../locales/number/uk-UA.json")),
    ("zh-CN", include_str!("../locales/number/zh-CN.json")),
];

/// The bundled number definition named `id`, with hyphens or underscores, such as `fr-FR` or
/// `fr_FR`.
pub fn bundled_number(id: &str) -> Option<NumberLocaleSpec> {
    let id = id.replace('_', "-");
    (BUNDLED_NUMBER.iter())
        .find(|(name, _)| *name == id)
        .map(|(name, json)| {
            NumberLocaleSpec::from_json(name, json).expect("bundled D3 number locale")
        })
}
