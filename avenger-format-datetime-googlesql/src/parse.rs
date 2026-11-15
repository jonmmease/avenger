//! Parsing: GoogleSQL's `PARSE_DATE`, `PARSE_TIME`, `PARSE_DATETIME`, and `PARSE_TIMESTAMP`.
//!
//! A string parses element by element. A field that the format does not have takes the value of
//! 1970-01-01 00:00:00. Whitespace in the format matches any amount of whitespace, numbers read up
//! to their width without padding, and names match full or abbreviated, in any case. GoogleSQL
//! rejects a weekday together with a week number, and an ISO week, as "not supported yet".

use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
    Weekday,
};
use chrono_tz::Tz;

use crate::element::Kind;
use crate::Error;

const DAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// The date `input` in the format `format`, as GoogleSQL's `PARSE_DATE` gives it.
pub fn parse_date(format: &str, input: &str) -> Result<NaiveDate, Error> {
    Ok(parse(format, input, Kind::Date, Zone::Utc)?.date_naive())
}

/// The time of day `input` in the format `format`, as GoogleSQL's `PARSE_TIME` gives it.
pub fn parse_time(format: &str, input: &str) -> Result<NaiveTime, Error> {
    Ok(parse(format, input, Kind::Time, Zone::Utc)?.time())
}

/// The date and time `input` in the format `format`, as GoogleSQL's `PARSE_DATETIME` gives it.
pub fn parse_datetime(format: &str, input: &str) -> Result<NaiveDateTime, Error> {
    Ok(parse(format, input, Kind::Datetime, Zone::Utc)?.naive_utc())
}

/// The instant `input` in the format `format`, as GoogleSQL's `PARSE_TIMESTAMP` gives it: a time
/// without an offset or a zone in the string is a civil time in `zone`.
pub fn parse_timestamp(format: &str, input: &str, zone: Tz) -> Result<DateTime<Utc>, Error> {
    parse(format, input, Kind::Timestamp, Zone::Named(zone))
}

/// A time zone of a parse: a name, or a fixed offset in seconds, as `%Z` reads either.
#[derive(Clone, Copy, Debug)]
enum Zone {
    Utc,
    Named(Tz),
    Fixed(i32),
}

