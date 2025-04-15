use std::any::TypeId;
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{TokenWithSpan, Word};
use sqlparser::dialect::{Dialect, GenericDialect, SnowflakeDialect};
use sqlparser::parser::{Parser as SqlParser, ParserError};
use sqlparser::tokenizer::{Token, Tokenizer};
use sqlparser::ast::{Expr as SqlExpr};
use lazy_static::lazy_static;


use crate::ast::{AvengerFile, DatasetPropDecl, ExprPropDecl, PropQualifier, Statement, Type, ValPropDecl};
use crate::error::AvengerLangError;

lazy_static! {
    static ref AVENGER_SQL_DIALECT: AvengerSqlDialect = AvengerSqlDialect::new();
}

/// Custom dialect for Avenger language that:
/// 1. Uses GenericDialect's identifier rules (accepting @ identifiers)
/// 2. But reports itself as SnowflakeDialect to get C-style comment handling
#[derive(Debug)]
struct AvengerSqlDialect {
    generic: GenericDialect,
    snowflake_type_id: TypeId,
}

impl AvengerSqlDialect {
    fn new() -> Self {
        Self {
            generic: GenericDialect {},
            snowflake_type_id: TypeId::of::<SnowflakeDialect>(),
        }
    }
}

// Implement Dialect trait using GenericDialect's behavior but with Snowflake's TypeId
impl Dialect for AvengerSqlDialect {
    // Report as SnowflakeDialect for tokenization to get C-style comment support
    fn dialect(&self) -> TypeId {
        self.snowflake_type_id
    }
    
    // Use GenericDialect's identifier rules for @ support
    fn is_identifier_start(&self, ch: char) -> bool {
        self.generic.is_identifier_start(ch)
    }
    
    fn is_identifier_part(&self, ch: char) -> bool {
        self.generic.is_identifier_part(ch)
    }
    
    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        self.generic.is_delimited_identifier_start(ch)
    }
    
    fn supports_group_by_expr(&self) -> bool {
        self.generic.supports_group_by_expr()
    }
    
    // Forward all other needed methods to GenericDialect
    fn supports_trailing_commas(&self) -> bool {
        self.generic.supports_trailing_commas()
    }
    
    fn supports_named_fn_args_with_assignment_operator(&self) -> bool {
        true 
    }
    
    fn supports_string_literal_backslash_escape(&self) -> bool {
        true
    }
}


pub struct AvengerParser {
    pub parser: SqlParser<'static>,
}


impl AvengerParser {
    pub fn new() -> Self {
        Self {
            parser: SqlParser::new(&*AVENGER_SQL_DIALECT)
        }
    }

    fn is_at_end(&self) -> bool {
        self.parser.peek_token_ref().token == Token::EOF
    }

    // // Rule parsers
    fn parse_type(&mut self) -> Result<Option<Type>, ParserError> {
        if self.parser.peek_token_ref().token == Token::Lt {
            self.parser.expect_token(&Token::Lt)?;
            let name = self.parser.parse_identifier()?;
            self.parser.expect_token(&Token::Gt)?;
            Ok(Some(Type(name.value)))
        } else {
            Ok(None)
        }
    }

    fn parse_qualifier(&mut self) -> Option<PropQualifier> {
        let qualifier = self.parser.parse_one_of_keywords(
            &[Keyword::IN, Keyword::OUT]
        );
        match qualifier {
            Some(Keyword::IN) => Some(PropQualifier::In),
            Some(Keyword::OUT) => Some(PropQualifier::Out),
            _ => None
        }
    }

    fn next_word(&mut self) -> Result<String, ParserError> {
        let next_token = self.parser.next_token();
        match next_token.token {
            Token::Word(w) => Ok(w.into_ident(next_token.span).value),
            _ => self.parser.expected("name", next_token),
        }
    }
    
    fn parse_statement(&mut self) -> Result<Statement, AvengerLangError> {
        // Parse and advance past the optional qualifier
        let qualifier = self.parse_qualifier();

        let kind = self.next_word()?;
        match kind.as_str() {
            "val" => {
                let ty = self.parse_type()?;
                let name = self.next_word()?;
                self.parser.expect_token(&Token::Colon)?;
                let value = self.parser.parse_expr()?;
                self.parser.expect_token(&Token::SemiColon)?;
                Ok(Statement::ValPropDecl(ValPropDecl { 
                    name, value, qualifier, type_: ty 
                }))
            }
            "expr" => {
                let ty = self.parse_type()?;
                let name = self.next_word()?;
                self.parser.expect_token(&Token::Colon)?;
                let value = self.parser.parse_expr()?;
                self.parser.expect_token(&Token::SemiColon)?;
                Ok(Statement::ExprPropDecl(ExprPropDecl { 
                    name, value, qualifier, type_: ty 
                }))
            }
            "dataset" => {
                let ty = self.parse_type()?;
                let name = self.next_word()?;
                self.parser.expect_token(&Token::Colon)?;
                let value = self.parser.parse_query()?;
                self.parser.expect_token(&Token::SemiColon)?;
                Ok(Statement::DatasetPropDecl(DatasetPropDecl { 
                    name, value, qualifier, type_: ty 
                }))
            }
            _ => {
                Err(AvengerLangError::UnexpectedToken(kind))
            }
        }
    }

    fn parse_file(&mut self) -> Result<AvengerFile, AvengerLangError> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }
        Ok(AvengerFile { statements })
    }

    // Public methods
    pub fn tokenize(&self, input: &str) -> Result<Vec<TokenWithSpan>, AvengerLangError> {
        let mut tokenizer = Tokenizer::new(&*AVENGER_SQL_DIALECT, input);
        let tokens = tokenizer.tokenize_with_location()?;
        Ok(tokens)
    }

    pub fn parse(&mut self) -> Result<AvengerFile, AvengerLangError> {
        let file = self.parse_file()?;
        Ok(file)
    }

    pub fn with_tokens_with_locations(mut self, tokens: Vec<TokenWithSpan>) -> Self {
        self.parser = self.parser.with_tokens_with_locations(tokens);
        self
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_file() {
        let src = r#"
        // This is a comment
        in val<int> my_val: 1 + 23;
        dataset my_dataset: select * from @my_table LIMIT 12;
        out expr my_expr: @my_val + 1;
        "#;
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(src).unwrap();
        println!("{:#?}", tokens);

        // Create a new parser with the tokens
        let mut parser = parser.with_tokens_with_locations(tokens);

        let file = parser.parse().unwrap();
        println!("{:#?}", file);
    }

    #[test]
    fn test_parse_file2() {
        let src = r#"
        // This is a comment
        in expr my_expr: (2 + 1) * 3;
        "#;
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(src).unwrap();
        let mut parser = parser.with_tokens_with_locations(tokens);
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
    }
}

