use arrow::array::{ArrayRef, BooleanArray, RecordBatch};
use arrow::datatypes::DataType;
use async_trait::async_trait;
use datafusion::common::{Column, DFSchema, ExprSchema, ScalarValue};
use datafusion::error::DataFusionError;
use datafusion::logical_expr::utils::expr_to_columns;
use datafusion::logical_expr::{Expr, ExprSchemable, TryCast};
use datafusion::optimizer::simplify_expressions::SimplifyInfo;
use datafusion::physical_expr::execution_props::ExecutionProps;
use datafusion::prelude::SessionContext;
use lazy_static::lazy_static;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::sync::Arc;

lazy_static! {
    pub static ref UNIT_RECORD_BATCH: RecordBatch = RecordBatch::try_from_iter(vec![(
        "__unit__",
        Arc::new(BooleanArray::from(vec![true])) as ArrayRef
    )])
    .unwrap();
    pub static ref UNIT_SCHEMA: DFSchema =
        DFSchema::try_from(UNIT_RECORD_BATCH.schema().as_ref().clone()).unwrap();
}

#[async_trait]
pub trait ExprHelpers {
    fn columns(&self) -> Result<HashSet<Column>, DataFusionError>;
    async fn eval_to_scalar(&self, ctx: &SessionContext) -> Result<ScalarValue, DataFusionError>;
    fn try_cast_to(
        self,
        cast_to_type: &DataType,
        schema: &dyn ExprSchema,
    ) -> Result<Expr, DataFusionError>;
}

#[async_trait]
impl ExprHelpers for Expr {
    fn columns(&self) -> Result<HashSet<Column>, DataFusionError> {
        let mut columns: HashSet<Column> = HashSet::new();
        expr_to_columns(self, &mut columns)?;
        Ok(columns)
    }

    async fn eval_to_scalar(&self, ctx: &SessionContext) -> Result<ScalarValue, DataFusionError> {
        if !self.columns()?.is_empty() {
            return Err(DataFusionError::Internal(format!(
                "Cannot eval_to_scalar for Expr with column references: {self:?}"
            )));
        }
        let df = ctx.read_batch(UNIT_RECORD_BATCH.clone())?;
        let res = df
            .select(vec![self.clone().alias("value")])?
            .collect()
            .await?;
        let row = res.get(0).unwrap();
        let col = row.column_by_name("value").unwrap();
        let scalar = ScalarValue::try_from_array(col, 0)?;
        Ok(scalar)
    }

    fn try_cast_to(
        self,
        cast_to_type: &DataType,
        schema: &dyn ExprSchema,
    ) -> Result<Expr, DataFusionError> {
        // Based on cast_to, using TryCast instead of Cast
        let this_type = self.get_type(schema)?;
        if this_type == *cast_to_type {
            return Ok(self);
        }
        Ok(Expr::TryCast(TryCast::new(
            Box::new(self),
            cast_to_type.clone(),
        )))
    }
}
