use crate::{
    fields::{Pattern, PatternToken},
    locale::expand,
    parse_datetime_spec, parse_datetime_timezone, DateTimeFormatError, ResolvedDateTimeLocale,
};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, Timelike, Utc};
use chrono_tz::Tz;

/// Civil values have calendar fields without an instant or display offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NaiveDateTimeInput {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}
impl NaiveDateTimeInput {
    pub(crate) fn datetime(self) -> NaiveDateTime {
        match self {
            Self::Date(date) => date.and_time(chrono::NaiveTime::MIN),
            Self::DateTime(value) => value,
        }
    }
}
/// An instant whose display zone is selected by the formatter.
pub type ZonedDateTimeInput = DateTime<Utc>;
/// Plain datetime label text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedDateTime {
    pub text: String,
}
/// An explicit display-zone override. Civil inputs cannot use this option.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeFormatOverrides {
    pub timezone: Option<String>,
}
/// Locale and concrete timezone resolved by the caller.
#[derive(Debug, Clone, Copy)]
pub struct DateTimeFormatContext<'a> {
    pub locale: &'a ResolvedDateTimeLocale,
    pub timezone: Tz,
}
impl<'a> DateTimeFormatContext<'a> {
    /// Use a resolved locale and display zone.
    pub fn new(locale: &'a ResolvedDateTimeLocale, timezone: Tz) -> Self {
        Self { locale, timezone }
    }
}

