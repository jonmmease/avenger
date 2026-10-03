use crate::unsupported;
use avenger_format::{DateTimeFormatError, DateTimeInputKind};
use icu_datetime::{pattern::DateTimePattern, provider::fields::FieldSymbol};

/// Parse ICU4X's supported subset of Unicode datetime patterns for the input type.
/// Reject unsupported fields that ICU4X would render as literals, including valid
/// Unicode fields such as `w` and standalone `SSS`. Date inputs reject time fields,
/// and Date and Naive inputs reject timezone fields.
pub(super) fn parse(
    source: &str,
    input: DateTimeInputKind,
) -> Result<DateTimePattern, DateTimeFormatError> {
    let pattern =
        source
            .parse::<DateTimePattern>()
            .map_err(|error| DateTimeFormatError::InvalidPattern {
                message: error.to_string(),
                position: None,
            })?;
    let mut chars = source.chars().peekable();
    let mut quoted = false;
    let mut second_suffix = None;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
            } else {
                quoted = !quoted;
            }
            second_suffix = None;
            continue;
        }
        if quoted {
            continue;
        }
        // ICU parses fractions only as a suffix of seconds, such as ss.SSS.
        if ch == '.' && second_suffix == Some('s') {
            second_suffix = Some('.');
            continue;
        }
        if ch == 'S' && second_suffix.is_some() && input != DateTimeInputKind::Date {
            second_suffix = Some('S');
            continue;
        }
        second_suffix = (ch == 's').then_some('s');
        if !ch.is_ascii_alphabetic() {
            continue;
        }
        // Z is an ICU parser alias for an offset field.
        let symbol = FieldSymbol::try_from(if ch == 'Z' { 'x' } else { ch })
            .map_err(|_| unsupported(input, format!("unsupported ICU pattern field `{ch}`")))?;
        let compatible = match symbol {
            FieldSymbol::TimeZone(_) => input == DateTimeInputKind::Zoned,
            FieldSymbol::DayPeriod(_)
            | FieldSymbol::Hour(_)
            | FieldSymbol::Minute
            | FieldSymbol::Second(_) => input != DateTimeInputKind::Date,
            _ => true,
        };
        if !compatible {
            return Err(unsupported(
                input,
                format!("ICU pattern field `{ch}` requires time or timezone information"),
            ));
        }
    }
    Ok(pattern)
}
