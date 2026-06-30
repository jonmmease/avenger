use chrono_tz::Tz;

use crate::error::DateTimeFormatError;

pub fn parse_datetime_timezone(value: &str) -> Result<Tz, DateTimeFormatError> {
    match value {
        "UTC" | "utc" => Ok(Tz::UTC),
        "local" => Err(DateTimeFormatError::InvalidTimezone(value.to_string())),
        other => other
            .parse::<Tz>()
            .map_err(|_| DateTimeFormatError::InvalidTimezone(other.to_string())),
    }
}
