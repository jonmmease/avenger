//! Formatting: GoogleSQL's text of a date, a time, a datetime, or an instant in a format.

use chrono::{
    DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;

use crate::element::{escape_absent, tokens, Kind, Pad, Token};
use crate::Error;

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A format string, prepared for values of one [`Kind`]: the elements that the kind lacks print as
/// written, as in GoogleSQL's `FORMAT_DATE`, `FORMAT_TIME`, `FORMAT_DATETIME`, and
/// `FORMAT_TIMESTAMP`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Format {
    tokens: Vec<Token>,
}

/// The fields of a value: its civil date and time, the seconds of its offset from UTC, and the
/// seconds since the epoch of its instant.
struct Fields {
    local: NaiveDateTime,
    offset: i32,
    epoch: i64,
}

impl Format {
    /// The format `format` for values of `kind`, or GoogleSQL's error for `%J`, which GoogleSQL
    /// documents but does not format.
    pub fn new(format: &str, kind: Kind) -> Result<Format, Error> {
        let tokens = tokens(&escape_absent(format, kind.absent()));
        let iso_day = Token::Element {
            letter: b'J',
            pad: Pad::Default,
            modifier: false,
        };
        // A `TIME` has no date, so its `%J` is text.
        if kind != Kind::Time && tokens.contains(&iso_day) {
            return Err(Error::Unsupported(
                "Format element %J not supported yet".into(),
            ));
        }
        // The `%J` of a `TIME` prints as written.
        let tokens = tokens
            .into_iter()
            .map(|token| match token {
                Token::Element { letter: b'J', .. } => Token::Text("%J".into()),
                token => token,
            })
            .collect();
        Ok(Format { tokens })
    }

    /// The text of the date `date`.
    pub fn format_date(&self, date: NaiveDate) -> String {
        self.format_datetime(date.and_time(NaiveTime::MIN))
    }

    /// The text of the time of day `time`, on 1970-01-01.
    pub fn format_time(&self, time: NaiveTime) -> String {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01");
        self.format_datetime(epoch.and_time(time))
    }

    /// The text of the date and time `datetime`, which formats as a time in UTC.
    pub fn format_datetime(&self, datetime: NaiveDateTime) -> String {
        self.render(&Fields {
            local: datetime,
            offset: 0,
            epoch: datetime.and_utc().timestamp(),
        })
    }

    /// The text of the instant `instant` in the time zone `zone`.
    pub fn format_timestamp(&self, instant: DateTime<Utc>, zone: Tz) -> String {
        let offset = zone.offset_from_utc_datetime(&instant.naive_utc()).fix();
        self.render(&Fields {
            local: instant.naive_utc() + offset,
            offset: offset.local_minus_utc(),
            epoch: instant.timestamp(),
        })
    }

    fn render(&self, fields: &Fields) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => out.push_str(text),
                Token::Element {
                    letter,
                    pad,
                    modifier,
                } => element(&mut out, *letter, *pad, *modifier, fields),
                Token::Seconds(digits) => seconds(&mut out, fields, Some(*digits)),
                Token::SecondsTrimmed => seconds(&mut out, fields, None),
                Token::Year4 => {
                    let year = fields.local.year();
                    let sign = if year < 0 { "-" } else { "" };
                    out.push_str(&format!("{sign}{:04}", year.unsigned_abs()));
                }
                Token::OffsetColon => offset(&mut out, fields.offset, ":", false),
                Token::OffsetSeconds => offset(&mut out, fields.offset, ":", true),
            }
        }
        out
    }
}

/// The number `value` padded to `width` by `pad`, or by `default` when `pad` is the element's own.
fn number(out: &mut String, value: i64, width: usize, default: char, pad: Pad) {
    let fill = match pad {
        Pad::Default => Some(default),
        Pad::None => None,
        Pad::Space => Some(' '),
        Pad::Zero => Some('0'),
    };
    let digits = value.unsigned_abs().to_string();
    if value < 0 {
        out.push('-');
    }
    if let Some(fill) = fill {
        for _ in digits.len()..width {
            out.push(fill);
        }
    }
    out.push_str(&digits);
}

/// The offset `seconds` as `+hhmm`, with `separator` between its parts, and its seconds when
/// `with_seconds`.
fn offset(out: &mut String, seconds: i32, separator: &str, with_seconds: bool) {
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.unsigned_abs();
    out.push(sign);
    out.push_str(&format!(
        "{:02}{separator}{:02}",
        seconds / 3600,
        seconds / 60 % 60
    ));
    if with_seconds {
        out.push_str(&format!("{separator}{:02}", seconds % 60));
    }
}

/// The seconds with `digits` digits of their fraction, or all of its digits without trailing
/// zeros when `None`.
fn seconds(out: &mut String, fields: &Fields, digits: Option<usize>) {
    out.push_str(&format!("{:02}", fields.local.second()));
    let nanos = format!("{:09}", fields.local.nanosecond() % 1_000_000_000);
    let fraction = match digits {
        Some(digits) => {
            let mut fraction: String = nanos.chars().take(digits).collect();
            fraction.extend(std::iter::repeat_n('0', digits.saturating_sub(9)));
            fraction
        }
        None => nanos.trim_end_matches('0').to_string(),
    };
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(&fraction);
    }
}

