use std::ops::ControlFlow;

use sqlparser::ast::{Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, Visit, Visitor, VisitorMut};

use crate::{context::TaskEvaluationContext, dependency::{Dependency, DependencyKind}, error::AvengerRuntimeError, variable::Variable};


pub struct CollectDependenciesVisitor {
    /// The variables that are dependencies of the task, without leading @
    deps: Vec<Dependency>,
}

impl CollectDependenciesVisitor {
    pub fn new() -> Self {
        Self { deps: vec![] }
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
            let parts = table_name[1..].split(".").map(|s| s.to_string()).collect::<Vec<_>>();

            self.deps.push(Dependency::new(
                parts, DependencyKind::Dataset)
            );
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        match &expr {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    self.deps.push(Dependency::new(
                        vec![ident.value[1..].to_string()], DependencyKind::ValOrExpr)
                    );
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts: Vec<String> = idents.iter().map(|ident| ident.value.clone()).collect();
                    // Drop the leading @
                    parts[0] = parts[0][1..].to_string();
                    self.deps.push(Dependency::new(
                        parts, DependencyKind::ValOrExpr)
                    );
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

pub fn collect_query_dependencies(query: &SqlQuery) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = query.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}



pub struct CompilationVisitor<'a> {
    ctx: &'a TaskEvaluationContext,
}

impl<'a> CompilationVisitor<'a> {
    pub fn new(ctx: &'a TaskEvaluationContext) -> Self {
        Self { ctx }
    }
}

impl<'a> VisitorMut for CompilationVisitor<'a> {
    type Break = Result<(), AvengerRuntimeError>;

    fn pre_visit_relation(&mut self, relation: &mut datafusion_sql::sqlparser::ast::ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();
    
        if table_name.starts_with("@") {
            let mut parts = relation.0.iter().map(|ident| ident.value.clone()).collect::<Vec<_>>();

            // Join on __ into a single string
            parts = vec![parts.join("__")];

            // Update the relation to use the mangled name
            let idents = parts.iter().map(|s| Ident::new(s.to_string())).collect::<Vec<_>>();

            *relation = ObjectName(idents);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    let variable = Variable::new(vec![ident.value[1..].to_string()]);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name())))
                        );
                    }
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts = idents.iter().map(|ident| ident.value.clone()).collect::<Vec<_>>();
                    parts[0] = parts[0][1..].to_string();
                    let variable = Variable::new(parts);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name())))
                        );
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}
