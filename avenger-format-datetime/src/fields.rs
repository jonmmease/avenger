use crate::style::DateTimeStyleLength;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeField {
    Era,
    Year,
    MonthFormat,
    MonthStandalone,
    DayOfMonth,
    DayOfYear,
    WeekdayFormat,
    WeekdayLocal,
    WeekdayStandalone,
    QuarterFormat,
    QuarterStandalone,
    DayPeriod,
    Hour12OneBased,
    Hour24ZeroBased,
    Hour12ZeroBased,
    Hour24OneBased,
    Minute,
    Second,
    FractionalSecond,
    IsoTimezoneZ,
    IsoTimezone,
    RfcTimezone,
    GmtTimezone,
}

impl DateTimeField {
    pub(crate) fn from_char(value: char) -> Option<Self> {
        match value {
            'G' => Some(Self::Era),
            'y' => Some(Self::Year),
            'M' => Some(Self::MonthFormat),
            'L' => Some(Self::MonthStandalone),
            'd' => Some(Self::DayOfMonth),
            'D' => Some(Self::DayOfYear),
            'E' => Some(Self::WeekdayFormat),
            'e' => Some(Self::WeekdayLocal),
            'c' => Some(Self::WeekdayStandalone),
            'q' => Some(Self::QuarterFormat),
            'Q' => Some(Self::QuarterStandalone),
            'a' => Some(Self::DayPeriod),
            'h' => Some(Self::Hour12OneBased),
            'H' => Some(Self::Hour24ZeroBased),
            'K' => Some(Self::Hour12ZeroBased),
            'k' => Some(Self::Hour24OneBased),
            'm' => Some(Self::Minute),
            's' => Some(Self::Second),
            'S' => Some(Self::FractionalSecond),
            'X' => Some(Self::IsoTimezoneZ),
            'x' => Some(Self::IsoTimezone),
            'Z' => Some(Self::RfcTimezone),
            'O' => Some(Self::GmtTimezone),
            _ => None,
        }
    }

    pub(crate) fn requires_timezone(self) -> bool {
        matches!(
            self,
            Self::IsoTimezoneZ | Self::IsoTimezone | Self::RfcTimezone | Self::GmtTimezone
        )
    }

    pub(crate) fn as_char(self) -> char {
        match self {
            Self::Era => 'G',
            Self::Year => 'y',
            Self::MonthFormat => 'M',
            Self::MonthStandalone => 'L',
            Self::DayOfMonth => 'd',
            Self::DayOfYear => 'D',
            Self::WeekdayFormat => 'E',
            Self::WeekdayLocal => 'e',
            Self::WeekdayStandalone => 'c',
            Self::QuarterFormat => 'q',
            Self::QuarterStandalone => 'Q',
            Self::DayPeriod => 'a',
            Self::Hour12OneBased => 'h',
            Self::Hour24ZeroBased => 'H',
            Self::Hour12ZeroBased => 'K',
            Self::Hour24OneBased => 'k',
            Self::Minute => 'm',
            Self::Second => 's',
            Self::FractionalSecond => 'S',
            Self::IsoTimezoneZ => 'X',
            Self::IsoTimezone => 'x',
            Self::RfcTimezone => 'Z',
            Self::GmtTimezone => 'O',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldToken {
    pub field: DateTimeField,
    pub width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    Date,
    Time,
    DateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleBlock {
    pub kind: StyleKind,
    pub length: Option<DateTimeStyleLength>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternToken {
    Literal(String),
    Field(FieldToken),
    Style(StyleBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub tokens: Vec<PatternToken>,
}

impl Pattern {
    pub(crate) fn has_timezone_field(&self) -> Option<char> {
        self.tokens.iter().find_map(|token| match token {
            PatternToken::Field(token) if token.field.requires_timezone() => {
                Some(token.field.as_char())
            }
            _ => None,
        })
    }
}
