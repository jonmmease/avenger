//! The check of a chart's format patterns, which a chart's labels, text, and tooltips read in
//! GoogleSQL's syntaxes, as [`crate::functions`] names them: a `FORMAT` specifier for a number and
//! the format elements of `FORMAT_DATE` for a date or a datetime. The check is the providers' that
//! a chart renders with, so a pattern that passes formats, and one that fails is an error at its
//! argument rather than when the chart renders. The formats of GoogleSQL's functions, such as
//! `FORMAT_DATE`, take no check: they print an element that GoogleSQL does not document as written.

use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, NumberFormatError, NumberFormatProvider,
};
use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use datafusion::arrow::datatypes::DataType;

/// The error of the format pattern `pattern` for values of the type `ty`, or for a value whose
/// type the analysis does not know, a pattern of neither numbers nor dates. `None` for a pattern
/// that formats, and for a type that formats as text, such as a string.
pub(super) fn pattern_error(pattern: &str, ty: Option<&DataType>) -> Option<String> {
    let numbers = GoogleSqlNumberFormatProvider::new();
    let dates = GoogleSqlDateTimeFormatProvider::new();
    let number = || numbers.prepare(pattern).err().map(number_message);
    match ty {
        Some(ty) if ty.is_numeric() => number(),
        Some(DataType::Date32 | DataType::Date64) => {
            dates.prepare_date(pattern).err().map(date_message)
        }
        Some(DataType::Timestamp(_, None)) => dates.prepare_naive(pattern).err().map(date_message),
        Some(DataType::Timestamp(_, Some(_))) => {
            dates.prepare_zoned(pattern).err().map(date_message)
        }
        Some(_) => None,
        // A timestamp's format holds every element of a date and of a time.
        None => match (number(), dates.prepare_zoned(pattern).is_err()) {
            (Some(_), true) => Some(format!(
                "`{pattern}` is a format of neither numbers nor dates"
            )),
            _ => None,
        },
    }
}

/// The message of a pattern of numbers that does not prepare, without its byte offset, since the
/// error is at its argument.
fn number_message(error: NumberFormatError) -> String {
    match error {
        NumberFormatError::InvalidPattern { message, .. } => message,
        error => error.to_string(),
    }
}

/// The message of a pattern of dates that does not prepare, without its byte offset.
fn date_message(error: DateTimeFormatError) -> String {
    match error {
        DateTimeFormatError::InvalidPattern { message, .. }
        | DateTimeFormatError::UnsupportedPattern { message, .. } => message,
        error => error.to_string(),
    }
}
