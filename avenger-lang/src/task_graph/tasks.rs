use std::{fmt::Debug, ops::ControlFlow};

use sqlparser::ast::{Expr as SqlExpr, ObjectName, Query as SqlQuery, Visit, Visitor};
use async_trait::async_trait;

use crate::{ast::{DatasetPropDecl, ExprPropDecl, ValPropDecl}, context::EvaluationContext, error::AvengerLangError, task_graph::{dependency::{Dependency, DependencyKind}, value::{TaskDataset, TaskValue}}};

use super::{value::TaskValueContext, variable::Variable};


#[async_trait]
pub trait Task: Debug + Send + Sync {
    /// Get the dependencies of the task
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        Ok(vec![])
    }

    /// Get the input variables of the task
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        Ok(self.input_dependencies()?.iter().map(
            |dep| Variable::new(dep.name.clone())
        ).collect())
    }

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError>;
}


/// Task storing a scalar value
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskValueTask {
    pub value: TaskValue,
}

impl TaskValueTask {
    pub fn new(value: TaskValue) -> Self {
        Self { value }
    }
}

#[async_trait]
impl Task for TaskValueTask {
    async fn evaluate(
        &self,
        _input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        Ok(self.value.clone())
    }
}

/// A task that evaluates to a scalarvalue
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValDeclTask {
    pub value: SqlExpr,
}

impl ValDeclTask {
    pub fn new(value: SqlExpr) -> Self {
        Self { value }
    }
}

#[async_trait]
impl Task for ValDeclTask {    
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.value.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let val = ctx.evaluate_expr(&self.value).await?;
        Ok(TaskValue::Val { value: val })
    }
}

impl From<ValPropDecl> for ValDeclTask {
    fn from(val_prop_decl: ValPropDecl) -> Self {
        Self { value: val_prop_decl.value }
    }
}

/// A task that evaluates to an expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprDeclTask {
    pub expr: SqlExpr,
}

impl ExprDeclTask {
    pub fn new(expr: SqlExpr) -> Self {
        Self { expr }
    }
}

#[async_trait]
impl Task for ExprDeclTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;

        let sql_expr = ctx.expand_expr(&self.expr)?;
        let task_value_context = TaskValueContext::from_combined_task_value_context(
            &self.input_variables()?, &input_values
        )?;
        Ok(TaskValue::Expr { context: task_value_context, sql_expr })
    }
}

impl From<ExprPropDecl> for ExprDeclTask {
    fn from(expr_prop_decl: ExprPropDecl) -> Self {
        Self { expr: expr_prop_decl.value }
    }
}

/// A task that evaluates to a dataset
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetDeclTask {
    pub query: Box<SqlQuery>,
    pub eval: bool,
}

impl DatasetDeclTask {
    pub fn new(query: SqlQuery, eval: bool) -> Self {
        Self { query: Box::new(query), eval }
    }
}

#[async_trait]
impl Task for DatasetDeclTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.query.visit(&mut visitor) {
            return Err(err);
        }
        println!("DatasetDeclTask deps: {:#?}", visitor.deps);
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        println!("Evaluating DatasetDeclTask with input_values: {:#?}", input_values);
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let plan = ctx.compile_query(&self.query).await?;

        if self.eval {
            // Eager evaluation, evaluate the logical plan
            let table = ctx.eval_query(&self.query).await?;
            Ok(TaskValue::Dataset { context: Default::default() , dataset: TaskDataset::ArrowTable(table) })
        } else {
            // Lazy evaluation, return the logical plan, along with the reference value context
            let task_value_context = TaskValueContext::from_combined_task_value_context(
                &self.input_variables()?, &input_values
            )?;
    
            Ok(TaskValue::Dataset { context: task_value_context, dataset: TaskDataset::LogicalPlan(plan) })
        }
    }
}

impl From<DatasetPropDecl> for DatasetDeclTask {
    fn from(dataset_prop_decl: DatasetPropDecl) -> Self {
        Self { query: dataset_prop_decl.value, eval: true }
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
}

impl Visitor for CollectDependenciesVisitor {
    type Break = Result<(), AvengerLangError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // 
        if table_name.starts_with("@") {
            self.deps.push(Dependency::new(table_name[1..].to_string(), DependencyKind::Dataset));
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::Identifier(ident) = expr.clone() {
            if ident.value.starts_with("@") {
                self.deps.push(Dependency::new(
                    ident.value[1..].to_string(), DependencyKind::ValOrExpr)
                );
            }
        }
        ControlFlow::Continue(())
    }
}
