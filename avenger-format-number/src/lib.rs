#![doc = include_str!("../README.md")]

mod adapters;
mod decimal;
mod digits;
mod error;
mod format;
mod locale;
mod parser;
mod registry;
mod spec;
mod typesetting;

pub use adapters::{
    prepare_number_float_format, prepare_number_prefix_format, prepare_number_step_format,
};
pub use error::{FormatError, ParseError};
pub use format::{format_number, NumberFormatOverrides, PreparedNumberFormat};
pub use locale::{LocaleId, NumberLocaleSpec, ResolvedNumberLocale};
pub use parser::parse_number_spec;
pub use registry::NumberLocaleRegistry;
pub use spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol};
pub use typesetting::{FormattedNumber, NumberTypesetting};
