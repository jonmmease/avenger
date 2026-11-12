//! The token pass: rewrites of the tokens of the text, before parsing, for the syntax that
//! sqlparser-rs 0.62 does not parse. It reads no schemas and no functions.

use datafusion::{
    common::Result,
    sql::sqlparser::tokenizer::{Token, TokenWithSpan},
};

use crate::error::at;

/// The prefix of the names that the passes generate, which the text of a query cannot hold.
pub(crate) const RESERVED: &str = "__avenger_";

/// The tokens of a query after the token pass. A word with the reserved prefix is an error at the
/// word.
pub(crate) fn rewrite(tokens: Vec<TokenWithSpan>) -> Result<Vec<TokenWithSpan>> {
    for token in &tokens {
        if let Token::Word(word) = &token.token {
            if word.value.to_lowercase().starts_with(RESERVED) {
                return Err(at(
                    token.span,
                    format!(
                        "`{}` starts with `{RESERVED}`, a prefix that avenger-sql reserves",
                        word.value
                    ),
                ));
            }
        }
    }
    Ok(tokens)
}
