use sqlparser::{ast::{Expr as SqlExpr, Query as SqlQuery, Spanned}, tokenizer::{Span, Word}};


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File {
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
    FunctionDef(FunctionDef),
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
            Statement::FunctionDef(function_def) => function_def.span(),
        }
    }
}

// Import
// ------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportItem {
    pub name: Identifier,
    pub as_: Option<(KeywordAs, Identifier)>,
}

impl Spanned for ImportItem {
    fn span(&self) -> Span {
        if let Some((as_, alias)) = &self.as_ {
            Span::union_iter([
                self.name.span(),
                as_.span(),
                alias.span(),
            ])
        } else {
            self.name.span()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportStatement {
    pub items: Vec<ImportItem>,
    pub from: Option<Identifier>,
}

impl Spanned for ImportStatement {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        for item in &self.items {
            span = span.union(&item.span());
        }
        if let Some(from) = &self.from {
            span = span.union(&from.span());
        }
        span
    }
}

// Val prop
// -------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Type {
    pub name: Identifier,
}

impl Spanned for Type {
    fn span(&self) -> Span {
        self.name.span()
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValProp {
    pub qualifier: Option<Qualifier>,
    pub type_: Option<Type>,
    pub name: Identifier,
    pub expr: SqlExpr,
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
        span = span.union(&self.name.span());
        span = span.union(&self.expr.span());
        span
    }
}

// Expr prop
// ---------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprProp {
    pub qualifier: Option<Qualifier>,
    pub type_: Option<Type>,
    pub name: Identifier,
    pub expr: SqlExpr,
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
        span = span.union(&self.name.span());
        span = span.union(&self.expr.span());
        span
    }
}

// Dataset prop
// ------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetProp {
    pub qualifier: Option<Qualifier>,
    pub type_: Option<Type>,
    pub name: Identifier,
    pub query: SqlQuery,
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
        span = span.union(&self.name.span());
        span = span.union(&self.query.span());
        span
    }
}

// Component prop
// --------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentProp {
    pub qualifier: Option<Qualifier>,
    // Name is optional for components
    pub prop_name: Option<Identifier>,
    pub component_name: Identifier,
    pub statements: Vec<Statement>,
}

impl Spanned for ComponentProp {
    fn span(&self) -> Span {
        let mut span = Span::empty();
        if let Some(qualifier) = &self.qualifier {
            span = span.union(&qualifier.span());
        }
        if let Some(prop_name) = &self.prop_name {
            span = span.union(&prop_name.span());
        }
        span = span.union(&self.component_name.span());
        for statement in &self.statements {
            span = span.union(&statement.span());
        }
        span
    }
}

// prop binding
// ------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropBinding {
    pub name: Identifier,
    pub expr: SqlExprOrQuery,
}

impl Spanned for PropBinding {
    fn span(&self) -> Span {
        Span::union_iter([
            self.name.span(),
            self.expr.span(),
        ])
    }
}

// function definition
// -------------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Val(KeywordVal),
    Expr(KeywordExpr),
    Dataset(KeywordDataset),
}

impl Spanned for ParamKind {
    fn span(&self) -> Span {
        match self {
            ParamKind::Val(kw) => kw.span(),
            ParamKind::Expr(kw) => kw.span(),
            ParamKind::Dataset(kw) => kw.span(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionParam {
    pub name: Identifier,
    pub type_: Option<Type>,
    pub kind: ParamKind,
}

impl Spanned for FunctionParam {
    fn span(&self) -> Span {
        Span::union_iter([
            self.name.span(),
            self.type_.as_ref().map_or(Span::empty(), |t| t.span()),
            self.kind.span(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionReturnParam {
    pub type_: Option<Type>,
    pub kind: ParamKind,
}

impl Spanned for FunctionReturnParam {
    fn span(&self) -> Span {
        Span::union_iter([
            self.type_.as_ref().map_or(Span::empty(), |t| t.span()),
            self.kind.span(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionStatement {
    ValProp(ValProp),
    ExprProp(ExprProp),
    DatasetProp(DatasetProp),
}

impl Spanned for FunctionStatement {
    fn span(&self) -> Span {
        match self {
            FunctionStatement::ValProp(prop) => prop.span(),
            FunctionStatement::ExprProp(prop) => prop.span(),
            FunctionStatement::DatasetProp(prop) => prop.span(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionReturn {
    pub keyword: KeywordReturn,
    pub value: SqlExprOrQuery,
}

impl Spanned for FunctionReturn {
    fn span(&self) -> Span {
        Span::union_iter([
            self.keyword.span(),
            self.value.span(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionDef {
    pub name: Identifier,
    pub params: Vec<FunctionParam>,
    pub return_param: FunctionReturnParam,
    pub statements: Vec<FunctionStatement>,
    pub return_statement: FunctionReturn,
}

impl Spanned for FunctionDef {
    fn span(&self) -> Span {
        Span::union_iter([
            self.name.span(),
            Span::union_iter(self.params.iter().map(|p| p.span())),
            self.return_param.span(),
            Span::union_iter(self.statements.iter().map(|s| s.span())),
            self.return_statement.span(),
        ])
    }
}

// sql expr or query
// -----------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SqlExprOrQuery {
    Expr(SqlExpr),
    Query(Box<SqlQuery>),
}

impl Spanned for SqlExprOrQuery {
    fn span(&self) -> Span {
        match self {
            SqlExprOrQuery::Expr(expr) => expr.span(),
            SqlExprOrQuery::Query(query) => query.span(),
        }
    }
}

// identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub span: Span,
    pub name: String,
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
