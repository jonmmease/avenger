use sqlparser::dialect::GenericDialect;
use sqlparser::ast::{Value as SqlValue, Expr as SqlExpr, Query as SqlQuery};

//
//  sql_query and sql_expr rules are defined by sqlparser-rs
//
//  type           → "<" + IDENTIFIER + ">"
//  prop_qualifier → "in" | "out"
//  val_prop       → prop_qualifier? + "val" + type? + ":" + sql_expr + ";"
//  expr_prop      → prop_qualifier? + "expr" + type? + ":" + sql_expr + ";"
//  dataset_prop   → prop_qualifier? + "dataset" + type? + ":" + sql_query + ";"
//  comp_prop      → prop_qualifier? + "comp" + type? + ":" + comp_instance + ";"
//  comp_instance  → PASCAL_IDENTIFIER + "{" + statement* + "}"
//
//  statement      → (val_prop | expr_prop | dataset_prop | comp_prop) + ";"
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


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Statement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
    CompPropDecl(CompPropDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerFile {
    pub statements: Vec<Statement>,
}

// Visitor trait for traversing the AST with immutable references
pub trait Visitor {
    fn visit_val_prop_decl(&mut self, val_prop: &ValPropDecl, scope_path: &[String]) {}
    
    fn visit_expr_prop_decl(&mut self, expr_prop: &ExprPropDecl, scope_path: &[String]) {}
    
    fn visit_dataset_prop_decl(&mut self, dataset_prop: &DatasetPropDecl, scope_path: &[String]) {}
    
    fn visit_comp_prop_decl(&mut self, comp_prop: &CompPropDecl, scope_path: &[String]) {}
    
    fn visit_comp_instance(&mut self, comp_instance: &CompInstance, scope_path: &[String]) {}
    
    fn visit_statement(&mut self, statement: &Statement, scope_path: &[String]) {}
    
    fn visit_avenger_file(&mut self, file: &AvengerFile, scope_path: &[String]) {}
}

// Mutable visitor trait for traversing and potentially modifying the AST
pub trait VisitorMut {
    fn visit_val_prop_decl(&mut self, val_prop: &mut ValPropDecl, scope_path: &[String]) {}
    
    fn visit_expr_prop_decl(&mut self, expr_prop: &mut ExprPropDecl, scope_path: &[String]) {}
    
    fn visit_dataset_prop_decl(&mut self, dataset_prop: &mut DatasetPropDecl, scope_path: &[String]) {}
    
    fn visit_comp_prop_decl(&mut self, comp_prop: &mut CompPropDecl, scope_path: &[String]) {}
    
    fn visit_comp_instance(&mut self, comp_instance: &mut CompInstance, scope_path: &[String]) {}
    
    fn visit_statement(&mut self, statement: &mut Statement, scope_path: &[String]) {}
    
    fn visit_avenger_file(&mut self, file: &mut AvengerFile, scope_path: &[String]) {}
}

// Implementation of accept methods for each AST node
impl ValPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_val_prop_decl(self, scope_path);
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_val_prop_decl(self, scope_path);
    }
}

impl ExprPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_expr_prop_decl(self, scope_path);
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_expr_prop_decl(self, scope_path);
    }
}

impl DatasetPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_dataset_prop_decl(self, scope_path);
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_dataset_prop_decl(self, scope_path);
    }
}

impl CompPropDecl {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        let mut new_scope_path = scope_path.to_vec();
        new_scope_path.push(self.name.clone());
        
        visitor.visit_comp_prop_decl(self, &new_scope_path);
        self.value.accept(visitor, &new_scope_path);
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        let mut new_scope_path = scope_path.to_vec();
        new_scope_path.push(self.name.clone());

        self.value.accept_mut(visitor, &new_scope_path);
        visitor.visit_comp_prop_decl(self, &new_scope_path);
    }
}

impl CompInstance {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_comp_instance(self, &scope_path);
        for statement in &self.statements {
            statement.accept(visitor, &scope_path);
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        // Create a new scope path that includes this component
        let mut new_scope_path = scope_path.to_vec();
        new_scope_path.push(self.name.clone());
        
        visitor.visit_comp_instance(self, &new_scope_path);
        for statement in &mut self.statements {
            statement.accept_mut(visitor, &new_scope_path);
        }
    }
}

impl Statement {
    pub fn accept<V: Visitor>(&self, visitor: &mut V, scope_path: &[String]) {
        visitor.visit_statement(self, scope_path);
        match self {
            Statement::ValPropDecl(val) => val.accept(visitor, scope_path),
            Statement::ExprPropDecl(expr) => expr.accept(visitor, scope_path),
            Statement::DatasetPropDecl(dataset) => dataset.accept(visitor, scope_path),
            Statement::CompPropDecl(comp) => comp.accept(visitor, scope_path),
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V, scope_path: &[String]) {
        match self {
            Statement::ValPropDecl(val) => val.accept_mut(visitor, scope_path),
            Statement::ExprPropDecl(expr) => expr.accept_mut(visitor, scope_path),
            Statement::DatasetPropDecl(dataset) => dataset.accept_mut(visitor, scope_path),
            Statement::CompPropDecl(comp) => comp.accept_mut(visitor, scope_path),
        }
        visitor.visit_statement(self, scope_path);
    }
}

impl AvengerFile {
    pub fn accept<V: Visitor>(&self, visitor: &mut V) {
        let root_path: Vec<String> = Vec::new();
        visitor.visit_avenger_file(self, &root_path);
        for statement in &self.statements {
            statement.accept(visitor, &root_path);
        }
    }
    
    pub fn accept_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        let root_path: Vec<String> = Vec::new();
        for statement in &mut self.statements {
            statement.accept_mut(visitor, &root_path);
        }
        visitor.visit_avenger_file(self, &root_path);
    }
}

