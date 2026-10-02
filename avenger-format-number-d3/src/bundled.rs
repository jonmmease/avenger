use crate::{FormatError, NumberLocaleSpec, ResolvedNumberLocale};

#[cfg(not(feature = "all-locales"))]
pub(crate) const LOCALES: &[(&str, &str)] = &[("en-US", include_str!("../locales/en-US.json"))];

#[cfg(feature = "all-locales")]
pub(crate) const LOCALES: &[(&str, &str)] = &[
    ("ar-001", include_str!("../locales/ar-001.json")),
    ("ar-AE", include_str!("../locales/ar-AE.json")),
    ("ar-BH", include_str!("../locales/ar-BH.json")),
    ("ar-DJ", include_str!("../locales/ar-DJ.json")),
    ("ar-DZ", include_str!("../locales/ar-DZ.json")),
    ("ar-EG", include_str!("../locales/ar-EG.json")),
    ("ar-EH", include_str!("../locales/ar-EH.json")),
    ("ar-ER", include_str!("../locales/ar-ER.json")),
    ("ar-IL", include_str!("../locales/ar-IL.json")),
    ("ar-IQ", include_str!("../locales/ar-IQ.json")),
    ("ar-JO", include_str!("../locales/ar-JO.json")),
    ("ar-KM", include_str!("../locales/ar-KM.json")),
    ("ar-KW", include_str!("../locales/ar-KW.json")),
    ("ar-LB", include_str!("../locales/ar-LB.json")),
    ("ar-LY", include_str!("../locales/ar-LY.json")),
    ("ar-MA", include_str!("../locales/ar-MA.json")),
    ("ar-MR", include_str!("../locales/ar-MR.json")),
    ("ar-OM", include_str!("../locales/ar-OM.json")),
    ("ar-PS", include_str!("../locales/ar-PS.json")),
    ("ar-QA", include_str!("../locales/ar-QA.json")),
    ("ar-SA", include_str!("../locales/ar-SA.json")),
    ("ar-SD", include_str!("../locales/ar-SD.json")),
    ("ar-SO", include_str!("../locales/ar-SO.json")),
    ("ar-SS", include_str!("../locales/ar-SS.json")),
    ("ar-SY", include_str!("../locales/ar-SY.json")),
    ("ar-TD", include_str!("../locales/ar-TD.json")),
    ("ar-TN", include_str!("../locales/ar-TN.json")),
    ("ar-YE", include_str!("../locales/ar-YE.json")),
    ("ca-ES", include_str!("../locales/ca-ES.json")),
    ("cs-CZ", include_str!("../locales/cs-CZ.json")),
    ("da-DK", include_str!("../locales/da-DK.json")),
    ("de-CH", include_str!("../locales/de-CH.json")),
    ("de-DE", include_str!("../locales/de-DE.json")),
    ("en-CA", include_str!("../locales/en-CA.json")),
    ("en-GB", include_str!("../locales/en-GB.json")),
    ("en-IE", include_str!("../locales/en-IE.json")),
    ("en-IN", include_str!("../locales/en-IN.json")),
    ("en-US", include_str!("../locales/en-US.json")),
    ("es-BO", include_str!("../locales/es-BO.json")),
    ("es-ES", include_str!("../locales/es-ES.json")),
    ("es-MX", include_str!("../locales/es-MX.json")),
    ("fi-FI", include_str!("../locales/fi-FI.json")),
    ("fr-CA", include_str!("../locales/fr-CA.json")),
    ("fr-FR", include_str!("../locales/fr-FR.json")),
    ("he-IL", include_str!("../locales/he-IL.json")),
    ("hu-HU", include_str!("../locales/hu-HU.json")),
    ("it-IT", include_str!("../locales/it-IT.json")),
    ("ja-JP", include_str!("../locales/ja-JP.json")),
    ("ko-KR", include_str!("../locales/ko-KR.json")),
    ("mk-MK", include_str!("../locales/mk-MK.json")),
    ("nl-NL", include_str!("../locales/nl-NL.json")),
    ("pl-PL", include_str!("../locales/pl-PL.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
    ("pt-PT", include_str!("../locales/pt-PT.json")),
    ("ru-RU", include_str!("../locales/ru-RU.json")),
    ("sl-SI", include_str!("../locales/sl-SI.json")),
    ("sv-SE", include_str!("../locales/sv-SE.json")),
    ("uk-UA", include_str!("../locales/uk-UA.json")),
    ("zh-CN", include_str!("../locales/zh-CN.json")),
];

pub(crate) fn resolve(id: &str) -> Result<ResolvedNumberLocale, FormatError> {
    let (_, json) = LOCALES
        .iter()
        .find(|(name, _)| *name == id)
        .ok_or_else(|| FormatError::LocaleNotFound(id.into()))?;
    let definition: NumberLocaleSpec =
        serde_json::from_str(json).expect("bundled D3 number locale");
    ResolvedNumberLocale::new(id, definition)
}
