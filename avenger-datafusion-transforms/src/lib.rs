//! Vega-style data transforms as DataFusion functions: `bin`, `stack`, `timeunit`, and
//! `impute`. [`register`] adds all of them and their optimizer rules to a session.
//!
//! ```sql
//! SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
//! SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
//! SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
//! SELECT * FROM impute(sales, month, [region], units, value => 0)
//! ```
//!
//! The options of the window functions are pairs of a name and a constant after the first
//! argument, in any order, because DataFusion's named arguments can omit only the last
//! parameters. A function of a whole window partition (`bin` and `timeunit`) takes no ORDER
//! BY or frame in its OVER clause.
//!
//! Each module describes its transform: [`bin`], [`stack`], [`timeunit`], and [`impute`].
//! [`table_function`] holds the machinery of table functions over an input relation, such as
//! `impute`.
//! [`StructGroupSplit`] groups an aggregate by the fields of a struct key, such as a bin,
//! then makes the struct again above the aggregate.

pub mod bin;
pub mod impute;
pub mod stack;
pub mod table_function;
pub mod timeunit;
mod utils;

pub use utils::StructGroupSplit;

use std::sync::Arc;

use datafusion::common::{Result, plan_err};
use datafusion::execution::SessionStateBuilder;
use datafusion::logical_expr::registry::FunctionRegistry;
use datafusion::logical_expr::{ScalarUDF, WindowUDF};
use datafusion::optimizer::OptimizerRule;
use datafusion::prelude::SessionContext;

use crate::bin::{BinParamsUdf, BinWindow, BinWindowRewrite};
use crate::stack::StackWindow;

// ---------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------

/// Adds the transforms to a session: the window functions `bin`, `stack`, and `timeunit`,
/// the function `bin_params`, the table function `impute`, and the optimizer rules that plan
/// the window functions as aggregates and joins.
///
/// The rules run right after the first simplification of expressions, which folds the options
/// of a call to constants, so `register` rebuilds the session's state with them in that place;
/// the session keeps its tables, functions, and configuration. The table functions plan their
/// operator with a query planner of their own, so the session must have DataFusion's default
/// query planner. Registering two times adds the rules once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    let state = ctx.state_ref();
    let mut state = state.write();
    let planner = format!("{:?}", state.query_planner());
    // DataFusion's default planner is not public, so only its name identifies it.
    if planner != "DefaultQueryPlanner" && planner != "TransformsQueryPlanner" {
        return plan_err!(
            "the table functions plan their operator with a query planner of their own, but the session has the query planner {planner}; add table_function::TableFunctionExtensionPlanner to it"
        );
    }
    let ours: Vec<Arc<dyn OptimizerRule + Send + Sync>> = vec![
        Arc::new(timeunit::TimeUnitWindowRewrite::default()),
        Arc::new(BinWindowRewrite::default()),
        Arc::new(StructGroupSplit),
    ];
    let mut rules = state.optimizers().to_vec();
    if !rules.iter().any(|r| r.name() == ours[0].name()) {
        let Some(at) = rules
            .iter()
            .position(|r| r.name() == "simplify_expressions")
        else {
            return plan_err!(
                "the transforms need the optimizer rule simplify_expressions, which the session does not have"
            );
        };
        rules.splice(at + 1..at + 1, ours);
    }
    *state = SessionStateBuilder::new_from_existing(state.clone())
        .with_optimizer_rules(rules)
        .with_query_planner(Arc::new(table_function::TransformsQueryPlanner))
        .build();
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(BinWindow::default())))?;
    state.register_udf(Arc::new(ScalarUDF::new_from_impl(BinParamsUdf::default())))?;
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(StackWindow::default())))?;
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(
        timeunit::TimeUnitWindow::default(),
    )))?;
    let catalogs = state.catalog_list().clone();
    state.register_relation_planner(Arc::new(table_function::TableFunctionPlanner::new(
        vec![Arc::new(impute::Impute)],
        catalogs,
    )))?;
    Ok(())
}
