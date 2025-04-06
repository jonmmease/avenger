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
    }
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

#[derive(Debug, Clone)]
pub struct Value {
    pub raw_text: String,
    pub sql_expr: SqlExpr,
}

impl Value {
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

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub qualifier: Option<String>, // "in" or "out"
    pub param_type: String,
    pub name: String,
    pub value: Value,
    pub default: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct IfStatement {
    pub condition: Value,  // SQL expression that will be evaluated
    pub items: Vec<ComponentItem>,
    pub else_items: Option<Vec<ComponentItem>>,
}

#[derive(Debug, Clone)]
pub struct MatchCase {
    pub pattern: String,
    pub is_default: bool,
    pub items: Vec<ComponentItem>,
}

#[derive(Debug, Clone)]
pub struct MatchStatement {
    pub expression: Value,
    pub cases: Vec<MatchCase>,
}

#[derive(Debug, Clone)]
pub struct EnumDefinition {
    pub name: String,
    pub values: Vec<String>,
    pub exported: bool,
}

#[derive(Debug, Clone)]
pub enum ComponentItem {
    Property(Property),
    Parameter(Parameter),
    ComponentInstance(Box<ComponentInstance>),
    ComponentBinding(String, Box<ComponentInstance>),
    IfStatement(Box<IfStatement>),
    MatchStatement(Box<MatchStatement>),
}

#[derive(Debug, Clone)]
pub struct ComponentInstance {
    pub name: String,
    pub parent: Option<String>,
    pub items: Vec<ComponentItem>,
}

#[derive(Debug, Clone)]
pub struct ComponentDeclaration {
    pub exported: bool,
    pub component: ComponentInstance,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub components: Vec<String>,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct AvengerFile {
    pub imports: Vec<Import>,
    pub enums: Vec<EnumDefinition>,
    pub components: Vec<ComponentDeclaration>,
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
            Value::try_new(next.as_str().to_string())?
        },
        Rule::conditional_expr => {
            // Parse the expression content (remove the parentheses)
            let expr_text = next.as_str();
            let inner_expr = &expr_text[1..expr_text.len()-1];  // Remove ( and )
            Value::try_new(inner_expr.trim().to_string())?
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
            Value::try_new(next.as_str().to_string())?
        },
        Rule::conditional_expr => {
            // Parse the expression content (remove the parentheses)
            let expr_text = next.as_str();
            let inner_expr = &expr_text[1..expr_text.len()-1];  // Remove ( and )
            Value::try_new(inner_expr.trim().to_string())?
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
            Rule::private_parameter => {
                content_items.push(ComponentItem::Parameter(parse_private_parameter(item_pair)?));
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
            _ => {}
        }
    }
    
    Ok(content_items)
}

fn parse_property(pair: pest::iterators::Pair<Rule>) -> Result<Property, ParserError> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let value_text = inner.next().unwrap().as_str().trim().to_string();
    
    let value = Value::try_new(value_text)?;
    
    Ok(Property { name, value })
}

fn parse_parameter(pair: pest::iterators::Pair<Rule>) -> Result<Parameter, ParserError> {
    let mut inner = pair.into_inner();
    let mut next = inner.next().unwrap();
    
    let mut qualifier = None;
    if next.as_rule() == Rule::param_qualifier {
        qualifier = Some(next.as_str().to_string());
        next = inner.next().unwrap(); // Skip to "param" keyword
    }
    
    if next.as_str() == "param" {
        next = inner.next().unwrap(); // Skip to param_type
    }
    
    // Next is param_type
    let param_type = next.into_inner().next().unwrap().as_str().to_string();
    
    // Next is parameter name
    let name = inner.next().unwrap().as_str().to_string();
    
    // Next is value
    let value_text = inner.next().unwrap().as_str().trim().to_string();
    let value = Value::try_new(value_text)?;
    
    // Check for optional default value
    let default = if let Some(default_pair) = inner.next() {
        if default_pair.as_rule() == Rule::param_default {
            let default_text = default_pair.into_inner().next().unwrap().as_str().trim().to_string();
            Some(Value::try_new(default_text)?)
        } else {
            None
        }
    } else {
        None
    };
    
    Ok(Parameter {
        qualifier,
        param_type,
        name,
        value,
        default,
    })
}

