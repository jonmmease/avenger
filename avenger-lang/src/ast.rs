use std::collections::HashMap;
use std::fmt;

use sqlparser::{
    ast::{CreateFunction, Expr as SqlExpr, Ident, Query as SqlQuery, Spanned},
    tokenizer::Span,
};

use crate::error::AvengerLangError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerFile {
    pub name: String,
    pub path: String,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Statement {
    Import(ImportStatement),
    ValProp(ValProp),
    ExprProp(ExprProp),
    DatasetProp(DatasetProp),
    ComponentProp(ComponentProp),
    PropBinding(PropBinding),
    CreateFunction {function: CreateFunction, span: Span},
}

impl Spanned for Statement {
    fn span(&self) -> Span {
        match self {
            Statement::Import(stmt) => stmt.span(),
            Statement::ValProp(val_prop) => val_prop.span(),
            Statement::ExprProp(expr_prop) => expr_prop.span(),
            Statement::DatasetProp(dataset_prop) => dataset_prop.span(),
            Statement::ComponentProp(component_prop) => component_prop.span(),
            Statement::PropBinding(prop_binding) => prop_binding.span(),
            Statement::CreateFunction { span, .. } => *span,
        }
    }
}

impl fmt::Display for AvengerFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for stmt in &self.statements {
            writeln!(f, "{}", stmt)?;
        }
        Ok(())
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::Import(stmt) => write!(f, "{}", stmt),
            Statement::ValProp(val_prop) => write!(f, "{}", val_prop),
            Statement::ExprProp(expr_prop) => write!(f, "{}", expr_prop),
            Statement::DatasetProp(dataset_prop) => write!(f, "{}", dataset_prop),
            Statement::ComponentProp(component_prop) => write!(f, "{}", component_prop),
            Statement::PropBinding(prop_binding) => write!(f, "{}", prop_binding),
            Statement::CreateFunction { function, .. } => write!(f, "{}", function),
        }
    }
}

// Import
// ------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportItem {
    pub name: Ident,
    pub as_keyword: Option<KeywordAs>,
    pub alias: Option<Ident>,
}

impl Spanned for ImportItem {
    fn span(&self) -> Span {
        if let (Some(as_keyword), Some(alias)) = (&self.as_keyword, &self.alias) {
            Span::union_iter([self.name.span, as_keyword.span, alias.span])
        } else {
            self.name.span
        }
    }
}

impl fmt::Display for ImportItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name.value)?;
        if let Some(alias) = &self.alias {
            write!(f, " as {}", alias.value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportStatement {
    pub import_keyword: KeywordImport,
    pub items: Vec<ImportItem>,
    pub from_keyword: Option<KeywordFrom>,
    pub from_path: Option<Ident>,
}

impl Spanned for ImportStatement {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        span = span.union(&self.import_keyword.span());
        for item in &self.items {
            span = span.union(&item.span());
        }
        if let Some(from_keyword) = &self.from_keyword {
            span = span.union(&from_keyword.span);
        }
        if let Some(from_path) = &self.from_path {
            span = span.union(&from_path.span);
        }
        span
    }
}

impl fmt::Display for ImportStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "import {{ ")?;
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?;
        }
        write!(f, " }}")?;
        if let Some(from_path) = &self.from_path {
            write!(f, " from '{}'", from_path.value)?;
        }
        write!(f, ";")
    }
}

// Val prop
// -------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Type {
    pub name: Ident,
}

impl Spanned for Type {
    fn span(&self) -> Span {
        self.name.span
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.name.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Qualifier {
    In(KeywordIn),
    Out(KeywordOut),
}

impl Spanned for Qualifier {
    fn span(&self) -> Span {
        match self {
            Qualifier::In(kw) => kw.span(),
            Qualifier::Out(kw) => kw.span(),
        }
    }
}

impl fmt::Display for Qualifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Qualifier::In(_) => write!(f, "in "),
            Qualifier::Out(_) => write!(f, "out "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValProp {
    pub qualifier: Option<Qualifier>,
    pub val_keyword: KeywordVal,
    pub type_: Option<Type>,
    pub name: Ident,
    pub expr: SqlExpr,
}

impl ValProp {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for ValProp {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        if let Some(qualifier) = &self.qualifier {
            span = span.union(&qualifier.span());
        }
        if let Some(type_) = &self.type_ {
            span = span.union(&type_.span());
        }
        span = span.union(&self.name.span);
        span = span.union(&self.expr.span());
        span
    }
}

impl fmt::Display for ValProp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(qualifier) = &self.qualifier {
            write!(f, "{}", qualifier)?;
        }
        write!(f, "val")?;
        if let Some(type_) = &self.type_ {
            write!(f, " {}", type_)?;
        }
        write!(f, " {}: {};", self.name.value, self.expr)
    }
}

// Expr prop
// ---------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprProp {
    pub qualifier: Option<Qualifier>,
    pub expr_keyword: KeywordExpr,
    pub type_: Option<Type>,
    pub name: Ident,
    pub expr: SqlExpr,
}

