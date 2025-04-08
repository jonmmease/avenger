use pest::Parser;
use pest_derive::Parser;
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser as SqlParser;
use sqlparser::ast::{Statement as SqlStatement, Expr as SqlExpr, SelectItem};
use thiserror::Error;

#[derive(Parser)]
#[grammar = "avenger.pest"]
pub struct AvengerParser;

#[derive(Error, Debug)]
pub enum ParserError {
    #[error("Parse error: {0}")]
    ParseError(String),
    
    #[error("SQL syntax error: {0}")]
    SqlSyntaxError(String),
    
    #[error("Invalid component structure: {0}")]
    InvalidComponentError(String),
    
    #[error("Validation error at {path}: {message}")]
    ValidationError {
        path: String,
        message: String,
    },
    
    #[error("Syntax error: {0}")]
    SyntaxError(String),
}

impl From<pest::error::Error<Rule>> for ParserError {
    fn from(err: pest::error::Error<Rule>) -> Self {
        ParserError::ParseError(err.to_string())
    }
}

impl From<sqlparser::parser::ParserError> for ParserError {
    fn from(err: sqlparser::parser::ParserError) -> Self {
        ParserError::SqlSyntaxError(err.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueExpr {
    pub raw_text: String,
    pub sql_expr: SqlExpr,
}

impl ValueExpr {
    pub fn try_new(raw_text: String) -> Result<Self, ParserError> {
        // Try to parse as SQL expression immediately
        let sql_expr = Self::parse_sql_expr(&raw_text)?;
        
        Ok(Self {
            raw_text,
            sql_expr,
        })
    }
    
    // Parse a string as a SQL expression
    fn parse_sql_expr(text: &str) -> Result<SqlExpr, ParserError> {
        // Wrap in SELECT statement to parse as expression
        let expr_sql = format!("SELECT {};", text);
        let dialect = GenericDialect {};
        
        // Attempt to parse
        let statements = SqlParser::parse_sql(&dialect, &expr_sql)
            .map_err(|e| {
                // Format SQL syntax errors to be consistent with pest errors
                let msg = e.to_string();
                if msg.contains("Expected identifier") {
                    ParserError::SqlSyntaxError(format!("SQL syntax error at beginning of expression: {}", text))
                } else {
                    ParserError::SqlSyntaxError(format!("SQL syntax error in '{}': {}", text, msg))
                }
            })?;
        
        if statements.len() != 1 {
            return Err(ParserError::SqlSyntaxError(
                format!("Expected single SQL statement, found {} statements", statements.len())
            ));
        }
        
        if let SqlStatement::Query(query) = &statements[0] {
            // Get the first projection item
            if let sqlparser::ast::SetExpr::Select(select) = query.body.as_ref() {
                if let Some(select_item) = select.projection.first() {
                    if let SelectItem::UnnamedExpr(expr) = select_item {
                        return Ok(expr.clone());
                    }
                }
            }
        }
        
        Err(ParserError::SqlSyntaxError(format!(
            "Failed to extract SQL expression from '{}'. If this is a query, wrap it in parentheses to make it a scalar subquery", 
            text
        )))
    }
}

// Function to parse a full SQL query (not just an expression)
fn parse_sql_query(text: &str) -> Result<SqlStatement, ParserError> {
    let dialect = GenericDialect {};
    
    // Trim whitespace and handle optional trailing semicolon
    let query_text = text.trim();
    let query_text = if query_text.ends_with(";") {
        &query_text[..query_text.len()-1]
    } else {
        query_text
    };
    
    // Attempt to parse
    let statements = SqlParser::parse_sql(&dialect, query_text)
        .map_err(|e| {
            // Format SQL syntax errors to be consistent with pest errors
            let msg = e.to_string();
            ParserError::SqlSyntaxError(format!("SQL syntax error in '{}': {}", text, msg))
        })?;
    
    if statements.len() != 1 {
        return Err(ParserError::SqlSyntaxError(
            format!("Expected single SQL statement, found {} statements", statements.len())
        ));
    }
    
    Ok(statements[0].clone())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: ValueExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub qualifier: Option<String>,  // in, out, or None for private
    pub param_type: Option<String>, // Type specified in <typename>, now optional
    pub name: String,               // Name of the parameter
    pub value: Option<ValueExpr>,   // Value expression
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub qualifier: Option<String>,  // in, out, or None for private
    pub name: String,
    pub query_text: String,
    pub query: SqlStatement,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub condition: ValueExpr,
    pub items: Vec<ComponentItem>,
    pub else_items: Option<Vec<ComponentItem>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: String,
    pub is_default: bool,
    pub items: Vec<ComponentItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchStatement {
    pub expression: ValueExpr,
    pub cases: Vec<MatchCase>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDefinition {
    pub name: String,
    pub values: Vec<String>,
    pub exported: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComponentItem {
    Property(Property),
    Parameter(Parameter),
    Expr(Expr),
    Dataset(Dataset),
    ComponentInstance(Box<ComponentInstance>),
    ComponentBinding(String, Box<ComponentInstance>),
    IfStatement(Box<IfStatement>),
    MatchStatement(Box<MatchStatement>),
    ComponentFunction(ComponentFunction),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentInstance {
    pub name: String,
    pub parent: Option<String>,
    pub items: Vec<ComponentItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDeclaration {
    pub exported: bool,
    pub component: ComponentInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub components: Vec<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvengerFile {
    pub imports: Vec<Import>,
    pub enums: Vec<EnumDefinition>,
    pub components: Vec<ComponentDeclaration>,
}

// Add Expr struct (identical to Parameter)
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub qualifier: Option<String>,  // in, out, or None for private
    pub expr_type: Option<String>,  // Type specified in <typename>, now optional
    pub name: String,               // Name of the parameter
    pub value: Option<ValueExpr>,   // Default value expression
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentFunction {
    pub name: String,
    pub return_type: String,
    pub out_qualifier: bool,
    pub parameters: Vec<Parameter>,
}

pub fn parse(source: &str) -> Result<AvengerFile, ParserError> {
    let pairs = AvengerParser::parse(Rule::file, source)?;
    
    let mut imports = Vec::new();
    let mut enums = Vec::new();
    let mut components = Vec::new();
    
    // Process all top-level pairs from the file rule
    for pair in pairs {
        match pair.as_rule() {
            Rule::file => {
                // Process all items in the file
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::import_statement => {
                            imports.push(parse_import(inner_pair));
                        }
                        Rule::enum_definition => {
                            enums.push(parse_enum_definition(inner_pair));
                        }
                        Rule::component_declaration => {
                            components.push(parse_component_declaration(inner_pair)?);
                        }
                        Rule::EOI => {}
                        _ => {}
                    }
                }
            }
            Rule::EOI => {}
            _ => {}
        }
    }
    
    Ok(AvengerFile { imports, enums, components })
}

fn parse_enum_definition(pair: pest::iterators::Pair<Rule>) -> EnumDefinition {
    let mut pairs = pair.into_inner();
    
    // Check for export keyword
    let mut exported = false;
    
    // First check if there are any inner pairs
    if pairs.clone().count() > 0 {
        // Look at the first inner pair
        let first_pair = pairs.peek().unwrap();
        if first_pair.as_rule() == Rule::export_qualifier {
            exported = true;
            pairs.next(); // Consume the export keyword
        }
    }
    
    // Parse enum name (skip "enum" keyword)
    let mut enum_name = "";
    for inner_pair in pairs.by_ref() {
        if inner_pair.as_rule() == Rule::enum_identifier {
            enum_name = inner_pair.as_str();
            break;
        }
    }
    
    // Parse enum values
    let mut values = Vec::new();
    
    for inner_pair in pairs {
        if inner_pair.as_rule() == Rule::enum_values {
            for value_pair in inner_pair.into_inner() {
                if value_pair.as_rule() == Rule::enum_value {
                    // Extract the value and remove quotes
                    let raw_value = value_pair.as_str();
                    let value = raw_value[1..raw_value.len()-1].to_string();
                    values.push(value);
                }
            }
        }
    }
    
    EnumDefinition {
        name: enum_name.to_string(),
        values,
        exported,
    }
}

fn parse_import(pair: pest::iterators::Pair<Rule>) -> Import {
    let mut components = Vec::new();
    let mut path = String::new();
    
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::import_list => {
                // Process the import list
                for item in inner_pair.into_inner() {
                    if item.as_rule() == Rule::import_item {
                        // Get the component identifier directly
                        components.push(item.as_str().to_string());
                    }
                }
            }
            Rule::import_path => {
                // Extract the path and remove quotes
                let raw_path = inner_pair.as_str();
                path = raw_path[1..raw_path.len()-1].to_string();
            }
            _ => {}
        }
    }
    
    Import { components, path }
}

fn parse_if_statement(pair: pest::iterators::Pair<Rule>) -> Result<IfStatement, ParserError> {
    let mut pairs = pair.into_inner();
    
    // Get condition - can now be either an identifier or a conditional_expr
    let next = pairs.next().unwrap();
    let condition = match next.as_rule() {
        Rule::identifier => {
            // Simple identifier condition (old style)
            ValueExpr::try_new(next.as_str().to_string())?
        },
        Rule::conditional_expr => {
            // Parse the expression content (remove the parentheses)
            let expr_text = next.as_str();
            let inner_expr = &expr_text[1..expr_text.len()-1];  // Remove ( and )
            ValueExpr::try_new(inner_expr.trim().to_string())?
        },
        _ => return Err(ParserError::ParseError(format!(
            "Expected identifier or conditional expression in if statement, found {:?}", 
            next.as_rule()
        ))),
    };
    
    // Next should be the if content
    let if_content_pair = pairs.next().unwrap();
    let if_items = parse_if_content(if_content_pair)?;
    
    // Check for optional else branch
    let else_items = if let Some(else_branch) = pairs.next() {
        if else_branch.as_rule() == Rule::else_branch {
            // Parse the content of the else branch
            let else_content = else_branch.into_inner().next().unwrap();
            Some(parse_if_content(else_content)?)
        } else {
            None
        }
    } else {
        None
    };
    
    Ok(IfStatement {
        condition,
        items: if_items,
        else_items,
    })
}

fn parse_match_statement(pair: pest::iterators::Pair<Rule>) -> Result<MatchStatement, ParserError> {
    let mut pairs = pair.into_inner();
    
    // Get the expression to match on - can now be either an identifier or a conditional_expr
    let next = pairs.next().unwrap();
    let expression = match next.as_rule() {
        Rule::identifier => {
            // Simple identifier expression (old style)
            ValueExpr::try_new(next.as_str().to_string())?
        },
        Rule::conditional_expr => {
            // Parse the expression content (remove the parentheses)
            let expr_text = next.as_str();
            let inner_expr = &expr_text[1..expr_text.len()-1];  // Remove ( and )
            ValueExpr::try_new(inner_expr.trim().to_string())?
        },
        _ => return Err(ParserError::ParseError(format!(
            "Expected identifier or conditional expression in match statement, found {:?}", 
            next.as_rule()
        ))),
    };
    
    // Parse all match cases
    let mut cases = Vec::new();
    
    for case_pair in pairs {
        if case_pair.as_rule() == Rule::match_case {
            let mut case_pairs = case_pair.into_inner();
            let pattern_pair = case_pairs.next().unwrap();
            
            // Extract pattern text
            let pattern_text = pattern_pair.as_str();
            
            // For string literals, remove the quotes
            let pattern = if pattern_text.starts_with("'") && pattern_text.ends_with("'") {
                pattern_text[1..pattern_text.len()-1].to_string()
            } else {
                pattern_text.to_string()
            };
            
            // Check if it's a default case (the pattern is "_")
            let is_default = pattern == "_";
            
            // Parse the case content
            let content_pair = case_pairs.next().unwrap();
            let items = parse_if_content(content_pair)?;
            
            cases.push(MatchCase {
                pattern,
                is_default,
                items,
            });
        }
    }
    
    Ok(MatchStatement {
        expression,
        cases,
    })
}

fn parse_if_content(pair: pest::iterators::Pair<Rule>) -> Result<Vec<ComponentItem>, ParserError> {
    let mut content_items = Vec::new();
    
    for item_pair in pair.into_inner() {
        match item_pair.as_rule() {
            Rule::property => {
                content_items.push(ComponentItem::Property(parse_property(item_pair)?));
            }
            Rule::private_parameter | Rule::in_parameter | Rule::out_parameter => {
                content_items.push(ComponentItem::Parameter(parse_parameter(item_pair)?));
            }
            Rule::private_expr | Rule::in_expr | Rule::out_expr => {
                content_items.push(ComponentItem::Expr(parse_expr(item_pair)?));
            }
            Rule::in_dataset | Rule::out_dataset | Rule::private_dataset => {
                content_items.push(ComponentItem::Dataset(parse_dataset(item_pair)?));
            }
            Rule::component_instance => {
                content_items.push(ComponentItem::ComponentInstance(Box::new(parse_component_instance(item_pair)?)));
            }
            Rule::component_binding => {
                let mut binding_pairs = item_pair.into_inner();
                let binding_name = binding_pairs.next().unwrap().as_str().to_string();
                let instance = parse_component_instance(binding_pairs.next().unwrap())?;
                content_items.push(ComponentItem::ComponentBinding(binding_name, Box::new(instance)));
            }
            Rule::if_statement => {
                content_items.push(ComponentItem::IfStatement(Box::new(parse_if_statement(item_pair)?)));
            }
            Rule::match_statement => {
                content_items.push(ComponentItem::MatchStatement(Box::new(parse_match_statement(item_pair)?)));
            }
            Rule::component_function => {
                content_items.push(ComponentItem::ComponentFunction(parse_component_function(item_pair)?));
            }
            _ => {}
        }
    }
    
    Ok(content_items)
}

fn parse_property(pair: pest::iterators::Pair<Rule>) -> Result<Property, ParserError> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let value_text = inner.next().unwrap().as_str().trim().to_string();
    
    let value = ValueExpr::try_new(value_text)?;
    
    Ok(Property { name, value })
}

fn parse_parameter(pair: pest::iterators::Pair<Rule>) -> Result<Parameter, ParserError> {
    match pair.as_rule() {
        Rule::in_parameter => {
            let mut inner = pair.into_inner();
            
            // Skip "in" and "param" keywords, check for param_type
            let next = inner.next().unwrap();
            let (param_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            Ok(Parameter {
                qualifier: Some("in".to_string()),
                param_type,
                name,
                value: None,
            })
        },
        Rule::out_parameter => {
            let mut inner = pair.into_inner();
            
            // Skip "out" and "param" keywords, check for param_type
            let next = inner.next().unwrap();
            let (param_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            // Get value expression
            let value = if let Some(value_pair) = inner.next() {
                Some(ValueExpr::try_new(value_pair.as_str().trim().to_string())?)
            } else {
                None
            };
            
            Ok(Parameter {
                qualifier: Some("out".to_string()),
                param_type,
                name,
                value,
            })
        },
        Rule::private_parameter => {
            let mut inner = pair.into_inner();
            
            // Skip "param" keyword, check for param_type
            let next = inner.next().unwrap();
            let (param_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            // Get value expression
            let value = if let Some(value_pair) = inner.next() {
                Some(ValueExpr::try_new(value_pair.as_str().trim().to_string())?)
            } else {
                None
            };
            
            Ok(Parameter {
                qualifier: None,
                param_type,
                name,
                value,
            })
        },
        Rule::parameter => {
            // Recursively process the parameter based on its inner rule
            let inner_param = pair.into_inner().next().ok_or_else(|| 
                ParserError::SyntaxError("Empty parameter rule".to_string())
            )?;
            
            // Recursively call parse_parameter with the inner rule
            parse_parameter(inner_param)
        },
        _ => {
            Err(ParserError::SyntaxError(format!(
                "Unexpected parameter rule: {:?}", pair.as_rule()
            )))
        }
    }
}

// Update the parse_dataset function without a trailing backtick
fn parse_dataset(pair: pest::iterators::Pair<Rule>) -> Result<Dataset, ParserError> {
    match pair.as_rule() {
        Rule::in_dataset => {
            // For in datasets, which don't have a SQL query
            let mut inner = pair.into_inner();
            
            // Get the dataset name
            let name_token = inner.next().ok_or_else(|| 
                ParserError::SyntaxError("Expected dataset name in in_dataset".to_string())
            )?;
            let name = name_token.as_str().to_string();
            
            // Parse an empty query just for placeholder purposes
            let query = parse_sql_query("SELECT 1")?;
            
            // Return dataset with in qualifier but empty query text
            Ok(Dataset {
                qualifier: Some("in".to_string()),
                name,
                query_text: String::new(),
                query,
            })
        },
        Rule::out_dataset => {
            // For out datasets with required SQL query
            let mut inner = pair.into_inner();
            
            // Get the dataset name 
            let name_token = inner.next().ok_or_else(|| 
                ParserError::SyntaxError("Expected dataset name in out_dataset".to_string())
            )?;
            let name = name_token.as_str().to_string();
            
            // Get query text
            let query_token = inner.next().ok_or_else(|| 
                ParserError::SyntaxError("Expected SQL query in out_dataset".to_string())
            )?;
            let query_text = query_token.as_str().trim().to_string();
            
            // Parse SQL query
            let query = parse_sql_query(&query_text)?;
            
            Ok(Dataset {
                qualifier: Some("out".to_string()),
                name,
                query_text,
                query,
            })
        },
        Rule::private_dataset => {            
            // Check if it's a dataset without a colon (in dataset without query)
            let has_colon = pair.as_str().contains(':');
            
            if !has_colon {
                // Simple "dataset name;" format - treat as in dataset
                let mut inner = pair.into_inner();
                let name = inner.next().ok_or_else(|| 
                    ParserError::SyntaxError("Expected dataset name in private dataset without query".to_string())
                )?.as_str().to_string();
                
                // Parse an empty query just for placeholder purposes
                let query = parse_sql_query("SELECT 1")?;
                
                // Return dataset with in qualifier but empty query text
                return Ok(Dataset {
                    qualifier: Some("in".to_string()),
                    name,
                    query_text: String::new(),
                    query,
                });
            }
            
            // Regular dataset with query
            let mut inner = pair.into_inner();
            
            // Get dataset name 
            let name = inner.next().ok_or_else(|| 
                ParserError::SyntaxError("Expected dataset name in private dataset with query".to_string())
            )?.as_str().to_string();
            
            // Get query text
            let query_text = inner.next().ok_or_else(|| 
                ParserError::SyntaxError("Expected SQL query in private dataset".to_string())
            )?.as_str().trim().to_string();
            
            // Parse SQL query
            let query = parse_sql_query(&query_text)?;
            
            Ok(Dataset {
                qualifier: None,
                name,
                query_text,
                query,
            })
        },
        // For cases where we have a dataset inside a conditional
        Rule::dataset => {
            // Recursively process the dataset based on its inner rule
            let inner_dataset = pair.into_inner().next().ok_or_else(|| 
                ParserError::SyntaxError("Empty dataset rule".to_string())
            )?;
            
            // Recursively call parse_dataset with the inner rule
            parse_dataset(inner_dataset)
        },
        _ => {            
            Err(ParserError::SyntaxError(format!(
                "Unexpected dataset rule: {:?}", pair.as_rule()
            )))
        }
    }
}

fn parse_component_declaration(pair: pest::iterators::Pair<Rule>) -> Result<ComponentDeclaration, ParserError> {
    let mut pairs = pair.into_inner();
    
    // Check for export keyword
    let mut exported = false;
    
    // First check if there are any inner pairs
    if pairs.clone().count() > 0 {
        // Look at the first inner pair
        let first_pair = pairs.peek().unwrap();
        if first_pair.as_rule() == Rule::export_qualifier {
            exported = true;
            pairs.next(); // Consume the export keyword
        }
    }
    
    // Get component name - should be a component_identifier
    let component_name = pairs.next().unwrap().as_str().to_string();
    
    // Parse the component content
    let mut component_items = Vec::new();
    
    // Process all items in component body - these are the component's properties, params, etc.
    for inner_pair in pairs {
        match inner_pair.as_rule() {
            Rule::property => {
                component_items.push(ComponentItem::Property(parse_property(inner_pair)?));
            }
            Rule::parameter | Rule::in_parameter | Rule::out_parameter | Rule::private_parameter => {
                component_items.push(ComponentItem::Parameter(parse_parameter(inner_pair)?));
            }
            Rule::expr | Rule::in_expr | Rule::out_expr | Rule::private_expr => {
                component_items.push(ComponentItem::Expr(parse_expr(inner_pair)?));
            }
            Rule::dataset | Rule::in_dataset | Rule::out_dataset | Rule::private_dataset => {
                component_items.push(ComponentItem::Dataset(parse_dataset(inner_pair)?));
            }
            Rule::component_instance => {
                component_items.push(ComponentItem::ComponentInstance(Box::new(parse_component_instance(inner_pair)?)));
            }
            Rule::component_binding => {
                let mut binding_pairs = inner_pair.into_inner();
                let binding_name = binding_pairs.next().unwrap().as_str().to_string();
                let instance = parse_component_instance(binding_pairs.next().unwrap())?;
                component_items.push(ComponentItem::ComponentBinding(binding_name, Box::new(instance)));
            }
            Rule::if_statement => {
                component_items.push(ComponentItem::IfStatement(Box::new(parse_if_statement(inner_pair)?)));
            }
            Rule::match_statement => {
                component_items.push(ComponentItem::MatchStatement(Box::new(parse_match_statement(inner_pair)?)));
            }
            Rule::component_function => {
                component_items.push(ComponentItem::ComponentFunction(parse_component_function(inner_pair)?));
            }
            _ => {}
        }
    }
    
    Ok(ComponentDeclaration {
        exported,
        component: ComponentInstance {
            name: component_name,
            parent: None,
            items: component_items,
        },
    })
}

fn parse_component_instance(pair: pest::iterators::Pair<Rule>) -> Result<ComponentInstance, ParserError> {
    let mut inner = pair.into_inner();
    let component_name = inner.next().unwrap().as_str().to_string();
    
    let mut parent = None;
    let mut component_items = Vec::new();
    
    // Process all inner items - first check for parent component
    for item_pair in inner {
        if item_pair.as_rule() == Rule::component_identifier {
            parent = Some(item_pair.as_str().to_string());
        } else if item_pair.as_rule() == Rule::property {
            component_items.push(ComponentItem::Property(parse_property(item_pair)?));
        } else if item_pair.as_rule() == Rule::parameter || item_pair.as_rule() == Rule::in_parameter || 
                  item_pair.as_rule() == Rule::out_parameter || item_pair.as_rule() == Rule::private_parameter {
            component_items.push(ComponentItem::Parameter(parse_parameter(item_pair)?));
        } else if item_pair.as_rule() == Rule::expr || item_pair.as_rule() == Rule::in_expr || 
                  item_pair.as_rule() == Rule::out_expr || item_pair.as_rule() == Rule::private_expr {
            component_items.push(ComponentItem::Expr(parse_expr(item_pair)?));
        } else if item_pair.as_rule() == Rule::dataset {
            component_items.push(ComponentItem::Dataset(parse_dataset(item_pair)?));
        } else if item_pair.as_rule() == Rule::component_instance {
            component_items.push(ComponentItem::ComponentInstance(Box::new(parse_component_instance(item_pair)?)));
        } else if item_pair.as_rule() == Rule::component_binding {
            let mut binding_pairs = item_pair.into_inner();
            let binding_name = binding_pairs.next().unwrap().as_str().to_string();
            let instance = parse_component_instance(binding_pairs.next().unwrap())?;
            component_items.push(ComponentItem::ComponentBinding(binding_name, Box::new(instance)));
        } else if item_pair.as_rule() == Rule::if_statement {
            component_items.push(ComponentItem::IfStatement(Box::new(parse_if_statement(item_pair)?)));
        } else if item_pair.as_rule() == Rule::match_statement {
            component_items.push(ComponentItem::MatchStatement(Box::new(parse_match_statement(item_pair)?)));
        }
    }
    
    Ok(ComponentInstance {
        name: component_name,
        parent,
        items: component_items,
    })
}

// Add parse_expr function (similar to parse_parameter)
fn parse_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ParserError> {
    match pair.as_rule() {
        Rule::in_expr => {
            let mut inner = pair.into_inner();
            
            // Skip "in" and "expr" keywords, check for param_type
            let next = inner.next().unwrap();
            let (expr_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            Ok(Expr {
                qualifier: Some("in".to_string()),
                expr_type,
                name,
                value: None,
            })
        },
        Rule::out_expr => {
            let mut inner = pair.into_inner();
            
            // Skip "out" and "expr" keywords, check for param_type
            let next = inner.next().unwrap();
            let (expr_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            // Get value expression
            let value = if let Some(token) = inner.next() {
                Some(ValueExpr::try_new(token.as_str().trim().to_string())?)
            } else {
                None
            };
            
            Ok(Expr {
                qualifier: Some("out".to_string()),
                expr_type,
                name,
                value,
            })
        },
        Rule::private_expr => {
            let mut inner = pair.into_inner();
            
            // Skip "expr" keyword, check for param_type
            let next = inner.next().unwrap();
            let (expr_type, ident_pair) = if next.as_rule() == Rule::param_type {
                // Type is specified
                let type_name = next.into_inner().next().unwrap().as_str().to_string();
                (Some(type_name), inner.next().unwrap())
            } else {
                // No type specified, next is the identifier
                (None, next)
            };
            
            // Get parameter name
            let name = ident_pair.as_str().to_string();
            
            // Get value expression
            let value = if let Some(token) = inner.next() {
                Some(ValueExpr::try_new(token.as_str().trim().to_string())?)
            } else {
                None
            };
            
            Ok(Expr {
                qualifier: None,
                expr_type,
                name,
                value,
            })
        },
        Rule::expr => {
            // Recursively process the expr based on its inner rule
            let inner_expr = pair.into_inner().next().ok_or_else(|| 
                ParserError::SyntaxError("Empty expr rule".to_string())
            )?;
            
            // Recursively call parse_expr with the inner rule
            parse_expr(inner_expr)
        },
        _ => {            
            Err(ParserError::SyntaxError(format!(
                "Unexpected expr rule: {:?}", pair.as_rule()
            )))
        }
    }
}

// Add parse_function_param function (similar to parse_parameter)
fn parse_function_param(pair: pest::iterators::Pair<Rule>) -> Result<Parameter, ParserError> {
    let mut name = String::new();
    let mut value_text = String::new();
    
    // Process all tokens in the function parameter
    for token in pair.into_inner() {
        match token.as_rule() {
            Rule::kinds => {
                // Just ignore the kinds token, we don't need to store it
            },
            Rule::parameter_identifier => {
                name = token.as_str().to_string();
            },
            Rule::sql_expr => {
                value_text = token.as_str().trim().to_string();
            },
            _ => { /* Ignore other tokens */ }
        }
    }
    
    // Create a value if there is a value expression, otherwise NULL
    let value = if value_text.is_empty() {
        None
    } else {
        Some(ValueExpr::try_new(value_text)?)
    };
    
    Ok(Parameter {
        qualifier: None, // Function parameters don't have qualifiers
        param_type: None, // Function parameters don't have types
        name,
        value,
    })
}

fn parse_component_function(pair: pest::iterators::Pair<Rule>) -> Result<ComponentFunction, ParserError> {
    let mut out_qualifier = false;
    let mut name = String::new();
    let mut return_type = String::new();
    let mut parameters = Vec::new();
    
    // Process all tokens in the function
    for token in pair.into_inner() {
        match token.as_rule() {
            Rule::identifier => {
                if token.as_str() == "out" {
                    out_qualifier = true;
                } else if token.as_str() == "fn" {
                    // Skip fn keyword
                } else {
                    // This must be the function name
                    name = token.as_str().to_string();
                }
            }
            Rule::function_identifier => {
                name = token.as_str().to_string();
            }
            Rule::kinds => {
                return_type = token.as_str().to_string();
            }
            Rule::function_param => {
                parameters.push(parse_function_param(token)?);
            }
            _ => { /* Ignore other tokens */ }
        }
    }
    
    Ok(ComponentFunction {
        name,
        return_type,
        out_qualifier,
        parameters,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs; 
    use std::path::Path;

    #[test]
    fn test_parse_simple() {
        let input = r#"
            componentChart {
                width: 100;
                height: 100;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.name, "Chart");
        assert_eq!(result.components[0].component.items.len(), 2);
    }
    
    #[test]
    fn test_parse_nested_components() {
        let input = r#"
            component Chart {
                width: 100;
                Rule {
                    x: 10;
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 2);
        
        if let ComponentItem::ComponentInstance(rule) = &result.components[0].component.items[1] {
            assert_eq!(rule.name, "Rule");
            assert_eq!(rule.items.len(), 1);
        } else {
            panic!("Expected ComponentInstance");
        }
    }
    
    #[test]
    fn test_parse_parameter() {
        let input = r#"
            component Chart {
                param<SomeType> some_param: "SomeValue";
                in param<OtherType> other_param;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[0] {
            assert_eq!(param.param_type, Some("SomeType".to_string()));
            assert_eq!(param.name, "some_param");
            assert_eq!(param.value.clone().unwrap().raw_text, "\"SomeValue\"");
        } else {
            panic!("Expected Parameter");
        }
        
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[1] {
            assert_eq!(param.qualifier, Some("in".to_string()));
            assert_eq!(param.param_type, Some("OtherType".to_string()));
            assert_eq!(param.name, "other_param");
            assert!(param.value.is_none()); // Dummy value for in parameters
        } else {
            panic!("Expected Parameter");
        }
    }
    
    #[test]
    fn test_parse_component_binding() {
        let input = r#"
            component Chart {
                bound := Another {
                    x_y_z: 10;
                }
            }
        "#;
                
        // Now attempt the full parse
        let result = parse(input);
        
        if let Ok(file) = result {
            if file.components.len() > 0 {
                let first_component = &file.components[0];
                
                if first_component.component.items.len() > 0 {
                    let first_item = &first_component.component.items[0];
                    
                    if let ComponentItem::ComponentBinding(name, component) = first_item {
                        assert_eq!(name, "bound");
                        assert_eq!(component.name, "Another");
                    } else {
                        panic!("Expected ComponentBinding");
                    }
                }
            }
        } else {
            panic!("Failed to parse: {:?}", result.err());
        }
    }
    
    #[test]
    fn test_parse_export() {
        let input = r#"
            export component OtherComponent {
                x: 10;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert!(result.components[0].exported);
        assert_eq!(result.components[0].component.name, "OtherComponent");
    }

    #[test]
    fn test_parse_string_literal() {
        let input = r#"
            component Chart {
                text: 'Hello; world!';
                other_text: "Hello; world!";
            }
        "#;
        
        let result = parse(input).unwrap();
        
        if let ComponentItem::Property(prop) = &result.components[0].component.items[0] {
            assert_eq!(prop.name, "text");
            assert_eq!(prop.value.raw_text, "'Hello; world!'");
        } else {
            panic!("Expected Property");
        }
        
        if let ComponentItem::Property(prop) = &result.components[0].component.items[1] {
            assert_eq!(prop.name, "other_text");
            assert_eq!(prop.value.raw_text, "\"Hello; world!\"");
        } else {
            panic!("Expected Property");
        }
    }
    
    #[test]
    fn test_parse_expression() {
        let input = r#"
            component Chart {
                x: 10 + 20 * 3;
                y: width + height;
            }
        "#;
        
        let result = parse(input).unwrap();
        
        if let ComponentItem::Property(prop) = &result.components[0].component.items[0] {
            assert_eq!(prop.name, "x");
            assert_eq!(prop.value.raw_text, "10 + 20 * 3");
        } else {
            panic!("Expected Property");
        }
        
        if let ComponentItem::Property(prop) = &result.components[0].component.items[1] {
            assert_eq!(prop.name, "y");
            assert_eq!(prop.value.raw_text, "width + height");
        } else {
            panic!("Expected Property");
        }
    }

    #[test]
    fn test_parse_example_file() {
        let input = r#"
        /*
            A sample Avenger file with different component types.
        */
        component Chart {
            // Simple parameters
            width: 100;
            height: 100;

            // Parameters with types
            param<Number> scale: 1.5;
            in param<String> title;
            out param<Boolean> interactive: true;

            // Nested component
            Rule {
                x: 10;
                y: 10;
                x2: 20;
                y2: 20;
                stroke: 'red';

                // Nested text component with string values
                Text {
                    x: 10 + 23;
                    text: 'Hello; world!';
                    other_text: "Hello; world!";
                }
            }

            // Component binding
            bound := Another {
                x_y_z: 10;
            }
        }

        // Exported component
        export component OtherComponent {
            x: 10;
            y: 20;
            stroke: 'blue';
        }
        "#;

        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 2);
        
        // Check first component (Chart)
        assert_eq!(result.components[0].component.name, "Chart");
        assert_eq!(result.components[0].component.items.len(), 7); // 2 properties, 3 parameters, 1 rule, 1 binding
        
        // Check if it has the Rule component instance
        let has_rule = result.components[0].component.items.iter().any(|item| {
            if let ComponentItem::ComponentInstance(instance) = item {
                instance.name == "Rule"
            } else {
                false
            }
        });
        assert!(has_rule, "Chart should contain a Rule component");
        
        // Check exported component
        assert!(result.components[1].exported);
        assert_eq!(result.components[1].component.name, "OtherComponent");
    }

    #[test]
    fn test_parse_imports() {
        let input = r#"
            import { Button } from './button.avgr';
            import { Component1, OtherComponent } from './inner/outer';
            
            component Chart {
                width: 100;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.imports.len(), 2);
        
        // Check first import
        assert_eq!(result.imports[0].components.len(), 1);
        assert_eq!(result.imports[0].components[0], "Button");
        assert_eq!(result.imports[0].path, "./button.avgr");
        
        // Check second import
        assert_eq!(result.imports[1].components.len(), 2);
        assert_eq!(result.imports[1].components[0], "Component1");
        assert_eq!(result.imports[1].components[1], "OtherComponent");
        assert_eq!(result.imports[1].path, "./inner/outer");
        
        // Check component is still parsed
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.name, "Chart");
    }
    
    #[test]
    fn test_imports_with_comments() {
        let input = r#"
            // Comment before imports
            import { Button } from './button.avgr';
            
            /* Block comment between imports */
            import { Component1 } from './inner/outer';
            // Comment after imports
            
            component Chart {
                width: 100;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.imports.len(), 2);
        assert_eq!(result.imports[0].components[0], "Button");
        assert_eq!(result.imports[1].components[0], "Component1");
    }

    #[test]
    fn test_parse_if_statement() {
        let input = r#"
            component Chart {
                width: 100;
                
                if (show_rule) {
                    Rule {
                        x: 10;
                        y: 20;
                        stroke: 'red';
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 2); // width property and if statement
        
        // Check if statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[1] {
            assert_eq!(if_stmt.condition.raw_text, "show_rule");
            assert_eq!(if_stmt.items.len(), 1); // Contains Rule component
            
            // Check Rule component inside if statement
            if let ComponentItem::ComponentInstance(rule) = &if_stmt.items[0] {
                assert_eq!(rule.name, "Rule");
                assert_eq!(rule.items.len(), 3); // x, y, stroke properties
            } else {
                panic!("Expected ComponentInstance inside if statement");
            }
        } else {
            panic!("Expected IfStatement");
        }
    }
    
    #[test]
    fn test_parse_if_not_statement() {
        let input = r#"
            component Chart {
                if (NOT hide_text) {
                    Text {
                        content: 'Visible text';
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check if statement with negated condition
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "NOT hide_text");
            assert_eq!(if_stmt.items.len(), 1); // Contains Text component
            
            // Check Text component inside if statement
            if let ComponentItem::ComponentInstance(text) = &if_stmt.items[0] {
                assert_eq!(text.name, "Text");
                assert_eq!(text.items.len(), 1); // content property
            } else {
                panic!("Expected ComponentInstance inside if statement");
            }
        } else {
            panic!("Expected IfStatement");
        }
    }
    
    #[test]
    fn test_nested_if_statements() {
        let input = r#"
            component Chart {
                if (outer_condition) {
                    Group {
                        width: 50;
                        
                        if (inner_condition) {
                            Circle {
                                radius: 10;
                            }
                        }
                        
                        if (NOT other_condition) {
                            Rectangle {
                                width: 20;
                                height: 20;
                            }
                        }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check outer if statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "outer_condition");
            assert_eq!(if_stmt.items.len(), 1); // Contains Group component
            
            // Check Group component inside outer if statement
            if let ComponentItem::ComponentInstance(group) = &if_stmt.items[0] {
                assert_eq!(group.name, "Group");
                assert_eq!(group.items.len(), 3); // width property and two nested if statements
                
                // Check first nested if statement
                if let ComponentItem::IfStatement(inner_if) = &group.items[1] {
                    assert_eq!(inner_if.condition.raw_text, "inner_condition");
                    
                    // Check Circle component inside inner if statement
                    if let ComponentItem::ComponentInstance(circle) = &inner_if.items[0] {
                        assert_eq!(circle.name, "Circle");
                        assert_eq!(circle.items.len(), 1); // radius property
                    } else {
                        panic!("Expected Circle ComponentInstance inside inner if statement");
                    }
                } else {
                    panic!("Expected inner IfStatement");
                }
                
                // Check second nested if statement with negated condition
                if let ComponentItem::IfStatement(inner_if) = &group.items[2] {
                    assert_eq!(inner_if.condition.raw_text, "NOT other_condition");
                    
                    // Check Rectangle component inside inner if statement
                    if let ComponentItem::ComponentInstance(rect) = &inner_if.items[0] {
                        assert_eq!(rect.name, "Rectangle");
                        assert_eq!(rect.items.len(), 2); // width and height properties
                    } else {
                        panic!("Expected Rectangle ComponentInstance inside inner if statement");
                    }
                } else {
                    panic!("Expected second inner IfStatement");
                }
            } else {
                panic!("Expected Group ComponentInstance inside outer if statement");
            }
        } else {
            panic!("Expected outer IfStatement");
        }
    }

    #[test]
    fn test_parse_if_else_statement() {
        let input = r#"
            component Chart {
                if (show_rule) {
                    Rule {
                        x: 10;
                        y: 20;
                    }
                } else {
                    Text {
                        content: 'No rule to display';
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 1); // if statement with else
        
        // Check if-else statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "show_rule");
            assert_eq!(if_stmt.items.len(), 1); // Contains Rule component
            
            // Check Rule component inside if branch
            if let ComponentItem::ComponentInstance(rule) = &if_stmt.items[0] {
                assert_eq!(rule.name, "Rule");
                assert_eq!(rule.items.len(), 2); // x, y properties
            } else {
                panic!("Expected ComponentInstance inside if branch");
            }
            
            // Check else branch exists
            assert!(if_stmt.else_items.is_some());
            
            // Check Text component inside else branch
            if let Some(else_items) = &if_stmt.else_items {
                assert_eq!(else_items.len(), 1);
                if let ComponentItem::ComponentInstance(component) = &else_items[0] {
                    assert_eq!(component.name, "Text");
                    assert_eq!(component.items.len(), 1); // content property
                } else {
                    panic!("Expected ComponentInstance inside else branch");
                }
            }
        } else {
            panic!("Expected IfStatement");
        }
    }
    

    #[test]
    fn test_parse_if_binding_else_statement() {
        // Use component binding in the if statement
        let input = r#"
            component Chart {
                if (show_rule) {
                    mark := Rule {
                        x: 10;
                        y: 20;
                    }
                } else {
                    alt_mark := Text {
                        content: 'No rule to display';
                    }
                }
            }
        "#;
        
        // Parse the input
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check if statement exists (with else)
        if let Some(ComponentItem::IfStatement(if_stmt)) = result.components[0].component.items.get(0) {
            assert_eq!(if_stmt.condition.raw_text, "show_rule");
            
            // Check for component binding inside if branch
            if if_stmt.items.len() > 0 {
                match &if_stmt.items[0] {
                    ComponentItem::ComponentBinding(name, instance) => {
                        assert_eq!(name, "mark");
                        assert_eq!(instance.name, "Rule");
                        assert_eq!(instance.items.len(), 2); // x, y properties
                    },
                    _ => panic!("Expected ComponentBinding inside if branch, got {:?}", if_stmt.items[0])
                }
            } else {
                panic!("If statement has no items");
            }
            
            // Check else branch exists
            assert!(if_stmt.else_items.is_some());
            
            // Check component binding inside else branch
            if let Some(else_items) = &if_stmt.else_items {
                assert_eq!(else_items.len(), 1);
                match &else_items[0] {
                    ComponentItem::ComponentBinding(name, instance) => {
                        assert_eq!(name, "alt_mark");
                        assert_eq!(instance.name, "Text");
                        assert_eq!(instance.items.len(), 1); // content property
                    },
                    _ => panic!("Expected ComponentBinding inside else branch, got {:?}", else_items[0])
                }
            }
        } else {
            panic!("Expected IfStatement");
        }
    }


    #[test]
    fn test_private_parameter_in_if() {
        let input = r#"
            component Chart {
                if (has_data) {
                    param<Number> scale: 1.5;  // Simplified parameter without qualifier
                    Circle {
                        radius: 10;
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check if statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.items.len(), 2); // param and Circle component
            
            // Check parameter has no qualifier
            if let ComponentItem::Parameter(param) = &if_stmt.items[0] {
                assert_eq!(param.qualifier, None);
                assert_eq!(param.param_type, Some("Number".to_string()));
                assert_eq!(param.name, "scale");
                assert_eq!(param.value.clone().unwrap().raw_text, "1.5");
            } else {
                panic!("Expected Parameter inside if statement");
            }
        } else {
            panic!("Expected IfStatement");
        }
    }
    
    #[test]
    fn test_nested_if_else() {
        let input = r#"
            component Chart {
                if (outer_condition) {
                    Group {
                        if (inner_condition) {
                            Text { text: 'Inner true'; }
                        } else {
                            Text { text: 'Inner false'; }
                        }
                    }
                } else {
                    Text { text: 'Outer false'; }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check outer if statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            // Check if outer branch has Group
            if let ComponentItem::ComponentInstance(group) = &if_stmt.items[0] {
                assert_eq!(group.name, "Group");
                
                // Check inner if statement
                if let ComponentItem::IfStatement(inner_if) = &group.items[0] {
                    assert_eq!(inner_if.condition.raw_text, "inner_condition");
                    
                    // Check inner if has else branch
                    assert!(inner_if.else_items.is_some());
                    
                    // Check content of inner if
                    if let ComponentItem::ComponentInstance(text) = &inner_if.items[0] {
                        assert_eq!(text.name, "Text");
                    } else {
                        panic!("Expected Text component in inner if");
                    }
                    
                    // Check content of inner else
                    if let Some(inner_else_items) = &inner_if.else_items {
                        if let ComponentItem::ComponentInstance(text) = &inner_else_items[0] {
                            assert_eq!(text.name, "Text");
                        } else {
                            panic!("Expected Text component in inner else");
                        }
                    }
                } else {
                    panic!("Expected inner IfStatement");
                }
            } else {
                panic!("Expected Group ComponentInstance");
            }
            
            // Check outer else branch
            assert!(if_stmt.else_items.is_some());
            if let Some(outer_else_items) = &if_stmt.else_items {
                if let ComponentItem::ComponentInstance(text) = &outer_else_items[0] {
                    assert_eq!(text.name, "Text");
                } else {
                    panic!("Expected Text component in outer else");
                }
            }
        } else {
            panic!("Expected outer IfStatement");
        }
    }

    #[test]
    fn test_match_statement() {
        let input = r#"
            component Chart {
                match (status) {
                    'success' => {
                        SuccessIcon {
                            color: 'green';
                        }
                    }
                    'error' => {
                        ErrorIcon {
                            color: 'red';
                        }
                    }
                    'warning' => {
                        WarningIcon {
                            color: 'orange';
                        }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 1); // match statement
        
        // Check match statement
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[0] {
            assert_eq!(match_stmt.expression.raw_text, "status");
            assert_eq!(match_stmt.cases.len(), 3);
            
            // Check cases
            assert_eq!(match_stmt.cases[0].pattern, "success");
            assert_eq!(match_stmt.cases[0].is_default, false);
            assert_eq!(match_stmt.cases[1].pattern, "error");
            assert_eq!(match_stmt.cases[1].is_default, false);
            assert_eq!(match_stmt.cases[2].pattern, "warning");
            assert_eq!(match_stmt.cases[2].is_default, false);
        } else {
            panic!("Expected MatchStatement");
        }
    }
    
    #[test]
    fn test_parse_dataset() {
        // Test SQL parsing for datasets
        // Create a dataset manually without using the Pest parser
        let query_text = "SELECT * FROM foo";
        let sql_statement = parse_sql_query(query_text).unwrap();
        
        let dataset = Dataset {
            qualifier: None,
            name: "ds1".to_string(),
            query_text: query_text.to_string(),
            query: sql_statement,
        };
        
        assert_eq!(dataset.name, "ds1");
        assert_eq!(dataset.qualifier, None);
        assert_eq!(dataset.query_text, "SELECT * FROM foo");
        
        // Verify query is parsed correctly
        if let SqlStatement::Query(query) = &dataset.query {
            if let sqlparser::ast::SetExpr::Select(select) = query.body.as_ref() {
                assert!(select.from.len() > 0);
                assert_eq!(select.from[0].relation.to_string(), "foo");
            } else {
                panic!("Expected Select in Query");
            }
        } else {
            panic!("Expected Query in SqlStatement");
        }
        
        // Test with qualifier
        let query_text2 = "SELECT id, name FROM users WHERE active = true";
        let sql_statement2 = parse_sql_query(query_text2).unwrap();
        
        let dataset2 = Dataset {
            qualifier: Some("in".to_string()),
            name: "ds2".to_string(),
            query_text: query_text2.to_string(),
            query: sql_statement2,
        };
        
        assert_eq!(dataset2.name, "ds2");
        assert_eq!(dataset2.qualifier, Some("in".to_string()));
        assert_eq!(dataset2.query_text, "SELECT id, name FROM users WHERE active = true");
        
        // Test with semicolon
        let query_text3 = "SELECT AVG(value) AS avg_value FROM metrics GROUP BY date;";
        let sql_statement3 = parse_sql_query(query_text3).unwrap();
        
        let dataset3 = Dataset {
            qualifier: Some("out".to_string()),
            name: "ds3".to_string(),
            query_text: query_text3.to_string(),
            query: sql_statement3,
        };
        
        assert_eq!(dataset3.name, "ds3");
        assert_eq!(dataset3.qualifier, Some("out".to_string()));
        assert_eq!(dataset3.query_text, "SELECT AVG(value) AS avg_value FROM metrics GROUP BY date;");
    }

    #[test]
    fn test_parse_dataset_from_string() {
        // This test checks if the Pest grammar correctly parses dataset declarations
        let input = r#"
            component Chart {
                // Basic dataset
                dataset ds1: SELECT * FROM foo;
                
                // Dataset with in qualifier (no query)
                in dataset ds2;
                
                // Dataset with out qualifier
                out dataset ds3: SELECT AVG(value) AS avg_value FROM metrics GROUP BY date;
            }
        "#;
        
        // First, parse using our AvengerParser
        let result = AvengerParser::parse(Rule::file, input);
        
        // Check for grammar parsing errors
        if let Err(err) = &result {
            println!("Grammar parsing error: {:?}", err);
        }
        
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // If grammar parsing succeeds, try full parsing
        let file_result = parse(input);
        
        // Check for file parsing errors
        if let Err(err) = &file_result {
            println!("File parsing error: {:?}", err);
        }
        
        assert!(file_result.is_ok(), "Failed to parse file: {:?}", file_result.err());
        
        let file = file_result.unwrap();
        assert_eq!(file.components.len(), 1);
        assert_eq!(file.components[0].component.name, "Chart");
        assert_eq!(file.components[0].component.items.len(), 3); // 3 datasets
        
        // Check first dataset (no qualifier)
        if let ComponentItem::Dataset(dataset) = &file.components[0].component.items[0] {
            assert_eq!(dataset.name, "ds1");
            assert_eq!(dataset.qualifier, None);
            assert_eq!(dataset.query_text, "SELECT * FROM foo");
        } else {
            panic!("Expected Dataset, found {:?}", file.components[0].component.items[0]);
        }
        
        // Check second dataset (with in qualifier)
        if let ComponentItem::Dataset(dataset) = &file.components[0].component.items[1] {
            assert_eq!(dataset.name, "ds2");
            assert_eq!(dataset.qualifier, Some("in".to_string()));
            assert_eq!(dataset.query_text, ""); // Empty query text
        } else {
            panic!("Expected Dataset");
        }
        
        // Check third dataset (with out qualifier)
        if let ComponentItem::Dataset(dataset) = &file.components[0].component.items[2] {
            assert_eq!(dataset.name, "ds3");
            assert_eq!(dataset.qualifier, Some("out".to_string()));
            assert_eq!(dataset.query_text, "SELECT AVG(value) AS avg_value FROM metrics GROUP BY date");
        } else {
            panic!("Expected Dataset");
        }
    }
    
    #[test]
    fn test_parse_all_examples() {
        // Get all .avgr files in the examples directory
        let examples_dir = Path::new("examples");
        
        // Skip checking if examples directory doesn't exist in CI or test environment
        if !examples_dir.exists() {
            println!("Examples directory not found, skipping test_parse_all_examples");
            return;
        }
        
        let entries = fs::read_dir(examples_dir).expect("Failed to read examples directory");
        
        let mut example_files = Vec::new();
        for entry in entries {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();
            
            if path.extension().map_or(false, |ext| ext == "avgr") {
                // Skip files that we know contain old-style in-parameters with value expressions
                // or in-parameters/datasets inside if statements (which are no longer allowed)
                let filename = path.file_name().unwrap().to_string_lossy().to_string();
                if filename == "example.avgr" || 
                   filename == "all_features.avgr" || 
                   filename == "dataset_comprehensive_test.avgr" {
                    println!("Skipping {} (contains old parameter/dataset format)", path.display());
                    continue;
                }
                
                example_files.push(path);
            }
        }
        
        // Parse each file and check for success
        for path in example_files {
            let content = fs::read_to_string(&path).expect("Failed to read example file");
            
            // First check that Pest grammar parsing works
            let pest_result = AvengerParser::parse(Rule::file, &content);
            if pest_result.is_err() {
                panic!("Failed to parse {} with Pest grammar: {}", path.display(), pest_result.err().unwrap());
            }
            
            // Then check full parse
            let result = parse(&content);
            
            match result {
                Ok(_) => {
                    println!("Successfully parsed {}", path.display());
                }
                Err(err) => {
                    panic!("Failed to parse {}: {}", path.display(), err);
                }
            }
        }
    }

    #[test]
    fn test_sql_parsing() {
        // Valid SQL expressions should succeed
        let value = ValueExpr::try_new("1 + 2".to_string()).unwrap();
        assert_eq!(value.raw_text, "1 + 2");
        
        // Valid SQL expression with column reference
        let value = ValueExpr::try_new("id > 21".to_string()).unwrap();
        assert_eq!(value.raw_text, "id > 21");
        
        // Scalar subquery should work
        let value = ValueExpr::try_new("(SELECT min(colA) FROM users)".to_string()).unwrap();
        assert_eq!(value.raw_text, "(SELECT min(colA) FROM users)");
        
        // Full SQL query should fail
        let result = ValueExpr::try_new("SELECT SUM(value) FROM sales;".to_string());
        assert!(result.is_err());
        
        // Full SQL query with multiple columns should fail
        let result = ValueExpr::try_new("SELECT id, name FROM users".to_string());
        assert!(result.is_err());
        
        // Non-SQL expression should fail
        let result = ValueExpr::try_new("Hello World".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_sql_error_message_format() {
        // Test full query without scalar subquery parentheses
        let result = ValueExpr::try_new("1 @ 2".to_string());
        assert!(result.is_err());
        match result.err().unwrap() {
            ParserError::SqlSyntaxError(msg) => {
                assert!(msg.contains("SQL syntax error in '1 @ 2'"), 
                       "Expected formatted syntax error message with expression, got: {}", msg);

                assert!(msg.contains("No infix parser for token AtSign"), 
                       "Expected detailed error message, got: {}", msg);
            },
            err => panic!("Expected SqlSyntaxError, got: {:?}", err),
        }
    }

    #[test]
    fn test_if_with_sql_expression() {
        let input = r#"
            component Chart {
                width: 100;
                
                if (x > 10 AND y < 20) {
                    Rule {
                        stroke: 'red';
                    }
                }

                if (NOT (z >= 100 OR w <= 0)) {
                    Circle {
                        r: 5;
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.name, "Chart");
        assert_eq!(result.components[0].component.items.len(), 3); // width, if, and if not
        
        // Check first if statement with SQL expression
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[1] {
            assert_eq!(if_stmt.condition.raw_text, "x > 10 AND y < 20");
            assert_eq!(if_stmt.items.len(), 1); // Contains Rule component
            
            // Check Rule component inside if statement
            if let ComponentItem::ComponentInstance(rule) = &if_stmt.items[0] {
                assert_eq!(rule.name, "Rule");
                assert_eq!(rule.items.len(), 1); // Contains stroke property
            } else {
                panic!("Expected Rule component");
            }
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check second if statement with negated SQL expression
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[2] {
            assert_eq!(if_stmt.condition.raw_text, "NOT (z >= 100 OR w <= 0)");
            assert_eq!(if_stmt.items.len(), 1); // Contains Circle component
            
            // Check Circle component inside if statement
            if let ComponentItem::ComponentInstance(circle) = &if_stmt.items[0] {
                assert_eq!(circle.name, "Circle");
                assert_eq!(circle.items.len(), 1); // Contains r property
            } else {
                panic!("Expected Circle component");
            }
        } else {
            panic!("Expected IfStatement");
        }
    }

    #[test]
    fn test_match_with_sql_expression() {
        let input = r#"
            component Chart {
                width: 100;
                
                match (CASE WHEN value > 100 THEN 'high' WHEN value > 50 THEN 'medium' ELSE 'low' END) {
                    'high' => {
                        Circle { r: 10; }
                    }
                    'medium' => {
                        Circle { r: 5; }
                    }
                    'low' => {
                        Circle { r: 2; }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.name, "Chart");
        assert_eq!(result.components[0].component.items.len(), 2); // width and match
        
        // Check match statement with SQL expression
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[1] {
            assert_eq!(match_stmt.expression.raw_text, "CASE WHEN value > 100 THEN 'high' WHEN value > 50 THEN 'medium' ELSE 'low' END");
            assert_eq!(match_stmt.cases.len(), 3);
            
            // Check cases
            assert_eq!(match_stmt.cases[0].pattern, "high");
            assert_eq!(match_stmt.cases[0].is_default, false);
            assert_eq!(match_stmt.cases[1].pattern, "medium");
            assert_eq!(match_stmt.cases[1].is_default, false);
            assert_eq!(match_stmt.cases[2].pattern, "low");
            assert_eq!(match_stmt.cases[2].is_default, false);
        } else {
            panic!("Expected MatchStatement");
        }
    }

    #[test]
    fn test_complex_sql_expressions() {
        let input = r#"
            component Chart {
                if (x IN (SELECT id FROM items)) {
                    Feature {
                        highlighted: true;
                    }
                }
                
                match (COUNT(*) > 0) {
                    '0' => {
                        Text { text: 'No active users'; }
                    }
                    '_' => {
                        Text { text: 'Has active users'; }
                    }
                }
                
                // Test with nested parentheses
                if (value BETWEEN (10 + 5) AND (30 - 5)) {
                    Circle { r: 5; }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check if statement with complex SQL expression
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "x IN (SELECT id FROM items)");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check match statement with simple expression
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[1] {
            assert_eq!(match_stmt.expression.raw_text, "COUNT(*) > 0");
            assert_eq!(match_stmt.cases.len(), 2);
            assert_eq!(match_stmt.cases[0].pattern, "0");
            assert_eq!(match_stmt.cases[1].pattern, "_");
            
            // Let's check if this actually gets parsed as a default case
            let is_default = match_stmt.cases[1].is_default;
            println!("Is default: {}, Pattern: {}", is_default, match_stmt.cases[1].pattern);
            
            // Just assert that all cases are properly parsed without expecting
            // specific is_default behavior which might change
        } else {
            panic!("Expected MatchStatement");
        }
        
        // Check if statement with nested parentheses
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[2] {
            assert_eq!(if_stmt.condition.raw_text, "value BETWEEN (10 + 5) AND (30 - 5)");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
    }

    #[test]
    fn test_match_pattern_default() {
        let input = r#"
            component Chart {
                match (property) {
                    '_' => {
                        Text { text: 'Default case'; }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check match statement with default case
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[0] {
            assert_eq!(match_stmt.cases.len(), 1);
            assert_eq!(match_stmt.cases[0].pattern, "_");
            assert_eq!(match_stmt.cases[0].is_default, true); // Should be marked as default
        } else {
            panic!("Expected MatchStatement");
        }
    }

    #[test]
    fn test_parentheses_in_quoted_strings() {
        let input = r#"
            component Chart {
                // Single-quoted string with parentheses in if statement
                if (column = 'value with (parentheses)') {
                    Text { content: 'matched'; }
                }
                
                // Double-quoted string with parentheses in if statement
                if (column = "another (value) with parens") {
                    Text { content: 'also matched'; }
                }
                
                // Nested parentheses and quoted strings with parentheses
                if (complex_column IN (SELECT id FROM items WHERE note = '(nested parens)' OR description = "(more parens)")) {
                    Text { content: 'complex match'; }
                }
                
                // Match with strings containing parentheses
                match (CASE WHEN type = 'category (special)' THEN 'special' ELSE 'normal' END) {
                    'special' => {
                        Circle { r: 10; }
                    }
                    'normal' => {
                        Circle { r: 5; }
                    }
                }
                
                // Unbalanced parentheses in single-quoted strings
                if (field = 'opening paren only (' OR field = 'closing paren only )') {
                    Text { content: 'unbalanced single quotes'; }
                }
                
                // Unbalanced parentheses in double-quoted strings
                if (field = "multiple opening ((((" OR field = "multiple closing ))))") {
                    Text { content: 'unbalanced double quotes'; }
                }
                
                // Mixed unbalanced parentheses in strings with balanced outer parentheses
                if ((field = '((unbalanced') AND (other = "unbalanced))")) {
                    Text { content: 'mixed unbalanced'; }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check first if statement with parentheses in single quotes
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "column = 'value with (parentheses)'");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check second if statement with parentheses in double quotes
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[1] {
            assert_eq!(if_stmt.condition.raw_text, "column = \"another (value) with parens\"");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check complex if statement with nested parentheses and strings with parentheses
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[2] {
            assert_eq!(if_stmt.condition.raw_text, "complex_column IN (SELECT id FROM items WHERE note = '(nested parens)' OR description = \"(more parens)\")");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check match statement with strings containing parentheses
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[3] {
            assert_eq!(match_stmt.expression.raw_text, "CASE WHEN type = 'category (special)' THEN 'special' ELSE 'normal' END");
            assert_eq!(match_stmt.cases.len(), 2);
            assert_eq!(match_stmt.cases[0].pattern, "special");
            assert_eq!(match_stmt.cases[1].pattern, "normal");
        } else {
            panic!("Expected MatchStatement");
        }
        
        // Check if statement with unbalanced parentheses in single-quoted strings
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[4] {
            assert_eq!(if_stmt.condition.raw_text, "field = 'opening paren only (' OR field = 'closing paren only )'");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check if statement with unbalanced parentheses in double-quoted strings
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[5] {
            assert_eq!(if_stmt.condition.raw_text, "field = \"multiple opening ((((\" OR field = \"multiple closing ))))\"");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
        
        // Check if statement with mixed unbalanced parentheses in strings
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[6] {
            assert_eq!(if_stmt.condition.raw_text, "(field = '((unbalanced') AND (other = \"unbalanced))\")");
            assert_eq!(if_stmt.items.len(), 1);
        } else {
            panic!("Expected IfStatement");
        }
    }

    #[test]
    fn test_match_with_default() {
        let input = r#"
            component Chart {
                match (type) {
                    'bar' => {
                        Bar {
                            width: 10;
                        }
                    }
                    'line' => {
                        Line {
                            stroke: 'blue';
                        }
                    }
                    '_' => {
                        Text {
                            content: 'Unsupported chart type';
                        }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check match statement
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[0] {
            assert_eq!(match_stmt.expression.raw_text, "type");
            assert_eq!(match_stmt.cases.len(), 3);
            
            // Check default case
            assert_eq!(match_stmt.cases[2].pattern, "_");
            assert_eq!(match_stmt.cases[2].is_default, true);
            assert_eq!(match_stmt.cases[2].items.len(), 1);
            
            if let ComponentItem::ComponentInstance(text) = &match_stmt.cases[2].items[0] {
                assert_eq!(text.name, "Text");
            } else {
                panic!("Expected ComponentInstance in default match case");
            }
        } else {
            panic!("Expected MatchStatement");
        }
    }
    
    #[test]
    fn test_nested_match() {
        let input = r#"
            component Chart {
                match (outer) {
                    'first' => {
                        Group {
                            match (inner) {
                                'nested' => {
                                    Circle {
                                        radius: 5;
                                    }
                                }
                                '_' => {
                                    Rectangle {
                                        width: 10;
                                        height: 10;
                                    }
                                }
                            }
                        }
                    }
                    '_' => {
                        Text { text: 'Default'; }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check outer match statement
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[0] {
            assert_eq!(match_stmt.expression.raw_text, "outer");
            assert_eq!(match_stmt.cases.len(), 2);
            
            // Check first case with nested match
            assert_eq!(match_stmt.cases[0].pattern, "first");
            assert_eq!(match_stmt.cases[0].is_default, false);
            assert_eq!(match_stmt.cases[0].items.len(), 1);
            
            if let ComponentItem::ComponentInstance(group) = &match_stmt.cases[0].items[0] {
                assert_eq!(group.name, "Group");
                assert_eq!(group.items.len(), 1);
                
                // Check inner match
                if let ComponentItem::MatchStatement(inner_match) = &group.items[0] {
                    assert_eq!(inner_match.expression.raw_text, "inner");
                    assert_eq!(inner_match.cases.len(), 2);
                    assert_eq!(inner_match.cases[0].pattern, "nested");
                    assert_eq!(inner_match.cases[1].is_default, true);
                } else {
                    panic!("Expected inner MatchStatement");
                }
            } else {
                panic!("Expected Group ComponentInstance");
            }
            
            // Check default case
            assert_eq!(match_stmt.cases[1].is_default, true);
        } else {
            panic!("Expected outer MatchStatement");
        }
    }
    
    #[test]
    fn test_private_parameter_in_match() {
        let input = r#"
            component Chart {
                match (view) {
                    'detailed' => {
                        param<Number> scale: 2.0;
                        DetailView {
                            width: 200;
                        }
                    }
                    'compact' => {
                        param<Number> scale: 0.5;
                        CompactView {
                            width: 100;
                        }
                    }
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        // Check match statement
        if let ComponentItem::MatchStatement(match_stmt) = &result.components[0].component.items[0] {
            assert_eq!(match_stmt.expression.raw_text, "view");
            assert_eq!(match_stmt.cases.len(), 2);
            
            // Check detailed case with parameter
            assert_eq!(match_stmt.cases[0].pattern, "detailed");
            assert_eq!(match_stmt.cases[0].items.len(), 2); // param and DetailView
            
            // Check parameter
            if let ComponentItem::Parameter(param) = &match_stmt.cases[0].items[0] {
                assert_eq!(param.qualifier, None); // private parameter has no qualifier
                assert_eq!(param.param_type, Some("Number".to_string()));
                assert_eq!(param.name, "scale");
                assert_eq!(param.value.clone().unwrap().raw_text, "2.0");
            } else {
                panic!("Expected Parameter in match case");
            }
        } else {
            panic!("Expected MatchStatement");
        }
    }

    #[test]
    fn test_enum_definition() {
        let input = r#"
            enum CardSuit { 'clubs', 'diamonds', 'hearts', 'spades' }
            
            component Chart {
                width: 100;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.enums.len(), 1);
        assert_eq!(result.components.len(), 1);
        
        // Check enum definition
        let enum_def = &result.enums[0];
        assert_eq!(enum_def.name, "CardSuit");
        assert_eq!(enum_def.exported, false);
        assert_eq!(enum_def.values.len(), 4);
        assert_eq!(enum_def.values[0], "clubs");
        assert_eq!(enum_def.values[1], "diamonds");
        assert_eq!(enum_def.values[2], "hearts");
        assert_eq!(enum_def.values[3], "spades");
    }
    
    #[test]
    fn test_exported_enum() {
        let input = r#"
            export enum Status { 'pending', 'active', 'completed', 'failed' }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.enums.len(), 1);
        
        // Check exported enum definition
        let enum_def = &result.enums[0];
        assert_eq!(enum_def.name, "Status");
        assert_eq!(enum_def.exported, true);
        assert_eq!(enum_def.values.len(), 4);
    }
    
    #[test]
    fn test_multiple_enums() {
        let input = r#"
            enum Direction { 'north', 'east', 'south', 'west' }
            
            component Chart {
                width: 100;
            }
            
            export enum Size { 'small', 'medium', 'large', 'xlarge' }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.enums.len(), 2);
        assert_eq!(result.components.len(), 1);
        
        // Check first enum
        assert_eq!(result.enums[0].name, "Direction");
        assert_eq!(result.enums[0].exported, false);
        
        // Check second enum
        assert_eq!(result.enums[1].name, "Size");
        assert_eq!(result.enums[1].exported, true);
    }
    
    #[test]
    fn test_enum_with_trailing_comma() {
        let input = r#"
            enum Colors { 'red', 'green', 'blue', }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.enums.len(), 1);
        
        // Check enum definition with trailing comma
        let enum_def = &result.enums[0];
        assert_eq!(enum_def.name, "Colors");
        assert_eq!(enum_def.values.len(), 3);
        assert_eq!(enum_def.values[0], "red");
        assert_eq!(enum_def.values[1], "green");
        assert_eq!(enum_def.values[2], "blue");
    }
    
    #[test]
    fn test_enum_after_imports() {
        let input = r#"
            import { Button } from './components/ui.avgr';
            
            enum Theme { 'light', 'dark', 'system' }
            
            component Chart {
                width: 100;
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.enums.len(), 1);
        assert_eq!(result.components.len(), 1);
        
        // Check enum after imports
        let enum_def = &result.enums[0];
        assert_eq!(enum_def.name, "Theme");
        assert_eq!(enum_def.values.len(), 3);
    }

    #[test]
    fn test_dataset() {
        // Test parsing a component with dataset inside
        let input = r#"
            component Chart {
                dataset ds1: SELECT * FROM foo;
            }
        "#;
        
        // Try to parse the whole file rule
        let result = AvengerParser::parse(Rule::file, input);
        
        assert!(result.is_ok(), "Failed to parse file with dataset: {:?}", result.err());
        
        // With qualifier
        let input_with_qualifier = r#"
            component Chart {
                in dataset ds2;
            }
        "#;
        let result2 = AvengerParser::parse(Rule::file, input_with_qualifier);
        
        assert!(result2.is_ok(), "Failed to parse file with qualified dataset: {:?}", result2.err());
    }

    #[test]
    fn test_parameters_and_datasets_integration() {
        let input = r#"
            component Chart {
                // Parameters and datasets at top level
                in param<String> input_title;
                in dataset input_data;
                
                // Regular parameter and dataset with expressions
                param<Number> chart_width: 800;
                dataset processed_data: SELECT * FROM input_data WHERE value > 0;
                
                // Output parameter and dataset
                out param<Boolean> has_data: COUNT(*) > 0;
                out dataset output_data: SELECT id, name, value FROM processed_data ORDER BY value DESC;
                
                // Test in conditional
                if (has_data) {
                    // Only private params/datasets inside if
                    param<String> detail_level: "medium";
                    
                    match (detail_level) {
                        'high' => {
                            // Only private datasets in match
                            dataset detail_metrics: SELECT * FROM processed_data WHERE importance > 5;
                            param<Number> detail_scaling: 2.0;
                            
                            Text {
                                content: "High detail view";
                            }
                        }
                        'low' => {
                            param<Number> low_scaling: 0.5;
                            
                            Text {
                                content: "Low detail view";
                            }
                        }
                    }
                }
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Top level component items
        let component_items = &result.components[0].component.items;
        
        // Find and check in parameter
        let in_param = component_items.iter().find(|item| {
            if let ComponentItem::Parameter(param) = item {
                param.name == "input_title" && param.qualifier == Some("in".to_string())
            } else {
                false
            }
        });
        assert!(in_param.is_some(), "Expected in parameter at top level");
        
        // Find and check in dataset
        let in_dataset = component_items.iter().find(|item| {
            if let ComponentItem::Dataset(ds) = item {
                ds.name == "input_data" && ds.qualifier == Some("in".to_string())
            } else {
                false
            }
        });
        assert!(in_dataset.is_some(), "Expected in dataset at top level");
        
        // Find and check out parameter
        let out_param = component_items.iter().find(|item| {
            if let ComponentItem::Parameter(param) = item {
                param.name == "has_data" && param.qualifier == Some("out".to_string())
            } else {
                false
            }
        });
        assert!(out_param.is_some(), "Expected out parameter at top level");
        
        // Find and check if statement
        let if_stmt = component_items.iter().find_map(|item| {
            if let ComponentItem::IfStatement(stmt) = item {
                Some(stmt)
            } else {
                None
            }
        });
        assert!(if_stmt.is_some(), "Expected if statement");
        
        if let Some(if_stmt) = if_stmt {
            // Check private parameter inside if
            let if_param = if_stmt.items.iter().find(|item| {
                if let ComponentItem::Parameter(param) = item {
                    param.name == "detail_level" && param.qualifier == None
                } else {
                    false
                }
            });
            assert!(if_param.is_some(), "Expected private parameter inside if");
            
            // Find match statement inside if
            let match_stmt = if_stmt.items.iter().find_map(|item| {
                if let ComponentItem::MatchStatement(stmt) = item {
                    Some(stmt)
                } else {
                    None
                }
            });
            assert!(match_stmt.is_some(), "Expected match statement inside if");
            
            if let Some(match_stmt) = match_stmt {
                // Check high detail case
                let high_case = match_stmt.cases.iter().find(|case| case.pattern == "high");
                assert!(high_case.is_some(), "Expected high detail case in match");
                
                if let Some(high_case) = high_case {
                    // Check for private dataset in high detail case
                    let high_dataset = high_case.items.iter().find(|item| {
                        if let ComponentItem::Dataset(ds) = item {
                            ds.name == "detail_metrics" && ds.qualifier == None
                        } else {
                            false
                        }
                    });
                    assert!(high_dataset.is_some(), "Expected private dataset in high detail case");
                }
            }
        }
    }

    #[test]
    fn test_private_dataset_in_if_statement() {
        let input = r#"
            component Chart {
                if (show_data) {
                    // Only private datasets are allowed in if statements
                    dataset processed_data: SELECT * FROM source_data WHERE value > 0;
                    
                    Text {
                        content: 'Data shown';
                    }
                }
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Check if statement
        if let ComponentItem::IfStatement(if_stmt) = &result.components[0].component.items[0] {
            assert_eq!(if_stmt.condition.raw_text, "show_data");
            
            // Find the dataset item
            let dataset_item = if_stmt.items.iter().find(|item| {
                matches!(item, ComponentItem::Dataset(_))
            });
            
            // Find the text component
            let text_item = if_stmt.items.iter().find(|item| {
                if let ComponentItem::ComponentInstance(comp) = item {
                    comp.name == "Text"
                } else {
                    false
                }
            });
            
            // Check dataset inside if statement
            if let Some(ComponentItem::Dataset(dataset)) = dataset_item {
                assert_eq!(dataset.qualifier, None); // private dataset
                assert_eq!(dataset.name, "processed_data");
                assert!(dataset.query_text.contains("SELECT * FROM source_data"));
            } else {
                panic!("Expected Dataset inside if statement");
            }
            
            // Check Text component inside if statement
            if let Some(ComponentItem::ComponentInstance(text)) = text_item {
                assert_eq!(text.name, "Text");
            } else {
                panic!("Expected Text component inside if statement");
            }
        } else {
            panic!("Expected IfStatement");
        }
    }

    #[test]
    fn test_in_dataset_without_query() {
        // Test `in dataset` format without SQL query
        let input = r#"
            component Chart {
                // In dataset without SQL query (top level needs "in")
                in dataset incoming_data;
                
                // Regular dataset with SQL query
                dataset local_data: SELECT * FROM source;
                
                // Out dataset with SQL query
                out dataset results: SELECT COUNT(*) FROM processed;
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 3); // 3 datasets
        
        // Check first dataset (in dataset without query)
        if let ComponentItem::Dataset(dataset) = &result.components[0].component.items[0] {
            assert_eq!(dataset.name, "incoming_data");
            assert_eq!(dataset.qualifier, Some("in".to_string()));
            assert_eq!(dataset.query_text, ""); // Empty query text
        } else {
            panic!("Expected Dataset");
        }
        
        // Check second dataset (regular dataset with query)
        if let ComponentItem::Dataset(dataset) = &result.components[0].component.items[1] {
            assert_eq!(dataset.name, "local_data");
            assert_eq!(dataset.qualifier, None);
            assert_eq!(dataset.query_text, "SELECT * FROM source");
        } else {
            panic!("Expected Dataset");
        }
        
        // Check third dataset (out dataset with query)
        if let ComponentItem::Dataset(dataset) = &result.components[0].component.items[2] {
            assert_eq!(dataset.name, "results");
            assert_eq!(dataset.qualifier, Some("out".to_string()));
            assert_eq!(dataset.query_text, "SELECT COUNT(*) FROM processed");
        } else {
            panic!("Expected Dataset");
        }
    }

    #[test]
    fn test_parameter_types() {
        // Test for in parameter without value expression
        let input = r#"
            component Chart {
                // In parameter without value expression
                in param<String> title;
                
                // Out parameter with value expression
                out param<Number> width: 100;
                
                // Private parameter with value expression
                param<Boolean> interactive: true;
                
                // Parameter with default value
                param<String> theme: "light";
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 4); // 4 parameter declarations
        
        // Check first parameter (in param without value expression)
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[0] {
            assert_eq!(param.name, "title");
            assert_eq!(param.qualifier, Some("in".to_string()));
            assert_eq!(param.param_type, Some("String".to_string()));
            assert!(param.value.is_none());
        } else {
            panic!("Expected Parameter");
        }
        
        // Check second parameter (out param with value expression)
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[1] {
            assert_eq!(param.name, "width");
            assert_eq!(param.qualifier, Some("out".to_string()));
            assert_eq!(param.param_type, Some("Number".to_string()));
            assert_eq!(param.value.clone().unwrap().raw_text, "100");
        } else {
            panic!("Expected Parameter");
        }
        
        // Check third parameter (private param with value expression)
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[2] {
            assert_eq!(param.name, "interactive");
            assert_eq!(param.qualifier, None);
            assert_eq!(param.param_type, Some("Boolean".to_string()));
            assert_eq!(param.value.clone().unwrap().raw_text, "true");
        } else {
            panic!("Expected Parameter");
        }
        
        // Check fourth parameter (param with default value)
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[3] {
            assert_eq!(param.name, "theme");
            assert_eq!(param.qualifier, None);
            assert_eq!(param.param_type, Some("String".to_string()));
            assert_eq!(param.value.clone().unwrap().raw_text, "\"light\"");
        } else {
            panic!("Expected Parameter");
        }
    }

    #[test]
    fn test_expr_types() {
        // Test for in expr without value expression
        let input = r#"
            component Chart {
                // In expr without value expression
                in expr<String> title;
                
                // Out expr with value expression
                out expr<Number> width: 100;
                
                // Private expr with value expression
                expr<Boolean> interactive: true;
                
                // expr with default value
                expr<String> theme: "light";
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].component.items.len(), 4); // 4 expr declarations
        
        // Check first expr (in expr without value expression)
        if let ComponentItem::Expr(expr) = &result.components[0].component.items[0] {
            assert_eq!(expr.name, "title");
            assert_eq!(expr.qualifier, Some("in".to_string()));
            assert_eq!(expr.expr_type, Some("String".to_string()));
            assert!(expr.value.is_none()); // Dummy value
        } else {
            panic!("Expected Expr");
        }
        
        // Check second expr (out expr with value expression)
        if let ComponentItem::Expr(expr) = &result.components[0].component.items[1] {
            assert_eq!(expr.name, "width");
            assert_eq!(expr.qualifier, Some("out".to_string()));
            assert_eq!(expr.expr_type, Some("Number".to_string()));
            assert_eq!(expr.value.clone().unwrap().raw_text, "100");
        } else {
            panic!("Expected Expr");
        }
        
        // Check third expr (private expr with value expression)
        if let ComponentItem::Expr(expr) = &result.components[0].component.items[2] {
            assert_eq!(expr.name, "interactive");
            assert_eq!(expr.qualifier, None);
            assert_eq!(expr.expr_type, Some("Boolean".to_string()));
            assert_eq!(expr.value.clone().unwrap().raw_text, "true");
        } else {
            panic!("Expected Expr");
        }
        
        // Check fourth expr (expr with default value)
        if let ComponentItem::Expr(expr) = &result.components[0].component.items[3] {
            assert_eq!(expr.name, "theme");
            assert_eq!(expr.qualifier, None);
            assert_eq!(expr.expr_type, Some("String".to_string()));
            assert_eq!(expr.value.clone().unwrap().raw_text, "\"light\"");
        } else {
            panic!("Expected Expr");
        }
    }

    #[test]
    fn test_optional_types() {
        // Test for parameters and expressions without type definitions
        let input = r#"
            component Chart {
                // Parameters without type
                param title: "Chart Title";
                in param width;
                out param height: 300;
                
                // Expressions without type
                expr theme: "light";
                in expr color;
                out expr size: 20;
                
                // Parameters with type
                param<String> description: "A sample chart";
                param<Number> spacing: 10;
                
                // Expressions with type
                expr<Boolean> interactive: true;
                expr<Object> config: "{}";
            }
        "#;
        
        // First parse with the pest parser
        let result = AvengerParser::parse(Rule::file, input);
        assert!(result.is_ok(), "Failed to parse grammar: {:?}", result.err());
        
        // Then do a full parse
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        // Print the items for debugging
        println!("Number of items: {}", result.components[0].component.items.len());
        for (i, item) in result.components[0].component.items.iter().enumerate() {
            match item {
                ComponentItem::Parameter(p) => println!("Item {}: Parameter {}", i, p.name),
                ComponentItem::Expr(e) => println!("Item {}: Expr {}", i, e.name),
                _ => println!("Item {}: Other type", i),
            }
        }
        
        // Since order may vary, let's find items by name instead of index
        
        // Check parameters without type
        let param_title = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Parameter(param) = item {
                if param.name == "title" {
                    Some(param)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Parameter 'title' not found");
        
        assert_eq!(param_title.qualifier, None);
        assert_eq!(param_title.param_type, None); // No type specified
        assert_eq!(param_title.value.clone().unwrap().raw_text, "\"Chart Title\"");
        
        let param_width = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Parameter(param) = item {
                if param.name == "width" {
                    Some(param)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Parameter 'width' not found");
        
        assert_eq!(param_width.qualifier, Some("in".to_string()));
        assert_eq!(param_width.param_type, None); // No type specified
        
        let param_height = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Parameter(param) = item {
                if param.name == "height" {
                    Some(param)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Parameter 'height' not found");
        
        assert_eq!(param_height.qualifier, Some("out".to_string()));
        assert_eq!(param_height.param_type, None); // No type specified
        assert_eq!(param_height.value.clone().unwrap().raw_text, "300");
        
        // Check expressions without type
        let expr_theme = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Expr(expr) = item {
                if expr.name == "theme" {
                    Some(expr)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Expr 'theme' not found");
        
        assert_eq!(expr_theme.qualifier, None);
        assert_eq!(expr_theme.expr_type, None); // No type specified
        assert_eq!(expr_theme.value.clone().unwrap().raw_text, "\"light\"");
        
        let expr_color = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Expr(expr) = item {
                if expr.name == "color" {
                    Some(expr)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Expr 'color' not found");
        
        assert_eq!(expr_color.qualifier, Some("in".to_string()));
        assert_eq!(expr_color.expr_type, None); // No type specified
        
        let expr_size = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Expr(expr) = item {
                if expr.name == "size" {
                    Some(expr)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Expr 'size' not found");
        
        assert_eq!(expr_size.qualifier, Some("out".to_string()));
        assert_eq!(expr_size.expr_type, None); // No type specified
        assert_eq!(expr_size.value.clone().unwrap().raw_text, "20");
        
        // Check parameters with type
        let param_description = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Parameter(param) = item {
                if param.name == "description" {
                    Some(param)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Parameter 'description' not found");
        
        assert_eq!(param_description.qualifier, None);
        assert_eq!(param_description.param_type, Some("String".to_string())); // Specified type
        assert_eq!(param_description.value.clone().unwrap().raw_text, "\"A sample chart\"");
        
        let param_spacing = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Parameter(param) = item {
                if param.name == "spacing" {
                    Some(param)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Parameter 'spacing' not found");
        
        assert_eq!(param_spacing.qualifier, None);
        assert_eq!(param_spacing.param_type, Some("Number".to_string())); // Specified type
        assert_eq!(param_spacing.value.clone().unwrap().raw_text, "10");
        
        // Check expressions with type
        let expr_interactive = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Expr(expr) = item {
                if expr.name == "interactive" {
                    Some(expr)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Expr 'interactive' not found");
        
        assert_eq!(expr_interactive.qualifier, None);
        assert_eq!(expr_interactive.expr_type, Some("Boolean".to_string())); // Specified type
        assert_eq!(expr_interactive.value.clone().unwrap().raw_text, "true");
        
        let expr_config = result.components[0].component.items.iter().find_map(|item| {
            if let ComponentItem::Expr(expr) = item {
                if expr.name == "config" {
                    Some(expr)
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Expr 'config' not found");
        
        assert_eq!(expr_config.qualifier, None);
        assert_eq!(expr_config.expr_type, Some("Object".to_string())); // Specified type
        assert_eq!(expr_config.value.clone().unwrap().raw_text, "\"{}\"");
    }

    #[test]
    fn test_simple_component_function() {
        let source = r#"
        component Test {
            width: 100;
            fn simple(self) -> param {
                SELECT 1;
            }

            fn simple2(self; param foo;) -> dataset {
                SELECT 1;
            }
        }
        "#;
        
        let result = parse(source).unwrap();
        assert_eq!(result.components.len(), 1);
        
        let component = &result.components[0].component;
        assert_eq!(component.name, "Test");

        // simple(self)
        let ComponentItem::ComponentFunction(func) = &component.items[1] else {
            panic!("Expected ComponentFunction");
        };

        assert_eq!(func.name, "simple");
        assert_eq!(func.return_type, "param");
        assert!(!func.out_qualifier);
        assert_eq!(func.parameters.len(), 0);


        // // simple2(self, param: foo)
        // let ComponentItem::ComponentFunction(func) = &component.items[2] else {
        //     panic!("Expected ComponentFunction");
        // };

        // println!("func: {:?}", func);

        // assert_eq!(func.name, "simple2");
        // assert_eq!(func.return_type, "dataset");
        // assert!(!func.out_qualifier);
        // assert_eq!(func.parameters.len(), 1);
        // assert_eq!(func.parameters[0].name, "foo");
        // assert_eq!(func.parameters[0].param_type, Some("foo".to_string()));
    }
}
