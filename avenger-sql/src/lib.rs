//! GoogleSQL pipe queries that build avenger charts, planned onto DataFusion.
//!
//! A mark of avenger-chart is an aggregate function, so a query builds a chart of its rows, or one
//! chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension
//! name `avenger.chart`:
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE count(*) AS n GROUP BY species
//! |> AGGREGATE bar(x => species, y => n) AS chart
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
//! [`register`] adds what the plans of the transforms need to run to a session: the query planner
//! of the table function `impute`, and the optimizer rule that groups by the fields of a struct.
//!
//! The parts of planning GoogleSQL with [`avenger_sql_syntax`]: [`library`] maps GoogleSQL's
//! functions and avenger's onto DataFusion, [`types`] maps GoogleSQL's types to Arrow types, and
//! [`catalog`] looks up the tables that a script names.

pub mod analysis;
pub mod catalog;
mod functions;
pub mod library;
mod pipeline;
pub mod types;

use datafusion::{common::Result, prelude::SessionContext};

pub use analysis::analyze;
pub use pipeline::{sql, sql_with_modules, ScriptError};

/// Registers in `ctx` what the plans of the transforms of avenger-datafusion-transforms need to
/// run: the query planner of the table function `impute`, so `ctx` must have DataFusion's default
/// query planner, and the optimizer rule that groups by the fields of a struct, such as a bin.
/// Registering two times adds the rule once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    use avenger_datafusion_transforms::{table_function::TransformsQueryPlanner, StructGroupSplit};
    use datafusion::{common::plan_err, execution::SessionStateBuilder, optimizer::OptimizerRule};
    use std::sync::Arc;

    let state = ctx.state_ref();
    let mut state = state.write();
    let planner = format!("{:?}", state.query_planner());
    // DataFusion's default planner is not public, so only its name identifies it.
    if planner != "DefaultQueryPlanner" && planner != "TransformsQueryPlanner" {
        return plan_err!(
            "avenger-sql plans its extension nodes with a query planner of its own, but the session has the query planner {planner}"
        );
    }
    let mut builder = SessionStateBuilder::new_from_existing(state.clone())
        .with_query_planner(Arc::new(TransformsQueryPlanner));
    if !(state.optimizers().iter()).any(|rule| rule.name() == StructGroupSplit.name()) {
        builder = builder.with_optimizer_rule(Arc::new(StructGroupSplit));
    }
    *state = builder.build();
    Ok(())
}