impl ExprProp {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for ExprProp {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        if let Some(qualifier) = &self.qualifier {
            span = span.union(&qualifier.span());
        }
        if let Some(type_) = &self.type_ {
            span = span.union(&type_.span());
        }
        span = span.union(&self.name.span);
        span = span.union(&self.expr.span());
        span
    }
}

impl fmt::Display for ExprProp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(qualifier) = &self.qualifier {
            write!(f, "{}", qualifier)?;
        }
        write!(f, "expr")?;
        if let Some(type_) = &self.type_ {
            write!(f, " {}", type_)?;
        }
        write!(f, " {}: {};", self.name.value, self.expr)
    }
}

// Dataset prop
// ------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetProp {
    pub qualifier: Option<Qualifier>,
    pub dataset_keyword: KeywordDataset,
    pub type_: Option<Type>,
    pub name: Ident,
    pub query: Box<SqlQuery>,
}

impl DatasetProp {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for DatasetProp {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        if let Some(qualifier) = &self.qualifier {
            span = span.union(&qualifier.span());
        }
        if let Some(type_) = &self.type_ {
            span = span.union(&type_.span());
        }
        span = span.union(&self.name.span);
        span = span.union(&self.query.span());
        span
    }
}

impl fmt::Display for DatasetProp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(qualifier) = &self.qualifier {
            write!(f, "{}", qualifier)?;
        }
        write!(f, "dataset")?;
        if let Some(type_) = &self.type_ {
            write!(f, " {}", type_)?;
        }
        write!(f, " {}: {};", self.name.value, self.query)
    }
}

// Component prop
// --------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentProp {
    // qualifier, keyword, and name are optional for components
    pub qualifier: Option<Qualifier>,
    pub component_keyword: Option<KeywordComp>,
    pub prop_name: Option<Ident>,
    pub component_type: Ident,
    pub statements: Vec<Statement>,
}

impl ComponentProp {
    pub fn name(&self) -> String {
        if let Some(prop_name) = &self.prop_name {
            prop_name.value.clone()
        } else {
            // Build a unique component name based on the location
            let start_location = self.component_type.span.start;
            format!("_comp_{}_{}", start_location.line, start_location.column)
        }
    }