/// A parsed pattern and resolved locale reusable across values.
#[derive(Debug, Clone)]
pub struct PreparedDateTimeFormat {
    pattern: Pattern,
    locale: ResolvedDateTimeLocale,
    timezone: Tz,
    instant_directive: Option<char>,
    timezone_override: bool,
}
impl PreparedDateTimeFormat {
    /// Prepare a D3 pattern. An omitted scalar pattern uses locale `%c`.
    pub fn new(
        spec: Option<&str>,
        overrides: DateTimeFormatOverrides,
        context: DateTimeFormatContext<'_>,
    ) -> Result<Self, DateTimeFormatError> {
        let pattern = expand(
            &parse_datetime_spec(spec.unwrap_or("%c"))?,
            &context.locale.patterns,
            &mut Vec::new(),
        )?;
        let instant_directive = pattern.0.iter().find_map(|token| match token {
            PatternToken::Directive {
                code: code @ ('Q' | 's' | 'Z'),
                ..
            } => Some(*code),
            _ => None,
        });
        let timezone = overrides
            .timezone
            .as_deref()
            .map(parse_datetime_timezone)
            .transpose()?
            .unwrap_or(context.timezone);
        Ok(Self {
            pattern,
            locale: context.locale.clone(),
            timezone,
            instant_directive,
            timezone_override: overrides.timezone.is_some(),
        })
    }
    /// Check civil-input compatibility before formatting a batch.
    pub fn validate_naive(&self) -> Result<(), DateTimeFormatError> {
        if self.timezone_override {
            return Err(DateTimeFormatError::TimezoneOverrideForNaive);
        }
        if let Some(code) = self.instant_directive {
            return Err(DateTimeFormatError::TimezoneFieldForNaive(format!(
                "%{code}"
            )));
        }
        Ok(())
    }
    /// Format civil calendar fields, rejecting directives that require an instant.
    pub fn format_naive(
        &self,
        value: NaiveDateTimeInput,
    ) -> Result<FormattedDateTime, DateTimeFormatError> {
        self.validate_naive()?;
        Ok(self.render(value.datetime(), 0, 0))
    }
    /// Format an instant without changing its epoch identity.
    pub fn format_zoned(&self, value: ZonedDateTimeInput) -> FormattedDateTime {
        let value = normalize_instant(value);
        let millis = value.timestamp_millis();
        let display = value.with_timezone(&self.timezone);
        self.render(
            display.naive_local(),
            millis,
            display.offset().fix().local_minus_utc() / 60,
        )
    }
    fn render(&self, date: NaiveDateTime, millis: i64, offset_minutes: i32) -> FormattedDateTime {
        let locale = &self.locale.definition;
        let mut text = String::new();
        for token in &self.pattern.0 {
            let PatternToken::Directive { code, padding } = token else {
                if let PatternToken::Literal(value) = token {
                    text.push_str(value);
                }
                continue;
            };
            let numeric: (i64, usize) = match code {
                'a' => {
                    text.push_str(
                        &locale.short_days[date.weekday().num_days_from_sunday() as usize],
                    );
                    continue;
                }
                'A' => {
                    text.push_str(&locale.days[date.weekday().num_days_from_sunday() as usize]);
                    continue;
                }
                'b' => {
                    text.push_str(&locale.short_months[date.month0() as usize]);
                    continue;
                }
                'B' => {
                    text.push_str(&locale.months[date.month0() as usize]);
                    continue;
                }
                'p' => {
                    text.push_str(&locale.periods[usize::from(date.hour() >= 12)]);
                    continue;
                }
                '%' => {
                    text.push('%');
                    continue;
                }
                'Z' => {
                    text.push_str(&format!(
                        "{}{:02}{:02}",
                        if offset_minutes < 0 { '-' } else { '+' },
                        offset_minutes.abs() / 60,
                        offset_minutes.abs() % 60
                    ));
                    continue;
                }
                'Q' => (millis, 0),
                's' => (millis.div_euclid(1000), 0),
                'd' | 'e' => (date.day() as i64, 2),
                'H' => (date.hour() as i64, 2),
                'I' => (((date.hour() + 11) % 12 + 1) as i64, 2),
                'j' => (date.ordinal() as i64, 3),
                'L' | 'f' => ((date.nanosecond() / 1_000_000) as i64, 3),
                'm' => (date.month() as i64, 2),
                'M' => (date.minute() as i64, 2),
                'q' => ((date.month0() / 3 + 1) as i64, 0),
                'S' => (date.second() as i64, 2),
                'u' => (date.weekday().number_from_monday() as i64, 0),
                'w' => (date.weekday().num_days_from_sunday() as i64, 0),
                'U' => (
                    ((date.ordinal0() + 7 - date.weekday().num_days_from_sunday()) / 7) as i64,
                    2,
                ),
                'W' => (
                    ((date.ordinal0() + 7 - date.weekday().num_days_from_monday()) / 7) as i64,
                    2,
                ),
                'V' => (date.iso_week().week() as i64, 2),
                'y' => ((date.year() % 100) as i64, 2),
                'Y' => ((date.year() % 10000) as i64, 4),
                'g' => ((date.iso_week().year() % 100) as i64, 2),
                'G' => ((date.iso_week().year() % 10000) as i64, 4),
                _ => unreachable!("locale directives expand during preparation"),
            };
            let digits = numeric.0.unsigned_abs().to_string();
            if numeric.0 < 0 {
                text.push('-');
            }
            if let Some(fill) = padding {
                text.extend(std::iter::repeat_n(
                    *fill,
                    numeric.1.saturating_sub(digits.len()),
                ));
            }
            text.push_str(&digits);
            if *code == 'f' {
                text.push_str("000");
            }
        }
        FormattedDateTime { text }
    }
}

/// Format a civil value with a D3 pattern.
pub fn format_naive_datetime(
    value: NaiveDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError> {
    PreparedDateTimeFormat::new(spec, overrides, context)?.format_naive(value)
}
/// Format an instant with a D3 pattern and an explicit display zone.
pub fn format_zoned_datetime(
    value: ZonedDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError> {
    Ok(PreparedDateTimeFormat::new(spec, overrides, context)?.format_zoned(value))
}

// JavaScript Date clips fractional epoch milliseconds toward zero.
pub(crate) fn normalize_instant(value: ZonedDateTimeInput) -> ZonedDateTimeInput {
    let millis = value.timestamp_millis()
        + i64::from(
            value.timestamp() < 0 && !value.timestamp_subsec_nanos().is_multiple_of(1_000_000),
        );
    DateTime::from_timestamp_millis(millis)
        .expect("millisecond truncation preserves the datetime range")
}
