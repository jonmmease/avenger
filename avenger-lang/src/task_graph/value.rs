use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};
use datafusion::{logical_expr::LogicalPlan, prelude::Expr};
use datafusion_common::ScalarValue;
use datafusion::arrow::array::RecordBatch;
use crate::error::AvengerLangError;

use super::memory::{inner_size_of_scalar, inner_size_of_table};

/// The value of a task
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaskValue {
    Val(ScalarValue),
    Expr(Expr),
    Dataset(LogicalPlan),
    Table(ArrowTable),
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

    /// Get the approximate size of the task value in bytes
    pub fn size_of(&self) -> usize {
        let inner_size = match self {
            TaskValue::Val(scalar) => inner_size_of_scalar(scalar),
            TaskValue::Table(table) => inner_size_of_table(table),
            // TODO: Add support for lazy types
            _ => 0,
        };

        std::mem::size_of::<Self>() + inner_size
    }
}

#[derive(Debug, Clone)]
pub struct ArrowTable {
    pub schema: Arc<Schema>,
    pub batches: Vec<RecordBatch>,
}

// Implement custom equality for ArrowTable (using schema equality only as a simplified approach)
impl PartialEq for ArrowTable {
    fn eq(&self, other: &Self) -> bool {
        // Just compare schemas for equality, not the actual data
        // This is a simplified approach for testing purposes
        self.schema == other.schema
    }
}

// Implement Eq marker trait for ArrowTable
impl Eq for ArrowTable {}

// Implement Hash for ArrowTable using schema hash only
impl std::hash::Hash for ArrowTable {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash only the schema for simplicity
        self.schema.hash(state);
    }
}

impl ArrowTable {
    pub fn try_new(schema: SchemaRef, partitions: Vec<RecordBatch>) -> Result<Self, AvengerLangError> {
        // Make all columns nullable
        let schema_fields: Vec<_> = schema
            .fields
            .iter()
            .map(|f| f.as_ref().clone().with_nullable(true))
            .collect();
        let schema = Arc::new(Schema::new(schema_fields));
        if partitions.iter().all(|batch| {
            let batch_schema_fields: Vec<_> = batch
                .schema()
                .fields
                .iter()
                .map(|f| f.as_ref().clone().with_nullable(true))
                .collect();
            let batch_schema = Arc::new(Schema::new(batch_schema_fields));
            schema.fields.contains(&batch_schema.fields)
        }) {
            Ok(Self {
                schema,
                batches: partitions,
            })
        } else {
            Err(AvengerLangError::InternalError(
                "Mismatch between schema and batches".to_string(),
            ))
        }
    }
}