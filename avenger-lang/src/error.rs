use thiserror::Error;
use sqlparser::tokenizer::TokenizerError;
use sqlparser::parser::ParserError;
use std::backtrace::Backtrace;


#[derive(Error, Debug)]
pub enum AvengerLangError {
    #[error("Internal error: `{0}`")]
    TokenizerError(#[from] TokenizerError),

    #[error("Internal error: `{0}`")]
    ParserError(#[from] ParserError),

    #[error("Unexpected token: `{0}`")]
    UnexpectedToken(String),
}
