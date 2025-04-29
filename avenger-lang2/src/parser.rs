use std::any::TypeId;
use std::path::PathBuf;
use std::fs::{self, File};
use std::io::Read;

use lazy_static::lazy_static;
use sqlparser::ast::{Ident, Spanned};
use sqlparser::dialect::{Dialect, GenericDialect, SnowflakeDialect};
use sqlparser::parser::{Parser as SqlParser, ParserError};
use sqlparser::tokenizer::{Token, TokenWithSpan, Tokenizer};

use crate::ast::{AvengerFile, AvengerProject, ComponentProp, DatasetProp, ExprProp, FunctionDef, FunctionParam, FunctionReturn, FunctionReturnParam, FunctionStatement, Identifier, ImportItem, ImportStatement, KeywordAs, KeywordComp, KeywordDataset, KeywordExpr, KeywordFn, KeywordFrom, KeywordImport, KeywordIn, KeywordOut, KeywordReturn, KeywordVal, ParamKind, PropBinding, Qualifier, SqlExprOrQuery, Statement, Type, ValProp};
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
    pub src: &'a str,
    pub name: &'a str,
    pub path: &'a str,
}


impl<'a> AvengerParser<'a> {
    pub fn new(src: &'a str, name: &'a str, path: &'a str) -> Result<Self, AvengerLangError> {
        let tokens = Tokenizer::new(&*AVENGER_SQL_DIALECT, src).tokenize_with_location()?;
        Ok(Self {
            parser: SqlParser::new(&*AVENGER_SQL_DIALECT).with_tokens_with_locations(tokens.clone()),
            tokens,
            src,
            name,
            path,
        })
    }

