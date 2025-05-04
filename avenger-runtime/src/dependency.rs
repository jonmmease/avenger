use crate::{error::AvengerRuntimeError, variable::Variable};
use sqlparser::ast::{CreateFunction, Expr as SqlExpr, ObjectName, Query as SqlQuery, Visit, Visitor};
use std::ops::ControlFlow;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
    Function,
    Mark,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub variable: Variable,
    pub kind: DependencyKind,
}

impl Dependency {
    pub fn new(parts: Vec<String>, kind: DependencyKind) -> Self {
        Self {
            variable: Variable::new(parts),
            kind,
        }
    }
}

pub struct CollectDependenciesVisitor {
    /// The variables that are dependencies of the task, without leading @
    deps: Vec<Dependency>,
}

impl CollectDependenciesVisitor {
    pub fn new() -> Self {
        Self { deps: vec![] }
    }

    pub fn add_dependency(&mut self, parts: Vec<String>, kind: DependencyKind) {
        let dep = Dependency::new(parts, kind);
        if !self.deps.contains(&dep) {
            self.deps.push(dep);
        }
    }
}

impl Visitor for CollectDependenciesVisitor {
    type Break = Result<(), AvengerRuntimeError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // Handle dataset references
        if table_name.starts_with("@") {
            // Drop leading @ and split on .
            let parts = table_name[1..]
                .split(".")
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            self.add_dependency(parts, DependencyKind::Dataset);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        match &expr {
            SqlExpr::Function(func) => {
                if func.name.0[0].value.starts_with("@") {
                    // Build variable, without the leading @
                    let mut parts: Vec<String> = func.name.0.iter().map(|ident| ident.value.clone()).collect();
                    parts[0] = parts[0][1..].to_string();
                    self.add_dependency(parts, DependencyKind::Function);
                }
            }
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    self.add_dependency(
                        vec![ident.value[1..].to_string()],
                        DependencyKind::ValOrExpr,
                    );
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts: Vec<String> =
                        idents.iter().map(|ident| ident.value.clone()).collect();
                    // Drop the leading @
                    parts[0] = parts[0][1..].to_string();
                    self.add_dependency(parts, DependencyKind::ValOrExpr);
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

pub fn collect_expr_dependencies(expr: &SqlExpr) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = expr.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}

pub fn collect_query_dependencies(
    query: &SqlQuery,
) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = query.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}

pub fn collect_function_dependencies(function: &CreateFunction) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = function.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}
