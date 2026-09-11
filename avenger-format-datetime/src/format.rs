use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, Offset, Timelike, Utc};
use chrono_tz::Tz;

use crate::{
    error::DateTimeFormatError,
    fields::{DateTimeField, FieldToken, Pattern, PatternToken, StyleBlock, StyleKind},
    locale::{Lengths, LocaleWeekday, ResolvedDateTimeLocale, Widths12, Widths2, Widths4, Widths7},
    parser::parse_datetime_spec,
    registry::DateTimeLocaleRegistry,
    style::{render_datetime_glue, DateTimeStyleLength},
    timezone::parse_datetime_timezone,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NaiveDateTimeInput {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

pub type ZonedDateTimeInput = DateTime<Utc>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedDateTime {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeFormatOverrides {
    pub timezone: Option<String>,
    pub date_style: Option<DateTimeStyleLength>,
    pub time_style: Option<DateTimeStyleLength>,
    pub datetime_style: Option<DateTimeStyleLength>,
}

#[derive(Debug, Clone, Copy)]
pub struct DateTimeFormatContext<'a> {
    pub locale: &'a ResolvedDateTimeLocale,
    pub timezone: Tz,
    pub registry: Option<&'a DateTimeLocaleRegistry>,
}

impl<'a> DateTimeFormatContext<'a> {
    pub fn new(locale: &'a ResolvedDateTimeLocale, timezone: Tz) -> Self {
        Self {
            locale,
            timezone,
            registry: None,
        }
    }

    pub fn with_registry(mut self, registry: &'a DateTimeLocaleRegistry) -> Self {
        self.registry = Some(registry);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeFields {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub ordinal: u32,
    pub weekday: chrono::Weekday,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanosecond: u32,
    pub offset: Option<FixedOffset>,
}

pub fn format_naive_datetime(
    value: NaiveDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError> {
    if overrides.timezone.is_some() {
        return Err(DateTimeFormatError::TimezoneOverrideForNaive);
    }
    let naive = match value {
        NaiveDateTimeInput::Date(date) => date.and_hms_opt(0, 0, 0).ok_or_else(|| {
            DateTimeFormatError::InvalidFormat("date could not be lowered to midnight".to_string())
        })?,
        NaiveDateTimeInput::DateTime(value) => value,
    };
    let fields = fields_from_naive(naive);
    let pattern = parse_datetime_spec(spec.unwrap_or("{datetime}"))?;
    if let Some(field) = pattern.has_timezone_field() {
        return Err(DateTimeFormatError::TimezoneFieldForNaive(
            field.to_string(),
        ));
    }
    Ok(FormattedDateTime {
        text: render_pattern(&pattern, &fields, &overrides, context)?,
    })
}

pub fn format_zoned_datetime(
    value: ZonedDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError> {
    let timezone = if let Some(timezone) = overrides.timezone.as_deref() {
        parse_datetime_timezone(timezone)?
    } else {
        context.timezone
    };
    let display = value.with_timezone(&timezone);
    let fields = DateTimeFields {
        year: display.year(),
        month: display.month(),
        day: display.day(),
        ordinal: display.ordinal(),
        weekday: display.weekday(),
        hour: display.hour(),
        minute: display.minute(),
        second: display.second(),
        nanosecond: display.nanosecond(),
        offset: Some(display.offset().fix()),
    };
    let pattern = parse_datetime_spec(spec.unwrap_or("{datetime}"))?;
    Ok(FormattedDateTime {
        text: render_pattern(&pattern, &fields, &overrides, context)?,
    })
}

fn fields_from_naive(value: NaiveDateTime) -> DateTimeFields {
    DateTimeFields {
        year: value.year(),
        month: value.month(),
        day: value.day(),
        ordinal: value.ordinal(),
        weekday: value.weekday(),
        hour: value.hour(),
        minute: value.minute(),
        second: value.second(),
        nanosecond: value.nanosecond(),
        offset: None,
    }
}

fn render_pattern(
    pattern: &Pattern,
    fields: &DateTimeFields,
    overrides: &DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<String, DateTimeFormatError> {
    let mut out = String::new();
    for token in &pattern.tokens {
        match token {
            PatternToken::Literal(value) => out.push_str(value),
            PatternToken::Field(field) => {
                out.push_str(&render_field(field, fields, context.locale)?);
            }
            PatternToken::Style(style) => {
                out.push_str(&render_style_block(style, fields, overrides, context)?);
            }
        }
    }
    Ok(out)
}

fn render_style_block(
    style: &StyleBlock,
    fields: &DateTimeFields,
    overrides: &DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<String, DateTimeFormatError> {
    let length = match style.kind {
        StyleKind::Date => style.length.or(overrides.date_style),
        StyleKind::Time => style.length.or(overrides.time_style),
        StyleKind::DateTime => style.length.or(overrides.datetime_style),
    }
    .unwrap_or_default();

    match style.kind {
        StyleKind::Date => render_locale_pattern(
            pattern_for_length(&context.locale.date_patterns, length),
            fields,
            overrides,
            context,
        ),
        StyleKind::Time => render_locale_pattern(
            pattern_for_length(&context.locale.time_patterns, length),
            fields,
            overrides,
            context,
        ),
        StyleKind::DateTime => {
            let date = render_locale_pattern(
                pattern_for_length(&context.locale.date_patterns, length),
                fields,
                overrides,
                context,
            )?;
            let time = render_locale_pattern(
                pattern_for_length(&context.locale.time_patterns, length),
                fields,
                overrides,
                context,
            )?;
            render_datetime_glue(
                pattern_for_length(&context.locale.datetime_glue, length),
                &date,
                &time,
            )
        }
    }
}

fn render_locale_pattern(
    spec: &str,
    fields: &DateTimeFields,
    overrides: &DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<String, DateTimeFormatError> {
    let pattern = parse_datetime_spec(spec)?;
    if fields.offset.is_none() {
        if let Some(field) = pattern.has_timezone_field() {
            return Err(DateTimeFormatError::TimezoneFieldForNaive(
                field.to_string(),
            ));
        }
    }
    render_pattern(&pattern, fields, overrides, context)
}

fn pattern_for_length(lengths: &Lengths, length: DateTimeStyleLength) -> &str {
    match length {
        DateTimeStyleLength::Short => &lengths.short,
        DateTimeStyleLength::Medium => &lengths.medium,
        DateTimeStyleLength::Long => &lengths.long,
        DateTimeStyleLength::Full => &lengths.full,
    }
}

fn render_field(
    token: &FieldToken,
    fields: &DateTimeFields,
    locale: &ResolvedDateTimeLocale,
) -> Result<String, DateTimeFormatError> {
    let width = token.width;
    let text = match token.field {
        DateTimeField::Era => {
            let era = if fields.year <= 0 { 0 } else { 1 };
            width_name2(&locale.eras, era, width)
        }
        DateTimeField::Year => {
            let year = if fields.year <= 0 {
                1 - fields.year
            } else {
                fields.year
            };
            if width == 2 {
                pad_number((year % 100) as u32, 2)
            } else if width > 1 {
                pad_signed_year(year, width)
            } else {
                year.to_string()
            }
        }
        DateTimeField::MonthFormat => render_month(&locale.months, fields.month, width),
        DateTimeField::MonthStandalone => {
            render_month(&locale.months_standalone, fields.month, width)
        }
        DateTimeField::DayOfMonth => pad_or_plain(fields.day, width),
        DateTimeField::DayOfYear => pad_or_plain(fields.ordinal, width),
        DateTimeField::WeekdayFormat => {
            render_weekday_name(&locale.weekdays, fields.weekday, width)
        }
        DateTimeField::WeekdayLocal => {
            if width <= 2 {
                let day = locale_weekday_number(fields.weekday, locale.first_day_of_week);
                if width == 2 {
                    pad_number(day, 2)
                } else {
                    day.to_string()
                }
            } else {
                render_weekday_name(&locale.weekdays, fields.weekday, width)
            }
        }
        DateTimeField::WeekdayStandalone => {
            if width <= 2 {
                locale_weekday_number(fields.weekday, locale.first_day_of_week).to_string()
            } else {
                render_weekday_name(&locale.weekdays_standalone, fields.weekday, width)
            }
        }
        DateTimeField::QuarterFormat => render_quarter(&locale.quarters, fields.month, width),
        DateTimeField::QuarterStandalone => {
            render_quarter(&locale.quarters_standalone, fields.month, width)
        }
        DateTimeField::DayPeriod => {
            if fields.hour < 12 {
                locale.day_periods.am.clone()
            } else {
                locale.day_periods.pm.clone()
            }
        }
        DateTimeField::Hour12OneBased => {
            let hour = fields.hour % 12;
            pad_or_plain(if hour == 0 { 12 } else { hour }, width)
        }
        DateTimeField::Hour24ZeroBased => pad_or_plain(fields.hour, width),
        DateTimeField::Hour12ZeroBased => pad_or_plain(fields.hour % 12, width),
        DateTimeField::Hour24OneBased => {
            pad_or_plain(if fields.hour == 0 { 24 } else { fields.hour }, width)
        }
        DateTimeField::Minute => pad_or_plain(fields.minute, width),
        DateTimeField::Second => pad_or_plain(fields.second, width),
        DateTimeField::FractionalSecond => render_fractional_second(fields.nanosecond, width),
        DateTimeField::IsoTimezoneZ
        | DateTimeField::IsoTimezone
        | DateTimeField::RfcTimezone
        | DateTimeField::GmtTimezone => {
            let offset = fields.offset.ok_or_else(|| {
                DateTimeFormatError::TimezoneFieldForNaive(token.field.as_char().to_string())
            })?;
            render_timezone_offset(token.field, offset.local_minus_utc(), width)
        }
    };
    Ok(apply_digits(&text, locale))
}

fn render_month(widths: &Widths12, month: u32, width: usize) -> String {
    let index = month.saturating_sub(1) as usize;
    if width <= 2 {
        if width == 2 {
            pad_number(month, 2)
        } else {
            month.to_string()
        }
    } else if width == 3 {
        widths.abbrev[index].clone()
    } else if width == 4 {
        widths.wide[index].clone()
    } else {
        widths.narrow[index].clone()
    }
}

fn render_weekday_name(widths: &Widths7, weekday: chrono::Weekday, width: usize) -> String {
    let index = weekday.num_days_from_sunday() as usize;
    if width == 4 {
        widths.wide[index].clone()
    } else if width >= 5 {
        widths.narrow[index].clone()
    } else {
        widths.abbrev[index].clone()
    }
}

fn render_quarter(widths: &Widths4, month: u32, width: usize) -> String {
    let quarter = ((month - 1) / 3) + 1;
    let index = (quarter - 1) as usize;
    if width <= 2 {
        if width == 2 {
            pad_number(quarter, 2)
        } else {
            quarter.to_string()
        }
    } else if width == 3 {
        widths.abbrev[index].clone()
    } else if width == 4 {
        widths.wide[index].clone()
    } else {
        widths.narrow[index].clone()
    }
}

fn width_name2(widths: &Widths2, index: usize, width: usize) -> String {
    if width == 4 {
        widths.wide[index].clone()
    } else if width >= 5 {
        widths.narrow[index].clone()
    } else {
        widths.abbrev[index].clone()
    }
}

fn locale_weekday_number(weekday: chrono::Weekday, first_day: LocaleWeekday) -> u32 {
    let day = weekday.num_days_from_sunday();
    let first = first_day.ordinal_from_sunday();
    ((day + 7 - first) % 7) + 1
}

fn pad_or_plain(value: u32, width: usize) -> String {
    if width > 1 {
        pad_number(value, width)
    } else {
        value.to_string()
    }
}

fn pad_number(value: u32, width: usize) -> String {
    format!("{value:0width$}")
}

fn pad_signed_year(value: i32, width: usize) -> String {
    if value < 0 {
        format!("-{:0width$}", value.abs(), width = width)
    } else {
        format!("{value:0width$}")
    }
}

fn render_fractional_second(nanos: u32, width: usize) -> String {
    let digits = format!("{nanos:09}");
    if width <= 9 {
        digits[..width].to_string()
    } else {
        format!("{}{}", digits, "0".repeat(width - 9))
    }
}

fn render_timezone_offset(field: DateTimeField, seconds: i32, width: usize) -> String {
    let zero = seconds == 0;
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.abs();
    let hours = abs / 3600;
    let minutes = (abs % 3600) / 60;
    let secs = abs % 60;

    match field {
        DateTimeField::IsoTimezoneZ => {
            if zero {
                "Z".to_string()
            } else {
                render_iso_offset(sign, hours, minutes, secs, width)
            }
        }
        DateTimeField::IsoTimezone => render_iso_offset(sign, hours, minutes, secs, width),
        DateTimeField::RfcTimezone => {
            if width >= 4 {
                format!("GMT{sign}{hours:02}:{minutes:02}")
            } else {
                format!("{sign}{hours:02}{minutes:02}")
            }
        }
        DateTimeField::GmtTimezone => {
            if zero {
                "GMT".to_string()
            } else if width == 1 {
                if minutes == 0 {
                    format!("GMT{sign}{hours}")
                } else {
                    format!("GMT{sign}{hours}:{minutes:02}")
                }
            } else {
                format!("GMT{sign}{hours:02}:{minutes:02}")
            }
        }
        _ => unreachable!("non-timezone field"),
    }
}

fn render_iso_offset(sign: char, hours: i32, minutes: i32, seconds: i32, width: usize) -> String {
    match width {
        1 => {
            if minutes == 0 {
                format!("{sign}{hours:02}")
            } else {
                format!("{sign}{hours:02}{minutes:02}")
            }
        }
        2 => format!("{sign}{hours:02}{minutes:02}"),
        3 => format!("{sign}{hours:02}:{minutes:02}"),
        4 => {
            if seconds == 0 {
                format!("{sign}{hours:02}{minutes:02}")
            } else {
                format!("{sign}{hours:02}{minutes:02}{seconds:02}")
            }
        }
        _ => {
            if seconds == 0 {
                format!("{sign}{hours:02}:{minutes:02}")
            } else {
                format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
            }
        }
    }
}

fn apply_digits(value: &str, locale: &ResolvedDateTimeLocale) -> String {
    let Some(digits) = &locale.digits else {
        return value.to_string();
    };
    let mut out = String::new();
    for ch in value.chars() {
        if let Some(index) = ch.to_digit(10) {
            out.push_str(&digits[index as usize]);
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        locale::{DateTimeLocaleSpec, LengthsSpec},
        registry::DateTimeLocaleRegistry,
    };
    use chrono::TimeZone;

    fn en_us() -> ResolvedDateTimeLocale {
        DateTimeLocaleRegistry::with_builtins()
            .resolve("en-US")
            .unwrap()
    }

    #[test]
    fn formats_basic_ldml_fields() {
        let locale = en_us();
        let ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let value = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
        let formatted = format_naive_datetime(
            NaiveDateTimeInput::Date(value),
            Some("G y yy MMMM MMM MM M d D EEEE E a h H K k m s SSS"),
            DateTimeFormatOverrides::default(),
            ctx,
        )
        .unwrap();
        assert_eq!(
            formatted.text,
            "AD 2024 24 January Jan 01 1 5 5 Friday Fri AM 12 0 0 24 0 0 000"
        );
    }

    #[test]
    fn formats_style_blocks_and_glue() {
        let locale = en_us();
        let ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let value = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
        assert_eq!(
            format_naive_datetime(
                NaiveDateTimeInput::Date(value),
                Some("{date:long}"),
                DateTimeFormatOverrides::default(),
                ctx,
            )
            .unwrap()
            .text,
            "January 5, 2024"
        );
        assert_eq!(
            format_naive_datetime(
                NaiveDateTimeInput::Date(value),
                Some("{datetime:medium}"),
                DateTimeFormatOverrides::default(),
                ctx,
            )
            .unwrap()
            .text,
            "Jan 5, 2024, 12:00:00 AM"
        );
    }

    #[test]
    fn formats_every_builtin_style_length_for_zoned_values() {
        let locale = en_us();
        let ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let value = Utc.with_ymd_and_hms(2024, 1, 5, 12, 34, 56).unwrap();

        let cases = [
            ("{date:short}", "1/5/24"),
            ("{date:medium}", "Jan 5, 2024"),
            ("{date:long}", "January 5, 2024"),
            ("{date:full}", "Friday, January 5, 2024"),
            ("{time:short}", "12:34 PM"),
            ("{time:medium}", "12:34:56 PM"),
            ("{time:long}", "12:34:56 PM GMT"),
            ("{time:full}", "12:34:56 PM GMT"),
            ("{datetime:short}", "1/5/24, 12:34 PM"),
            ("{datetime:medium}", "Jan 5, 2024, 12:34:56 PM"),
            ("{datetime:long}", "January 5, 2024 at 12:34:56 PM GMT"),
            (
                "{datetime:full}",
                "Friday, January 5, 2024 at 12:34:56 PM GMT",
            ),
        ];

        for (spec, expected) in cases {
            let formatted =
                format_zoned_datetime(value, Some(spec), DateTimeFormatOverrides::default(), ctx)
                    .unwrap();
            assert_eq!(formatted.text, expected, "{spec}");
        }
    }

    #[test]
    fn preserves_naive_and_zoned_split() {
        let locale = en_us();
        let utc_ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let ny_ctx = DateTimeFormatContext::new(&locale, Tz::America__New_York);
        let naive = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(
            format_naive_datetime(
                NaiveDateTimeInput::DateTime(naive),
                Some("y-MM-dd HH:mm"),
                DateTimeFormatOverrides::default(),
                ny_ctx,
            )
            .unwrap()
            .text,
            "2024-01-01 00:00"
        );
        let instant = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            format_zoned_datetime(
                instant,
                Some("y-MM-dd HH:mm XXX"),
                DateTimeFormatOverrides::default(),
                utc_ctx,
            )
            .unwrap()
            .text,
            "2024-01-01 00:00 Z"
        );
        assert_eq!(
            format_zoned_datetime(
                instant,
                Some("y-MM-dd HH:mm XXX"),
                DateTimeFormatOverrides::default(),
                ny_ctx,
            )
            .unwrap()
            .text,
            "2023-12-31 19:00 -05:00"
        );
    }

    #[test]
    fn rejects_timezone_fields_and_overrides_for_naive() {
        let locale = en_us();
        let ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let value = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert!(format_naive_datetime(
            NaiveDateTimeInput::Date(value),
            Some("XXX"),
            DateTimeFormatOverrides::default(),
            ctx,
        )
        .is_err());
        assert!(format_naive_datetime(
            NaiveDateTimeInput::Date(value),
            Some("y"),
            DateTimeFormatOverrides {
                timezone: Some("UTC".to_string()),
                ..Default::default()
            },
            ctx,
        )
        .is_err());
    }

    #[test]
    fn applies_digit_substitution_to_numeric_tokens_only() {
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        registry
            .register_custom_locale(
                "digits",
                DateTimeLocaleSpec {
                    digits: Some(
                        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
                            .map(|value| format!("[{value}]")),
                    ),
                    date_patterns: Some(LengthsSpec {
                        long: Some("'year' y MMMM d".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        let locale = registry.resolve("digits").unwrap();
        let ctx = DateTimeFormatContext::new(&locale, Tz::UTC);
        let value = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
        assert_eq!(
            format_naive_datetime(
                NaiveDateTimeInput::Date(value),
                Some("{date:long}"),
                DateTimeFormatOverrides::default(),
                ctx,
            )
            .unwrap()
            .text,
            "year [2][0][2][4] January [5]"
        );
    }
}
