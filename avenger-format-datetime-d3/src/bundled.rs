use crate::{DateTimeFormatError, DateTimeLocaleSpec, ResolvedDateTimeLocale};

#[cfg(not(feature = "all-locales"))]
pub(crate) const LOCALES: &[(&str, &str)] = &[("en-US", include_str!("../locales/en-US.json"))];

#[cfg(feature = "all-locales")]
pub(crate) const LOCALES: &[(&str, &str)] = &[
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

pub(crate) fn resolve(id: &str) -> Result<ResolvedDateTimeLocale, DateTimeFormatError> {
    let (_, json) = LOCALES
        .iter()
        .find(|(name, _)| *name == id)
        .ok_or_else(|| DateTimeFormatError::LocaleUnavailable {
            locale: id.into(),
            message: "no bundled D3 definition is available in this build".into(),
        })?;
    let definition: DateTimeLocaleSpec =
        serde_json::from_str(json).expect("bundled D3 time locale");
    ResolvedDateTimeLocale::new(id, definition)
}
