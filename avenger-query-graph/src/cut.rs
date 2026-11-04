//! Cuts: the scans through which a node reads the nodes below it, and the providers that serve a
//! kept result to those scans.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{RecordBatch, RecordBatchOptions},
        compute::cast,
        datatypes::{Field, Schema, SchemaRef},
    },
    catalog::{Session, TableProvider},
    common::{internal_err, project_schema, Column, Result},
    datasource::{memory::MemorySourceConfig, source::DataSourceExec, TableType},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, Projection, TableScan, TableSource},
    physical_plan::ExecutionPlan,
};

/// The source of a cut's scan before a run gives it a result: the schema of the node below the
/// cut, with its columns named by position.
#[derive(Debug)]
pub(crate) struct CutSource {
    pub schema: SchemaRef,
    /// The position of the node below the cut among the inputs of the node above it.
    pub input: usize,
}

impl TableSource for CutSource {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}

/// The schema of a cut's scan of `plan`'s output: a column for each of the output's columns,
/// named `c0`, `c1`, and so on, with the column's type, nullability, and metadata.
pub(crate) fn cut_schema(plan: &LogicalPlan) -> SchemaRef {
    let fields: Vec<Field> = (plan.schema().fields().iter().enumerate())
        .map(|(index, field)| {
            Field::new(
                format!("c{index}"),
                field.data_type().clone(),
                field.is_nullable(),
            )
            .with_metadata(field.metadata().clone())
        })
        .collect();
    Arc::new(Schema::new(fields))
}

/// The name of the scan of the input at `input`.
pub(crate) fn cut_name(input: usize) -> String {
    format!("input_{input}")
}

/// A cut of `plan`, the input at `input` of a node: a scan of the input's result under a
/// projection that gives each column the qualifier, the name, and the metadata that `plan` gave
/// it, so the plan above resolves its columns as before.
pub(crate) fn cut_scan(plan: &LogicalPlan, input: usize) -> Result<LogicalPlan> {
    let name = cut_name(input);
    let source = Arc::new(CutSource {
        schema: cut_schema(plan),
        input,
    });
    let scan = LogicalPlanBuilder::scan(name.clone(), source, None)?.build()?;
    let exprs = (plan.schema().iter().enumerate())
        .map(|(index, (qualifier, field))| {
            Expr::Column(Column::new(Some(name.clone()), format!("c{index}")))
                .alias_qualified(qualifier.cloned(), field.name())
        })
        .collect();
    Ok(LogicalPlan::Projection(Projection::try_new_with_schema(
        exprs,
        Arc::new(scan),
        plan.schema().clone(),
    )?))
}

/// The columns that each cut's scan of an optimized plan reads, by the position of the cut's
/// input: every column when the scan has no projection.
pub(crate) fn cut_requests(plan: &LogicalPlan, inputs: usize) -> Result<Vec<BTreeSet<usize>>> {
    let mut requests = vec![BTreeSet::new(); inputs];
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            if let Some(cut) = scan.source.as_ref().downcast_ref::<CutSource>() {
                let columns: Vec<usize> = match &scan.projection {
                    Some(projection) => projection.clone(),
                    None => (0..cut.schema.fields().len()).collect(),
                };
                requests[cut.input].extend(columns);
            }
        }
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    })?;
    Ok(requests)
}

/// `plan` with each cut's scan reading `results[input]`.
pub(crate) fn with_results(plan: LogicalPlan, results: &[Arc<Kept>]) -> Result<LogicalPlan> {
    use datafusion::common::tree_node::Transformed;
    Ok(plan
        .transform_up_with_subqueries(|node| {
            let LogicalPlan::TableScan(scan) = &node else {
                return Ok(Transformed::no(node));
            };
            let Some(cut) = scan.source.as_ref().downcast_ref::<CutSource>() else {
                return Ok(Transformed::no(node));
            };
            let table = KeptTable {
                schema: cut.schema.clone(),
                kept: results[cut.input].clone(),
            };
            let source = datafusion::datasource::provider_as_source(Arc::new(table));
            Ok(Transformed::yes(LogicalPlan::TableScan(TableScan {
                source,
                ..scan.clone()
            })))
        })?
        .data)
}

/// A node's kept result: some of the columns of its output, by position, and their batches.
#[derive(Debug)]
pub(crate) struct Kept {
    /// The positions of the result's columns in the node's output, in ascending order.
    pub columns: Vec<usize>,
    /// The batches, with a column for each of `columns`.
    pub batches: Vec<RecordBatch>,
    pub rows: usize,
    pub bytes: usize,
}

impl Kept {
    pub fn new(columns: Vec<usize>, batches: Vec<RecordBatch>) -> Self {
        let rows = batches.iter().map(|batch| batch.num_rows()).sum();
        let bytes = (batches.iter())
            .map(|batch| batch.get_array_memory_size())
            .sum();
        Self {
            columns,
            batches,
            rows,
            bytes,
        }
    }

    /// Whether the result holds every column of `columns`.
    pub fn holds(&self, columns: &BTreeSet<usize>) -> bool {
        columns.iter().all(|column| self.columns.contains(column))
    }

    /// The batches with the columns `columns` of the node's output, of the schema `schema`.
    pub fn project(&self, columns: &[usize], schema: SchemaRef) -> Result<Vec<RecordBatch>> {
        let positions = (columns.iter())
            .map(
                |column| match self.columns.iter().position(|kept| kept == column) {
                    Some(position) => Ok(position),
                    None => internal_err!("a kept result lacks the column at {column}"),
                },
            )
            .collect::<Result<Vec<_>>>()?;
        (self.batches.iter())
            .map(|batch| {
                // A physical plan can return another type of a column than its logical plan, such
                // as `Utf8View` for `Utf8`, so each column takes the logical type.
                let arrays = (positions.iter().zip(schema.fields()))
                    .map(|(position, field)| {
                        let array = batch.column(*position);
                        match array.data_type() == field.data_type() {
                            true => Ok(array.clone()),
                            false => Ok(cast(array, field.data_type())?),
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                let options = RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
                Ok(RecordBatch::try_new_with_options(
                    schema.clone(),
                    arrays,
                    &options,
                )?)
            })
            .collect()
    }
}

/// A kept result as a table, with the schema of the cut's scan.
#[derive(Debug)]
struct KeptTable {
    schema: SchemaRef,
    kept: Arc<Kept>,
}

#[async_trait]
impl TableProvider for KeptTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let columns: Vec<usize> = match projection {
            Some(projection) => projection.clone(),
            None => (0..self.schema.fields().len()).collect(),
        };
        let schema = project_schema(&self.schema, Some(&columns))?;
        let batches = self.kept.project(&columns, schema.clone())?;
        let count = state.config_options().execution.target_partitions.max(1);
        let mut partitions = vec![Vec::new(); count.min(batches.len()).max(1)];
        let length = partitions.len();
        for (index, batch) in batches.into_iter().enumerate() {
            partitions[index % length].push(batch);
        }
        let source = MemorySourceConfig::try_new(&partitions, schema, None)?.with_limit(limit);
        Ok(DataSourceExec::from_data_source(source))
    }
}
