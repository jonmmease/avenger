use arrow::csv::ReaderBuilder;
use arrow::csv::reader::Format;
use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::catalog::Session;
use datafusion::catalog::TableFunctionImpl;
use datafusion::common::{ScalarValue, plan_err};
use datafusion::datasource::TableProvider;
use datafusion::datasource::memory::MemorySourceConfig;
use datafusion::error::Result;
use datafusion::logical_expr::{Expr, TableType};
use datafusion::optimizer::simplify_expressions::ExprSimplifier;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::*;
use datafusion::{execution::context::ExecutionProps, logical_expr::simplify::SimplifyContext};
use std::fs::File;
use std::io::Seek;
use std::path::Path;
use std::sync::Arc;

/// Table Function that mimics the [`read_csv`] function in DuckDB.
///
/// Based on DataFusion example
///
/// Usage: `read_csv(filename, [limit])`
///
/// [`read_csv`]: https://duckdb.org/docs/data/csv/overview.html
#[derive(Debug)]
pub struct LocalCsvTable {
    schema: SchemaRef,
    limit: Option<usize>,
    batches: Vec<RecordBatch>,
}

#[async_trait]
impl TableProvider for LocalCsvTable {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let batches = if let Some(max_return_lines) = self.limit {
            // get max return rows from self.batches
            let mut batches = vec![];
            let mut lines = 0;
            for batch in &self.batches {
                let batch_lines = batch.num_rows();
                if lines + batch_lines > max_return_lines {
                    let batch_lines = max_return_lines - lines;
                    batches.push(batch.slice(0, batch_lines));
                    break;
                } else {
                    batches.push(batch.clone());
                    lines += batch_lines;
                }
            }
            batches
        } else {
            self.batches.clone()
        };
        Ok(MemorySourceConfig::try_new_exec(
            &[batches],
            TableProvider::schema(self),
            projection.cloned(),
        )?)
    }
}

#[derive(Debug)]
pub struct LocalCsvTableFunc {}

impl TableFunctionImpl for LocalCsvTableFunc {
    fn call(&self, exprs: &[Expr]) -> Result<Arc<dyn TableProvider>> {
        let Some(Expr::Literal(ScalarValue::Utf8(Some(path)))) = exprs.first().cloned() else {
            return plan_err!("read_csv requires at least one string argument");
        };

        let limit = exprs
            .get(1)
            .map(|expr| {
                // try to simplify the expression, so 1+2 becomes 3, for example
                let execution_props = ExecutionProps::new();
                let info = SimplifyContext::new(&execution_props);
                let expr = ExprSimplifier::new(info).simplify(expr.clone())?;

                if let Expr::Literal(ScalarValue::Int64(Some(limit))) = expr {
                    Ok(limit as usize)
                } else {
                    plan_err!("Limit must be an integer")
                }
            })
            .transpose()?;

        let (schema, batches) = read_csv_batches(path)?;

        let table = LocalCsvTable {
            schema,
            limit,
            batches,
        };
        Ok(Arc::new(table))
    }
}

fn read_csv_batches(csv_path: impl AsRef<Path>) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let mut file = File::open(csv_path)?;
    let (schema, _) = Format::default()
        .with_header(true)
        .infer_schema(&mut file, None)?;
    file.rewind()?;

    let reader = ReaderBuilder::new(Arc::new(schema.clone()))
        .with_header(true)
        .build(file)?;
    let mut batches = vec![];
    for batch in reader {
        batches.push(batch?);
    }
    let schema = Arc::new(schema);
    Ok((schema, batches))
}
