use crate::unsupported;
use avenger_format::{DateTimeFormatError, DateTimeInputKind};
use icu_datetime::{
    fieldsets::{
        builder::{DateFields, FieldSetBuilder, ZoneStyle},
        enums::CompositeFieldSet,
    },
    options::{Length, SubsecondDigits, TimePrecision, YearStyle},
};
use icu_locale_core::preferences::extensions::unicode::keywords::HourCycle;
use std::collections::BTreeSet;

/// A validated request with literal text already unescaped.
pub(super) struct Spec {
    pub prefix: String,
    pub suffix: String,
    pub fields: CompositeFieldSet,
    pub hour_cycle: Option<HourCycle>,
}

impl Spec {
    pub fn parse(source: &str, input: DateTimeInputKind) -> Result<Self, DateTimeFormatError> {
        let mut parser = Parser {
            source,
            position: 0,
        };
        let prefix = parser.literal()?;
        let block_start = parser.position;
        parser.expect('{')?;
        let mut builder = FieldSetBuilder::new();
        let mut hour_cycle = None;
        let mut fraction = None;
        let mut seen = BTreeSet::new();
        loop {
            parser.whitespace();
            if parser.peek() == Some('}') {
                parser.advance();
                break;
            }
            let position = parser.position;
            let name = parser.token()?;
            if !seen.insert(name) {
                return Err(syntax(position, format!("duplicate option `{name}`")));
            }
            parser.whitespace();
            parser.expect('=')?;
            parser.whitespace();
            let value = parser.token()?;
            let invalid = || option(name, format!("unsupported value `{value}`"));
            match name {
                "dateFields" => {
                    builder.date_fields = Some(match value {
                        "weekday" => DateFields::E,
                        "day-weekday" => DateFields::DE,
                        "month-day" => DateFields::MD,
                        "month-day-weekday" => DateFields::MDE,
                        "year-month-day" => DateFields::YMD,
                        "year-month-day-weekday" => DateFields::YMDE,
                        "day" => DateFields::D,
                        "month" => DateFields::M,
                        "year-month" => DateFields::YM,
                        "year" => DateFields::Y,
                        _ => return Err(invalid()),
                    })
                }
                "dateLength" => {
                    builder.length = Some(match value {
                        "short" => Length::Short,
                        "medium" => Length::Medium,
                        "long" => Length::Long,
                        _ => return Err(invalid()),
                    })
                }
                "timePrecision" => {
                    builder.time_precision = Some(match value {
                        "hour" => TimePrecision::Hour,
                        "minute" => TimePrecision::Minute,
                        "second" => TimePrecision::Second,
                        _ => return Err(invalid()),
                    })
                }
                "hour12" => {
                    hour_cycle = Some(match value {
                        "true" => HourCycle::H12,
                        "false" => HourCycle::H23,
                        _ => return Err(invalid()),
                    })
                }
                "fractionalSecondDigits" => {
                    fraction = Some(match value.as_bytes() {
                        [digit @ b'1'..=b'9'] => {
                            SubsecondDigits::try_from_int(digit - b'0').ok_or_else(invalid)?
                        }
                        _ => return Err(invalid()),
                    });
                }
                "timeZoneStyle" => {
                    builder.zone_style = Some(match value {
                        "short" => ZoneStyle::SpecificShort,
                        "long" => ZoneStyle::SpecificLong,
                        "shortGeneric" => ZoneStyle::GenericShort,
                        "longGeneric" => ZoneStyle::GenericLong,
                        "shortOffset" => ZoneStyle::LocalizedOffsetShort,
                        "longOffset" => ZoneStyle::LocalizedOffsetLong,
                        _ => return Err(invalid()),
                    })
                }
                _ => return Err(option(name, "unknown option")),
            }
            if !matches!(parser.peek(), Some('}'))
                && !parser.peek().is_some_and(|c| c.is_ascii_whitespace())
            {
                return Err(syntax(parser.position, "expected whitespace or `}`"));
            }
        }
        let suffix = parser.literal()?;
        if parser.peek().is_some() {
            return Err(syntax(
                parser.position,
                "only one formatting block is allowed",
            ));
        }
        if seen.is_empty() {
            return Err(syntax(block_start, "formatting block cannot be empty"));
        }
        if builder.length.is_some() && builder.date_fields.is_none() {
            return Err(option("dateLength", "requires dateFields"));
        }
        for (name, present) in [
            ("hour12", hour_cycle.is_some()),
            ("timeZoneStyle", builder.zone_style.is_some()),
        ] {
            if present && builder.time_precision.is_none() {
                return Err(option(name, "requires timePrecision"));
            }
        }
        if let Some(fraction) = fraction {
            if builder.time_precision != Some(TimePrecision::Second) {
                return Err(option(
                    "fractionalSecondDigits",
                    "requires timePrecision=second",
                ));
            }
            builder.time_precision = Some(TimePrecision::Subsecond(fraction));
        }
        if input == DateTimeInputKind::Date && builder.time_precision.is_some() {
            return Err(unsupported(input, "time options require a datetime".into()));
        }
        if input != DateTimeInputKind::Zoned && builder.zone_style.is_some() {
            return Err(unsupported(
                input,
                "timeZoneStyle requires a zoned datetime".into(),
            ));
        }
        if matches!(
            builder.date_fields,
            Some(DateFields::Y | DateFields::YM | DateFields::YMD | DateFields::YMDE)
        ) {
            builder.year_style = Some(YearStyle::Full);
        }
        // ICU validates combinations such as calendar periods with a time component.
        let fields = builder
            .build_composite()
            .map_err(|error| unsupported(input, error.to_string()))?;
        Ok(Self {
            prefix,
            suffix,
            fields,
            hour_cycle,
        })
    }
}

/// Track byte positions while scanning Unicode literals and ASCII options.
struct Parser<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(ch) = self.peek() {
            self.position += ch.len_utf8();
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), DateTimeFormatError> {
        if self.peek() != Some(expected) {
            return Err(syntax(self.position, format!("expected `{expected}`")));
        }
        self.advance();
        Ok(())
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
            self.advance();
        }
    }

    fn token(&mut self) -> Result<&'a str, DateTimeFormatError> {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            self.advance();
        }
        if self.position == start {
            return Err(syntax(start, "expected an option name or value"));
        }
        Ok(&self.source[start..self.position])
    }

    /// Stop before an unescaped opening brace, preserving all literal whitespace.
    fn literal(&mut self) -> Result<String, DateTimeFormatError> {
        let mut literal = String::new();
        while let Some(ch) = self.peek() {
            match ch {
                '{' => break,
                '}' => return Err(syntax(self.position, "unmatched `}`")),
                '\\' => {
                    let position = self.position;
                    self.advance();
                    match self.peek() {
                        Some(escaped @ ('{' | '}' | '\\')) => literal.push(escaped),
                        _ => {
                            return Err(syntax(position, "expected an escaped brace or backslash"))
                        }
                    }
                    self.advance();
                }
                _ => {
                    literal.push(ch);
                    self.advance();
                }
            }
        }
        Ok(literal)
    }
}

fn syntax(position: usize, message: impl Into<String>) -> DateTimeFormatError {
    DateTimeFormatError::InvalidPattern {
        message: message.into(),
        position: Some(position),
    }
}

fn option(name: &str, message: impl Into<String>) -> DateTimeFormatError {
    DateTimeFormatError::InvalidOption {
        option: name.into(),
        message: message.into(),
    }
}
