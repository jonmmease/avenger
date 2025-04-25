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
//  component_decl → "component" + PASCAL_IDENTIFIER + "inherits" + PASCAL_IDENTIFIER + "{" + statement* + "}"
//  import_path   → IDENTIFIER + ("/" + IDENTIFIER)*
//  import       → "import" + import_path + ("as" + IDENTIFIER)? + ";"
//
//  statement      → (val_prop | expr_prop | dataset_prop | comp_prop | prop_binding | component_decl | import) + ";"
//  file           → comp_instance
//
#[derive(Debug, Clone, PartialEq, Eq, Hash) ]
pub struct Type(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
pub struct ComponentDef {
    pub name: String,
    pub inherits: String,
    pub statements: Vec<Statement>,
}

impl ComponentDef {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_component_def(self, ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, ctx)?;
        }
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_component_def(self, ctx)?;
        for statement in &mut self.statements {
            statement.accept_mut(visitor, ctx)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Statement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
    CompPropDecl(CompPropDecl),
    PropBinding(PropBinding),
    ComponentDef(ComponentDef),
    Import(Import),
    FunctionDef(FunctionDef),
    ConditionalIfComponents(ConditionalIfComponents),
    ConditionalMatchComponents(ConditionalMatchComponents),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import {
    pub from: Option<String>,
    pub items: Vec<ImportItem>,
}

impl Import {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_import(self, ctx)?;
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_import(self, ctx)?;
        Ok(())
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReturnStatement {
    pub value: SqlExprOrQuery,
}

impl ReturnStatement {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_return_statement(self, ctx)?;
        Ok(())
    }

    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_return_statement(self, ctx)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionValArgValue {
    pub value: SqlExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionExprArgValue {
    pub value: SqlExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionDatasetArgValue {
    pub value: Box<SqlQuery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionParamKind {
    Val,
    Expr,
    Dataset,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionParam {
    pub name: String,
    pub kind: FunctionParamKind,
    pub type_: Option<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionReturnParam {
    pub kind: FunctionParamKind,
    pub type_: Option<Type>,
}

impl FunctionParam {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionStatement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
}

impl TryFrom<Statement> for FunctionStatement {
    type Error = AvengerLangError;

    fn try_from(stmt: Statement) -> Result<Self, Self::Error> {
        match stmt {
            Statement::ValPropDecl(val_prop) => Ok(FunctionStatement::ValPropDecl(val_prop)),
            Statement::ExprPropDecl(expr_prop) => Ok(FunctionStatement::ExprPropDecl(expr_prop)),
            Statement::DatasetPropDecl(dataset_prop) => Ok(FunctionStatement::DatasetPropDecl(dataset_prop)),
            _ => Err(AvengerLangError::InternalError("Unsupported function statement".to_string())),
        }
    }
}

impl FunctionStatement {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        match self {
            FunctionStatement::ValPropDecl(val_prop) => val_prop.accept(visitor, ctx),
            FunctionStatement::ExprPropDecl(expr_prop) => expr_prop.accept(visitor, ctx),
            FunctionStatement::DatasetPropDecl(dataset_prop) => dataset_prop.accept(visitor, ctx),
        }
    }

    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        match self {
            FunctionStatement::ValPropDecl(val_prop) => val_prop.accept_mut(visitor, ctx),
            FunctionStatement::ExprPropDecl(expr_prop) => expr_prop.accept_mut(visitor, ctx),
            FunctionStatement::DatasetPropDecl(dataset_prop) => dataset_prop.accept_mut(visitor, ctx),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionDef {
    pub name: String,
    pub is_method: bool,
    pub params: Vec<FunctionParam>,
    pub return_param: FunctionReturnParam,
    pub statements: Vec<FunctionStatement>,
    pub return_statement: ReturnStatement,
}

impl FunctionDef {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_function_def(self, ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, ctx)?;
        }
        self.return_statement.accept(visitor, ctx)?;
        Ok(())
    }

    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_function_def(self, ctx)?;
        for statement in &mut self.statements {
            statement.accept_mut(visitor, ctx)?;
        }
        self.return_statement.accept_mut(visitor, ctx)?;
        Ok(())
    }
}


/// Statement allowed inside conditional component blocks
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConditionalComponentsStatement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
    CompPropDecl(CompPropDecl),
}

impl TryFrom<Statement> for ConditionalComponentsStatement {
    type Error = AvengerLangError;

    fn try_from(stmt: Statement) -> Result<Self, Self::Error> {
        match stmt {
            Statement::ValPropDecl(val_prop) => Ok(ConditionalComponentsStatement::ValPropDecl(val_prop)),
            Statement::ExprPropDecl(expr_prop) => Ok(ConditionalComponentsStatement::ExprPropDecl(expr_prop)),
            Statement::DatasetPropDecl(dataset_prop) => Ok(ConditionalComponentsStatement::DatasetPropDecl(dataset_prop)),
            Statement::CompPropDecl(comp_prop) => Ok(ConditionalComponentsStatement::CompPropDecl(comp_prop)),
            _ => Err(AvengerLangError::InternalError("Unsupported conditional components statement".to_string())),
        }
    }
}

impl ConditionalComponentsStatement {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_components_statement(self, ctx)?;
        match self {
            ConditionalComponentsStatement::ValPropDecl(val_prop) => val_prop.accept(visitor, ctx),
            ConditionalComponentsStatement::ExprPropDecl(expr_prop) => expr_prop.accept(visitor, ctx),
            ConditionalComponentsStatement::DatasetPropDecl(dataset_prop) => dataset_prop.accept(visitor, ctx),
            ConditionalComponentsStatement::CompPropDecl(comp_prop) => comp_prop.accept(visitor, ctx),
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_components_statement(self, ctx)?;
        match self {
            ConditionalComponentsStatement::ValPropDecl(val_prop) => val_prop.accept_mut(visitor, ctx),
            ConditionalComponentsStatement::ExprPropDecl(expr_prop) => expr_prop.accept_mut(visitor, ctx),
            ConditionalComponentsStatement::DatasetPropDecl(dataset_prop) => dataset_prop.accept_mut(visitor, ctx),
            ConditionalComponentsStatement::CompPropDecl(comp_prop) => comp_prop.accept_mut(visitor, ctx),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionalIfBranch {
    pub condition: SqlExpr,
    pub statements: Vec<ConditionalComponentsStatement>,
}

impl ConditionalIfBranch {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_if_branch(self, ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, ctx)?;
        }
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_if_branch(self, ctx)?;
        for statement in &mut self.statements {
            statement.accept_mut(visitor, ctx)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionalIfComponents {
    pub if_branches: Vec<ConditionalIfBranch>,
    pub else_branch: Option<Vec<ConditionalComponentsStatement>>,
}

impl ConditionalIfComponents {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_if_components(self, ctx)?;
        for branch in &self.if_branches {
            branch.accept(visitor, ctx)?;
            for statement in &branch.statements {
                statement.accept(visitor, ctx)?;
            }
        }
        if let Some(statements) = &self.else_branch {
            for statement in statements {
                statement.accept(visitor, ctx)?;
            }
        }
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_if_components(self, ctx)?;
        for branch in &mut self.if_branches {
            branch.accept_mut(visitor, ctx)?;
            for statement in &mut branch.statements {
                statement.accept_mut(visitor, ctx)?;
            }
        }
        if let Some(statements) = &mut self.else_branch {
            for statement in statements {
                statement.accept_mut(visitor, ctx)?;
            }
        }
        Ok(())
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionalMatchBranch {
    pub match_value: String,
    pub statements: Vec<ConditionalComponentsStatement>,
}

impl ConditionalMatchBranch {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_match_branch(self, ctx)?;
        for statement in &self.statements {
            statement.accept(visitor, ctx)?;
        }
        Ok(())
    }

    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_match_branch(self, ctx)?;
        for statement in &mut self.statements {
            statement.accept_mut(visitor, ctx)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionalMatchDefaultBranch {
    pub statements: Vec<ConditionalComponentsStatement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConditionalMatchComponents {
    pub match_expr: SqlExpr,
    pub branches: Vec<ConditionalMatchBranch>,
    pub default_branch: Option<ConditionalMatchDefaultBranch>,
}

impl ConditionalMatchComponents {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_match_components(self, ctx)?;
        for branch in &self.branches {
            branch.accept(visitor, ctx)?;
            for statement in &branch.statements {
                statement.accept(visitor, ctx)?;
            }
        }
        if let Some(statements) = &self.default_branch {
            for statement in &statements.statements {
                statement.accept(visitor, ctx)?;
            }
        }
        Ok(())
    }

    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        visitor.visit_conditional_match_components(self, ctx)?;
        for branch in &mut self.branches {
            branch.accept_mut(visitor, ctx)?;
            for statement in &mut branch.statements {
                statement.accept_mut(visitor, ctx)?;
            }
        }
        if let Some(statements) = &mut self.default_branch {
            for statement in &mut statements.statements {
                statement.accept_mut(visitor, ctx)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerFile {
    pub imports: Vec<Import>,
    pub main_component: CompInstance,
}

// Visitor trait for traversing the AST with immutable references
#[derive(Debug, Clone)]
pub struct VisitorContext {
    pub scope_path: Vec<String>,
    pub component_type: String,
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

    fn visit_component_def(&mut self, component_def: &ComponentDef, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_import(&mut self, import: &Import, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_function_def(&mut self, function_def: &FunctionDef, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_return_statement(&mut self, return_statement: &ReturnStatement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_prop_binding(&mut self, prop_binding: &PropBinding, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_statement(&mut self, statement: &Statement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_components_statement(&mut self, statement: &ConditionalComponentsStatement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_if_branch(&mut self, branch: &ConditionalIfBranch, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_if_components(&mut self, components: &ConditionalIfComponents, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_match_branch(&mut self, branch: &ConditionalMatchBranch, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_match_components(&mut self, components: &ConditionalMatchComponents, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
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

    fn visit_component_def(&mut self, component_def: &mut ComponentDef, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_import(&mut self, import: &mut Import, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_function_def(&mut self, function_def: &mut FunctionDef, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_return_statement(&mut self, return_statement: &mut ReturnStatement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }
    
    fn visit_statement(&mut self, statement: &mut Statement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_components_statement(&mut self, statement: &mut ConditionalComponentsStatement, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_if_branch(&mut self, branch: &mut ConditionalIfBranch, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_if_components(&mut self, components: &mut ConditionalIfComponents, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_match_branch(&mut self, branch: &mut ConditionalMatchBranch, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        Ok(())
    }

    fn visit_conditional_match_components(&mut self, components: &mut ConditionalMatchComponents, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
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

        self.value.accept(visitor, &new_ctx)?;
        visitor.visit_comp_prop_decl(self, &new_ctx)?;
        Ok(())
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
        visitor.visit_comp_prop_decl(self, &new_ctx)?;
        Ok(())
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
            Statement::ComponentDef(comp_decl) => comp_decl.accept(visitor, ctx),
            Statement::Import(import) => import.accept(visitor, ctx),
            Statement::FunctionDef(function_def) => function_def.accept(visitor, ctx),
            Statement::ConditionalIfComponents(components) => components.accept(visitor, ctx),
            Statement::ConditionalMatchComponents(components) => components.accept(visitor, ctx),
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        match self {
            Statement::ValPropDecl(val) => val.accept_mut(visitor, ctx)?,
            Statement::ExprPropDecl(expr) => expr.accept_mut(visitor, ctx)?,
            Statement::DatasetPropDecl(dataset) => dataset.accept_mut(visitor, ctx)?,
            Statement::CompPropDecl(comp) => comp.accept_mut(visitor, ctx)?,
            Statement::PropBinding(prop) => prop.accept_mut(visitor, ctx)?,
            Statement::ComponentDef(comp_decl) => comp_decl.accept_mut(visitor, ctx)?,
            Statement::Import(import) => import.accept_mut(visitor, ctx)?,
            Statement::FunctionDef(function_def) => function_def.accept_mut(visitor, ctx)?,
            Statement::ConditionalIfComponents(components) => components.accept_mut(visitor, ctx)?,
            Statement::ConditionalMatchComponents(components) => components.accept_mut(visitor, ctx)?,
        }
        visitor.visit_statement(self, ctx)
    }
}

impl AvengerFile {
    pub fn accept<V: Visitor>(&self, visitor: &mut V) -> Result<(), AvengerLangError> {
        let root_ctx = VisitorContext {
            scope_path: Vec::new(),
            component_type: "App".to_string(),
        };
        visitor.visit_avenger_file(self, &root_ctx)?;
        for import in &self.imports {
            import.accept(visitor, &root_ctx)?;
        }
        self.main_component.accept(visitor, &root_ctx)?;
        Ok(())
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V) -> Result<(), AvengerLangError> {
        let root_ctx = VisitorContext {
            scope_path: Vec::new(),
            component_type: "App".to_string(),
        };
        visitor.visit_avenger_file(self, &root_ctx)?;
        for import in &mut self.imports {
            import.accept_mut(visitor, &root_ctx)?;
        }
        self.main_component.accept_mut(visitor, &root_ctx)?;
        Ok(())
    }
}

