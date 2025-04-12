use std::ops::ControlFlow;

use sqlparser::ast::{Expr as SqlExpr, ObjectName, Query as SqlQuery, Visit, Visitor};
use async_trait::async_trait;

use crate::{context::EvaluationContext, error::AvengerLangError, value::{TaskValue, Variable, VariableKind}};


#[async_trait]
pub trait Task {
    /// Get the dependencies of the task
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        Ok(vec![])
    }

    fn output_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        Ok(vec![])
    }

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<(TaskValue, Vec<TaskValue>), AvengerLangError>;
}


/// A task that evaluates to a scalarvalue
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValDeclTask {
    pub name: String,
    pub value: SqlExpr,
}

impl ValDeclTask {
    pub fn new(name: String, value: SqlExpr) -> Self {
        Self { name, value }
    }
}

#[async_trait]
impl Task for ValDeclTask {    
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.value.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<(TaskValue, Vec<TaskValue>), AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let val = ctx.evaluate_expr(&self.value).await?;
        Ok((TaskValue::Val(val), vec![]))
    }
}

/// A task that evaluates to an expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprTask {
    pub name: String,
    pub expr: SqlExpr,
}

impl ExprTask {
    pub fn new(name: String, expr: SqlExpr) -> Self {
        Self { name, expr }
    }
}

#[async_trait]
impl Task for ExprTask {
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<(TaskValue, Vec<TaskValue>), AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let expr = ctx.compile_expr(&self.expr)?;
        Ok((TaskValue::Expr(expr), vec![]))
    }
}

/// A task that evaluates to a dataset
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetTask {
    pub name: String,
    pub query: SqlQuery,
}

impl DatasetTask {
    pub fn new(name: String, query: SqlQuery) -> Self {
        Self { name, query }
    }
}

#[async_trait]
impl Task for DatasetTask {
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.query.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<(TaskValue, Vec<TaskValue>), AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let plan = ctx.compile_query(&self.query).await?;
        Ok((TaskValue::Dataset(plan), vec![]))
    }
}


pub struct CollectDependenciesVisitor {
    /// The variables that are dependencies of the task, without leading @
    deps: Vec<Variable>,
}

impl CollectDependenciesVisitor {
    pub fn new() -> Self {
        Self { deps: vec![] }
    }
}

impl Visitor for CollectDependenciesVisitor {
    type Break = Result<(), AvengerLangError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // 
        if table_name.starts_with("@") {
            self.deps.push(Variable::new(table_name[1..].to_string(), VariableKind::Dataset));
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::Identifier(ident) = expr.clone() {
            if ident.value.starts_with("@") {
                self.deps.push(Variable::new(
                    ident.value[1..].to_string(), VariableKind::ValOrExpr)
                );
            }
        }
        ControlFlow::Continue(())
    }
}



// Eventually a Callback task that will output variables