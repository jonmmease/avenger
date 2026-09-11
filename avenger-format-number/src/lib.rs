//! Reusable number formatting for Avenger.
//!
//! This crate owns d3-compatible numeric specifier parsing, Avenger extension
//! parsing, locale resolution, scalar formatting, and tick-set preparation.
//! It deliberately has no dependency on chart, guide, scenegraph, scale, or
//! Typst crates.

pub mod cldr_pattern;
pub mod compact;
pub mod currency;
pub mod digits;
pub mod error;
pub mod format;
pub mod locale;
pub mod parser;
pub mod registry;
pub mod spec;
pub mod ticks;
pub mod typesetting;

pub use compact::CompactTier;
pub use error::{FormatError, ParseError};
pub use format::{
    format_number, CompatibilityPolicy, NumberFormatContext, NumberFormatOverrides,
    ResolvedNumberFormat,
};
pub use locale::{
    CldrCurrencyFormatSpec, CurrencyDisplay, CurrencyDisplayNames, CurrencyFormat,
    CurrencyFormatSpec, CurrencyPattern, DecimalPattern, DecimalPatternSpec, GroupingSpec,
    LocaleId, NumberLocaleSpec, ResolvedNumberLocale,
};
pub use parser::parse_number_spec;
pub use registry::NumberLocaleRegistry;
pub use spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol};
pub use ticks::{prepare_number_tick_format, PreparedNumberTickFormat};
pub use typesetting::{ExponentMarker, FormattedNumber, NumberTypesetting};
