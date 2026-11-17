use crate::{NumberFormatError, ResolvedNumberLocale};

/// The bundled locale named `id`, from avenger-format-locales-d3.
pub(crate) fn resolve(id: &str) -> Result<ResolvedNumberLocale, NumberFormatError> {
    let definition = avenger_format_locales_d3::bundled_number(id).ok_or_else(|| {
        NumberFormatError::LocaleUnavailable {
            locale: id.into(),
            message: "no bundled D3 definition is available in this build".into(),
        }
    })?;
    ResolvedNumberLocale::new(id, definition)
}
