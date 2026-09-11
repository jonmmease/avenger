use serde::{Deserialize, Serialize};

use crate::error::DateTimeFormatError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DateTimeStyleLength {
    Short,
    #[default]
    Medium,
    Long,
    Full,
}

impl DateTimeStyleLength {
    #[allow(
        clippy::should_implement_trait,
        reason = "Preserve the existing optional parser API."
    )]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "short" => Some(Self::Short),
            "medium" => Some(Self::Medium),
            "long" => Some(Self::Long),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

pub(crate) fn render_datetime_glue(
    pattern: &str,
    date: &str,
    time: &str,
) -> Result<String, DateTimeFormatError> {
    render_datetime_glue_inner(pattern, date, time).map(|rendered| rendered.output)
}

pub(crate) fn validate_datetime_glue_pattern(pattern: &str) -> Result<(), DateTimeFormatError> {
    let rendered = render_datetime_glue_inner(pattern, "", "")?;
    if !rendered.saw_date || !rendered.saw_time {
        return Err(DateTimeFormatError::InvalidLocaleData(
            "datetime glue must contain `{0}` and `{1}`".to_string(),
        ));
    }
    Ok(())
}

struct RenderedDateTimeGlue {
    output: String,
    saw_date: bool,
    saw_time: bool,
}

fn render_datetime_glue_inner(
    pattern: &str,
    date: &str,
    time: &str,
) -> Result<RenderedDateTimeGlue, DateTimeFormatError> {
    let mut output = String::new();
    let mut saw_date = false;
    let mut saw_time = false;
    let mut in_quote = false;
    let mut i = 0;

    while i < pattern.len() {
        let ch = pattern[i..].chars().next().expect("valid char boundary");
        if ch == '\'' {
            if pattern[i + ch.len_utf8()..].starts_with('\'') {
                output.push('\'');
                i += 2;
            } else {
                in_quote = !in_quote;
                i += ch.len_utf8();
            }
            continue;
        }

        if !in_quote && pattern[i..].starts_with("{0}") {
            output.push_str(time);
            saw_time = true;
            i += 3;
            continue;
        }
        if !in_quote && pattern[i..].starts_with("{1}") {
            output.push_str(date);
            saw_date = true;
            i += 3;
            continue;
        }

        if !in_quote && (ch == '{' || ch == '}') {
            return Err(DateTimeFormatError::InvalidLocaleData(
                "datetime glue braces must be `{0}` or `{1}` placeholders".to_string(),
            ));
        }

        output.push(ch);
        i += ch.len_utf8();
    }

    if in_quote {
        return Err(DateTimeFormatError::InvalidLocaleData(
            "datetime glue has an unterminated quoted literal".to_string(),
        ));
    }

    Ok(RenderedDateTimeGlue {
        output,
        saw_date,
        saw_time,
    })
}

#[cfg(test)]
mod tests {
    use super::{render_datetime_glue, validate_datetime_glue_pattern};

    #[test]
    fn renders_placeholders_and_ldml_quotes() {
        assert_eq!(
            render_datetime_glue("{1} 'at' {0}", "Jan 5", "12:00").unwrap(),
            "Jan 5 at 12:00"
        );
        assert_eq!(
            render_datetime_glue("{1} '' {0}", "Jan 5", "12:00").unwrap(),
            "Jan 5 ' 12:00"
        );
    }

    #[test]
    fn validates_required_glue_placeholders() {
        assert!(validate_datetime_glue_pattern("{1}, {0}").is_ok());
        assert!(validate_datetime_glue_pattern("{1} only").is_err());
        assert!(validate_datetime_glue_pattern("{date} {time}").is_err());
        assert!(validate_datetime_glue_pattern("{1} 'bad {0}").is_err());
    }
}
