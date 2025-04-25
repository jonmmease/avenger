use std::any::TypeId;
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{TokenWithSpan, Word};
use sqlparser::dialect::{Dialect, GenericDialect, SnowflakeDialect};
use sqlparser::parser::{Parser as SqlParser, ParserError};
use sqlparser::tokenizer::{Token, Tokenizer};
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery};
use lazy_static::lazy_static;


use crate::ast::{AvengerFile, CompInstance, CompPropDecl, ComponentDef, ConditionalComponentsStatement, ConditionalIfBranch, ConditionalIfComponents, ConditionalMatchBranch, ConditionalMatchComponents, ConditionalMatchDefaultBranch, DatasetPropDecl, ExprPropDecl, FunctionDef, FunctionParam, FunctionParamKind, FunctionReturnParam, FunctionStatement, Import, ImportItem, PropBinding, PropQualifier, ReturnStatement, SqlExprOrQuery, Statement, Type, ValPropDecl};
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
    pub parser: SqlParser<'static>
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

    fn expect_word(&mut self, expected: &str) -> Result<(), ParserError> {
        if self.peek_word()? != expected.to_string() {
            self.parser.expected(expected, self.parser.peek_token())
        } else {
            self.parser.next_token();
            Ok(())
        }
    }

    fn peek_word(&mut self) -> Result<String, ParserError> {
        let next_token = self.parser.peek_token();
        match next_token.token {
            Token::Word(w) => Ok(w.into_ident(next_token.span).value),
            _ => self.parser.expected("word", next_token),
        }
    }

    fn expect_single_quoted_string(&mut self) -> Result<String, ParserError> {
        let next_token = self.parser.next_token();
        match next_token.token {
            Token::SingleQuotedString(s) => Ok(s),
            _ => self.parser.expected("single quoted string", next_token),
        }
    }
    
    fn parse_comp_instance(&mut self) -> Result<CompInstance, AvengerLangError> {
        let name = self.next_word()?;
        self.parser.expect_token(&Token::LBrace)?;
        
        let mut statements = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            statements.push(self.parse_statement()?);
        }
        
        self.parser.expect_token(&Token::RBrace)?;
        
        Ok(CompInstance { name, statements })
    }
    
    fn parse_statement(&mut self) -> Result<Statement, AvengerLangError> {
        // Parse and advance past the optional qualifier
        let qualifier = self.parse_qualifier();

        let kind = self.peek_word()?;
        match kind.as_str() {
            "val" => {
                self.next_word()?;
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
                self.next_word()?;
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
                self.next_word()?;
                let ty = self.parse_type()?;
                let name = self.next_word()?;
                self.parser.expect_token(&Token::Colon)?;
                let value = self.parser.parse_query()?;
                self.parser.expect_token(&Token::SemiColon)?;
                Ok(Statement::DatasetPropDecl(DatasetPropDecl { 
                    name, value, qualifier, type_: ty 
                }))
            }
            "comp" => {
                self.next_word()?;
                let ty = self.parse_type()?;
                let name = self.next_word()?;
                self.parser.expect_token(&Token::Colon)?;
                let value = self.parse_comp_instance()?;
                Ok(Statement::CompPropDecl(CompPropDecl {
                    name: format!("_comp_{}", self.parser.index()),
                    value,
                    qualifier,
                    type_: ty,
                }))
            }
            "import" => {
                let value = self.parse_import()?;
                Ok(Statement::Import(value))
            }
            "component" => {
                let value = self.parse_component_def()?;
                Ok(Statement::ComponentDef(value))
            }
            "fn" => {
                let value = self.parse_function_def()?;
                Ok(Statement::FunctionDef(value))
            }
            "if" => {
                let value = self.parse_conditional_if_components()?;
                Ok(Statement::ConditionalIfComponents(value))
            }
            "match" => {
                let value = self.parse_conditional_match_components()?;
                Ok(Statement::ConditionalMatchComponents(value))
            }
            // Handle binding
            name if self.parser.peek_nth_token_ref(1).token == Token::Assignment => {
                // Consume the name
                self.next_word()?;
                self.parser.expect_token(&Token::Assignment)?;
                let expr_or_query = self.parse_sql_expr_or_query(true)?;
                Ok(Statement::PropBinding(PropBinding { name: name.to_string(), value: expr_or_query }))
            }
            // Handle anonymouse component instance
            _ if self.parser.peek_nth_token_ref(1).token == Token::LBrace => {
                let comp_instance = self.parse_comp_instance()?;
                Ok(Statement::CompPropDecl(CompPropDecl {
                    name: format!("_comp_{}", self.parser.index()),
                    value: comp_instance,
                    qualifier: None,
                    type_: None,
                }))
            }
            _ => {
                Err(AvengerLangError::UnexpectedToken(kind))
            }
        }
    }

    fn parse_sql_expr_or_query(&mut self, required_semi_colon: bool) -> Result<SqlExprOrQuery, AvengerLangError> {
        let index = self.parser.index();
        
        let next_token = self.parser.peek_token_ref().token.clone();
        let expr_or_query = if let Ok(sql_query) = self.parser.parse_query() {
            let value = SqlExprOrQuery::Query(sql_query);
            value
        } else {
            // Reset index to the index we were at prior to the attempt to parse as a query
            while self.parser.index() > index {
                self.parser.prev_token();
            }
            // Consume the assignment token if we jumped back to it
            let _ = self.parser.expect_token(&Token::Assignment);
            if let Ok(sql_expr) = self.parser.parse_expr() {
                let value = SqlExprOrQuery::Expr(sql_expr);
                value
            } else {
                return Err(AvengerLangError::UnexpectedToken(next_token.to_string()))
            }
        };

        // Consume optional semi-colon
        if required_semi_colon || self.parser.peek_token_ref().token == Token::SemiColon {
            self.parser.expect_token(&Token::SemiColon)?;
        }

        Ok(expr_or_query)
    }

    /// Parses an import statement
    /// 
    /// import 'path/to/Component';
    /// import 'path/to/Component' as alias;
    fn parse_import(&mut self) -> Result<Import, AvengerLangError> {
        self.expect_word("import")?;

        self.parser.expect_token(&Token::LBrace)?;

        let mut items = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            let name = self.next_word()?;
            let alias = if self.peek_word() == Ok("as".to_string()) {
                self.next_word()?;
                Some(self.next_word()?)
            } else {
                None
            };
            items.push(ImportItem { name, alias });

            // If the next token is a comma, consume it
            // This handles trailing commas
            if self.parser.peek_token_ref().token == Token::Comma {
                self.parser.next_token();
            }
        }
        self.parser.expect_token(&Token::RBrace)?;

        // Check for explicit import path
        let from = if self.peek_word() == Ok("from".to_string()) {
            self.next_word()?;
            let from = self.expect_single_quoted_string()?;
            // Semi-colon only required if there is a from path;
            self.parser.expect_token(&Token::SemiColon)?;
            Some(from)
        } else {
            None
        };

        Ok(Import { from, items })
    }

    /// Parses a component definition
    /// 
    /// component ComponentName inherits ParentComponent {
    ///     statements*
    /// }
    fn parse_component_def(&mut self) -> Result<ComponentDef, AvengerLangError> {
        self.expect_word("component")?;

        let name = self.next_word()?;
        self.expect_word("inherits")?;
        let inherits = self.next_word()?;

        self.parser.expect_token(&Token::LBrace)?;

        let mut statements = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            statements.push(self.parse_statement()?);
        }
        self.parser.expect_token(&Token::RBrace)?;
        Ok(ComponentDef { name, inherits, statements })
    }

    fn parse_function_param_kind(&mut self) -> Result<FunctionParamKind, AvengerLangError> {
        match self.next_word()?.as_str() {
            "val" => Ok(FunctionParamKind::Val),
            "expr" => Ok(FunctionParamKind::Expr),
            "dataset" => Ok(FunctionParamKind::Dataset),
            t => Err(AvengerLangError::UnexpectedToken(t.to_string()))
        }
    }

    fn parse_function_def(&mut self) -> Result<FunctionDef, AvengerLangError> {
        self.expect_word("fn")?;
        let name = self.next_word()?;
        self.parser.expect_token(&Token::LParen)?;

        // Check if first arg is self
        let is_method = if self.peek_word() == Ok("self".to_string()) {
            // Consume the self keyword and the comma if it exists
            self.next_word()?;
            if self.parser.peek_token_ref().token == Token::Comma {
                self.parser.next_token();
            }
            true
        } else {
            false
        };

        let mut params = Vec::new();

        while self.parser.peek_token_ref().token != Token::RParen {
            let kind = self.parse_function_param_kind()?;
            let name = self.next_word()?;
            let ty = self.parse_type()?;
            params.push(FunctionParam { 
                name, 
                type_: ty, 
                kind 
            });

            // If the next token is a comma, consume it
            // This handles trailing commas
            if self.parser.peek_token_ref().token == Token::Comma {
                self.parser.next_token();
            }
        }
        self.parser.expect_token(&Token::RParen)?;

        self.parser.expect_token(&Token::Arrow)?;
        let return_kind = self.parse_function_param_kind()?;
        let return_type = self.parse_type()?;
        let return_param = FunctionReturnParam { kind: return_kind, type_: return_type };

        // Open function body
        self.parser.expect_token(&Token::LBrace)?;

        // Parse statements until return statement
        let mut statements = Vec::new();
        while self.peek_word() != Ok("return".to_string()) {
            let stmt = self.parse_statement()?;
            let function_stmt = FunctionStatement::try_from(stmt)?;
            statements.push(function_stmt);
        }

        // Parse return statement
        self.expect_word("return")?;
        let expr_or_query = self.parse_sql_expr_or_query(false)?;
        let return_statement = ReturnStatement{ value: expr_or_query };
        
        // Close function body
        self.parser.expect_token(&Token::RBrace)?;

        Ok(FunctionDef { name, is_method, params, statements, return_param, return_statement })
    }

    fn parse_conditional_if_branch(&mut self) -> Result<ConditionalIfBranch, AvengerLangError> {
        self.expect_word("if")?;
        self.parser.expect_token(&Token::LParen)?;
        let condition = self.parser.parse_expr()?;
        self.parser.expect_token(&Token::RParen)?;
        self.parser.expect_token(&Token::LBrace)?;

        let mut statements = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            statements.push(ConditionalComponentsStatement::try_from(self.parse_statement()?)?);
        }
        self.parser.expect_token(&Token::RBrace)?;
        Ok(ConditionalIfBranch { condition, statements })
    }

    fn parse_conditional_if_components(&mut self) -> Result<ConditionalIfComponents, AvengerLangError> {
        let mut branches = Vec::new();
        let else_statements = loop {
            branches.push(self.parse_conditional_if_branch()?);
            if self.peek_word() != Ok("else".to_string()) {
                // Finished parsing branches and there is no else branch
                break None;
            }
            // Consume the else keyword
            self.next_word()?;

            if self.peek_word() == Ok("if".to_string()) {
                // Parse the next if-else branch
                branches.push(self.parse_conditional_if_branch()?);
            } else {
                // Parse the else branch
                let mut else_statements = Vec::new();
                self.parser.expect_token(&Token::LBrace)?;
                while self.parser.peek_token_ref().token != Token::RBrace {
                    else_statements.push(
                        ConditionalComponentsStatement::try_from(self.parse_statement()?)?
                    );
                }
                self.parser.expect_token(&Token::RBrace)?;
                break Some(else_statements);
            }
        };

        Ok(ConditionalIfComponents { if_branches: branches, else_branch: else_statements })
    }


    fn parse_conditional_match_branch(&mut self) -> Result<ConditionalMatchBranch, AvengerLangError> {
        let match_value = self.expect_single_quoted_string()?;
        self.parser.expect_token(&Token::RArrow)?;

        self.parser.expect_token(&Token::LBrace)?;
        let mut statements = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            statements.push(ConditionalComponentsStatement::try_from(self.parse_statement()?)?);
        }
        self.parser.expect_token(&Token::RBrace)?;

        Ok(ConditionalMatchBranch { match_value, statements })
    }

    fn parse_conditional_match_default_branch(&mut self) -> Result<ConditionalMatchDefaultBranch, AvengerLangError> {
        self.expect_word("_")?;
        self.parser.expect_token(&Token::RArrow)?;
        self.parser.expect_token(&Token::LBrace)?;
        let mut statements = Vec::new();
        while self.parser.peek_token_ref().token != Token::RBrace {
            statements.push(ConditionalComponentsStatement::try_from(self.parse_statement()?)?);
        }
        self.parser.expect_token(&Token::RBrace)?;

        Ok(ConditionalMatchDefaultBranch { statements })
    }

    fn parse_conditional_match_components(&mut self) -> Result<ConditionalMatchComponents, AvengerLangError> {
        self.expect_word("match")?;
        self.parser.expect_token(&Token::LParen)?;
        let match_expr = self.parser.parse_expr()?;
        self.parser.expect_token(&Token::RParen)?;
        self.parser.expect_token(&Token::LBrace)?;

        let mut branches = Vec::new();
        let default_branch = loop {
            let next_token = self.parser.peek_token().token;
            println!("next_token: {:?}", next_token);

            if next_token == Token::RBrace {
                // Finished parsing branches and there is no default branch
                self.parser.expect_token(&Token::RBrace)?;
                break None;
            } else if self.peek_word() == Ok("_".to_string()) {
                // Found default branch
                break Some(self.parse_conditional_match_default_branch()?);
            } else {
                // Found non-default branch
                branches.push(self.parse_conditional_match_branch()?);
            }
        };

        Ok(ConditionalMatchComponents { match_expr, branches, default_branch })
    }


    fn parse_file(&mut self) -> Result<AvengerFile, AvengerLangError> {
        let mut imports = Vec::new();
        while self.peek_word() == Ok("import".to_string()) {
            imports.push(self.parse_import()?);
        }
        let main_component = self.parse_comp_instance()?;
        self.parser.expect_token(&Token::EOF)?;
        Ok(AvengerFile { 
            imports, 
            main_component 
        })
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

    pub fn parse_single_file(input: &str) -> Result<AvengerFile, AvengerLangError> {
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(input)?;
        let mut parser = parser.with_tokens_with_locations(tokens);
        let file = parser.parse_file()?;
        Ok(file)
    }

    pub fn parse_single_query(input: &str) -> Result<Box<SqlQuery>, AvengerLangError> {
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(input)?;
        let mut parser = parser.with_tokens_with_locations(tokens);
        let query = parser.parser.parse_query()?;
        Ok(query)
    }

    pub fn parse_single_expr(input: &str) -> Result<SqlExpr, AvengerLangError> {
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(input)?;
        let mut parser = parser.with_tokens_with_locations(tokens);
        let expr = parser.parser.parse_expr()?;
        Ok(expr)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_file() {
        let src = r#"
        // This is a comment
        Group {
            import { ComponentA, CompB }
            in val<int> my_val: 1 + 23;
            dataset my_dataset: select * from @my_table LIMIT 12;
            out expr my_expr: @my_val + 1;

            fn my_fn(val my_val) -> val {
                val inner_val: 10;
                return @my_val + @inner_val;
            }

            if (@my_val > 10) {
                val another_val: 10;
                Rect {
                    x := @another_val;
                    y := 20;
                    width := 100;
                    height := 100;
                }
            } else {
                val another_val: 20;
                Arc {
                    x := 10;
                    y := 20;
                    radius := @another_val;
                }
            }
        }
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
        import { ComponentA, CompB }
        import { Slider as MySlider, } from 'avenger/widgets';
        Group {
            in expr my_expr: ("a" + 1) * 3;

            fn my_fn(self, val my_val) -> expr { 
                expr inner_expr: @self.my_expr + 1;
                return @my_val + @inner_expr;
            }

            match (@my_val) {
                'a' => {
                    val another_val: 10;
                    Rect {
                        x := @another_val;
                    }
                }
                'b' => {
                    val another_val: 20;
                    comp foo: Arc {
                        x := @another_val;
                    }
                }
                _ => {
                    val another_val: 30;
                    Rect {
                        x := @another_val;
                    }
                }
            }
        }
        "#;
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(src).unwrap();
        let mut parser = parser.with_tokens_with_locations(tokens);
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
    }

    #[test]
    fn test_parse_file_binding() {
        let src = r#"
        Group {
            component MyComp inherits Rect {
                in val my_val: null;
            }
            comp my_comp: MyComp {
                my_val := 1 + 23;
            }
 
            my_expr := ("a" + 1) * 3;
        }
        "#;
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(src).unwrap();
        let mut parser = parser.with_tokens_with_locations(tokens);
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
    }

    #[test]
    fn test_parse_comp_declaration() {
        let src = r#"
        // This is a component
        Group {
            out comp<Widget> my_comp: MyComponent {
                val internal_val: 42;
                expr result: @internal_val * 2;
            }
            Rect {
                x := 10;
                y := 20;
                width := 100;
                height := 100;
            }
        }
        "#;
        let parser = AvengerParser::new();
        let tokens = parser.tokenize(src).unwrap();
        let mut parser = parser.with_tokens_with_locations(tokens);
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
    }
}

