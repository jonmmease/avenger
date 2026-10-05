use avenger_format::{DateTimeFormatError, DateTimeInputKind};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, Timelike, Utc};
use chrono_tz::Tz;
use icu_calendar::{types::RataDie, AsCalendar, Date};
use icu_time::{
    zone::{models::AtTime, TimeZone, TimeZoneInfo, UtcOffset, ZoneNameTimestamp},
    Time,
};

/// Convert the absolute day, preserving Chrono's full date range.
pub(crate) fn date<A: AsCalendar>(value: NaiveDate, calendar: A) -> Date<A> {
    Date::from_rata_die(RataDie::new(i64::from(value.num_days_from_ce())), calendar)
}

/// Chrono represents a leap second with nanoseconds greater than or equal to one billion.
pub(crate) fn time(
    value: NaiveTime,
    input: DateTimeInputKind,
) -> Result<Time, DateTimeFormatError> {
    Time::try_new(
        value.hour() as u8,
        value.minute() as u8,
        (value.second() + value.nanosecond() / 1_000_000_000) as u8,
        value.nanosecond() % 1_000_000_000,
    )
    .map_err(|error| DateTimeFormatError::UnsupportedValue {
        input,
        message: error.to_string(),
    })
}

/// Retain transition rules and ICU's ID for per-value localized timezone names.
#[derive(Debug)]
pub(crate) struct Zone {
    timezone: Tz,
    id: TimeZone,
}

impl Zone {
    pub fn new(timezone: Tz) -> Self {
        Self {
            timezone,
            id: TimeZone::from_iana_id(timezone.name()),
        }
    }

    pub fn timezone(&self) -> Tz {
        self.timezone
    }

    pub fn localize(
        &self,
        value: DateTime<Utc>,
    ) -> Result<(NaiveDateTime, TimeZoneInfo<AtTime>), DateTimeFormatError> {
        let offset = value.with_timezone(&self.timezone).offset().fix();
        let local = value
            .naive_utc()
            .checked_add_offset(offset)
            .ok_or(DateTimeFormatError::OutOfRange)?;
        let offset = UtcOffset::try_from_seconds(offset.local_minus_utc())
            .map_err(|_| DateTimeFormatError::OutOfRange)?;
        let zone = self
            .id
            .with_offset(Some(offset))
            .with_zone_name_timestamp(ZoneNameTimestamp::from_epoch_seconds(value.timestamp()));
        Ok((local, zone))
    }
}
