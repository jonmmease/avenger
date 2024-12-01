use std::{collections::HashSet, sync::Arc};

use arrow::array::{ArrayRef, BooleanArray, RecordBatch};
use datafusion::common::{Column, DFSchema, ScalarValue};
use datafusion::logical_expr::utils::expr_to_columns;
use datafusion::logical_expr::ColumnarValue;
use datafusion::physical_expr::PhysicalExpr;
use datafusion::prelude::{Expr, SessionContext};

use crate::error::AvengerError;

lazy_static! {
    pub static ref UNIT_RECORD_BATCH: RecordBatch = RecordBatch::try_from_iter(vec![(
        "__unit__",
        Arc::new(BooleanArray::from(vec![true])) as ArrayRef
    )])
    .unwrap();
    pub static ref UNIT_SCHEMA: DFSchema =
        DFSchema::try_from(UNIT_RECORD_BATCH.schema().as_ref().clone()).unwrap();
}

pub trait ExprHelpers {
    fn columns(&self) -> Result<HashSet<Column>, AvengerError>;
    fn to_phys_expr(&self, ctx: &SessionContext) -> Result<Arc<dyn PhysicalExpr>, AvengerError>;
    fn eval_to_scalar(&self, ctx: &SessionContext) -> Result<ScalarValue, AvengerError>;
    fn eval(
        &self,
        ctx: &SessionContext,
        batch: &RecordBatch,
    ) -> Result<ColumnarValue, AvengerError>;
}

impl ExprHelpers for Expr {
    fn columns(&self) -> Result<HashSet<Column>, AvengerError> {
        let mut columns: HashSet<Column> = HashSet::new();
        expr_to_columns(self, &mut columns)?;
        Ok(columns)
    }

    fn to_phys_expr(&self, ctx: &SessionContext) -> Result<Arc<dyn PhysicalExpr>, AvengerError> {
        let phys_expr = ctx.create_physical_expr(self.clone(), &UNIT_SCHEMA)?;
        Ok(phys_expr)
    }

    fn eval_to_scalar(&self, ctx: &SessionContext) -> Result<ScalarValue, AvengerError> {
        if !self.columns()?.is_empty() {
            return Err(AvengerError::InternalError(format!(
                "Cannot eval_to_scalar for Expr with column references: {self:?}"
            )));
        }

        let phys_expr = self.to_phys_expr(ctx)?;
        let col_result = phys_expr.evaluate(&UNIT_RECORD_BATCH)?;
        match col_result {
            ColumnarValue::Scalar(scalar) => Ok(scalar),
            ColumnarValue::Array(array) => {
                if array.len() != 1 {
                    return Err(AvengerError::InternalError(format!(
                        "Unexpected non-scalar array result when evaluate expr: {self:?}"
                    )));
                }
                Ok(ScalarValue::try_from_array(&array, 0)?)
            }
        }
    }

    fn eval(
        &self,
        ctx: &SessionContext,
        batch: &RecordBatch,
    ) -> Result<ColumnarValue, AvengerError> {
        let phys_expr = self.to_phys_expr(ctx)?;
        Ok(phys_expr.evaluate(batch)?)
    }
}
