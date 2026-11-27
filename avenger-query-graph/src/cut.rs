//! Cuts: the scans through which a node reads the nodes below it, and the providers that serve a
//! kept result to those scans.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::{RecordBatch, RecordBatchOptions},
        compute::{cast, SortOptions},
        datatypes::{Field, Schema, SchemaRef},
    },
    catalog::{Session, TableProvider},
    common::{internal_err, plan_err, project_schema, Column, DFSchemaRef, Result},
    datasource::{memory::MemorySourceConfig, source::DataSourceExec, TableType},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, Projection, TableScan, TableSource},
    physical_expr::{expressions::Column as PhysicalColumn, LexOrdering, PhysicalSortExpr},
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
    let source = Arc::new(CutSource {
        schema: cut_schema(plan),
        input,
    });
    scan_under(plan.schema(), &cut_name(input), source)
}

/// A scan of the table `name` of `source`, whose columns are named by position, under a
/// projection that gives each column the qualifier and the name that `schema` gives it.
pub(crate) fn scan_under(
    schema: &DFSchemaRef,
    name: &str,
    source: Arc<dyn TableSource>,
) -> Result<LogicalPlan> {
    let scan = LogicalPlanBuilder::scan(name, source, None)?.build()?;
    let exprs = (schema.iter().enumerate())
        .map(|(index, (qualifier, field))| {
            Expr::Column(Column::new(Some(name), format!("c{index}")))
                .alias_qualified(qualifier.cloned(), field.name())
        })
        .collect();
    Ok(LogicalPlan::Projection(Projection::try_new_with_schema(
        exprs,
        Arc::new(scan),
        schema.clone(),
    )?))
}

/// The columns that each cut's scan of an optimized plan reads, by the position of the cut's
/// input: every column when the scan has no projection, and `None` for a cut that the optimizer
/// removed, such as below a filter that is false.
pub(crate) fn cut_requests(
    plan: &LogicalPlan,
    inputs: usize,
) -> Result<Vec<Option<BTreeSet<usize>>>> {
    let mut requests: Vec<Option<BTreeSet<usize>>> = vec![None; inputs];
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            if let Some(cut) = scan.source.as_ref().downcast_ref::<CutSource>() {
                let columns: Vec<usize> = match &scan.projection {
                    Some(projection) => projection.clone(),
                    None => (0..cut.schema.fields().len()).collect(),
                };
                requests[cut.input].get_or_insert_default().extend(columns);
            }
        }
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    })?;
    Ok(requests)
}

/// `plan` with each cut's scan reading `results[input]`, which is present for each cut that the
/// plan scans.
pub(crate) fn with_results(
    plan: LogicalPlan,
    results: &[Option<Arc<Kept>>],
) -> Result<LogicalPlan> {
    use datafusion::common::tree_node::Transformed;
    Ok(plan
        .transform_up_with_subqueries(|node| {
            let LogicalPlan::TableScan(scan) = &node else {
                return Ok(Transformed::no(node));
            };
            let Some(cut) = scan.source.as_ref().downcast_ref::<CutSource>() else {
                return Ok(Transformed::no(node));
            };
            let Some(kept) = results[cut.input].clone() else {
                return internal_err!("a cut's scan has no result");
            };
            let table = KeptTable {
                schema: cut.schema.clone(),
                kept,
                names: None,
            };
            let source = datafusion::datasource::provider_as_source(Arc::new(table));
            Ok(Transformed::yes(LogicalPlan::TableScan(TableScan {
                source,
                ..scan.clone()
            })))
        })?
        .data)
}

