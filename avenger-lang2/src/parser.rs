use std::any::TypeId;

use lazy_static::lazy_static;
use sqlparser::ast::Spanned;
use sqlparser::dialect::{Dialect, GenericDialect, SnowflakeDialect};
use sqlparser::parser::{Parser as SqlParser, ParserError};
use sqlparser::tokenizer::{Token, TokenWithSpan, Tokenizer};

use crate::ast::{AvengerFile, ComponentProp, DatasetProp, ExprProp, Identifier, ImportItem, ImportStatement, KeywordAs, KeywordComp, KeywordDataset, KeywordExpr, KeywordFrom, KeywordImport, KeywordIn, KeywordOut, KeywordVal, PropBinding, Qualifier, SqlExprOrQuery, Statement, Type, ValProp};
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
    pub sql: &'a str
}


impl<'a> AvengerParser<'a> {
    pub fn new(sql: &'a str) -> Result<Self, AvengerLangError> {
        let tokens = Tokenizer::new(&*AVENGER_SQL_DIALECT, sql).tokenize_with_location()?;
        Ok(Self {
            parser: SqlParser::new(&*AVENGER_SQL_DIALECT).with_tokens_with_locations(tokens.clone()),
            tokens,
            sql
        })
    }

    pub fn parse(&mut self) -> Result<AvengerFile, AvengerLangError> {
        let statements = self.parse_statements()?;
        // Expect end of file
        self.parser.expect_token(&Token::EOF)?;
        Ok(AvengerFile { statements })
    }

    fn parse_statements(&mut self) -> Result<Vec<Statement>, AvengerLangError> {
        let mut statements = Vec::new();
        while !self.is_at_end() && self.parser.peek_token().token != Token::RBrace {
            statements.push(self.parse_statement()?);
        }
        Ok(statements)
    }

    fn is_at_end(&self) -> bool {
        self.parser.peek_token().token == Token::EOF
    }

    fn is_ident(&self, expected: Option<&str>) -> bool {
        let next_token = self.parser.peek_token();
        match next_token.token {
            Token::Word(w) => {
                if let Some(expected) = expected {
                    return w.value == expected;
                }
                true
            }
            _ => false,
        }
    }

    /// Expect a word token, with optional expected value. Returns the word value.
    fn expect_ident(&mut self, expected: Option<&str>, msg: &str) -> Result<Identifier, ParserError> {
        let token = self.parser.next_token();
        match &token.token {
            Token::Word(w) => {
                if let Some(expected) = expected {
                    if w.value != expected {
                        return self.parser.expected(expected, token);
                    }
                }
                Ok(Identifier { 
                    span: token.span, 
                    name: w.value.clone() 
                })
            },
            _ => self.parser.expected(msg, token),
        }
    }

    fn expect_single_quoted_string(&mut self) -> Result<Identifier, ParserError> {
        let next_token = self.parser.next_token();
        match next_token.token {
            Token::SingleQuotedString(s) => Ok(Identifier { name: s, span: next_token.span }),
            _ => self.parser.expected("single quoted string", next_token),
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, AvengerLangError> {
        // Get the first identifier
        let mut ident = self.expect_ident(None, "start of statement")?;
        // See if we have a qualifier
        let qualifier = match ident.name.as_str() {
            "in" => {
                // Build qualifier and advance to next identifier
                let q = Qualifier::In(KeywordIn { span: ident.span });
                ident = self.expect_ident(None, "property name")?;
                Some(q)
            },
            "out" => {
                // Build qualifier and advance to next identifier
                let q = Qualifier::Out(KeywordOut { span: ident.span });
                ident = self.expect_ident(None, "property name")?;
                Some(q)
            },
            _ => None,
        };
        match ident.name.as_str() {
            "import" => self.parse_import_statement(qualifier, ident),
            "val" => self.parse_val_statement(qualifier, ident),
            "expr" => self.parse_expr_statement(qualifier, ident),
            "dataset" => self.parse_dataset_statement(qualifier, ident),
            "comp" => self.parse_comp_prop_statement(qualifier, ident),
            "fn" => self.parse_fn_statement(),
            
            // Handle binding or the form:
            // name := query_or_expr;
            _ if self.parser.peek_token().token == Token::Assignment => {
                // Consume the assignment token
                self.parser.next_token();

                // Parse the query or expression, with required trailing semi-colon
                let expr_or_query = self.parse_sql_expr_or_query(true)?;

                Ok(Statement::PropBinding(PropBinding { name: ident, expr: expr_or_query }))
            }
            _ => {
                // Assume anonymous component value of the form:
                // Rect {...}
                self.error_if_qualifier(qualifier, "anonymous component")?;
                self.parse_component_value(None, None, None)
            },
        }
    }

    /// Error if a qualifier is present for a statement that does not support it
    fn error_if_qualifier(&mut self, qualifier: Option<Qualifier>, statement_name: &str) -> Result<(), AvengerLangError> {
        if let Some(qualifier) = qualifier {
            return Err(AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
                message: format!("{} statements do not support in/out qualifiers", statement_name),
                line: qualifier.span().start.line as i32,
                column: qualifier.span().start.column as i32,
                len: (qualifier.span().end.column - qualifier.span().start.column) as usize,
            }));
        }
        Ok(())
    }