    pub fn parse(&mut self) -> Result<AvengerFile, AvengerLangError> {
        let statements = self.parse_statements()?;
        // Expect end of file
        self.parser.expect_token(&Token::EOF)?;
        Ok(AvengerFile { name: self.name.to_string(), path: self.path.to_string(), statements })
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
    fn expect_ident(&mut self, expected: Option<&str>, msg: &str) -> Result<Ident, ParserError> {
        let token = self.parser.next_token();
        match &token.token {
            Token::Word(w) => {
                if let Some(expected) = expected {
                    if w.value != expected {
                        return self.parser.expected(expected, token);
                    }
                }
                Ok(Ident { 
                    value: w.value.clone(),
                    quote_style: w.quote_style,
                    span: token.span,
                })
            },
            _ => self.parser.expected(msg, token),
        }
    }

    fn expect_single_quoted_string(&mut self) -> Result<Ident, ParserError> {
        let next_token = self.parser.next_token();
        match next_token.token {
            Token::SingleQuotedString(s) => Ok(Ident { value: s, quote_style: Some('\''), span: next_token.span }),
            _ => self.parser.expected("single quoted string", next_token),
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, AvengerLangError> {
        // Get the first identifier
        let mut ident = self.expect_ident(None, "start of statement")?;
        // See if we have a qualifier
        let qualifier = match ident.value.as_str() {
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
        match ident.value.as_str() {
            "import" => Ok(Statement::Import(self.parse_import_statement(qualifier, ident)?)),
            "val" => Ok(Statement::ValProp(self.parse_val_statement(qualifier, ident)?)),
            "expr" => Ok(Statement::ExprProp(self.parse_expr_statement(qualifier, ident)?)),
            "dataset" => Ok(Statement::DatasetProp(self.parse_dataset_statement(qualifier, ident)?)),
            "comp" => Ok(Statement::ComponentProp(self.parse_comp_prop_statement(qualifier, ident)?)),
            "fn" => Ok(Statement::FunctionDef(self.parse_fn_statement(qualifier, ident)?)),
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
                Ok(Statement::ComponentProp(
                    self.parse_component_value(None, None, None, ident
                )?))
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
    fn parse_import_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<ImportStatement, AvengerLangError> {
        self.error_if_qualifier(qualifier, "import")?;

        // Build import keyword from statement identifier
        let import_keyword = KeywordImport { span: statement_iden.span };

        // import keyword already consumed
        self.parser.expect_token(&Token::LBrace)?;

        // Parse import items
        let mut items: Vec<ImportItem> = Vec::new();
        while self.parser.peek_token().token != Token::RBrace {

            let component = self.expect_ident(None, "import component")?;
            let (as_keyword, alias) = if self.is_ident(Some("as")) {
                // Get the as keyword
                let as_ident = self.expect_ident(Some("as"), "as")?;
                let as_keyword = KeywordAs { span: as_ident.span };

                // Get the alias
                let alias = self.expect_ident(None, "import alias")?;
                (Some(as_keyword), Some(alias))
            } else {
                (None, None)
            };
            items.push(ImportItem {
                name: component,
                as_keyword,
                alias,
            });
            // Consume comma if present
            if self.parser.peek_token().token == Token::Comma {
                self.parser.next_token();
            }
        }
        self.parser.expect_token(&Token::RBrace)?;

        // Parse from clause
        let (from_keyword, from_path) = if self.is_ident(Some("from")) {
            let from_ident = self.expect_ident(Some("from"), "from")?;
            let from_keyword = KeywordFrom { span: from_ident.span };

            // Get the path
            let path = self.expect_single_quoted_string()?;
            // semi-colon required after path
            self.parser.expect_token(&Token::SemiColon)?;
            (Some(from_keyword), Some(path))
        } else {
            (None, None)
        };

        Ok(ImportStatement {
            import_keyword,
            items,
            from_keyword,
            from_path,
        })
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

    fn parse_param_kind(&mut self) -> Result<ParamKind, AvengerLangError> {
        let ident = self.expect_ident(None, "param kind")?;
        match ident.value.as_str() {
            "val" => Ok(ParamKind::Val(KeywordVal { span: ident.span })),
            "expr" => Ok(ParamKind::Expr(KeywordExpr { span: ident.span })),
            "dataset" => Ok(ParamKind::Dataset(KeywordDataset { span: ident.span })),
            _ => Err(AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
                message: format!("invalid param kind: {}", ident.value),
                line: ident.span.start.line as i32,
                column: ident.span.start.column as i32,
                len: (ident.span.end.column - ident.span.start.column) as usize,
            })),
        }
    }

    /// Parse a val statement of the form:
    /// val <Type> foo: 23;
    fn parse_val_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<ValProp, AvengerLangError> {
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

        Ok(ValProp { qualifier, val_keyword, type_, name, expr })
    }

    fn parse_expr_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<ExprProp, AvengerLangError> {
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

        Ok(ExprProp { qualifier, expr_keyword, type_, name, expr })
    }

    fn parse_dataset_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<DatasetProp, AvengerLangError> {
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

        Ok(DatasetProp { qualifier, dataset_keyword, type_, name, query })
    }

    /// Parse a component property statement of the form:
    /// comp foo: Rect {...}
    fn parse_comp_prop_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<ComponentProp, AvengerLangError> {
        // Build comp keyword from statement identifier
        let comp_keyword = KeywordComp { span: statement_iden.span };

        // Get the component type
        let name = self.expect_ident(None, "comp property name")?;

        // Colon
        self.parser.expect_token(&Token::Colon)?;

        // Component name
        let component_name = self.expect_ident(None, "component name")?;

        // Get the statements
        self.parse_component_value(qualifier, Some(comp_keyword), Some(name), component_name)
    }

    /// Parse a component value statement of the form:
    /// Rect {...}
    fn parse_component_value(
        &mut self, 
        qualifier: Option<Qualifier>, 
        keyword: Option<KeywordComp>, 
        prop_name: Option<Ident>, 
        component_name: Ident
    ) -> Result<ComponentProp, AvengerLangError> {

        // Open brace
        self.parser.expect_token(&Token::LBrace)?;

        // Parse statements
        let statements = self.parse_statements()?;

        // Close brace
        self.parser.expect_token(&Token::RBrace)?;

        Ok(ComponentProp { 
            qualifier,
            component_keyword: keyword,
            prop_name, 
            component_name, 
            statements, 
        })
    }

 

    /// Parse a function statement of the form:
    /// fn foo() -> val { ... }
    /// fn foo(val bar, expr baz) -> dataset { ... }
    /// fn foo(val <int> bar, expr <string> baz) -> dataset { ... }
    fn parse_fn_statement(&mut self, qualifier: Option<Qualifier>, statement_iden: Ident) -> Result<FunctionDef, AvengerLangError> {
        self.error_if_qualifier(qualifier, "fn")?;

        // Build import keyword from statement identifier
        let fn_keyword = KeywordFn { span: statement_iden.span };

        // Get the name
        let name = self.expect_ident(None, "function name")?;

        // Open parenthesis
        self.parser.expect_token(&Token::LParen)?;

        // Parse parameters
        let params = self.parse_fn_parameters()?;

        // Close parenthesis
        self.parser.expect_token(&Token::RParen)?;

        // Parse return kind and type
        self.parser.expect_token(&Token::Arrow)?;
        let return_kind = self.parse_param_kind()?;
        let return_type_ = self.parse_type()?;
        let return_param = FunctionReturnParam { type_: return_type_, kind: return_kind };

        // Parse statements
        self.parser.expect_token(&Token::LBrace)?;
        let statements = self.parse_fn_body_statements()?;
        let return_statement = self.parse_function_return()?;
        self.parser.expect_token(&Token::RBrace)?;

        // Semi-colon required after return type
        Ok(FunctionDef {
            fn_keyword,
            name,
            params,
            return_param,
            statements,
            return_statement,
        })
    }

    fn parse_fn_parameters(&mut self) -> Result<Vec<FunctionParam>, AvengerLangError> {
        let mut parameters = Vec::new();
        while self.parser.peek_token().token != Token::RParen {
            let kind = self.parse_param_kind()?;
            let type_ = self.parse_type()?;
            let name = self.expect_ident(None, "parameter name")?;
            parameters.push(FunctionParam { kind, type_, name });

            // Consume comma if present
            if self.parser.peek_token().token == Token::Comma {
                self.parser.next_token();
            }
        }
        Ok(parameters)
    }

    fn parse_fn_body_statements(&mut self) -> Result<Vec<FunctionStatement>, AvengerLangError> {
        let mut statements = Vec::new();
        while !self.is_ident(Some("return")) {
            statements.push(self.parse_function_body_statement()?);
        }
        Ok(statements)
    }

    fn parse_function_return(&mut self) -> Result<FunctionReturn, AvengerLangError> {
        let return_ident = self.expect_ident(Some("return"), "return")?;
        let return_keyword = KeywordReturn { span: return_ident.span };
        let return_expr_or_query = self.parse_sql_expr_or_query(false)?;
        Ok(FunctionReturn { keyword: return_keyword, value: return_expr_or_query })
    }

    fn parse_function_body_statement(&mut self) -> Result<FunctionStatement, AvengerLangError> {
        // Get the first identifier
        let ident = self.expect_ident(None, "start of statement")?;
        match ident.value.as_str() {
            "val" => Ok(FunctionStatement::ValProp(self.parse_val_statement(None, ident)?)),
            "expr" => Ok(FunctionStatement::ExprProp(self.parse_expr_statement(None, ident)?)),
            "dataset" => Ok(FunctionStatement::DatasetProp(self.parse_dataset_statement(None, ident)?)),
            _ => Err(AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
                message: format!("invalid function statement: {}", ident.value),
                line: ident.span.start.line as i32,
                column: ident.span.start.column as i32,
                len: (ident.span.end.column - ident.span.start.column) as usize,
            })),
        }
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

/// Parse a project from the given path, recursively finding all Avenger files (.avgr extension)
pub fn parse_project(project_path: &PathBuf) -> Result<AvengerProject, AvengerLangError> {
    let mut files = Vec::new();
    
    if project_path.is_file() {
        // Parse a single file
        parse_single_file(project_path, "", &mut files)?;
    } else if project_path.is_dir() {
        // Recursively parse all files in directory
        parse_directory(project_path, "", &mut files)?;
    } else {
        return Err(AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
            message: format!("Path '{}' is neither a file nor a directory", project_path.display()),
            line: 0,
            column: 0,
            len: 0,
        }));
    }
    
    Ok(AvengerProject { 
        files: files.into_iter().map(|file| (file.name.clone(), file)).collect() 
    })
}

fn parse_single_file(file_path: &PathBuf, rel_dir: &str, files: &mut Vec<AvengerFile>) -> Result<(), AvengerLangError> {
    // Only parse files with .avgr or .avenger extension
    let extension = file_path.extension().and_then(|ext| ext.to_str());
    if !matches!(extension, Some("avgr") | Some("avenger")) {
        return Ok(());
    }
    
    // Read file contents
    let mut file = File::open(file_path).map_err(|e| AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
        message: format!("Failed to open file '{}': {}", file_path.display(), e),
        line: 0,
        column: 0,
        len: 0,
    }))?;
    
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(|e| AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
        message: format!("Failed to read file '{}': {}", file_path.display(), e),
        line: 0,
        column: 0,
        len: 0,
    }))?;
    
    // Get file name without extension
    let file_name = file_path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    
    // Use rel_dir as the path (without including the filename)
    let rel_path = rel_dir.to_string();
    
    // Parse file
    let mut parser = AvengerParser::new(&contents, file_name, &rel_path).map_err(|e| {
        e.pretty_print(&contents, file_name).unwrap();
        return e;
    })?;
    let file = parser.parse().map_err(|e| {
        e.pretty_print(&contents, file_name).unwrap();
        return e;
    })?;
    files.push(file);
    
    Ok(())
}

