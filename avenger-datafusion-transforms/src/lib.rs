//! Vega-style data transforms as DataFusion functions: `bin` and `stack`. [`register`] adds
//! all of them and their optimizer rules to a session.
//!
//! ```sql
//! SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
//! SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
//! ```
//!
//! The options of the window functions are pairs of a name and a constant after the first
//! argument, in any order, because DataFusion's named arguments can omit only the last
//! parameters. `bin`, a function of a whole window partition, takes no ORDER BY or frame in
//! its OVER clause.
//!
//! Each module describes its transform: [`bin`] and [`stack`]. [`StructGroupSplit`] groups
//! an aggregate by the fields of a struct key, such as a bin, then makes the struct again
//! above the aggregate.

pub mod bin;
pub mod stack;
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

/// Adds the transforms to a session: the window functions `bin` and `stack`, the function
/// `bin_params`, and the optimizer rules that plan `bin` as an aggregate and a join.
///
/// The rules run right after the first simplification of expressions, which folds the
/// options of a call to constants, so `register` rebuilds the session's state with them in
/// that place; the session keeps its tables, functions, and configuration. Registering two
/// times adds the rules once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    let state = ctx.state_ref();
    let mut state = state.write();
    let ours: Vec<Arc<dyn OptimizerRule + Send + Sync>> = vec![
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
        .build();
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(BinWindow::default())))?;
    state.register_udf(Arc::new(ScalarUDF::new_from_impl(BinParamsUdf::default())))?;
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(StackWindow::default())))?;
    Ok(())
}