    /// parse import statement of the form:
    /// 
    /// import { Foo } from "bar";
    /// import { Foo as Bar, Baz } from "bar";
    /// import { Blah }
    fn parse_import_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Identifier) -> Result<Statement, AvengerLangError> {
        self.error_if_qualifier(qualifier, "import")?;

        // Build import keyword from statement identifier
        let import_keyword = KeywordImport { span: statement_iden.span };

        // import keyword already consumed
        self.parser.expect_token(&Token::LBrace)?;

        // Parse import items
        let mut items: Vec<ImportItem> = Vec::new();
        while self.parser.peek_token().token != Token::RBrace {

            let component = self.expect_ident(None, "import component")?;
            let as_ = if self.is_ident(Some("as")) {
                // Get the as keyword
                let as_ident = self.expect_ident(Some("as"), "as")?;
                let as_keyword = KeywordAs { span: as_ident.span };

                // Get the alias
                let alias = self.expect_ident(None, "import alias")?;
                Some((
                    as_keyword, alias
                ))
            } else {
                None
            };
            items.push(ImportItem {
                name: component,
                as_,
            });
            // Consume comma if present
            if self.parser.peek_token().token == Token::Comma {
                self.parser.next_token();
            }
        }
        self.parser.expect_token(&Token::RBrace)?;

        // Parse from clause
        let from = if self.is_ident(Some("from")) {
            let from_ident = self.expect_ident(Some("from"), "from")?;
            let from_keyword = KeywordFrom { span: from_ident.span };

            // Get the path
            let path = self.expect_single_quoted_string()?;
            // semi-colon required after path
            self.parser.expect_token(&Token::SemiColon)?;
            Some((from_keyword, path))
        } else {
            None
        };

        Ok(Statement::Import(ImportStatement {
            import_keyword,
            items,
            from,
        }))
    }


    /// Parse a type statement of the form:
    /// <Type>
    fn parse_type(&mut self) -> Result<Option<Type>, AvengerLangError> {
        if self.parser.peek_token().token == Token::Lt {
            self.parser.next_token();
            let name = self.expect_ident(None, "type name")?;
            self.parser.expect_token(&Token::Gt)?;
            Ok(Some(Type { name }))
        } else {
            // Type is optional
            Ok(None)
        }
    }

    /// Parse a val statement of the form:
    /// val <Type> foo: 23;
    fn parse_val_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Identifier) -> Result<Statement, AvengerLangError> {
        // Build val keyword from statement identifier
        let val_keyword = KeywordVal { span: statement_iden.span };

        // Get the type
        let type_ = self.parse_type()?;

        // Get the property name
        let name = self.expect_ident(None, "val property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the expression
        let expr = self.parser.parse_expr()?;

        // Semi-colon required after expression
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(Statement::ValProp(ValProp { qualifier, val_keyword, type_, name, expr }))
    }

    fn parse_expr_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Identifier) -> Result<Statement, AvengerLangError> {
        // Build expr keyword from statement identifier
        let expr_keyword = KeywordExpr { span: statement_iden.span };

        // Get the type
        let type_ = self.parse_type()?;

        // Get the property name
        let name = self.expect_ident(None, "expr property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the expression
        let expr = self.parser.parse_expr()?;

        // Semi-colon required after expression
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(Statement::ExprProp(ExprProp { qualifier, expr_keyword, type_, name, expr }))
    }

    fn parse_dataset_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Identifier) -> Result<Statement, AvengerLangError> {
        // Build dataset keyword from statement identifier
        let dataset_keyword = KeywordDataset { span: statement_iden.span };

        // Get the type
        let type_ = self.parse_type()?;

        // Get the property name
        let name = self.expect_ident(None, "dataset property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the query
        let query = self.parser.parse_query()?;

        // Semi-colon required after query
        self.parser.expect_token(&Token::SemiColon)?;

        Ok(Statement::DatasetProp(DatasetProp { qualifier, dataset_keyword, type_, name, query }))
    }

    /// Parse a component property statement of the form:
    /// comp foo: Rect {...}
    fn parse_comp_prop_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Identifier) -> Result<Statement, AvengerLangError> {
        // Build comp keyword from statement identifier
        let comp_keyword = KeywordComp { span: statement_iden.span };

        // Get the component type
        let name = self.expect_ident(None, "comp property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Get the statements
        self.parse_component_value(qualifier, Some(comp_keyword), Some(name))
    }

    /// Parse a component value statement of the form:
    /// Rect {...}
    fn parse_component_value(&mut self, qualifier: Option<Qualifier>, keyword: Option<KeywordComp>, prop_name: Option<Identifier>) -> Result<Statement, AvengerLangError> {

        // parse component name
        let component_name = self.expect_ident(None, "component name")?;

        // Open brace
        self.parser.expect_token(&Token::LBrace)?;

        // Parse statements
        let statements = self.parse_statements()?;

        // Close brace
        self.parser.expect_token(&Token::RBrace)?;

        Ok(Statement::ComponentProp(ComponentProp { 
            qualifier,
            component_keyword: keyword,
            prop_name, 
            component_name, 
            statements, 
        }))
    }

    fn parse_fn_statement(&mut self) -> Result<Statement, AvengerLangError> {
        todo!()
    }

    fn parse_sql_expr_or_query(&mut self, required_semi_colon: bool) -> Result<SqlExprOrQuery, AvengerLangError> {
        let index = self.parser.index();
        let expr_or_query = if let Ok(sql_query) = self.parser.parse_query() {
            let value = SqlExprOrQuery::Query(sql_query);
            value
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
    fn test_parse_import_statement() {
        let sql = r#"
import { Foo } from 'bar';
import { Foo as Bar, Baz } from 'bar';
import { Blah }
        "#;
        let mut parser = AvengerParser::new(sql).unwrap();
        let file = parser.parse().unwrap();
        assert_eq!(file.statements.len(), 3);
    }

    #[test]
    fn test_parse_val_statement() {
        let sql = r#"
        val foo: 23;
        "#;
        let mut parser = AvengerParser::new(sql).unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_expr_statement() {
        let sql = r#"
        expr foo: 23;
        "#;
        let mut parser = AvengerParser::new(sql).unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_dataset_statement() {
        let sql = r#"
        dataset foo: select * from bar;
        "#;
        let mut parser = AvengerParser::new(sql).unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_component_statement() {
        let sql = r#"
comp foo: Rect {}"#;
        let mut parser = AvengerParser::new(sql).unwrap();

        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }


    #[test]
    fn test_parse_binding_statement() {
        let sql = r#"
        foo := select * from bar;
        "#;
        let mut parser = AvengerParser::new(sql).unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn try_error_for_missing_semi_colon() {
        let sql = r#"
        dataset foo: select * from bar
"#;
        let mut parser = AvengerParser::new(sql).unwrap();
        match parser.parse() {
            Ok(_) => panic!("Expected error"),
            Err(e) => {
                println!("{:?}", e);
                e.pretty_print(sql, "test.avgr").unwrap();
            }
        }
    }

    #[test]
    fn try_parse_with_trailing_junk() {
        let sql = r#"
comp foo: Rect {}}"#;
        let mut parser = AvengerParser::new(sql).unwrap();
        match parser.parse() {
            Ok(_) => {
                println!("No error");
            }
            Err(e) => {
                e.pretty_print(sql, "test.avgr").unwrap();
            }
        }
    }
}
