//! Shared lexical and SQL-island frontend for the Avenger language.

mod dialect;
mod identifier;
mod normalize;
mod parser;
mod token;

pub use dialect::AvengerSqlDialect;
pub use normalize::{
    BindingOccurrence, BindingVersion, NormalizedSql, SqlFrontendError, normalize_bindings,
};
pub use parser::{
    DOMAIN_RANGE_HELPERS, ParsedSqlIsland, RESERVED_HELPER_NAMES, SqlParseLimits,
    is_reserved_helper_name, parse_sql_expression, parse_sql_expression_with_limits,
    parse_sql_query, parse_sql_query_with_limits,
};
pub use token::{
    CommentKind, DocCommentBlock, LanguageToken, LosslessToken, LosslessTokenKind,
    LosslessTokenStream, TokenClass, TokenStream, TokenizeError, WhitespaceKind, tokenize,
    tokenize_lossless, tokenize_lossless_with_limit,
};

pub(crate) use identifier::{
    is_unquoted_identifier, is_unquoted_identifier_part, is_unquoted_identifier_start,
};