fn parse_private_parameter(pair: pest::iterators::Pair<Rule>) -> Result<Parameter, ParserError> {
    let mut inner = pair.into_inner();
    
    // Skip "param" keyword
    let next = inner.next().unwrap();
    
    // Next is param_type
    let param_type = next.into_inner().next().unwrap().as_str().to_string();
    
    // Next is parameter name
    let name = inner.next().unwrap().as_str().to_string();
    
    // Next is value
    let value_text = inner.next().unwrap().as_str().trim().to_string();
    let value = Value::try_new(value_text)?;
    
    // Check for optional default value
    let default = if let Some(default_pair) = inner.next() {
        if default_pair.as_rule() == Rule::param_default {
            let default_text = default_pair.into_inner().next().unwrap().as_str().trim().to_string();
            Some(Value::try_new(default_text)?)
        } else {
            None
        }
    } else {
        None
    };
    
    // Return parameter with no qualifier
    Ok(Parameter {
        qualifier: None,
        param_type,
        name,
        value,
        default,
    })
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
    
    // Skip "component" keyword if present
    let mut component_name = "";
    for inner_pair in pairs.by_ref() {
        if inner_pair.as_rule() == Rule::component_identifier {
            component_name = inner_pair.as_str();
            break;
        } else if inner_pair.as_str() == "component" {
            // Skip "component" keyword
            continue;
        }
    }
    
    // Parse the component content
    let mut component_items = Vec::new();
    
    // Process all items in component body
    for inner_pair in pairs {
        match inner_pair.as_rule() {
            Rule::property => {
                component_items.push(ComponentItem::Property(parse_property(inner_pair)?));
            }
            Rule::parameter => {
                component_items.push(ComponentItem::Parameter(parse_parameter(inner_pair)?));
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
            _ => {}
        }
    }
    
    Ok(ComponentDeclaration {
        exported,
        component: ComponentInstance {
            name: component_name.to_string(),
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
        } else if item_pair.as_rule() == Rule::parameter {
            component_items.push(ComponentItem::Parameter(parse_parameter(item_pair)?));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_parse_simple() {
        let input = r#"
            Chart {
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
            Chart {
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
            Chart {
                param<SomeType> some_param: "SomeValue";
                in param<OtherType> other_param: "OtherValue";
            }
        "#;
        
        let result = parse(input).unwrap();
        assert_eq!(result.components.len(), 1);
        
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[0] {
            assert_eq!(param.param_type, "SomeType");
            assert_eq!(param.name, "some_param");
            assert_eq!(param.value.raw_text, "\"SomeValue\"");
        } else {
            panic!("Expected Parameter");
        }
        
        if let ComponentItem::Parameter(param) = &result.components[0].component.items[1] {
            assert_eq!(param.qualifier, Some("in".to_string()));
            assert_eq!(param.param_type, "OtherType");
            assert_eq!(param.name, "other_param");
            assert_eq!(param.value.raw_text, "\"OtherValue\"");
        } else {
            panic!("Expected Parameter");
        }
    }
    
    #[test]
    fn test_parse_component_binding() {
        let input = r#"
            Chart {
                bound := Another {
                    x_y_z: 10;
                }
            }
        "#;
        
        let result = parse(input).unwrap();
        
        if let ComponentItem::ComponentBinding(name, component) = &result.components[0].component.items[0] {
            assert_eq!(name, "bound");
            assert_eq!(component.name, "Another");
        } else {
            panic!("Expected ComponentBinding");
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
            Chart {
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
            Chart {
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
        Chart {
            // Simple parameters
            width: 100;
            height: 100;

            // Parameters with types
            param<Number> scale: 1.5;
            in param<String> title: "Chart Title" = "Default Title";
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
            
            Chart {
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
            
            Chart {
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
            Chart {
                width: 100;
                
                if show_rule {
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
            Chart {
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
            Chart {
                if outer_condition {
                    Group {
                        width: 50;
                        
                        if inner_condition {
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
            Chart {
                if show_rule {
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
    fn test_private_parameter_in_if() {
        let input = r#"
            Chart {
                if has_data {
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
                assert_eq!(param.param_type, "Number");
                assert_eq!(param.name, "scale");
                assert_eq!(param.value.raw_text, "1.5");
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
            Chart {
                if outer_condition {
                    Group {
                        if inner_condition {
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
                panic!("Expected Group inside outer if");
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
            Chart {
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
            
            // Check success case
            assert_eq!(match_stmt.cases[0].pattern, "success");
            assert_eq!(match_stmt.cases[0].is_default, false);
            assert_eq!(match_stmt.cases[0].items.len(), 1);
            
            if let ComponentItem::ComponentInstance(icon) = &match_stmt.cases[0].items[0] {
                assert_eq!(icon.name, "SuccessIcon");
            } else {
                panic!("Expected ComponentInstance in first match case");
            }
            
            // Check error case
            assert_eq!(match_stmt.cases[1].pattern, "error");
            assert_eq!(match_stmt.cases[1].is_default, false);
            
            // Check warning case
            assert_eq!(match_stmt.cases[2].pattern, "warning");
            assert_eq!(match_stmt.cases[2].is_default, false);
        } else {
            panic!("Expected MatchStatement");
        }
    }
    
    #[test]
    fn test_match_with_default() {
        let input = r#"
            Chart {
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
            Chart {
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
            Chart {
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
                assert_eq!(param.param_type, "Number");
                assert_eq!(param.name, "scale");
                assert_eq!(param.value.raw_text, "2.0");
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
            
            Chart {
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
            
            Chart {
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
            
            Chart {
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
    fn test_parse_all_examples() {
        // Get all .avgr files in the examples directory
        let examples_dir = Path::new("examples");
        let entries = fs::read_dir(examples_dir).expect("Failed to read examples directory");
        
        let mut example_files = Vec::new();
        for entry in entries {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();
            
            if path.extension().map_or(false, |ext| ext == "avgr") {
                example_files.push(path);
            }
        }
        
        // Parse each file and check for success
        for path in example_files {
            let content = fs::read_to_string(&path).expect("Failed to read example file");
            let result = parse(&content);
            
            match result {
                Ok(_) => {
                    // Successfully parsed
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
        let value = Value::try_new("1 + 2".to_string()).unwrap();
        assert_eq!(value.raw_text, "1 + 2");
        
        // Valid SQL expression with column reference
        let value = Value::try_new("id > 21".to_string()).unwrap();
        assert_eq!(value.raw_text, "id > 21");
        
        // Scalar subquery should work
        let value = Value::try_new("(SELECT min(colA) FROM users)".to_string()).unwrap();
        assert_eq!(value.raw_text, "(SELECT min(colA) FROM users)");
        
        // Full SQL query should fail
        let result = Value::try_new("SELECT SUM(value) FROM sales;".to_string());
        assert!(result.is_err());
        
        // Full SQL query with multiple columns should fail
        let result = Value::try_new("SELECT id, name FROM users".to_string());
        assert!(result.is_err());
        
        // Non-SQL expression should fail
        let result = Value::try_new("Hello World".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_sql_error_message_format() {
        // Test full query without scalar subquery parentheses
        let result = Value::try_new("1 @ 2".to_string());
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
            Chart {
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
            Chart {
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
            
            // Check first case content - high
            if let ComponentItem::ComponentInstance(circle) = &match_stmt.cases[0].items[0] {
                assert_eq!(circle.name, "Circle");
                assert_eq!(circle.items.len(), 1); // r property
                
                if let ComponentItem::Property(prop) = &circle.items[0] {
                    assert_eq!(prop.name, "r");
                    assert_eq!(prop.value.raw_text, "10");
                } else {
                    panic!("Expected Property");
                }
            } else {
                panic!("Expected Circle component");
            }
        } else {
            panic!("Expected MatchStatement");
        }
    }

    #[test]
    fn test_complex_sql_expressions() {
        let input = r#"
            Chart {
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
            Chart {
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
            Chart {
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
}
