//! The query of a chart script, ready to run: its frame, unrun, and its collect through the
//! project's store of results.

use avenger_query_graph::{Inputs, ResultStore, SourceOptions, StoreConfig};
use avenger_sql::analysis::PlannedQuery;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    common::{tree_node::TreeNodeRecursion, Result},
    dataframe::DataFrame,
    datasource::{listing::ListingTable, source_as_provider},
    logical_expr::LogicalPlan,
    prelude::SessionContext,
};

/// The bytes of the results that a project's store keeps before it evicts the least recently
/// used.
const BUDGET: usize = 512 << 20;

/// The bytes of the largest result that a project's store keeps.
const MAX_RESULT: usize = 64 << 20;

/// A store of the results of a project's chart queries.
pub(crate) fn store() -> ResultStore {
    ResultStore::new(StoreConfig {
        budget: BUDGET,
        max_result: MAX_RESULT,
    })
}

/// The last query of a chart script, ready to run, as [`crate::LoadedProject::run_chart`] plans
/// it: its frame, unrun, and a collect of its rows through the project's store.
///
/// [`ChartQuery::collect`] runs the query through an avenger-query-graph graph of the project's
/// store, with the values of the script's variables as its parameters, so a run reuses the
/// results of the subplans that an earlier run of the project computed and that read nothing
/// that changed: the same tables, and the same values of the variables that they read. Before it
/// reuses a result that reads a table of files, a run lists the table's files, so a changed file
/// gives the rows that a run of the frame reads. A Hive layout's table, which a run can't list,
/// changes when the project builds the table again, as at a reload of its files.
///
/// The frame is the query as DataFusion runs it, without the store: a host that reads its
/// schema, explains it, limits it, or streams its rows, such as a table that reads a page at a
/// time, reads the frame, since a run through the store computes and keeps whole results.
#[derive(Clone)]
pub struct ChartQuery {
    query: PlannedQuery,
    frame: DataFrame,
    ctx: SessionContext,
    store: ResultStore,
}

impl std::fmt::Debug for ChartQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartQuery")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl ChartQuery {
    pub(crate) async fn new(
        query: PlannedQuery,
        ctx: &SessionContext,
        store: &ResultStore,
    ) -> Result<Self> {
        let frame = query.dataframe(ctx).await?;
        Ok(Self {
            query,
            frame,
            ctx: ctx.clone(),
            store: store.clone(),
        })
    }

    /// The rows of the query as a frame, unrun.
    pub fn frame(&self) -> &DataFrame {
        &self.frame
    }

    /// The rows of the query as a frame, unrun.
    pub fn into_frame(self) -> DataFrame {
        self.frame
    }

    /// The schema of the query's rows.
    pub fn schema(&self) -> SchemaRef {
        self.frame.schema().inner().clone()
    }

    /// The rows of the query, run through the project's store, as [`ChartQuery`] describes.
    pub async fn collect(&self) -> Result<Vec<RecordBatch>> {
        for table in listing_tables(&self.query.plan)? {
            let options = SourceOptions {
                materialize: false,
                ttl: Some(std::time::Duration::ZERO),
            };
            self.store.set_source_options(&table, options);
        }
        let graph = (self.store).compile(&self.ctx.state(), vec![self.query.plan.clone()])?;
        let selections = (self.query.selections.iter())
            .map(|(name, state)| {
                (
                    avenger_sql::analysis::selection_placeholder(name),
                    state.clone(),
                )
            })
            .collect();
        let inputs = Inputs {
            params: self.query.params.clone(),
            selections,
            ..Inputs::default()
        };
        let run = graph.run(&inputs).await?;
        Ok(run.batches.into_iter().next().unwrap_or_default())
    }
}

/// The names of the tables of `plan` whose providers are `ListingTable`s, as the plan's scans
/// name them.
fn listing_tables(plan: &LogicalPlan) -> Result<Vec<String>> {
    let mut tables = Vec::new();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            // A scan whose source is not a provider reads no table of files.
            let provider = source_as_provider(&scan.source).ok();
            if provider.is_some_and(|provider| provider.downcast_ref::<ListingTable>().is_some()) {
                tables.push(scan.table_name.to_quoted_string());
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(tables)
}
