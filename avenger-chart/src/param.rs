//! Parameter support for parameterized plots

use datafusion::logical_expr::expr::Placeholder;
use datafusion::prelude::Expr;
use datafusion::scalar::ScalarValue;
use std::fmt::Debug;

/// A parameter that can be used in plot expressions
#[derive(Debug, Clone)]
pub struct Param {
    /// The name of the parameter
    pub name: String,
    /// The default value of the parameter
    pub default: ScalarValue,
}

impl Param {
    /// Create a new parameter with a name and default value
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
        }
    }

    /// Get a DataFusion expression for this parameter as a placeholder
    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder {
            id: format!("${}", self.name),
            data_type: Some(self.default.data_type()),
        })
    }
}

impl From<(String, ScalarValue)> for Param {
    fn from(params: (String, ScalarValue)) -> Self {
        Param::new(params.0, params.1)
    }
}

impl From<Param> for Expr {
    fn from(param: Param) -> Self {
        param.expr()
    }
}

impl From<&Param> for Expr {
    fn from(param: &Param) -> Self {
        param.expr()
    }
}