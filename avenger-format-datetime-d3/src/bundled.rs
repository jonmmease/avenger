use crate::{DateTimeFormatError, ResolvedDateTimeLocale};

/// The bundled locale named `id`, from avenger-format-locales-d3.
pub(crate) fn resolve(id: &str) -> Result<ResolvedDateTimeLocale, DateTimeFormatError> {
    let definition = avenger_format_locales_d3::bundled(id).ok_or_else(|| {
        DateTimeFormatError::LocaleUnavailable {
            locale: id.into(),
            message: "no bundled D3 definition is available in this build".into(),
        }
    })?;
    ResolvedDateTimeLocale::new(id, definition)
}
