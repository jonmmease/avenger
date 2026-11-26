//! Cuts: the scans through which a node reads the nodes below it, and the providers that serve a
//! kept result to those scans.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use avenger_physical_graph::{
    cut::{CutInputExec, CutScan},
    kept::kept_scan,
};

pub(crate) use avenger_physical_graph::kept::Kept;
use datafusion::{
    arrow::datatypes::{Field, Schema, SchemaRef},
    catalog::{Session, TableProvider},
    common::{internal_err, Column, Result},
    datasource::TableType,
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
                input: cut.input,
                kept,
            };
            let source = datafusion::datasource::provider_as_source(Arc::new(table));
            Ok(Transformed::yes(LogicalPlan::TableScan(TableScan {
                source,
                ..scan.clone()
            })))
        })?
        .data)
}

/// A kept result as a table, with the schema of the cut's scan and the position of its input.
#[derive(Debug)]
struct KeptTable {
    schema: SchemaRef,
    input: usize,
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
        let cut = CutScan {
            input: self.input,
            schema: self.schema.clone(),
            projection: projection.cloned(),
            limit,
        };
        let partitions = state.config_options().execution.target_partitions;
        let scan = kept_scan(&cut, &self.kept, partitions)?;
        Ok(Arc::new(CutInputExec::new(cut, scan)))
    }
}

#[cfg(test)]
mod tests;
