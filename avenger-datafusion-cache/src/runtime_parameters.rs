//! Execution-time parameter relation used by reusable physical plans.

use std::{fmt, sync::Arc};

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use async_trait::async_trait;
use datafusion::{
    catalog::{Session, TableProvider},
    execution::TaskContext,
    logical_expr::{Expr, TableType},
    physical_expr::EquivalenceProperties,
    physical_plan::{
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
        SendableRecordBatchStream,
        execution_plan::{Boundedness, EmissionType, SchedulingType},
        memory::MemoryStream,
    },
};
use datafusion_common::{Result, Statistics, internal_err};

/// Immutable one-row parameter values installed on one query execution.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeParameterBindings {
    batch: RecordBatch,
}

impl RuntimeParameterBindings {
    pub(crate) fn new(batch: RecordBatch) -> Self {
        Self { batch }
    }

    pub(crate) fn schema(&self) -> SchemaRef {
        self.batch.schema()
    }
}

/// Logical table whose physical scan reads values from the task context.
#[derive(Debug)]
pub(crate) struct RuntimeParameterTable {
    schema: SchemaRef,
}

impl RuntimeParameterTable {
    pub(crate) fn new(schema: SchemaRef) -> Self {
        Self { schema }
    }
}

#[async_trait]
impl TableProvider for RuntimeParameterTable {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !filters.is_empty() {
            return internal_err!("runtime parameter scans do not accept pushed filters");
        }
        if limit.is_some_and(|limit| limit == 0) {
            return internal_err!("runtime parameter scans require their single row");
        }
        Ok(Arc::new(RuntimeParameterExec::new(
            Arc::clone(&self.schema),
            projection.cloned(),
        )?))
    }
}

/// One-partition execution node that reads the current binding batch.
pub(crate) struct RuntimeParameterExec {
    input_schema: SchemaRef,
    projection: Option<Vec<usize>>,
    properties: Arc<PlanProperties>,
}

impl RuntimeParameterExec {
    fn new(input_schema: SchemaRef, projection: Option<Vec<usize>>) -> Result<Self> {
        let output_schema = match projection.as_ref() {
            Some(projection) => Arc::new(input_schema.project(projection)?),
            None => Arc::clone(&input_schema),
        };
        let properties = PlanProperties::new(
            EquivalenceProperties::new(output_schema),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        )
        .with_scheduling_type(SchedulingType::Cooperative);
        Ok(Self {
            input_schema,
            projection,
            properties: Arc::new(properties),
        })
    }
}

impl fmt::Debug for RuntimeParameterExec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeParameterExec")
            .field("schema", &self.input_schema)
            .field("projection", &self.projection)
            .finish()
    }
}

impl DisplayAs for RuntimeParameterExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RuntimeParameterExec")
    }
}

impl ExecutionPlan for RuntimeParameterExec {
    fn name(&self) -> &'static str {
        "RuntimeParameterExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        Vec::new()
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            Ok(self)
        } else {
            internal_err!("RuntimeParameterExec is a leaf and accepts no children")
        }
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return internal_err!("RuntimeParameterExec partition {partition} is out of range");
        }
        let bindings = context
            .session_config()
            .get_extension::<RuntimeParameterBindings>()
            .ok_or_else(|| {
                datafusion_common::DataFusionError::Internal(
                    "runtime parameter bindings are missing from the task context".to_string(),
                )
            })?;
        if bindings.schema().as_ref() != self.input_schema.as_ref() {
            return internal_err!(
                "runtime parameter schema mismatch: expected {:?}, got {:?}",
                self.input_schema,
                bindings.schema()
            );
        }
        let batch = match self.projection.as_ref() {
            Some(projection) => bindings.batch.project(projection)?,
            None => bindings.batch.clone(),
        };
        Ok(Box::pin(MemoryStream::try_new(
            vec![batch],
            self.schema(),
            None,
        )?))
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Arc<Statistics>> {
        if partition.is_some_and(|partition| partition != 0) {
            return internal_err!(
                "RuntimeParameterExec partition {} is out of range",
                partition.unwrap()
            );
        }
        Ok(Arc::new(Statistics::new_unknown(&self.schema())))
    }
}
