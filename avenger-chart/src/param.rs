use std::collections::HashMap;

use datafusion::{logical_expr::expr::Placeholder, prelude::Expr, scalar::ScalarValue};

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: ScalarValue,
}

impl Param {
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
        }
    }

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
