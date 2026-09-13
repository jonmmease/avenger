use crate::{
    error::DateTimeFormatError,
    fields::{Pattern, PatternToken},
    parser::parse_datetime_spec,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    pub date_time: String,
    pub date: String,
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

/// A validated locale shared by prepared datetime formatters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDateTimeLocale {
    id: LocaleId,
    definition: Arc<DateTimeLocaleSpec>,
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
        let source = [&definition.date_time, &definition.date, &definition.time];
        let parsed = source
            .map(|spec| parse_datetime_spec(spec))
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
                    return Err(DateTimeFormatError::InvalidLocaleData(format!(
                        "recursive `%{code}` locale pattern"
                    )));
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
