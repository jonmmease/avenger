use std::any::TypeId;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;

use lazy_static::lazy_static;
use sqlparser::ast::{Expr as SqlExpr, Ident, Query as SqlQuery, Statement as SqlStatement, Spanned};
use sqlparser::dialect::{Dialect, GenericDialect, SnowflakeDialect};
use sqlparser::parser::{Parser as SqlParser, ParserError};
use sqlparser::tokenizer::{Span, Token, TokenWithSpan, Tokenizer};
use crate::ast::{AvengerScript, Block, ExprDecl, KeywordExpr, KeywordTable, KeywordVal, ScriptStatement, SqlExprOrQuery, TableDecl, ValDecl, VarAssignment};
use crate::error::{AvengerLangError, PositionalParseErrorInfo};

lazy_static! {
    static ref AVENGER_SQL_DIALECT: AvengerSqlDialect = AvengerSqlDialect::new();
}

/// Custom dialect for Avenger language that:
/// 1. Uses GenericDialect's identifier rules (accepting @ identifiers)
/// 2. But reports itself as SnowflakeDialect to get C-style comment handling
#[derive(Debug)]
struct AvengerSqlDialect {
    generic: GenericDialect,
    type_id: TypeId,
}

impl AvengerSqlDialect {
    pub fn new() -> Self {
        Self {
            generic: GenericDialect {},
            // Start with snowflake for slash comment support
            type_id: TypeId::of::<GenericDialect>(),
        }
    }

    /// Copy the dialect for tokenization purposes, using SnowflakeDialect's TypeId
    /// for c-style comment support
    pub fn copy_for_tokenization(&self) -> Self {
        Self {
            generic: GenericDialect {},
            type_id: TypeId::of::<SnowflakeDialect>(),
        }
    }
}

// Implement Dialect trait using GenericDialect's behavior but with Snowflake's TypeId
impl Dialect for AvengerSqlDialect {
    // Report as SnowflakeDialect for tokenization to get C-style comment support
    fn dialect(&self) -> TypeId {
        self.type_id
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

    fn supports_trailing_commas(&self) -> bool {
        true
    }

    fn supports_named_fn_args_with_assignment_operator(&self) -> bool {
        true
    }

    fn supports_string_literal_backslash_escape(&self) -> bool {
        true
    }
}

pub struct AvengerParser<'a> {
    pub parser: SqlParser<'static>,
    pub tokens: Vec<TokenWithSpan>,
    /// The source code being parsed
    pub src: &'a str,
}

impl<'a> AvengerParser<'a> {
    pub fn new(src: &'a str) -> Result<Self, AvengerLangError> {
        let tokens = Tokenizer::new(&AVENGER_SQL_DIALECT.copy_for_tokenization(), src)
            .tokenize_with_location()?;

        // println!("Tokens: {:#?}", tokens);

        Ok(Self {
            parser: SqlParser::new(&*AVENGER_SQL_DIALECT)
                .with_tokens_with_locations(tokens.clone()),
            tokens,
            src,
        })
    }

    pub fn parse_script(&mut self) -> Result<AvengerScript, AvengerLangError> {
        let statements = self.parse_statements()?;
        // Expect end of file
        self.parser.expect_token(&Token::EOF)?;
        Ok(AvengerScript {
            statements,
        })
    }

    fn parse_statements(&mut self) -> Result<Vec<ScriptStatement>, AvengerLangError> {
        let mut statements = Vec::new();
        while !self.is_at_end() && self.parser.peek_token().token != Token::RBrace {
            statements.push(self.parse_statement()?);
        }
        Ok(statements)
    }

    fn is_at_end(&self) -> bool {
        self.parser.peek_token().token == Token::EOF
    }

    fn parse_statement(&mut self) -> Result<ScriptStatement, AvengerLangError> {
        // Check for block start
        let statement: ScriptStatement = if self.parser.peek_token() == Token::LBrace {
            // Consume the opening brace
            self.parser.expect_token(&Token::LBrace)?;
            let statements = self.parse_statements()?;
            self.parser.expect_token(&Token::RBrace)?;
            ScriptStatement::Block(Block {
                statements,
            })
        } else {
            // Get the first identifier
            let ident = self.expect_ident(None, "start of statement")?;
            match ident.value.to_lowercase().as_str() {
                "val" => ScriptStatement::ValDecl(self.parse_val_decl(ident)?),
                "expr" => ScriptStatement::ExprDecl(self.parse_expr_decl(ident)?),
                "table" => ScriptStatement::TableDecl(self.parse_table_decl(ident)?),
                // name := query_or_expr;
                _ if self.parser.peek_token().token == Token::Assignment => {
                    // Consume the assignment token
                    self.parser.next_token();

                    // Parse the query or expression, with required trailing semi-colon
                    let expr_or_query = self.parse_sql_expr_or_query(true)?;

                    ScriptStatement::VarAssignment(VarAssignment {
                        name: ident,
                        expr: expr_or_query,
                    })
                }
                _ => {
                    return Err(AvengerLangError::PositionalParseError(
                        PositionalParseErrorInfo {
                            message: format!(
                                "expected statement, found {}",
                                ident
                            ),
                            line: ident.span.start.line as i32,
                            column: ident.span.start.column as i32,
                            len: (ident.span.end.column - ident.span.start.column) as usize,
                        },
                    ));
                }
            }
        };

        Ok(statement)
    }

