use std::collections::HashMap;
use std::sync::Arc;

use sqlparser::dialect::GenericDialect;
use sqlparser::ast::{Value as SqlValue, Expr as SqlExpr, Query as SqlQuery};

use crate::error::AvengerLangError;
use crate::task_graph::component_registry::ComponentRegistry;

//
//  sql_query and sql_expr rules are defined by sqlparser-rs
//
//  type           → "<" + IDENTIFIER + ">"
//  prop_qualifier → "in" | "out"
//  sql_expr_or_query → sql_expr | sql_query
//  val_prop       → prop_qualifier? + "val" + type? + ":" + sql_expr + ";"
//  expr_prop      → prop_qualifier? + "expr" + type? + ":" + sql_expr + ";"
//  dataset_prop   → prop_qualifier? + "dataset" + type? + ":" + sql_query + ";"
//  comp_prop      → prop_qualifier? + "comp" + type? + ":" + comp_instance + ";"
//  comp_instance  → PASCAL_IDENTIFIER + "{" + statement* + "}"
//  prop_binding   → IDENTIFIER + ":=" + sql_expr_or_query + ";"
//
//  statement      → (val_prop | expr_prop | dataset_prop | comp_prop | prop_binding) + ";"
//  file           → statement*
//
#[derive(Debug, Clone, PartialEq, Eq, Hash) ]
pub struct Type(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropQualifier {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValPropDecl {
    pub qualifier: Option<PropQualifier>,
    pub name: String,
    pub type_: Option<Type>,
    pub value: SqlExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprPropDecl {
    pub qualifier: Option<PropQualifier>,
    pub name: String,
    pub type_: Option<Type>,
    pub value: SqlExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetPropDecl {
    pub qualifier: Option<PropQualifier>,
    pub name: String,
    pub type_: Option<Type>,
    pub value: Box<SqlQuery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompPropDecl {
    pub qualifier: Option<PropQualifier>,
    pub name: String,
    pub type_: Option<Type>,
    pub value: CompInstance,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompInstance {
    pub name: String,
    pub statements: Vec<Statement>,
}

impl CompInstance {
    pub fn prop_bindings(&self) -> HashMap<String, &PropBinding> {
        self.statements.iter().filter_map(|stmt| {
            if let Statement::PropBinding(binding) = stmt {
                Some((binding.name.clone(), binding))
            } else {
                None
            }
        }).collect()
    }

    pub fn child_comp_decls(&self) -> Vec<&CompPropDecl> {
        self.statements.iter().filter_map(|stmt| {
            if let Statement::CompPropDecl(comp_prop) = stmt {
                Some(comp_prop)
            } else {
                None
            }
        }).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SqlExprOrQuery {
    Expr(SqlExpr),
    Query(Box<SqlQuery>),
}

impl SqlExprOrQuery {
    pub fn into_expr(self) -> Result<SqlExpr, AvengerLangError> {
        match self {
            SqlExprOrQuery::Expr(expr) => Ok(expr),
            SqlExprOrQuery::Query(query) => Err(
                AvengerLangError::InternalError("Query not allowed".to_string())
            ),
        }
    }

    pub fn into_query(self) -> Result<Box<SqlQuery>, AvengerLangError> {
        match self {
            SqlExprOrQuery::Expr(expr) => Err(
                AvengerLangError::InternalError("Expr not allowed".to_string())
            ),
            SqlExprOrQuery::Query(query) => Ok(query),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropBinding {
    pub name: String,
    pub value: SqlExprOrQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Statement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
    CompPropDecl(CompPropDecl),
    PropBinding(PropBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerFile {
    pub statements: Vec<Statement>,
}

// Visitor trait for traversing the AST with immutable references
#[derive(Debug, Clone)]
pub struct VisitorContext {
    pub scope_path: Vec<String>,
    pub component_type: String,
    pub component_registry: Arc<ComponentRegistry>,
}

pub trait Visitor {
    fn visit_val_prop_decl(&mut self, val_prop: &ValPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_expr_prop_decl(&mut self, expr_prop: &ExprPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_dataset_prop_decl(&mut self, dataset_prop: &DatasetPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_comp_prop_decl(&mut self, comp_prop: &CompPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_comp_instance(&mut self, comp_instance: &CompInstance, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_prop_binding(&mut self, prop_binding: &PropBinding, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_statement(&mut self, statement: &Statement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_avenger_file(&mut self, file: &AvengerFile, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
}

// Mutable visitor trait for traversing and potentially modifying the AST
pub trait VisitorMut {
    fn visit_val_prop_decl(&mut self, val_prop: &mut ValPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_expr_prop_decl(&mut self, expr_prop: &mut ExprPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_dataset_prop_decl(&mut self, dataset_prop: &mut DatasetPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_comp_prop_decl(&mut self, comp_prop: &mut CompPropDecl, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_comp_instance(&mut self, comp_instance: &mut CompInstance, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_prop_binding(&mut self, prop_binding: &mut PropBinding, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_statement(&mut self, statement: &mut Statement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_avenger_file(&mut self, file: &mut AvengerFile, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
}

// Implementation of accept methods for each AST node
impl ValPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_val_prop_decl(self, ctx)
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_val_prop_decl(self, ctx)
    }
}

impl ExprPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_expr_prop_decl(self, ctx)
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_expr_prop_decl(self, ctx)
    }
}

impl DatasetPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_dataset_prop_decl(self, ctx)
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_dataset_prop_decl(self, ctx)
    }
}

impl CompPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        let mut new_scope_path = ctx.scope_path.to_vec();
        new_scope_path.push(self.name.clone());

        let new_ctx = VisitorContext {
            scope_path: new_scope_path,
            component_type: self.value.name.clone(),
            ..ctx.clone()
        };

        visitor.visit_comp_prop_decl(self, &new_ctx)?;
        self.value.accept(visitor, &new_ctx)
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        let mut new_scope_path = ctx.scope_path.to_vec();
        new_scope_path.push(self.name.clone());

        let new_ctx = VisitorContext {
            scope_path: new_scope_path,
            component_type: self.name.clone(),
            ..ctx.clone()
        };
        self.value.accept_mut(visitor, &new_ctx)?;
        visitor.visit_comp_prop_decl(self, &new_ctx)
    }
}

impl CompInstance {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_comp_instance(self, ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, ctx)?;
        }
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_comp_instance(self, ctx)?;
        for statement in &mut self.statements {
            statement.accept_mut(visitor, &ctx)?;
        }
        Ok(())
    }
}

impl PropBinding {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_prop_binding(self, ctx)
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_prop_binding(self, ctx)
    }
}

impl Statement {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_statement(self, ctx)?;
        match self {
            Statement::ValPropDecl(val) => val.accept(visitor, ctx),
            Statement::ExprPropDecl(expr) => expr.accept(visitor, ctx),
            Statement::DatasetPropDecl(dataset) => dataset.accept(visitor, ctx),
            Statement::CompPropDecl(comp) => comp.accept(visitor, ctx),
            Statement::PropBinding(prop) => prop.accept(visitor, ctx),
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        match self {
            Statement::ValPropDecl(val) => val.accept_mut(visitor, ctx)?,
            Statement::ExprPropDecl(expr) => expr.accept_mut(visitor, ctx)?,
            Statement::DatasetPropDecl(dataset) => dataset.accept_mut(visitor, ctx)?,
            Statement::CompPropDecl(comp) => comp.accept_mut(visitor, ctx)?,
            Statement::PropBinding(prop) => prop.accept_mut(visitor, ctx)?,
        }
        visitor.visit_statement(self, ctx)
    }
}

impl AvengerFile {
    pub fn accept<V: Visitor>(&self, visitor: &mut V) -> Result<(), AvengerLangError> {
        let root_ctx = VisitorContext {
            scope_path: Vec::new(),
            component_type: "App".to_string(),
            component_registry: Arc::new(ComponentRegistry::new_with_marks()),
        };
        visitor.visit_avenger_file(self, &root_ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, &root_ctx)?;
        }
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V) -> Result<(), AvengerLangError> {
        let root_ctx = VisitorContext {
            scope_path: Vec::new(),
            component_type: "App".to_string(),
            component_registry: Arc::new(ComponentRegistry::new_with_marks()),
        };
        for statement in &mut self.statements {
            statement.accept_mut(visitor, &root_ctx)?;
        }
        visitor.visit_avenger_file(self, &root_ctx)
    }
}