/// The element `letter`, with the `E` or `O` modifier when `modifier`, as GoogleSQL formats it in
/// its en-US locale.
fn element(out: &mut String, letter: u8, pad: Pad, modifier: bool, fields: &Fields) {
    let local = fields.local;
    let weekday = local.weekday().num_days_from_sunday() as usize;
    let hour12 = (local.hour() + 11) % 12 + 1;
    let am = local.hour() < 12;
    let composite = |out: &mut String, format: &str| {
        for token in tokens(format) {
            match token {
                Token::Text(text) => out.push_str(&text),
                Token::Element {
                    letter, modifier, ..
                } => element(out, letter, Pad::Default, modifier, fields),
                _ => unreachable!("composites have letters only"),
            }
        }
    };
    match letter {
        b'A' => out.push_str(DAYS[weekday]),
        b'a' => out.push_str(&DAYS[weekday][..3]),
        b'B' => out.push_str(MONTHS[local.month0() as usize]),
        b'b' | b'h' => out.push_str(&MONTHS[local.month0() as usize][..3]),
        b'C' => number(out, i64::from(local.year()).div_euclid(100), 2, '0', pad),
        // The C library writes the year of `%c` in at least four digits.
        b'c' => composite(out, "%a %b %e %H:%M:%S %EY"),
        b'D' | b'x' => composite(out, "%m/%d/%y"),
        b'd' => number(out, i64::from(local.day()), 2, '0', pad),
        b'e' => number(out, i64::from(local.day()), 2, ' ', pad),
        b'F' => composite(out, "%Y-%m-%d"),
        b'G' => number(out, i64::from(local.iso_week().year()), 4, '0', pad),
        b'g' => number(
            out,
            i64::from(local.iso_week().year()).rem_euclid(100),
            2,
            '0',
            pad,
        ),
        b'H' => number(out, i64::from(local.hour()), 2, '0', pad),
        b'I' => number(out, i64::from(hour12), 2, '0', pad),
        b'j' => number(out, i64::from(local.ordinal()), 3, '0', pad),
        b'k' => number(out, i64::from(local.hour()), 2, ' ', pad),
        b'l' => number(out, i64::from(hour12), 2, ' ', pad),
        b'M' => number(out, i64::from(local.minute()), 2, '0', pad),
        b'm' => number(out, i64::from(local.month()), 2, '0', pad),
        b'n' => out.push('\n'),
        b'P' => out.push_str(if am { "am" } else { "pm" }),
        b'p' => out.push_str(if am { "AM" } else { "PM" }),
        b'Q' => number(out, i64::from(local.month0() / 3 + 1), 1, '0', pad),
        b'R' => composite(out, "%H:%M"),
        b'r' => composite(out, "%I:%M:%S %p"),
        b'S' => number(out, i64::from(local.second()), 2, '0', pad),
        b's' => number(out, fields.epoch, 1, '0', pad),
        b'T' | b'X' => composite(out, "%H:%M:%S"),
        b't' => out.push('\t'),
        b'U' => {
            let week = (local.ordinal0() + 7 - local.weekday().num_days_from_sunday()) / 7;
            number(out, i64::from(week), 2, '0', pad)
        }
        b'u' => number(
            out,
            i64::from(local.weekday().number_from_monday()),
            1,
            '0',
            pad,
        ),
        b'V' => number(out, i64::from(local.iso_week().week()), 2, '0', pad),
        b'W' => {
            let week = (local.ordinal0() + 7 - local.weekday().num_days_from_monday()) / 7;
            number(out, i64::from(week), 2, '0', pad)
        }
        b'w' => number(out, weekday as i64, 1, '0', pad),
        // `%Y` writes as many digits as the year has, and its forms with a flag or a modifier
        // four or more, as the C library writes them.
        b'Y' if pad == Pad::Default && !modifier => {
            number(out, i64::from(local.year()), 1, '0', pad)
        }
        b'Y' => number(out, i64::from(local.year()), 4, '0', pad),
        b'y' => number(out, i64::from(local.year()).rem_euclid(100), 2, '0', pad),
        b'Z' => {
            out.push_str("UTC");
            if fields.offset != 0 {
                let sign = if fields.offset < 0 { '-' } else { '+' };
                let minutes = fields.offset.unsigned_abs() / 60;
                out.push(sign);
                match minutes % 60 {
                    0 => out.push_str(&(minutes / 60).to_string()),
                    rest => out.push_str(&format!("{:02}{rest:02}", minutes / 60)),
                }
            }
        }
        b'z' => offset(out, fields.offset, "", false),
        _ => unreachable!("the tokenizer takes documented letters only"),
    }
}
