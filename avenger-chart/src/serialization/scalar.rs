//! Serializable wrapper for ScalarValue

use super::SerializableExpr;
use crate::error::AvengerChartError;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

/// Wrapper for ScalarValue that implements Serialize/Deserialize
/// by converting to/from a literal Expr
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableScalar {
    expr: SerializableExpr,
}

impl SerializableScalar {
    /// Create from a ScalarValue
    pub fn from_scalar(scalar: ScalarValue) -> Result<Self, AvengerChartError> {
        // Convert ScalarValue to literal Expr then serialize
        let expr = lit(scalar);
        Ok(Self {
            expr: SerializableExpr::from_expr(expr)?,
        })
    }

    /// Convert back to ScalarValue
    pub fn to_scalar(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<ScalarValue, AvengerChartError> {
        // Deserialize as Expr then extract the literal value
        let expr = self.expr.to_expr(ctx)?;

        // Extract scalar from literal expression
        match expr {
            Expr::Literal(scalar, _) => Ok(scalar),
            _ => Err(AvengerChartError::InternalError(
                "Expected literal expression when deserializing ScalarValue".to_string(),
            )),
        }
    }
}

impl PartialEq for SerializableScalar {
    fn eq(&self, other: &Self) -> bool {
        // Use PartialEq implementation from SerializableExpr
        self.expr == other.expr
    }
}
