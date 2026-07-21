//! Strict, schema-free parsing and lossless concrete source ownership.

mod format;
mod parser;

pub use format::{format_parsed, format_source};
pub use parser::{
    ConcreteFile, ConcreteNode, ConcreteNodeKind, ParseError, ParsedFile, SqlIslandContext,
    SqlIslandRoot, SqlIslandSite, SyntaxLimits, SyntaxNodeId, parse_file, parse_file_with_limits,
};
