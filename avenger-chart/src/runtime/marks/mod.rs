pub mod arc;
pub mod encoding;
pub mod symbol;

use super::context::CompilationContext;
use crate::utils::ExprHelpers;
use crate::{
    error::AvengerChartError,
    types::mark::{Encoding, Mark},
};
use arrow::{
    array::{ArrayRef, Float32Array, RecordBatch},
    compute::{cast, concat_batches},
    datatypes::DataType,
};
use async_trait::async_trait;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::{DataFrame, Expr};
use indexmap::IndexMap;

#[async_trait]
pub trait MarkCompiler: Send + Sync + 'static {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

#[derive(Clone, Debug, PartialEq)]
struct EncodingBatches {
    scalar_batch: RecordBatch,
    column_batch: RecordBatch,
}

impl EncodingBatches {
    pub fn new(scalar_batch: RecordBatch, column_batch: RecordBatch) -> Self {
        Self {
            scalar_batch,
            column_batch,
        }
    }

    pub fn get_scalar_batch(&self) -> &RecordBatch {
        &self.scalar_batch
    }

    pub fn get_column_batch(&self) -> &RecordBatch {
        &self.column_batch
    }

    pub fn len(&self) -> usize {
        self.column_batch.num_rows()
    }

    pub fn array_for_field(&self, field: &str) -> Option<ArrayRef> {
        let array = if let Some(array) = self.column_batch.column_by_name(field) {
            array
        } else if let Some(array) = self.scalar_batch.column_by_name(field) {
            array
        } else {
            return None;
        };

        Some(array.clone())
    }

    pub fn f32_scalar_or_array_for_field(
        &self,
        field: &str,
    ) -> Result<Option<ScalarOrArray<f32>>, AvengerChartError> {
        // Loop in each batch for the named field
        let (array, is_scalar) = if let Some(array) = self.column_batch.column_by_name(field) {
            (array, false)
        } else if let Some(array) = self.scalar_batch.column_by_name(field) {
            (array, true)
        } else {
            // Not found in either batch
            return Ok(None);
        };

        // Cast to f32 then downcast to f32 array
        let array = cast(array, &DataType::Float32)?;
        let array = array.as_any().downcast_ref::<Float32Array>().ok_or(
            AvengerChartError::InternalError(format!(
                "Failed to downcast {field} to Float32Array: {array:?}",
            )),
        )?;

        if is_scalar {
            Ok(Some(ScalarOrArray::new_scalar(array.value(0))))
        } else {
            Ok(Some(ScalarOrArray::new_array(array.values().to_vec())))
        }
    }
}

async fn eval_encoding_exprs(
    from: &Option<DataFrame>,
    encodings: &IndexMap<String, Expr>,
    context: &CompilationContext,
) -> Result<EncodingBatches, AvengerChartError> {
    // Get the dataset to use for this mark
    let from_df = if let Some(from) = from {
        from.clone()
    } else {
        // Single row DataFrame with no columns
        context.ctx.read_empty()?
    };

    // Exprs that don't reference any columns, that we will evaluate against the empty DataFrame
    let mut scalar_exprs: Vec<Expr> = Vec::new();

    // Exprs that reference columns, that we will evaluate against the from_df DataFrame
    let mut column_exprs: Vec<Expr> = Vec::new();

    for (name, encoding) in encodings.iter() {
        if encoding.column_refs().is_empty() {
            // scalar_exprs.push(encoding.clone().apply_params(&context.params)?.alias(name));
            scalar_exprs.push(encoding.clone().alias(name));
        } else {
            // column_exprs.push(encoding.clone().apply_params(&context.params)?.alias(name));
            column_exprs.push(encoding.clone().alias(name));
        }
    }

    // Get record batch for result of column exprs
    let column_exprs_df = from_df
        .select(column_exprs)?
        .with_param_values(context.param_values.clone())?;
    let column_exprs_schema = column_exprs_df.schema().inner().clone();
    let column_exprs_batch =
        concat_batches(&column_exprs_schema, &column_exprs_df.collect().await?)?;

    // Get/build DataFrame to evaluate scalar expressions against
    let scalar_exprs_df = context
        .ctx
        .read_empty()?
        .select(scalar_exprs)?
        .with_param_values(context.param_values.clone())?;

    let scalar_schema = scalar_exprs_df.schema().inner().clone();
    let scalar_exprs_batch = concat_batches(&scalar_schema, &scalar_exprs_df.collect().await?)?;

    Ok(EncodingBatches::new(scalar_exprs_batch, column_exprs_batch))
}
