#![doc = include_str!("../README.md")]

mod decimal;
mod digits;
pub mod error;
pub mod format;
pub mod locale;
pub mod parser;
pub mod registry;
pub mod spec;
pub mod ticks;
pub mod typesetting;

pub use error::{FormatError, ParseError};
pub use format::{
    format_number, NumberFormatContext, NumberFormatOverrides, PreparedNumberFormat,
    ResolvedNumberFormat,
};
pub use locale::{LocaleId, NumberLocaleSpec, ResolvedNumberLocale};
pub use parser::parse_number_spec;
pub use registry::NumberLocaleRegistry;
pub use spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol};
pub use ticks::{
    prepare_number_float_format, prepare_number_prefix_format, prepare_number_span_format,
    prepare_number_tick_format, PreparedNumberTickFormat,
};
pub use typesetting::{ExponentMarker, FormattedNumber, NumberTypesetting};
