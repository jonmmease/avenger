//! Kept results: the batches that a node computed, with the columns and the order of its output
//! that they hold, the scans that a node's physical plan reads them through, and the substitution
//! of those scans before each computation.

use std::{collections::BTreeSet, sync::Arc};

use arrow::{
    array::{RecordBatch, RecordBatchOptions},
    compute::{cast, SortOptions},
    datatypes::SchemaRef,
};
use datafusion_common::{
    internal_err, project_schema,
    tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    Result,
};
use datafusion_datasource::{memory::MemorySourceConfig, source::DataSourceExec};
use datafusion_physical_expr::{
    expressions::Column as PhysicalColumn, LexOrdering, PhysicalSortExpr,
};
use datafusion_physical_plan::ExecutionPlan;

use crate::cut::{CutInputExec, CutScan};

/// A node's kept result: some of the columns of its output, by position, their batches in the
/// order that the node computed them, and whether that order is the order of the rows.
#[derive(Debug)]
pub struct Kept {
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

    /// The result of executing `plan`, which computes the columns `columns` of its node's output,
    /// in `ctx`. One partition gives the rows in an order that a reader can depend on, and the
    /// plan can declare which order it is.
    pub async fn compute(
        plan: Arc<dyn ExecutionPlan>,
        columns: Vec<usize>,
        ctx: Arc<datafusion_execution::TaskContext>,
    ) -> Result<Self> {
        let properties = plan.properties();
        let ordered = properties.output_partitioning().partition_count() == 1;
        let ordering = match ordered {
            true => (properties.output_ordering().into_iter().flatten())
                .map_while(|sort| {
                    let column = sort.expr.downcast_ref::<PhysicalColumn>()?;
                    Some((*columns.get(column.index())?, sort.options))
                })
                .collect(),
            false => Vec::new(),
        };
        let batches = datafusion_physical_plan::collect(plan, ctx).await?;
        Ok(Self::new(columns, batches, ordered, ordering))
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

/// The number of partitions of the scan of `kept` in a session of `target` partitions.
pub fn partitions(kept: &Kept, target: usize) -> usize {
    match kept.ordered {
        true => 1,
        false => target.min(kept.batches.len()).max(1),
    }
}

/// The scan of `kept` through the cut `cut`. Ordered batches replay in their order in one
/// partition, with the ordering that the plan above can use, so a limit above the cut keeps the
/// rows that it keeps without the cut. Other batches spread over the session's `target`
/// partitions, so the plan above reads them in parallel.
pub fn kept_scan(cut: &CutScan, kept: &Kept, target: usize) -> Result<Arc<dyn ExecutionPlan>> {
    let schema = project_schema(&cut.schema, Some(&kept.columns))?;
    let batches = kept.project(&kept.columns, schema.clone())?;
    let position = |column: usize| kept.columns.iter().position(|kept| *kept == column);
    let projection = match &cut.projection {
        Some(projection) => Some(
            (projection.iter())
                .map(|column| match position(*column) {
                    Some(position) => Ok(position),
                    None => internal_err!("a kept result lacks the column at {column}"),
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        None => None,
    };
    if !kept.ordered {
        let count = partitions(kept, target);
        let mut parts = vec![Vec::new(); count];
        for (index, batch) in batches.into_iter().enumerate() {
            parts[index % count].push(batch);
        }
        let source = MemorySourceConfig::try_new(&parts, schema, projection)?;
        return Ok(DataSourceExec::from_data_source(
            source.with_limit(cut.limit),
        ));
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
    Ok(DataSourceExec::from_data_source(
        source.with_limit(cut.limit),
    ))
}

/// What of a kept result decides the properties of its scan: its columns, whether it is ordered,
/// its ordering, and the number of its partitions in a session of `target` partitions. A physical
/// plan that one computation planned serves another whose inputs have the same shapes.
pub type Shape = (Vec<usize>, bool, Vec<(usize, SortOptions)>, usize);

/// The shape of each input's result, or `None` for an input that the plan does not scan.
pub fn shapes(results: &[Option<Arc<Kept>>], target: usize) -> Vec<Option<Shape>> {
    (results.iter())
        .map(|kept| {
            kept.as_ref().map(|kept| {
                let partitions = partitions(kept, target);
                (
                    kept.columns.clone(),
                    kept.ordered,
                    kept.ordering.clone(),
                    partitions,
                )
            })
        })
        .collect()
}

/// `plan`, a physical plan of a node, with each cut's leaf reading `results[input]`.
pub fn substitute(
    plan: Arc<dyn ExecutionPlan>,
    results: &[Option<Arc<Kept>>],
    target: usize,
) -> Result<Arc<dyn ExecutionPlan>> {
    Ok(plan
        .transform_down(|node| {
            let Some(leaf) = node.downcast_ref::<CutInputExec>() else {
                return Ok(Transformed::no(node));
            };
            let cut = leaf.cut().clone();
            let Some(kept) = results.get(cut.input).cloned().flatten() else {
                return internal_err!("a cut's leaf has no result");
            };
            let scan = kept_scan(&cut, &kept, target)?;
            let leaf: Arc<dyn ExecutionPlan> = Arc::new(CutInputExec::new(cut, scan));
            Ok(Transformed::new(leaf, true, TreeNodeRecursion::Jump))
        })?
        .data)
}
