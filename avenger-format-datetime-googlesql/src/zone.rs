//! Time zones, as GoogleSQL's functions and `%Z` read them.

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;

/// A time zone: a zone of the tz database, or a fixed offset from UTC, which the tz database does
/// not hold, such as `+05:30`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    Named(Tz),
    Fixed(FixedOffset),
}

impl From<Tz> for Zone {
    fn from(tz: Tz) -> Self {
        Zone::Named(tz)
    }
}

impl Zone {
    /// UTC, as an offset of zero.
    pub const UTC: Zone = Zone::Fixed(FixedOffset::east_opt(0).expect("UTC"));

    /// The zone that GoogleSQL reads from `name`: a name of the tz database in any case, such as
    /// `America/New_York` or `utc`, or an offset such as `+05:30`, `+0530`, `-08`, or `UTC+5`.
    pub fn parse(name: &str) -> Option<Zone> {
        let named = name.parse::<Tz>().ok().or_else(|| {
            (chrono_tz::TZ_VARIANTS.iter())
                .find(|tz| tz.name().eq_ignore_ascii_case(name))
                .copied()
        });
        if let Some(tz) = named {
            return Some(Zone::Named(tz));
        }
        let offset = name.strip_prefix("UTC").unwrap_or(name);
        if offset.is_empty() {
            return Some(Zone::UTC);
        }
        let (sign, digits) = match offset.as_bytes()[0] {
            b'+' => (1, &offset[1..]),
            b'-' => (-1, &offset[1..]),
            _ => return None,
        };
        let (hours, minutes) = match digits.split_once(':') {
            Some((hours, minutes)) => (hours, minutes),
            None if digits.len() > 2 => digits.split_at(digits.len() - 2),
            None => (digits, "0"),
        };
        let all_digits =
            |s: &str| !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit());
        if !all_digits(hours) || !all_digits(minutes) {
            return None;
        }
        let (hours, minutes): (i32, i32) = (hours.parse().ok()?, minutes.parse().ok()?);
        if hours > 14 || minutes >= 60 {
            return None;
        }
        FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60)).map(Zone::Fixed)
    }

    /// The seconds of the zone's offset from UTC at the instant `instant`.
    pub(crate) fn offset(self, instant: DateTime<Utc>) -> i32 {
        match self {
            Zone::Fixed(offset) => offset.local_minus_utc(),
            Zone::Named(tz) => tz
                .offset_from_utc_datetime(&instant.naive_utc())
                .fix()
                .local_minus_utc(),
        }
    }

    /// The civil date and time in the zone of the instant `instant`.
    pub fn local(self, instant: DateTime<Utc>) -> NaiveDateTime {
        instant.naive_utc() + Duration::seconds(self.offset(instant).into())
    }

    /// The instant of the civil time `local` in the zone. A time that a transition skips takes the
    /// offset before the transition, and a time that it repeats takes the earlier instant.
    pub(crate) fn instant(self, local: NaiveDateTime) -> DateTime<Utc> {
        match self {
            Zone::Fixed(offset) => {
                (local - Duration::seconds(offset.local_minus_utc().into())).and_utc()
            }
            Zone::Named(tz) => match tz.from_local_datetime(&local) {
                chrono::LocalResult::Single(t) => t.with_timezone(&Utc),
                chrono::LocalResult::Ambiguous(earlier, _) => earlier.with_timezone(&Utc),
                chrono::LocalResult::None => {
                    let before = tz
                        .offset_from_utc_datetime(&(local - Duration::days(1)))
                        .fix();
                    (local - Duration::seconds(before.local_minus_utc().into())).and_utc()
                }
            },
        }
    }
}
