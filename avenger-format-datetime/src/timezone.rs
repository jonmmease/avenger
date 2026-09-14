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

#[cfg(test)]
mod tests {
    use super::parse_datetime_timezone;
    use chrono_tz::Tz;

    #[test]
    fn parses_utc_and_iana_timezones() {
        assert_eq!(parse_datetime_timezone("UTC").unwrap(), Tz::UTC);
        assert_eq!(
            parse_datetime_timezone("America/New_York").unwrap(),
            Tz::America__New_York
        );
    }

    #[test]
    fn rejects_local_timezone_placeholder() {
        assert!(parse_datetime_timezone("local").is_err());
    }
}
