//! Strict, schema-free parsing and lossless concrete source ownership.

mod parser;

pub use parser::{ConcreteFile, ParseError, ParsedFile, parse_file};
