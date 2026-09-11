//! Reusable LDML datetime formatting for Avenger.
//!
//! This crate owns Avenger's datetime format-string parser, locale registry,
//! scalar rendering, and timezone validation. It deliberately has no dependency
//! on chart, guide, scenegraph, scale, or Typst crates.

pub mod error;
pub mod fields;
pub mod format;
pub mod locale;
pub mod parser;
pub mod registry;
pub mod style;
pub mod timezone;

pub use error::{DateTimeFormatError, DateTimeParseError};
pub use fields::{DateTimeField, FieldToken, Pattern, PatternToken, StyleBlock, StyleKind};
pub use format::{
    format_naive_datetime, format_zoned_datetime, DateTimeFields, DateTimeFormatContext,
    DateTimeFormatOverrides, FormattedDateTime, NaiveDateTimeInput, ZonedDateTimeInput,
};
pub use locale::{
    DateTimeLocaleSpec, DayPeriods, DayPeriodsSpec, Lengths, LengthsSpec, LocaleId, LocaleWeekday,
    ResolvedDateTimeLocale, Widths12, Widths12Spec, Widths2, Widths2Spec, Widths4, Widths4Spec,
    Widths7, Widths7Spec,
};
pub use parser::parse_datetime_spec;
pub use registry::DateTimeLocaleRegistry;
pub use style::DateTimeStyleLength;
pub use timezone::parse_datetime_timezone;
