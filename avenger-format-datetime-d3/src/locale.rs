use crate::{
    parser::{parse_datetime_spec, Pattern, PatternToken},
    DateTimeFormatError,
};
use avenger_format_locales_d3::{DateTimeLocaleSpec, LocaleId};
use std::sync::Arc;

/// A validated locale shared by prepared datetime formatters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDateTimeLocale {
    id: LocaleId,
    pub(crate) definition: Arc<DateTimeLocaleSpec>,
    pub(crate) patterns: [Pattern; 3],
}
impl ResolvedDateTimeLocale {
    /// Return the name assigned when this locale was resolved.
    pub fn id(&self) -> &LocaleId {
        &self.id
    }

    /// Borrow the validated D3 definition used by the compiled locale patterns.
    pub fn definition(&self) -> &DateTimeLocaleSpec {
        &self.definition
    }

    /// Validate locale patterns and reject recursive `%c`, `%x`, or `%X` expansion.
    pub fn new(
        id: impl Into<String>,
        definition: DateTimeLocaleSpec,
    ) -> Result<Self, DateTimeFormatError> {
        let id = id.into();
        let source = [
            ("dateTime", &definition.date_time),
            ("date", &definition.date),
            ("time", &definition.time),
        ];
        let parsed = source
            .map(|(field, spec)| {
                parse_datetime_spec(spec).map_err(|error| DateTimeFormatError::InvalidLocaleData {
                    message: format!("locale `{id}` field `{field}`: {error}"),
                })
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let patterns = [
            expand(&parsed[0], &parsed, &mut vec!['c'])?,
            expand(&parsed[1], &parsed, &mut vec!['x'])?,
            expand(&parsed[2], &parsed, &mut vec!['X'])?,
        ];
        Ok(Self {
            id: LocaleId::new(id),
            definition: Arc::new(definition),
            patterns,
        })
    }

    /// Resolve the bundled U.S. English locale.
    pub fn en_us() -> Self {
        Self::new("en-US", DateTimeLocaleSpec::default()).expect("bundled D3 time locale")
    }
}

pub(crate) fn expand(
    pattern: &Pattern,
    locale: &[Pattern],
    stack: &mut Vec<char>,
) -> Result<Pattern, DateTimeFormatError> {
    let mut output = Vec::new();
    for token in &pattern.0 {
        if let PatternToken::Directive { code, .. } = token {
            if let Some(index) = ['c', 'x', 'X'].iter().position(|item| item == code) {
                if stack.contains(code) {
                    return Err(DateTimeFormatError::InvalidLocaleData {
                        message: format!("recursive `%{code}` locale pattern"),
                    });
                }
                stack.push(*code);
                output.extend(expand(&locale[index], locale, stack)?.0);
                stack.pop();
                continue;
            }
        }
        output.push(token.clone());
    }
    Ok(Pattern(output))
}
