//! GoogleSQL pipe queries that build avenger charts, planned onto DataFusion.
//!
//! A mark of avenger-chart is an aggregate function, so a query builds a chart of its rows, or one
//! chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension
//! name `avenger.chart`:
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE count(*) AS n GROUP BY species
//! |> AGGREGATE mark_bar(x => species, y => n) AS chart
//! ```
//!
//! [`sql`] plans a script of GoogleSQL's pipe syntax, the subset that [`avenger_sql_syntax`]
//! parses, over the tables of a session: it parses the script, loads the tables that it names,
//! and analyzes it into DataFusion plans, without DataFusion's SQL planner. The functions of a
//! script are those of the [`library`]: GoogleSQL's functions, the marks and constructors of
//! avenger-chart, and the transforms of avenger-datafusion-transforms, `bin`, `stack`,
//! `timeunit`, `lump`, and `impute`. The errors of a script are a [`ScriptError`] of its
//! diagnostics, each at its range.
//!
//! [`register`] adds what the plans of the transforms need to run to a session: [`QueryPlanner`],
//! which plans `impute` and the inputs that a plan shares, and the optimizer rules of shared inputs
//! and of groups by a struct.
//!
//! The parts of planning GoogleSQL with [`avenger_sql_syntax`]: [`library`] maps GoogleSQL's
//! functions and avenger's onto DataFusion, [`types`] maps GoogleSQL's types to Arrow types, and
//! [`catalog`] looks up the tables that a script names.
//!
//! A project's [`data`] scripts register the tables that its scripts read, with GoogleSQL's
//! `CREATE EXTERNAL TABLE`, `LOAD DATA`, `CREATE VIEW`, `CREATE TABLE ... AS`, and `CREATE
//! SCHEMA`. [`analyze_data`] analyzes a data script: what each statement defines, its schema,
//! and the reads that leave the script.

pub mod analysis;
pub mod catalog;
pub mod data;
mod functions;
pub mod library;
mod pipeline;
pub mod streaming;
pub mod types;

use datafusion::{common::Result, prelude::SessionContext};

pub use analysis::{analyze, analyze_data};
pub use pipeline::{lift, sql, sql_with_modules, ScriptError};

/// Registers in `ctx` what the plans of avenger-sql need to run: [`QueryPlanner`], which plans the
/// extension nodes of avenger, so `ctx` must have DataFusion's default query planner, the rule that
/// computes cheap shared inputs again in place of sharing them, the rule that groups by the
/// fields of a struct, such as a bin, and the rule that plans an aggregate grouped by a lump
/// without joining the lump to its rows. Registering two times adds the rules once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    use avenger_datafusion_shared::InlineShared;
    use avenger_datafusion_transforms::{lump::LumpAggregateFusion, StructGroupSplit};
    use datafusion::{
        common::plan_err,
        execution::SessionStateBuilder,
        optimizer::{AnalyzerRule, OptimizerRule},
    };
    use std::sync::Arc;

    let state = ctx.state_ref();
    let mut state = state.write();
    let planner = format!("{:?}", state.query_planner());
    // DataFusion's default planner is not public, so only its name identifies it.
    if planner != "DefaultQueryPlanner" && planner != "QueryPlanner" {
        return plan_err!(
            "avenger-sql plans its extension nodes with a query planner of its own, but the session has the query planner {planner}"
        );
    }
    let mut rules = state.optimizers().to_vec();
    if !rules.iter().any(|rule| rule.name() == InlineShared.name()) {
        // Before the rules that push filters and projections, which then reach an inlined input.
        rules.insert(0, Arc::new(InlineShared));
    }
    if !rules
        .iter()
        .any(|rule| rule.name() == StructGroupSplit.name())
    {
        // Early in each pass, so that the later rules merge and prune the projection that it adds
        // above an aggregate.
        let Some(at) = (rules.iter()).position(|rule| rule.name() == "simplify_expressions") else {
            return plan_err!("the session has no optimizer rule simplify_expressions");
        };
        rules.insert(at + 1, Arc::new(StructGroupSplit));
    }
    let mut analyzers = state.analyzer().rules.clone();
    if !(analyzers.iter()).any(|rule| rule.name() == LumpAggregateFusion.name()) {
        // After DataFusion's rules, which inline views and coerce the types of the plan.
        analyzers.push(Arc::new(LumpAggregateFusion));
    }
    *state = SessionStateBuilder::new_from_existing(state.clone())
        .with_query_planner(Arc::new(QueryPlanner))
        .with_analyzer_rules(analyzers)
        .with_optimizer_rules(rules)
        .build();
    Ok(())
}

/// The query planner of avenger's extension nodes: DataFusion's default planner, with the
/// extension planners of `impute` and of shared inputs. A session has one query planner, so it
/// plans every extension node of avenger.
#[derive(Debug)]
pub struct QueryPlanner;

#[async_trait::async_trait]
impl datafusion::execution::context::QueryPlanner for QueryPlanner {
    async fn create_physical_plan(
        &self,
        plan: &datafusion::logical_expr::LogicalPlan,
        session: &dyn datafusion::catalog::Session,
    ) -> Result<std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        use avenger_datafusion_shared::SharedExtensionPlanner;
        use avenger_datafusion_transforms::table_function::TableFunctionExtensionPlanner;
        use datafusion::physical_planner::{DefaultPhysicalPlanner, PhysicalPlanner};
        use std::sync::Arc;

        DefaultPhysicalPlanner::with_extension_planners(vec![
            Arc::new(TableFunctionExtensionPlanner),
            Arc::new(SharedExtensionPlanner::default()),
        ])
        .create_physical_plan(plan, session)
        .await
    }
}
