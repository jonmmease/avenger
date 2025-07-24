//! Parameter types for interactive visualizations

use datafusion::scalar::ScalarValue;
use std::fmt::Debug;

/// A parameter that can be used in expressions and updated by controllers
#[derive(Debug, Clone)]
pub struct Param {
    name: String,
    value: ScalarValue,
}

impl Param {
    /// Create a new parameter with a name and initial value
    pub fn new(name: impl Into<String>, value: ScalarValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Get the parameter name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the current value
    pub fn value(&self) -> &ScalarValue {
        &self.value
    }

    /// Convert to a DataFusion expression placeholder
    pub fn expr(&self) -> datafusion::logical_expr::Expr {
        use datafusion::logical_expr::{Expr, expr::Placeholder};
        Expr::Placeholder(Placeholder {
            id: self.name.clone(),
            data_type: None,
        })
    }
}

/// Manages a collection of parameters
#[derive(Debug, Clone, Default)]
pub struct ParamRegistry {
    params: Vec<Param>,
}

impl ParamRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter to the registry
    pub fn add(&mut self, param: Param) -> &mut Self {
        self.params.push(param);
        self
    }

    /// Get all parameters
    pub fn params(&self) -> &[Param] {
        &self.params
    }
}