    /// Parse a val declaration of the form:
    /// val foo: 23;
    fn parse_val_decl(
        &mut self,
        statement_ident: Ident,
    ) -> Result<ValDecl, AvengerLangError> {
        // Build val keyword from statement identifier
        let val_keyword = KeywordVal {
            span: statement_ident.span,
        };

        // Get the property name
        let name = self.expect_ident(None, "val property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the expression
        let expr = self.parser.parse_expr()?;

        // Semi-colon required after expression
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(ValDecl {
            val_keyword,
            name,
            expr,
        })
    }

    /// Parse a val declaration of the form:
    /// expr foo: 23 + "bar";
    fn parse_expr_decl(
        &mut self,
        statement_ident: Ident,
    ) -> Result<ExprDecl, AvengerLangError> {
        // Build val keyword from statement identifier
        let expr_keyword = KeywordExpr {
            span: statement_ident.span,
        };

        // Get the property name
        let name = self.expect_ident(None, "expr property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the expression
        let expr = self.parser.parse_expr()?;

        // Semi-colon required after expression
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(ExprDecl {
            expr_keyword,
            name,
            expr,
        })
    }

    fn parse_table_decl(
        &mut self,
        statement_iden: Ident,
    ) -> Result<TableDecl, AvengerLangError> {
        // Build dataset keyword from statement identifier
        let dataset_keyword = KeywordTable {
            span: statement_iden.span,
        };

        // Get the property name
        let name = self.expect_ident(None, "table property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the query
        let query = self.parser.parse_query()?;

        // Semi-colon required after query
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(TableDecl {
            table_keyword: dataset_keyword,
            name,
            query,
        })
    }

    fn peek_ident(&mut self, nth: usize) -> Result<Ident, ParserError> {
        let next_token = self.parser.peek_nth_token(nth);
        match next_token.token {
            Token::Word(w) => Ok(Ident {
                value: w.value.clone(),
                quote_style: w.quote_style,
                span: next_token.span,
            }),
            _ => self.parser.expected("identifier", next_token),
        }
    }

    /// Expect a word token, with optional expected value. Returns the word value.
    fn expect_ident(&mut self, expected: Option<&str>, msg: &str) -> Result<Ident, ParserError> {
        let token = self.parser.next_token();
        match &token.token {
            Token::Word(w) => {
                if let Some(expected) = expected {
                    if w.value.to_lowercase() != expected.to_lowercase() {
                        return self.parser.expected(expected, token);
                    }
                }
                Ok(Ident {
                    value: w.value.clone(),
                    quote_style: w.quote_style,
                    span: token.span,
                })
            }
            _ => self.parser.expected(msg, token),
        }
    }

    fn parse_sql_expr_or_query(
        &mut self,
        required_semi_colon: bool,
    ) -> Result<SqlExprOrQuery, AvengerLangError> {
        let index = self.parser.index();

        let expr_or_query = if self.parser.peek_token() == Token::LParen {
            // Parse a parenthesized expression
            SqlExprOrQuery::Expr(self.parser.parse_expr()?)
        } else if let Ok(sql_query) = self.parser.parse_query() {
            SqlExprOrQuery::Query(sql_query)
        } else {
            // Reset index to the index we were at prior to the attempt to parse as a query
            while self.parser.index() > index {
                self.parser.prev_token();
            }
            // Consume the assignment token if we jumped back to it, but don't fail if we don't find it
            let _ = self.parser.expect_token(&Token::Assignment);
            SqlExprOrQuery::Expr(self.parser.parse_expr()?)
        };

        // Consume optional semi-colon
        if required_semi_colon || self.parser.peek_token_ref().token == Token::SemiColon {
            self.parser.expect_token(&Token::SemiColon)?;
        }

        Ok(expr_or_query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_val_decl() {
        let src = r#"
val foo: 23;
foo := @foo * 2;
{
    table a: select 1;
}
expr baz: "other" / 23;
"#;
        let mut parser = AvengerParser::new(src).unwrap();
        let script = parser.parse_script().unwrap();
        println!("{:#?}\n{}", script, script);
    }
}