impl Zone {
    /// The zone of the text of `%Z`: a name, `UTC`, or an offset such as `+05:30` or `UTC+5`.
    fn of(name: &str) -> Option<Zone> {
        if let Ok(tz) = name.parse::<Tz>() {
            return Some(Zone::Named(tz));
        }
        let offset = name.strip_prefix("UTC").unwrap_or(name);
        if offset.is_empty() {
            return Some(Zone::Utc);
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
        (hours <= 14 && minutes < 60).then_some(Zone::Fixed(sign * (hours * 3600 + minutes * 60)))
    }

    /// The instant of the civil time `local` in the zone. A time that a transition skips takes the
    /// offset before the transition, and a time that it repeats takes the earlier instant.
    fn instant(self, local: NaiveDateTime) -> DateTime<Utc> {
        match self {
            Zone::Utc => local.and_utc(),
            Zone::Fixed(offset) => (local - Duration::seconds(offset.into())).and_utc(),
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

/// An element that resolves the date after the loop: a week, a weekday, a day of the year, or an
/// ISO year, with its letter, its place among the elements, and its number.
#[derive(Clone, Copy, Debug)]
struct DateElement {
    letter: u8,
    position: usize,
    value: i64,
}

/// The fields that the elements set, which start at 1970-01-01 00:00:00.
struct Fields {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    micros: i64,
    weekday: i64,
    century: i64,
    use_century: bool,
    explicit_century: bool,
    twelve_hour: bool,
    afternoon: bool,
    offset: Option<i32>,
    zone: Zone,
    epoch: Option<i64>,
    last_year: Option<usize>,
    last_month: Option<usize>,
    last_day: Option<usize>,
    non_iso: bool,
    elements: Vec<DateElement>,
}

/// The input and the place of the next byte to read.
struct Input<'a> {
    text: &'a str,
    at: usize,
}

impl Input<'_> {
    fn rest(&self) -> &[u8] {
        &self.text.as_bytes()[self.at..]
    }

    fn done(&self) -> bool {
        self.at >= self.text.len()
    }

    fn skip_space(&mut self) {
        while self.rest().first().is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    /// Reads an integer of at most `width` characters, a leading `-` included, in `min..=max`.
    fn int(&mut self, width: usize, min: i64, max: i64) -> Option<i64> {
        let rest = self.rest();
        let negative = rest.first() == Some(&b'-');
        let width = width
            .checked_sub(usize::from(negative))
            .filter(|w| *w > 0)?;
        let start = usize::from(negative);
        let digits = (rest[start..].iter())
            .take(width)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        let mut value: i64 = std::str::from_utf8(&rest[start..start + digits])
            .ok()?
            .parse()
            .ok()?;
        if negative {
            if value == 0 {
                return None;
            }
            value = -value;
        }
        if !(min..=max).contains(&value) {
            return None;
        }
        self.at += start + digits;
        Some(value)
    }

    /// Reads a fraction of a second after a `.`, of at most `digits` digits (all of them for 0),
    /// in microseconds, when the input has a `.`.
    fn fraction(&mut self, digits: usize) -> Option<Option<i64>> {
        if self.rest().first() != Some(&b'.') {
            return Some(None);
        }
        let rest = &self.rest()[1..];
        let count = (rest.iter())
            .take(if digits == 0 { usize::MAX } else { digits })
            .take_while(|b| b.is_ascii_digit())
            .count();
        if count == 0 {
            return None;
        }
        let micros: String = (rest[..count].iter().take(6).map(|b| *b as char)).collect();
        let micros: i64 = format!("{micros:0<6}").parse().ok()?;
        self.at += 1 + count;
        Some(Some(micros))
    }

    /// Reads one of `names`, in full or by its first three letters, without regard to case, and
    /// returns its index.
    fn name(&mut self, names: &[&str]) -> Option<usize> {
        let rest = self.rest();
        let starts = |name: &str| {
            rest.len() >= name.len() && rest[..name.len()].eq_ignore_ascii_case(name.as_bytes())
        };
        for (index, name) in names.iter().enumerate() {
            for name in [*name, &name[..3]] {
                if starts(name) {
                    self.at += name.len();
                    return Some(index);
                }
            }
        }
        None
    }

    /// Reads `AM` or `PM` in any case, and returns whether it is `PM`.
    fn meridian(&mut self) -> Option<bool> {
        let rest = self.rest();
        let pm = match rest.get(..2)?.to_ascii_lowercase().as_slice() {
            b"am" => false,
            b"pm" => true,
            _ => return None,
        };
        self.at += 2;
        Some(pm)
    }

    /// Reads an offset: a sign, two digits of hours, `separator` if any, and two digits of
    /// minutes if any, in minutes.
    fn offset(&mut self, separator: Option<u8>) -> Option<i64> {
        let rest = self.rest();
        let sign = match rest.first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let digits = |slice: &[u8]| slice.len() >= 2 && slice[..2].iter().all(u8::is_ascii_digit);
        if !digits(&rest[1..]) {
            return None;
        }
        let hours = i64::from(rest[1] - b'0') * 10 + i64::from(rest[2] - b'0');
        if hours > 23 {
            return None;
        }
        let mut length = 3;
        let mut after = 3;
        if separator.is_some_and(|s| rest.get(3) == Some(&s)) {
            after = 4;
        }
        let mut minutes = 0;
        if digits(&rest[after.min(rest.len())..]) {
            let m = i64::from(rest[after] - b'0') * 10 + i64::from(rest[after + 1] - b'0');
            if m <= 59 {
                minutes = m;
                length = after + 2;
            }
        }
        self.at += length;
        Some(sign * (hours * 60 + minutes))
    }
}

/// The text of `input` as GoogleSQL's messages quote a string.
fn quoted(input: &str) -> String {
    let mut out = String::from("\"");
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Checks that `format` has no element of a part that a value of `kind` lacks, as GoogleSQL's
/// `PARSE_DATE`, `PARSE_TIME`, and `PARSE_DATETIME` do before they parse.
fn validate(format: &str, kind: Kind) -> Result<(), Error> {
    let (name, invalid): (&str, &[u8]) = match kind {
        Kind::Date => ("DATE", b"cHIklMPpRrSsTXZz"),
        Kind::Time => ("TIME", b"AaBbhCcDdeFGgjmsUuVWwxYyZz"),
        Kind::Datetime => ("DATETIME", b"Zz"),
        Kind::Timestamp => return Ok(()),
    };
    let b = format.as_bytes();
    let mut cur = 0;
    while cur < b.len() {
        while cur < b.len() && b[cur] != b'%' {
            cur += 1;
        }
        let percent = cur;
        while cur < b.len() && b[cur] == b'%' {
            cur += 1;
        }
        if cur == b.len() || (cur - percent) % 2 == 0 {
            continue;
        }
        let error = |element: &[u8]| {
            Err(Error(format!(
                "Invalid format: %{} is not allowed for the {name} type.",
                String::from_utf8_lossy(element)
            )))
        };
        if invalid.contains(&b[cur]) {
            return error(&b[cur..cur + 1]);
        }
        let prev = cur;
        if (b[cur] != b'E' && b[cur] != b'O') || cur + 1 == b.len() {
            continue;
        }
        cur += 1;
        if b[prev] == b'E' {
            let mut end = cur;
            let invalid_extension = invalid.contains(&b[cur])
                || (invalid.contains(&b'S') && (b[cur] == b'*' || b[cur].is_ascii_digit()) && {
                    end += 1;
                    end < b.len() && b[end] == b'S'
                })
                || (invalid.contains(&b'Y') && b[cur] == b'4' && {
                    end = cur + 1;
                    end < b.len() && b[end] == b'Y'
                });
            if invalid_extension {
                return error(&b[prev..=end]);
            }
        } else if invalid.contains(&b[cur]) {
            return error(&b[prev..=cur]);
        }
    }
    Ok(())
}

/// The instant of `input` in `format` for a value of `kind`, as GoogleSQL's parsing functions give
/// it, with `default` for a time without an offset or a zone.
fn parse(format: &str, input: &str, kind: Kind, default: Zone) -> Result<DateTime<Utc>, Error> {
    validate(format, kind)?;
    let failed = || Error(format!("Failed to parse input string {}", quoted(input)));
    let mut data = Input { text: input, at: 0 };
    data.skip_space();
    let mut f = Fields {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        micros: 0,
        weekday: 4,
        century: 0,
        use_century: false,
        explicit_century: false,
        twelve_hour: false,
        afternoon: false,
        offset: None,
        zone: default,
        epoch: None,
        last_year: None,
        last_month: None,
        last_day: None,
        non_iso: false,
        elements: Vec::new(),
    };
    let fmt = format.as_bytes();
    let mut at = 0;
    let mut position = 0;
    let mut ok = true;
    while ok && !data.done() && at < fmt.len() {
        if fmt[at].is_ascii_whitespace() {
            data.skip_space();
            while at < fmt.len() && fmt[at].is_ascii_whitespace() {
                at += 1;
            }
            continue;
        }
        if fmt[at] != b'%' {
            if data.rest()[0] == fmt[at] {
                data.at += 1;
                at += 1;
                continue;
            }
            return Err(Error(format!(
                "Mismatch between format character '{}' and string character '{}' at string index: {} with format: '{format}'",
                fmt[at] as char,
                input[data.at..].chars().next().unwrap_or_default(),
                data.at
            )));
        }
        at += 1;
        if at == fmt.len() {
            return Err(Error("Format string cannot end with a single '%'".into()));
        }
        position += 1;
        let next_is_element = |end: usize| fmt.get(end) == Some(&b'%');
        let letter = fmt[at];
        at += 1;
        ok = match letter {
            b'Y' => year(&mut data, &mut f, next_is_element(at), position),
            b'C' => century(&mut data, &mut f, next_is_element(at), position),
            b'y' => short_year(&mut data, &mut f, position),
            b'm' => data
                .int(2, 1, 12)
                .map(|m| {
                    f.month = m;
                    f.last_month = Some(position);
                    f.non_iso = true;
                })
                .is_some(),
            b'd' => data
                .int(2, 1, 31)
                .map(|d| {
                    f.day = d;
                    f.last_day = Some(position);
                    f.non_iso = true;
                })
                .is_some(),
            b'H' => {
                f.twelve_hour = false;
                data.int(2, 0, 23).map(|h| f.hour = h).is_some()
            }
            b'M' => data.int(2, 0, 59).map(|m| f.minute = m).is_some(),
            b'S' => data.int(2, 0, 60).map(|s| f.second = s).is_some(),
            b'Q' => {
                f.non_iso = true;
                f.last_month = Some(position);
                f.last_day = Some(position);
                data.int(1, 1, 4)
                    .map(|q| {
                        f.month = (q - 1) * 3 + 1;
                        f.day = 1;
                    })
                    .is_some()
            }
            b'P' | b'p' => data.meridian().map(|pm| f.afternoon = pm).is_some(),
            b'I' | b'l' => twelve_hour(&mut data, &mut f),
            b'r' => {
                twelve_hour(&mut data, &mut f)
                    && literal(&mut data, b':')
                    && data.int(2, 0, 59).map(|m| f.minute = m).is_some()
                    && literal(&mut data, b':')
                    && data.int(2, 0, 60).map(|s| f.second = s).is_some()
                    && {
                        data.skip_space();
                        data.meridian().map(|pm| f.afternoon = pm).is_some()
                    }
            }
            b'z' => match data.offset(None) {
                Some(minutes) => {
                    check_offset(minutes)?;
                    f.offset = Some(minutes as i32 * 60);
                    true
                }
                None => false,
            },
            b'Z' => {
                let start = data.at;
                while data
                    .rest()
                    .first()
                    .is_some_and(|b| !b.is_ascii_whitespace())
                {
                    data.at += 1;
                }
                let name = &input[start..data.at];
                if name.is_empty() {
                    false
                } else {
                    f.zone = Zone::of(name)
                        .ok_or_else(|| Error(format!("Invalid time zone: {name}")))?;
                    f.offset = None;
                    true
                }
            }
            b's' => {
                f.non_iso = true;
                f.last_year = Some(position);
                f.last_month = Some(position);
                f.last_day = Some(position);
                // The range of timestamps applies after the loop.
                data.int(12, -99_999_999_999, 999_999_999_999)
                    .map(|s| f.epoch = Some(s))
                    .is_some()
            }
            b'E' => extension(fmt, &mut at, &mut data, &mut f, position)?,
            b'O' => modifier(fmt, &mut at, &mut data, &mut f, position),
            b'U' | b'W' => {
                f.non_iso = true;
                date_element(&mut data, &mut f, letter, position, 2, 0, 53)
            }
            b'V' => date_element(&mut data, &mut f, b'V', position, 2, 1, 53),
            b'J' => date_element(&mut data, &mut f, b'J', position, 3, 1, 371),
            b'j' => {
                f.non_iso = true;
                date_element(&mut data, &mut f, b'j', position, 3, 1, 366)
            }
            b'g' => date_element(&mut data, &mut f, b'g', position, 2, 0, 99),
            b'G' => date_element(&mut data, &mut f, b'G', position, 20, 0, 99_999),
            b'u' => date_element(&mut data, &mut f, b'u', position, 1, 1, 7),
            b'w' => date_element(&mut data, &mut f, b'w', position, 1, 0, 6),
            b'A' | b'a' => match data.name(&DAYS) {
                Some(day) => {
                    f.elements.push(DateElement {
                        letter,
                        position,
                        value: day as i64,
                    });
                    true
                }
                None => false,
            },
            b't' | b'n' => {
                data.skip_space();
                true
            }
            _ => {
                // The C library's elements: names, composites, and `%e`.
                match letter {
                    b'B' | b'b' | b'h' => {
                        f.non_iso = true;
                        f.last_month = Some(position);
                    }
                    b'e' => {
                        f.non_iso = true;
                        f.last_day = Some(position);
                    }
                    b'D' | b'F' | b'x' | b'c' => {
                        f.non_iso = true;
                        f.last_year = Some(position);
                        f.last_month = Some(position);
                        f.last_day = Some(position);
                    }
                    _ => {}
                }
                if matches!(letter, b'c' | b'R' | b'T' | b'X') {
                    f.twelve_hour = false;
                }
                library(letter, &mut data, &mut f)
            }
        };
    }
    if f.twelve_hour && f.afternoon {
        f.hour += 12;
    }
    if ok {
        data.skip_space();
    }
    // The format's trailing whitespace, `%n`, and `%t` match nothing.
    while at < fmt.len() && (fmt[at].is_ascii_whitespace() || fmt[at] == b'%') {
        if fmt[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        at += 1;
        if at == fmt.len() {
            return Err(Error("Format string cannot end with a single '%'".into()));
        }
        if fmt[at] == b'n' || fmt[at] == b't' {
            at += 1;
        } else {
            break;
        }
    }
    if !ok || !data.done() || at != fmt.len() {
        return Err(failed());
    }
    let invalid = || Error("Invalid result from parsing function".into());
    let in_range = |t: DateTime<Utc>| {
        let low = NaiveDate::from_ymd_opt(1, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let high = NaiveDate::from_ymd_opt(9999, 12, 31)
            .unwrap()
            .and_hms_micro_opt(23, 59, 59, 999_999)
            .unwrap();
        (low..=high).contains(&t.naive_utc())
    };
    if let Some(epoch) = f.epoch {
        let t = DateTime::from_timestamp(epoch, 0).ok_or_else(invalid)?;
        return if in_range(t) { Ok(t) } else { Err(invalid()) };
    }
    let mut subseconds = Duration::microseconds(f.micros);
    if f.second == 60 {
        f.second = 59;
        subseconds = Duration::seconds(1);
    }
    let mut year = f.year;
    if f.use_century {
        year += f.century * 100 - 1900;
    }
    let (mut month, mut day) = (f.month, f.day);
    resolve_date(&mut year, &mut month, &mut day, &f)?;
    let out_of_range = || Error("Out-of-range datetime field in parsing function".into());
    let date = i32::try_from(year)
        .ok()
        .and_then(|y| NaiveDate::from_ymd_opt(y, month as u32, day as u32))
        .ok_or_else(out_of_range)?;
    let local = date
        .and_hms_opt(f.hour as u32, f.minute as u32, f.second as u32)
        .ok_or_else(out_of_range)?;
    let instant = match f.offset {
        Some(offset) => Zone::Fixed(offset).instant(local),
        None => f.zone.instant(local),
    } + subseconds;
    if in_range(instant) {
        Ok(instant)
    } else {
        Err(invalid())
    }
}

fn literal(data: &mut Input, byte: u8) -> bool {
    if data.rest().first() == Some(&byte) {
        data.at += 1;
        true
    } else {
        false
    }
}

fn check_offset(minutes: i64) -> Result<(), Error> {
    if minutes.abs() > 14 * 60 {
        let sign = if minutes < 0 { '-' } else { '+' };
        return Err(Error(format!(
            "Timezone offset out of valid range -14:00 to +14:00: {sign}{:02}:{:02}",
            minutes.abs() / 60,
            minutes.abs() % 60
        )));
    }
    Ok(())
}

/// `%Y`: four digits before another element, else five, in 0..=10000.
fn year(data: &mut Input, f: &mut Fields, before_element: bool, position: usize) -> bool {
    let (width, max) = if before_element {
        (4, 9999)
    } else {
        (5, 10_000)
    };
    f.use_century = false;
    f.explicit_century = false;
    f.last_year = Some(position);
    f.non_iso = true;
    data.int(width, 0, max).map(|y| f.year = y).is_some()
}

/// `%C`: the century, two digits before another element, else three.
fn century(data: &mut Input, f: &mut Fields, before_element: bool, position: usize) -> bool {
    let (width, max) = if before_element { (2, 99) } else { (3, 100) };
    let read = data.int(width, 0, max);
    if let Some(century) = read {
        f.century = century;
        if !f.use_century {
            f.year = 1900;
        }
    }
    f.use_century = true;
    f.explicit_century = true;
    f.last_year = Some(position);
    f.non_iso = true;
    read.is_some()
}

/// `%y`: two digits of the year, in the century of `%C`, or 1969 to 2068.
fn short_year(data: &mut Input, f: &mut Fields, position: usize) -> bool {
    let read = data.int(2, 0, 99);
    if let Some(y) = read {
        f.year = 1900 + y;
        if !f.explicit_century {
            f.century = if y < 69 { 20 } else { 19 };
        }
    }
    f.use_century = true;
    f.non_iso = true;
    f.last_year = Some(position);
    read.is_some()
}

/// `%I` and `%l`: an hour of the 12-hour clock, after any whitespace.
fn twelve_hour(data: &mut Input, f: &mut Fields) -> bool {
    data.skip_space();
    data.int(2, 1, 12)
        .map(|h| {
            f.hour = h % 12;
            f.twelve_hour = true;
        })
        .is_some()
}

/// An element that resolves the date after the loop, read as a number.
fn date_element(
    data: &mut Input,
    f: &mut Fields,
    letter: u8,
    position: usize,
    width: usize,
    min: i64,
    max: i64,
) -> bool {
    match data.int(width, min, max) {
        Some(value) => {
            f.elements.push(DateElement {
                letter,
                position,
                value,
            });
            true
        }
        None => false,
    }
}

/// `%E` and its extensions: `%Ez`, `%EY`, `%Ey`, `%EC`, `%E*S`, `%E4Y`, `%E#S` up to 6 digits,
/// and the C library's `%Ec`, `%Ex`, and `%EX`.
fn extension(
    fmt: &[u8],
    at: &mut usize,
    data: &mut Input,
    f: &mut Fields,
    position: usize,
) -> Result<bool, Error> {
    let next = fmt.get(*at).copied();
    let after = fmt.get(*at + 1).copied();
    match (next, after) {
        (Some(b'z'), _) => {
            *at += 1;
            if data.rest().first() == Some(&b'Z') {
                data.at += 1;
                f.offset = Some(0);
                return Ok(true);
            }
            Ok(match data.offset(Some(b':')) {
                Some(minutes) => {
                    check_offset(minutes)?;
                    f.offset = Some(minutes as i32 * 60);
                    true
                }
                None => false,
            })
        }
        (Some(b'Y'), _) => {
            *at += 1;
            Ok(year(data, f, after == Some(b'%'), position))
        }
        (Some(b'y'), _) => {
            *at += 1;
            Ok(short_year(data, f, position))
        }
        (Some(b'C'), _) => {
            *at += 1;
            Ok(century(data, f, after == Some(b'%'), position))
        }
        (Some(b'*'), Some(b'S')) => {
            *at += 2;
            Ok(seconds(data, f, 0))
        }
        (Some(b'4'), Some(b'Y')) => {
            *at += 2;
            let start = data.at;
            let read = data.int(4, 0, 9999);
            f.use_century = false;
            f.non_iso = true;
            f.last_year = Some(position);
            Ok(match read {
                Some(y) if data.at - start == 4 => {
                    f.year = y;
                    true
                }
                _ => false,
            })
        }
        (Some(d), _) if d.is_ascii_digit() => {
            let digits = fmt[*at..]
                .iter()
                .take(2)
                .take_while(|b| b.is_ascii_digit())
                .count();
            let n: usize = std::str::from_utf8(&fmt[*at..*at + digits])
                .unwrap()
                .parse()
                .unwrap();
            if n <= 6 && fmt.get(*at + digits) == Some(&b'S') {
                *at += digits + 1;
                return Ok(seconds(data, f, n));
            }
            *at += 1;
            Ok(false)
        }
        (Some(c), _) => {
            *at += 1;
            if c == b'c' || c == b'X' {
                f.twelve_hour = false;
            }
            Ok(match c {
                b'c' | b'x' | b'X' => library(c, data, f),
                _ => false,
            })
        }
        (None, _) => Ok(false),
    }
}

/// The seconds and a fraction of at most `digits` digits after a `.`, all of them for 0.
fn seconds(data: &mut Input, f: &mut Fields, digits: usize) -> bool {
    let Some(second) = data.int(2, 0, 60) else {
        return false;
    };
    f.second = second;
    match data.fraction(digits) {
        Some(Some(micros)) => {
            f.micros = micros;
            true
        }
        Some(None) => true,
        None => false,
    }
}

/// `%O` and its forms, which read as the elements without it.
fn modifier(fmt: &[u8], at: &mut usize, data: &mut Input, f: &mut Fields, position: usize) -> bool {
    let Some(&letter) = fmt.get(*at) else {
        return false;
    };
    *at += 1;
    match letter {
        b'H' => {
            f.twelve_hour = false;
            library(b'H', data, f)
        }
        b'I' => twelve_hour(data, f),
        b'u' => date_element(data, f, b'u', position, 1, 1, 7),
        b'w' => date_element(data, f, b'w', position, 1, 0, 6),
        b'U' | b'W' => {
            f.non_iso = true;
            date_element(data, f, letter, position, 2, 0, 53)
        }
        b'V' => date_element(data, f, b'V', position, 2, 1, 53),
        b'd' | b'e' | b'm' | b'M' | b'S' | b'y' => library(letter, data, f),
        _ => false,
    }
}

/// An element that GoogleSQL reads with the C library's `strptime` in its C locale: the names,
/// the composites, and `%e`. Numbers skip leading whitespace.
fn library(letter: u8, data: &mut Input, f: &mut Fields) -> bool {
    let number = |data: &mut Input, width: usize, min: i64, max: i64| {
        data.skip_space();
        data.int(width, min, max)
    };
    let pattern = |data: &mut Input, f: &mut Fields, pattern: &[u8]| {
        pattern.iter().all(|&c| match c {
            b' ' => {
                data.skip_space();
                true
            }
            b'/' | b'-' | b':' => literal(data, c),
            letter => library(letter, data, f),
        })
    };
    match letter {
        b'B' | b'b' | b'h' => data.name(&MONTHS).map(|m| f.month = m as i64 + 1).is_some(),
        b'd' | b'e' => number(data, 2, 1, 31).map(|d| f.day = d).is_some(),
        b'm' => number(data, 2, 1, 12).map(|m| f.month = m).is_some(),
        b'H' | b'k' => number(data, 2, 0, 23).map(|h| f.hour = h).is_some(),
        b'M' => number(data, 2, 0, 59).map(|m| f.minute = m).is_some(),
        b'S' => number(data, 2, 0, 60).map(|s| f.second = s).is_some(),
        b'Y' => number(data, 4, 0, 9999).map(|y| f.year = y).is_some(),
        b'y' => number(data, 2, 0, 99)
            .map(|y| f.year = if y < 69 { 2000 + y } else { 1900 + y })
            .is_some(),
        b'a' => data.name(&DAYS).map(|d| f.weekday = d as i64).is_some(),
        b'c' => pattern(data, f, b"a b e H:M:S Y"),
        b'D' | b'x' => pattern(data, f, b"m/d/y"),
        b'F' => pattern(data, f, b"Y-m-d"),
        b'R' => pattern(data, f, b"H:M"),
        b'T' | b'X' => pattern(data, f, b"H:M:S"),
        b'%' => literal(data, b'%'),
        _ => false,
    }
}

/// The days of an ISO year: 371 or 364.
fn iso_year_days(year: i64) -> i64 {
    let p = |y: i64| (y + y / 4 - y / 100 + y / 400) % 7;
    let weeks = 52 + i64::from(p(year) == 4 || p(year - 1) == 3);
    weeks * 7
}

/// The year, month, and day after the elements of weeks, weekdays, and days of the year, as
/// GoogleSQL resolves them.
fn resolve_date(year: &mut i64, month: &mut i64, day: &mut i64, f: &Fields) -> Result<(), Error> {
    if f.elements.is_empty() {
        return Ok(());
    }
    let last = |letters: &[u8]| (f.elements.iter()).rposition(|e| letters.contains(&e.letter));
    let weekday = last(b"Aauw");
    let iso_year = last(b"Gg").filter(|_| !f.non_iso);
    let date = |y: i64, m: u32, d: u32| NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, m, d);
    let invalid_date = || Error("Out-of-range datetime field in parsing function".into());
    if let Some(iso_year) = iso_year {
        let year_element = f.elements[iso_year];
        let iso = match year_element.letter {
            b'g' if year_element.value <= 68 => 2000 + year_element.value,
            b'g' => 1900 + year_element.value,
            _ => year_element.value,
        };
        let mut week = last(b"V");
        let mut day_of_year = last(b"J");
        if let Some(j) = day_of_year {
            if week.is_none_or(|w| j > w) || weekday.is_none() {
                week = None;
            } else {
                day_of_year = None;
            }
        }
        if let Some(week) = week {
            if f.elements[week].value == 53 && iso_year_days(iso) < 371 {
                return Err(Error(format!(
                    "Invalid ISO week 53 specified for ISO year {iso}"
                )));
            }
            return Err(Error("ISO parse elements are not supported yet".into()));
        }
        let jan5 = date(iso, 1, 5).ok_or_else(invalid_date)?;
        let mut first = jan5 - Duration::days(1);
        while first.weekday() != Weekday::Mon {
            first -= Duration::days(1);
        }
        let mut result = first;
        if let Some(j) = day_of_year {
            let j = f.elements[j].value;
            if j > 364 && j > iso_year_days(iso) {
                return Err(Error(format!(
                    "ISO Year {iso} has {} days, but the specified day of year was {j}",
                    iso_year_days(iso)
                )));
            }
            result += Duration::days(j - 1);
        }
        *year = result.year().into();
        *month = result.month().into();
        *day = result.day().into();
        return Ok(());
    }
    let mut week = last(b"UW");
    let mut day_of_year = last(b"j");
    let mut weekday = weekday;
    if let Some(j) = day_of_year {
        if week.is_none_or(|w| j > w) || weekday.is_none() {
            weekday = None;
            week = None;
        } else {
            day_of_year = None;
        }
    } else if week.is_none() {
        weekday = None;
    }
    let (result, element) = if let Some(w) = week {
        let element = f.elements[w];
        let start = if element.letter == b'U' {
            Weekday::Sun
        } else {
            Weekday::Mon
        };
        let jan1 = date(*year, 1, 1).ok_or_else(invalid_date)?;
        let first_week = i64::from(jan1.weekday() == start);
        let leap = date(*year, 2, 29).is_some();
        let days = if leap { 366 } else { 365 };
        let jan1_number = jan1.weekday().num_days_from_sunday() as i64 + 1;
        let (first_day, jan1_number) = match element.letter {
            b'U' => (8, if jan1_number == 1 { 8 } else { jan1_number }),
            _ => (
                9,
                if jan1_number <= 2 {
                    jan1_number + 7
                } else {
                    jan1_number
                },
            ),
        };
        let last_week = if days - (first_day - jan1_number) > 364 {
            53
        } else {
            52
        };
        if element.value < first_week || element.value > last_week {
            return Err(Error(format!(
                "Week number {} is invalid for year {}",
                element.value, *year
            )));
        }
        let mut first = jan1;
        while first.weekday() != start {
            first += Duration::days(1);
        }
        if weekday.is_some() {
            return Err(Error("Weekday parse elements are not supported yet".into()));
        }
        (first + Duration::days(7 * (element.value - 1)), element)
    } else if let Some(j) = day_of_year {
        let element = f.elements[j];
        if element.value == 366 && date(*year, 2, 29).is_none() {
            return Err(Error(format!(
                "Year {} has 365 days, but the specified day of year was 366",
                *year
            )));
        }
        let jan1 = date(*year, 1, 1).ok_or_else(invalid_date)?;
        (jan1 + Duration::days(element.value - 1), element)
    } else {
        return Ok(());
    };
    if f.last_year.is_none_or(|p| p < element.position) {
        *year = result.year().into();
    }
    if f.last_month.is_none_or(|p| p < element.position) {
        *month = result.month().into();
    }
    if f.last_day.is_none_or(|p| p < element.position) {
        *day = result.day().into();
    }
    Ok(())
}
