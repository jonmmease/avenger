use std::fmt;
use sqlparser::{
    ast::{CreateFunction, Expr as SqlExpr, Ident, Query as SqlQuery, Spanned},
    tokenizer::Span,
};
use crate::error::AvengerLangError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvengerScript {
    pub statements: Vec<ScriptStatement>,
}

impl fmt::Display for AvengerScript {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for stmt in &self.statements {
            writeln!(f, "{}", stmt)?;
        }
        Ok(())
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScriptStatement {
    ValDecl(ValDecl),
    ExprDecl(ExprDecl),
    TableDecl(TableDecl),
    VarAssignment(VarAssignment),
    Block(Block)
}

impl fmt::Display for ScriptStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptStatement::ValDecl(val_prop) => write!(f, "{}", val_prop),
            ScriptStatement::ExprDecl(expr_prop) => write!(f, "{}", expr_prop),
            ScriptStatement::TableDecl(dataset_prop) => write!(f, "{}", dataset_prop),
            ScriptStatement::VarAssignment(var_assignment) => write!(f, "{}", var_assignment),
            ScriptStatement::Block(block) => write!(f, "{}", block),
        }
    }
}

impl Spanned for ScriptStatement {
    fn span(&self) -> Span {
        match self {
            ScriptStatement::ValDecl(val_prop) => val_prop.span(),
            ScriptStatement::ExprDecl(expr_prop) => expr_prop.span(),
            ScriptStatement::TableDecl(dataset_prop) => dataset_prop.span(),
            ScriptStatement::VarAssignment(var_assignment) => var_assignment.span(),
            ScriptStatement::Block(block) => block.span(),
        }
    }
}

// Block
// -----
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    pub statements: Vec<ScriptStatement>,
}

impl Spanned for Block {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        for stmt in &self.statements {
            span = span.union(&stmt.span());
        }
        span
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indent = 0;
        let next_indent = indent + 2;

        let indent_str = " ".repeat(indent);
        let next_indent_str = " ".repeat(next_indent);

        // Fix the indentation for nested blocks
        writeln!(f, "{}{{", indent_str)?;
        for stmt in &self.statements {
            writeln!(f, "{next_indent_str}{stmt}")?;
        }
        writeln!(f, "{indent_str}}}")?;
        Ok(())
    }
}

// Val Declaration
// ---------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValDecl {
    pub val_keyword: KeywordVal,
    pub name: Ident,
    pub expr: SqlExpr,
}

impl ValDecl {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for ValDecl {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        span = span.union(&self.name.span);
        span = span.union(&self.expr.span());
        span
    }
}

impl fmt::Display for ValDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "val {}: {};", self.name.value, self.expr)
    }
}

// Expression Declaration
// ----------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprDecl {
    pub expr_keyword: KeywordExpr,
    pub name: Ident,
    pub expr: SqlExpr,
}

impl ExprDecl {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for ExprDecl {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        span = span.union(&self.name.span);
        span = span.union(&self.expr.span());
        span
    }
}

impl fmt::Display for ExprDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expr {}: {};", self.name.value, self.expr)
    }
}


// Dataset Declaration
// -------------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableDecl {
    pub table_keyword: KeywordTable,
    pub name: Ident,
    pub query: Box<SqlQuery>,
}

impl TableDecl {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for TableDecl {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        span = span.union(&self.name.span);
        span = span.union(&self.query.span());
        span
    }
}

impl fmt::Display for TableDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "table {}: {};", self.name.value, self.query)
    }
}

// prop binding
// ------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VarAssignment {
    pub name: Ident,
    pub expr: SqlExprOrQuery,
}

impl VarAssignment {
    pub fn name(&self) -> &str {
        self.name.value.as_str()
    }
}

impl Spanned for VarAssignment {
    fn span(&self) -> Span {
        Span::union_iter([self.name.span, self.expr.span()])
    }
}

impl fmt::Display for VarAssignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} := {};", self.name.value, self.expr)
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
define_keyword!(KeywordTable);
define_keyword!(KeywordComponent);
define_keyword!(KeywordComp);
define_keyword!(KeywordFn);
define_keyword!(KeywordReturn);
define_keyword!(KeywordFrom);


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

