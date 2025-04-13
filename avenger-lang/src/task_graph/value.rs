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

impl Variable {
    pub fn new<T: Into<String>>(name: T, kind: VariableKind) -> Self {
        Self { name: name.into(), kind }
    }
}