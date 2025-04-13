use datafusion::{logical_expr::LogicalPlan, prelude::Expr};
use datafusion_common::ScalarValue;

use crate::error::AvengerLangError;

/// The value of a task
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub enum TaskValue {
    Val(ScalarValue),
    Expr(Expr),
    Dataset(LogicalPlan),
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

    pub fn as_dataset(&self) -> Result<&LogicalPlan, AvengerLangError> {
        match self {
            TaskValue::Dataset(df) => Ok(df),
            _ => Err(AvengerLangError::InternalError("Expected a dataset".to_string())),
        }
    }
}
