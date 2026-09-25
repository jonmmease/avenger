#![doc = include_str!("../README.md")]

mod adapters;
mod compact;
mod decimal;
mod digits;
mod error;
mod format;
mod formatted_number;
mod locale;
mod parser;
mod registry;
mod spec;

pub use adapters::{
    prepare_number_float_format, prepare_number_prefix_format, prepare_number_step_format,
};
pub use compact::{CompactPluralRule, CompactTier};
pub use error::{FormatError, ParseError};
pub use format::{format_number, NumberFormatOverrides, PreparedNumberFormat};
pub use formatted_number::{FormattedNumber, NumberTypesetting};
pub use locale::{LocaleId, NumberLocaleExtensions, NumberLocaleSpec, ResolvedNumberLocale};
pub use parser::parse_number_spec;
pub use registry::NumberLocaleRegistry;
pub use spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol};