fn parse_directory(dir_path: &PathBuf, rel_dir: &str, files: &mut Vec<AvengerFile>) -> Result<(), AvengerLangError> {
    // Read directory entries
    let entries = fs::read_dir(dir_path).map_err(|e| AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
        message: format!("Failed to read directory '{}': {}", dir_path.display(), e),
        line: 0,
        column: 0,
        len: 0,
    }))?;
    
    // Process each entry
    for entry in entries {
        let entry = entry.map_err(|e| AvengerLangError::PositionalParseError(PositionalParseErrorInfo {
            message: format!("Failed to read directory entry: {}", e),
            line: 0,
            column: 0,
            len: 0,
        }))?;
        
        let path = entry.path();
        
        if path.is_file() {
            parse_single_file(&path, rel_dir, files)?;
        } else if path.is_dir() {
            // Get directory name
            let dir_name = path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
                
            // Create new relative directory path for this entry
            let new_rel_dir = if rel_dir.is_empty() {
                dir_name.to_string()
            } else {
                format!("{}/{}", rel_dir, dir_name)
            };
            
            // Recursively process subdirectories
            parse_directory(&path, &new_rel_dir, files)?;
        }
    }
    
    Ok(())
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
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        assert_eq!(file.statements.len(), 3);
    }

    #[test]
    fn test_parse_val_statement() {
        let sql = r#"
        val foo: 23;
        "#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_expr_statement() {
        let sql = r#"
        expr foo: 23;
        "#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_dataset_statement() {
        let sql = r#"
        dataset foo: select * from bar;
        "#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_component_statement() {
        let sql = r#"
comp foo: Rect {}"#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();

        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }


    #[test]
    fn test_parse_binding_statement() {
        let sql = r#"
        foo := select * from bar;
        "#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 1);
    }

    #[test]
    fn test_parse_function_statement() {
        let sql = r#"
        fn foo() -> val { return 23 }
        fn bar() -> dataset { return select * from bar }
        fn baz(val a, expr b) -> dataset { return select * from bar }
        fn qux(val <int> a, expr <string> b) -> val <string> { return "hello" }
        "#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
        let file = parser.parse().unwrap();
        println!("{:#?}", file);
        assert_eq!(file.statements.len(), 4);
    }

    #[test]
    fn try_error_for_missing_semi_colon() {
        let sql = r#"
        dataset foo: select * from bar
"#;
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
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
        let mut parser = AvengerParser::new(sql, "test.avgr", "").unwrap();
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