/// A node's kept result: some of the columns of its output, by position, their batches in the
/// order that the node computed them, and whether that order is the order of the rows.
#[derive(Debug)]
pub(crate) struct Kept {
    /// The positions of the result's columns in the node's output, in ascending order.
    pub columns: Vec<usize>,
    /// The batches, with a column for each of `columns`.
    pub batches: Vec<RecordBatch>,
    /// Whether one partition computed the batches, so their order is the order of the rows,
    /// which a reader such as a limit can depend on. The batches of several partitions have no
    /// order.
    pub ordered: bool,
    /// The sort order of the rows that the computing plan declared, by columns of the node's
    /// output, when `ordered`. A sort by an expression that is not an output column declares
    /// none, although its rows are in order.
    pub ordering: Vec<(usize, SortOptions)>,
    pub rows: usize,
    pub bytes: usize,
}

impl Kept {
    pub fn new(
        columns: Vec<usize>,
        batches: Vec<RecordBatch>,
        ordered: bool,
        ordering: Vec<(usize, SortOptions)>,
    ) -> Self {
        let rows = batches.iter().map(|batch| batch.num_rows()).sum();
        let bytes = (batches.iter())
            .map(|batch| batch.get_array_memory_size())
            .sum();
        Self {
            columns,
            batches,
            ordered,
            ordering,
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
    /// The names of the columns of the node's output, for a table of a baked graph, whose scan of a
    /// column that the table does not hold names the column.
    names: Option<Vec<String>>,
}

/// `kept` as a table of the schema `schema`, the schema of a cut's scan of its node, as a baked
/// graph reads each of its tables. `names` are the names of the columns of the node's output.
pub(crate) fn kept_table(
    schema: SchemaRef,
    kept: Arc<Kept>,
    names: Vec<String>,
) -> Arc<dyn TableProvider> {
    Arc::new(KeptTable {
        schema,
        kept,
        names: Some(names),
    })
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
        let kept = &self.kept;
        let schema = project_schema(&self.schema, Some(&kept.columns))?;
        let batches = kept.project(&kept.columns, schema.clone())?;
        let position = |column: usize| kept.columns.iter().position(|kept| *kept == column);
        // A scan of every column of a result that holds some of them names the columns, so it
        // fails for the first that the result lacks, as a scan with a projection does.
        let every: Vec<usize> = (0..self.schema.fields().len()).collect();
        let projection = match projection {
            None if kept.columns.len() < every.len() => Some(&every),
            projection => projection,
        };
        let projection = match projection {
            Some(projection) => Some(
                (projection.iter())
                    .map(|column| match (position(*column), &self.names) {
                        (Some(position), _) => Ok(position),
                        (None, Some(names)) => plan_err!(
                            "a selection tests the column `{}`, which the graph's tables do not hold",
                            names.get(*column).map_or("?", String::as_str)
                        ),
                        (None, None) => {
                            internal_err!("a kept result lacks the column at {column}")
                        }
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            None => None,
        };
        // Ordered batches replay in their order in one partition, with the ordering that the plan
        // above can use, so a limit above the cut keeps the rows that it keeps without the cut.
        // Other batches spread over the session's partitions, so the plan above reads them in
        // parallel.
        if !kept.ordered {
            let count = (state.config_options().execution.target_partitions)
                .min(batches.len())
                .max(1);
            let mut partitions = vec![Vec::new(); count];
            for (index, batch) in batches.into_iter().enumerate() {
                partitions[index % count].push(batch);
            }
            let source = MemorySourceConfig::try_new(&partitions, schema, projection)?;
            return Ok(DataSourceExec::from_data_source(source.with_limit(limit)));
        }
        let ordering: Vec<PhysicalSortExpr> = (kept.ordering.iter())
            .map_while(|(column, options)| {
                let position = position(*column)?;
                let field = schema.field(position);
                Some(PhysicalSortExpr::new(
                    Arc::new(PhysicalColumn::new(field.name(), position)),
                    *options,
                ))
            })
            .collect();
        let mut source = MemorySourceConfig::try_new(&[batches], schema, projection)?;
        if let Some(ordering) = LexOrdering::new(ordering) {
            source = source.try_with_sort_information(vec![ordering])?;
        }
        Ok(DataSourceExec::from_data_source(source.with_limit(limit)))
    }
}

#[cfg(test)]
mod tests;
