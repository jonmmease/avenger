use std::io;

use regex::Regex;
use sqlparser::{parser::ParserError, tokenizer::TokenizerError};
use thiserror::Error;
use ariadne::{ColorGenerator, Label, Report, ReportKind, Source};


#[derive(Error, Debug)]
pub enum AvengerLangError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    /// A parser error with a position in the source code
    #[error("{0:?}")]
    PositionalParseError(PositionalParseErrorInfo),

    /// A parser error with a position in the source code
    #[error("Parser error: `{0}`")]
    GeneralParserError(String),
}

impl From<TokenizerError> for AvengerLangError {
    fn from(error: TokenizerError) -> Self {
        AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
            message: error.message,
            line: error.location.line as i32,
            column: error.location.column as i32,
            len: 1,
        })
    }
}


impl From<ParserError> for AvengerLangError {
    fn from(error: ParserError) -> Self {

        println!("try from: {:#?}", error);

        let msg = match error {
            ParserError::TokenizerError(msg) => msg.clone(),
            ParserError::ParserError(msg) => msg.clone(),
            ParserError::RecursionLimitExceeded => {
                return AvengerLangError::GeneralParserError("Recursion limit exceeded".to_string())
            }
        };

        // Try to extract position information from the message
        // Check for length of found item in message, default to 1 if not found
        let found = Regex::new(r"found: ([^ ]+)").ok()
            .and_then(|re| re.captures(&msg))
            .and_then(|captures| captures.get(1))
            .map(|found| found.as_str());


        // Handle EOF, where we don't have a line or column reported
        if found == Some("EOF") {
            return Self::PositionalParseError(PositionalParseErrorInfo {
                message: msg.as_str().to_string(),
                line: -1,
                column: -1,
                len: 1,
            })
        }

        let len = found.map(|found| found.len() + 1).unwrap_or(1);
        Regex::new(r"(.*) at Line: (\d+), Column: (\d+)").ok()
            .and_then(|re| re.captures(&msg))
            .and_then(|captures| {
                if let (Some(msg), Some(line), Some(column)) = (captures.get(1), captures.get(2), captures.get(3)) {
                    Some(
                        Self::PositionalParseError(PositionalParseErrorInfo {
                            message: msg.as_str().to_string(),
                            line: line.as_str().parse().unwrap(),
                            column: column.as_str().parse().unwrap(),
                            len,
                        }
                    ))
                } else {
                    None
                }
            }).unwrap_or_else(|| Self::GeneralParserError(msg.to_string()))
    }
}


impl AvengerLangError {
    pub fn pretty_print(&self, src: &str, file_name: &str) -> io::Result<()> {
        match self {
            Self::PositionalParseError(info) => {
                info.pretty_print(src, file_name)?;
            }
            _ => {
                // Fallback to default error message
                println!("{}", self);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PositionalParseErrorInfo {
    pub message: String,
    /// Line number, -1 for last line
    pub line: i32,
    /// Column number, -1 for last column
    pub column: i32,
    pub len: usize,
}

impl PositionalParseErrorInfo {
    pub fn pretty_print(&self, src: &str, file_name: &str) -> io::Result<()> {
        let lines = src.lines().collect::<Vec<_>>();
        let line_lens = lines.iter().map(|line| line.len() + 1).collect::<Vec<_>>();

        // Locate span start
        let span_start = if self.line == -1 {
            // Find last non-whitespace character, and add 1
            let num_trailing_ws = src.chars().rev().take_while(|c| c.is_whitespace()).count();
            src.len() - num_trailing_ws
        } else {
            line_lens[..self.line as usize - 1].iter().sum::<usize>() + self.column as usize - 1
        };

        let mut colors = ColorGenerator::new();
    
        Report::build(ReportKind::Error, (file_name, span_start..span_start))
            .with_message("Parsing error")
            .with_label(
                Label::new((file_name, span_start..span_start + self.len - 1))
                    .with_message(&self.message)
                    .with_color(colors.next()),
            )
            .finish()
            .print((file_name, Source::from(src)))
            .unwrap();

        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positional_parse_error_info() {
        let info = PositionalParseErrorInfo {
            message: "Expected ';'".to_string(),
            line: 1,
            column: 10,
            len: 1,
        };

        info.pretty_print("let x = 1", "foo.sql").unwrap();
    }

    #[test]
    fn try_avenger_lang_error() {
        use sqlparser::dialect::GenericDialect;
        use sqlparser::parser::Parser;
    
        let src = r#"
    SELECT a, b, 123, myfunc(b)
    FROM table_1 as "asdf"
    WHERE a AND b < 100 WHERE other
    ORDER BY a DESC, b"#;

        let dialect = GenericDialect {};
    
        match Parser::parse_sql(&dialect, src) {
            Ok(ast) => {
                // Process the AST here
                println!("AST: {:?}", ast);
            }
            Err(e) => {
                let error = AvengerLangError::from(e);
                error.pretty_print(src, "foo.sql").unwrap();
            }
        }
    }
    
}