    pub fn val_props(&self) -> HashMap<String, ValProp> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::ValProp(val_prop) = stmt {
                    Some((val_prop.name().to_string(), val_prop.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn prop_bindings(&self) -> HashMap<String, PropBinding> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::PropBinding(prop_binding) = stmt {
                    Some((prop_binding.name().to_string(), prop_binding.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn expr_props(&self) -> HashMap<String, ExprProp> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::ExprProp(expr_prop) = stmt {
                    Some((expr_prop.name().to_string(), expr_prop.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn dataset_props(&self) -> HashMap<String, DatasetProp> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::DatasetProp(dataset_prop) = stmt {
                    Some((dataset_prop.name().to_string(), dataset_prop.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn component_props(&self) -> HashMap<String, ComponentProp> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::ComponentProp(component_prop) = stmt {
                    Some((component_prop.name().to_string(), component_prop.clone()))
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Spanned for ComponentProp {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        if let Some(qualifier) = &self.qualifier {
            span = span.union(&qualifier.span());
        }
        if let Some(prop_name) = &self.prop_name {
            span = span.union(&prop_name.span);
        }
        span = span.union(&self.component_type.span);
        for statement in &self.statements {
            span = span.union(&statement.span());
        }
        span
    }
}

impl fmt::Display for ComponentProp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_indent(f, 4)
    }
}

impl ComponentProp {
    // Helper method to format with a specific indentation level
    fn fmt_with_indent(&self, f: &mut fmt::Formatter<'_>, indent_level: usize) -> fmt::Result {
        // If this is a named component prop
        if let Some(_component_keyword) = &self.component_keyword {
            if let Some(qualifier) = &self.qualifier {
                write!(f, "{}", qualifier)?;
            }
            write!(f, "comp {}: ", self.prop_name.as_ref().unwrap().value)?;
        }
        
        // Write component type and open brace
        write!(f, "{} {{", self.component_type.value)?;
        
        // Write statements with proper indentation
        if !self.statements.is_empty() {
            writeln!(f)?;
            
            // Create indent string for this level
            let indent = " ".repeat(indent_level);
            
            for statement in &self.statements {
                match statement {
                    // For nested components, handle the nested component formatting
                    Statement::ComponentProp(nested_comp) => {
                        // Write the indentation for this component's opening line
                        write!(f, "{}", indent)?;
                        
                        if nested_comp.component_keyword.is_some() {
                            if let Some(qualifier) = &nested_comp.qualifier {
                                write!(f, "{}", qualifier)?;
                            }
                            write!(f, "comp {}: ", nested_comp.prop_name.as_ref().unwrap().value)?;
                        }
                        
                        // Write the component type and open brace
                        writeln!(f, "{} {{", nested_comp.component_type.value)?;
                        
                        // Format each statement in the nested component with increased indent level
                        let next_level = indent_level + 4;
                        let next_indent = " ".repeat(next_level);
                        
                        for nested_stmt in &nested_comp.statements {
                            // For each statement in this component
                            match nested_stmt {
                                // If it's another component, call formatting recursively
                                Statement::ComponentProp(inner_comp) => {
                                    // Format the inner component using String as a buffer
                                    let inner_result = format!("{}", inner_comp);
                                    
                                    // Add the correctly indented inner component
                                    for line in inner_result.lines() {
                                        writeln!(f, "{}{}", next_indent, line)?;
                                    }
                                },
                                // For regular statements, just add the indentation
                                _ => {
                                    writeln!(f, "{}{}", next_indent, nested_stmt)?;
                                }
                            }
                        }
                        
                        // Close this nested component with correct indentation
                        writeln!(f, "{}}}", indent)?;
                    },
                    // For non-component statements
                    _ => {
                        writeln!(f, "{}{}", indent, statement)?;
                    }
                }
            }
        }
        
        // Close brace (without trailing newline)
        write!(f, "}}")
    }
}

// prop binding
// ------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropBinding {
    pub name: Ident,
    pub expr: SqlExprOrQuery,
}

impl PropBinding {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for PropBinding {
    fn span(&self) -> Span {
        Span::union_iter([self.name.span, self.expr.span()])
    }
}

impl fmt::Display for PropBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} := {};", self.name.value, self.expr)
    }
}


// sql expr or query
// -----------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SqlExprOrQuery {
    Expr(SqlExpr),
    Query(Box<SqlQuery>),
}

impl SqlExprOrQuery {
    pub fn into_expr(self) -> Result<SqlExpr, AvengerLangError> {
        match self {
            SqlExprOrQuery::Expr(expr) => Ok(expr),
            SqlExprOrQuery::Query(q) => Err(AvengerLangError::InternalError(format!(
                "Query not allowed: {:#?}",
                q
            ))),
        }
    }

    pub fn into_query(self) -> Result<Box<SqlQuery>, AvengerLangError> {
        match self {
            SqlExprOrQuery::Expr(expr) => Err(AvengerLangError::InternalError(format!(
                "Expr not allowed: {}",
                expr
            ))),
            SqlExprOrQuery::Query(query) => Ok(query),
        }
    }
}

impl Spanned for SqlExprOrQuery {
    fn span(&self) -> Span {
        match self {
            SqlExprOrQuery::Expr(expr) => expr.span(),
            SqlExprOrQuery::Query(query) => query.span(),
        }
    }
}

impl fmt::Display for SqlExprOrQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlExprOrQuery::Expr(expr) => write!(f, "{}", expr),
            SqlExprOrQuery::Query(query) => write!(f, "{}", query),
        }
    }
}

// identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

impl Spanned for Identifier {
    fn span(&self) -> Span {
        self.span
    }
}

// Keywords
// --------
macro_rules! define_keyword {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name {
            pub span: Span,
        }

        impl Spanned for $name {
            fn span(&self) -> Span {
                self.span
            }
        }

        impl $name {
            pub fn new(span: Span) -> Self {
                Self { span }
            }
        }
    };
}

define_keyword!(KeywordAs);
define_keyword!(KeywordIn);
define_keyword!(KeywordOut);
define_keyword!(KeywordImport);
define_keyword!(KeywordVal);
define_keyword!(KeywordExpr);
define_keyword!(KeywordDataset);
define_keyword!(KeywordComponent);
define_keyword!(KeywordComp);
define_keyword!(KeywordFn);
define_keyword!(KeywordReturn);
define_keyword!(KeywordFrom);


#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::tokenizer::Span;

    struct TestVisitor {
        visited: Vec<String>,
    }

    // impl Visitor for TestVisitor {
    //     type Break = ();

    //     fn post_visit_expr(&mut self, _expr: &SqlExpr) -> std::ops::ControlFlow<Self::Break> {

    //     }
    // }

    #[test]
    fn test_display_val_prop() {
        use sqlparser::ast::{Expr, Value};

        let val_prop = ValProp {
            qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
            val_keyword: KeywordVal { span: Span::empty() },
            type_: Some(Type { 
                name: Ident {
                    value: "int".to_string(),
                    quote_style: None,
                    span: Span::empty(),
                }
            }),
            name: Ident {
                value: "foo".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            expr: Expr::Value(Value::Number("42".to_string(), false)),
        };

        assert_eq!(val_prop.to_string(), "in val <int> foo: 42;");
    }

    #[test]
    fn test_display_component_prop() {
        let component = ComponentProp {
            qualifier: None,
            component_keyword: Some(KeywordComp { span: Span::empty() }),
            prop_name: Some(Ident {
                value: "chart".to_string(),
                quote_style: None,
                span: Span::empty(),
            }),
            component_type: Ident {
                value: "Rect".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "width".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("500".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "height".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("300".to_string(), false)),
                }),
            ],
        };

        let expected = "comp chart: Rect {\n    val width: 500;\n    val height: 300;\n}";
        assert_eq!(component.to_string(), expected);
    }

    #[test]
    fn test_display_nested_component_prop() {
        // Create a nested Group component
        let nested_group = ComponentProp {
            qualifier: None,
            component_keyword: None,
            prop_name: None,
            component_type: Ident {
                value: "Group".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "a".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("0".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "b".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("277".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "res".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    // Just use a simple expression since we're testing formatting
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("42".to_string(), false)),
                }),
            ],
        };
        
        // Create a parent component that contains the nested group
        let parent_component = ComponentProp {
            qualifier: None,
            component_keyword: None,
            prop_name: None,
            component_type: Ident {
                value: "Group".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "b".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("12".to_string(), false)),
                }),
                Statement::ComponentProp(nested_group),
            ],
        };

        let output = parent_component.to_string();
        println!("Nested component output:\n{}", output);
        
        // Check that the nested group is properly indented
        let expected = "Group {\n    val b: 12;\n    Group {\n        in val a: 0;\n        in val b: 277;\n        val res: 42;\n    }\n}";
        assert_eq!(output, expected);
    }


    #[test]
    fn test_display_deeply_nested_component_prop() {
        // Create the innermost (level 3) component
        let innermost_group = ComponentProp {
            qualifier: None,
            component_keyword: None,
            prop_name: None,
            component_type: Ident {
                value: "Group".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "a".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("177".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "b".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("1".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "res".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("355".to_string(), false)),
                }),
            ],
        };
        
        // Create the middle level (level 2) component
        let middle_group = ComponentProp {
            qualifier: None,
            component_keyword: None,
            prop_name: None,
            component_type: Ident {
                value: "Group".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "a".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("0".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: Some(Qualifier::In(KeywordIn { span: Span::empty() })),
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "b".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("277".to_string(), false)),
                }),
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "res".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("42".to_string(), false)),
                }),
                // Add the innermost component
                Statement::ComponentProp(innermost_group),
            ],
        };
        
        // Create the outer (level 1) component
        let outer_component = ComponentProp {
            qualifier: None,
            component_keyword: None,
            prop_name: None,
            component_type: Ident {
                value: "Group".to_string(),
                quote_style: None,
                span: Span::empty(),
            },
            statements: vec![
                Statement::ValProp(ValProp {
                    qualifier: None,
                    val_keyword: KeywordVal { span: Span::empty() },
                    type_: None,
                    name: Ident {
                        value: "b".to_string(),
                        quote_style: None,
                        span: Span::empty(),
                    },
                    expr: SqlExpr::Value(sqlparser::ast::Value::Number("12".to_string(), false)),
                }),
                // Add the middle component
                Statement::ComponentProp(middle_group),
            ],
        };

        let output = outer_component.to_string();
        println!("Deeply nested component output:\n{}", output);
        
        // Check that the nested components are properly indented with increasing indentation levels
        let expected = "Group {\n    val b: 12;\n    Group {\n        in val a: 0;\n        in val b: 277;\n        val res: 42;\n        Group {\n            in val a: 177;\n            in val b: 1;\n            val res: 355;\n        }\n    }\n}";
        assert_eq!(output, expected);
    }
}
