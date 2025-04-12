use datafusion::prelude::{DataFrame, Expr, SessionContext};
use datafusion_common::ScalarValue;
use sqlparser::ast::{Query as SqlQuery, Expr as SqlExpr};
use async_trait::async_trait;

use crate::{compiler::{compile_expr, evaluate_val_expr}, context::EvaluationContext, error::AvengerLangError};



/// The value of a task
pub enum TaskValue {
    Val(ScalarValue),
    Expr(Expr),
    Dataset(DataFrame),
}

impl TaskValue {
    pub fn as_val(&self) -> Result<&ScalarValue, AvengerLangError> {
        match self {
            TaskValue::Val(val) => Ok(val),
            _ => Err(AvengerLangError::InternalError("Expected a value".to_string())),
        }
    }

    pub fn as_expr(&self) -> Result<&Expr, AvengerLangError> {
        match self {
            TaskValue::Expr(expr) => Ok(expr),
            _ => Err(AvengerLangError::InternalError("Expected an expression".to_string())),
        }
    }

    pub fn as_dataset(&self) -> Result<&DataFrame, AvengerLangError> {
        match self {
            TaskValue::Dataset(df) => Ok(df),
            _ => Err(AvengerLangError::InternalError("Expected a dataset".to_string())),
        }
    }
}



#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VariableKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
}

#[async_trait]
pub trait Task {
    /// Get the dependencies of the task
    fn dependencies(&self) -> Result<Vec<Variable>, AvengerLangError>;

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        ctx: &EvaluationContext,
        dependencies: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError>;
}


/// A task that evaluates to a scalarvalue
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValTask {
    pub name: String,
    pub value: SqlExpr,
}

#[async_trait]
impl Task for ValTask {
    fn dependencies(&self) -> Result<Vec<Variable>, AvengerLangError> {
        Ok(vec![])
    }
    
    async fn evaluate(
        &self,
        ctx: &EvaluationContext,
        dependencies: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let expr = compile_expr(&self.value, ctx).await?;
        let val = evaluate_val_expr(expr, ctx).await?;
        Ok(TaskValue::Val(val))
    }
}



/// A task that evaluates to an expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprTask {
    pub name: String,
    pub expr: SqlExpr,
}

/// A task that evaluates to a dataset
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetTask {
    pub name: String,
    pub query: SqlQuery,
}

