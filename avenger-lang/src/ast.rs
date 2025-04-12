


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
//  statement      → (val_prop | expr_prop | dataset_prop) + ";"
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
pub enum Statement {
    ValPropDecl(ValPropDecl),
    ExprPropDecl(ExprPropDecl),
    DatasetPropDecl(DatasetPropDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerFile {
    pub statements: Vec<Statement>,
}

