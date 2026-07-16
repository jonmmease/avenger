//! Shared lexical and SQL-island frontend for the Avenger language.

mod dialect;
mod normalize;
mod parser;
mod token;

pub use dialect::AvengerSqlDialect;
pub use normalize::{
    BindingOccurrence, BindingVersion, NormalizedSql, SqlFrontendError, normalize_bindings,
};
pub use parser::{
    DOMAIN_RANGE_HELPERS, ParsedSqlIsland, RESERVED_HELPER_NAMES, is_reserved_helper_name,
    parse_sql_expression, parse_sql_query,
};
pub use token::{
    CommentKind, DocCommentBlock, LanguageToken, TokenClass, TokenStream, TokenizeError,
    WhitespaceKind, tokenize,
};
