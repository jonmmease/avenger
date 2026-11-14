//! Data transforms for DataFusion: the Vega-style `bin`, `stack`, `timeunit`, and `impute`, and
//! `lump`, which keeps the top categories and lumps the rest into an Other group. [`register`]
//! adds the window functions and the optimizer rules to a session.
//!
//! `bin`, `stack` and `timeunit` are window functions, whose options are pairs of a name and a
//! constant after the first argument, in any order. A function of a whole window partition
//! (`bin` and `timeunit`) takes no ORDER BY or frame in its OVER clause. `impute` is a table
//! function over an input relation, and `lump` a plan over its input, which a front end builds
//! with [`impute::plan`] and [`lump::plan`] from its planned arguments, or for a lump in a
//! projection, with [`lump::window`] and [`lump::plan_windows`].
//!
//! Each module describes its transform: [`bin`], [`stack`], [`timeunit`], [`impute`], and
//! [`lump`]. [`table_function`] holds the machinery of table functions over an input relation,
//! such as `impute`. [`StructGroupSplit`] groups an aggregate by the fields of a struct key, such
//! as a bin, then makes the struct again above the aggregate.

pub mod bin;
pub mod impute;
pub mod lump;
pub mod stack;
pub mod table_function;
pub mod timeunit;
mod utils;

pub use utils::StructGroupSplit;

/// An option of a window transform, for the bindings that take options as named arguments: its
/// name, its GoogleSQL type, the strings that it takes when it takes one of a set, and its doc,
/// which gives its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionSpec {
    pub name: &'static str,
    /// The type of the option's value, such as `FLOAT64` or `ARRAY<FLOAT64>`.
    pub ty: &'static str,
    /// The strings that the option takes, or none for an option of any value of its type.
    pub values: &'static [&'static str],
    pub doc: &'static str,
}

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

/// Adds the transforms to a session: the window functions `bin`, `stack`, and `timeunit`, the
/// function `bin_params`, the optimizer rules that plan the window functions as aggregates and
/// joins, and the query planner of the table functions, such as `impute`.
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
    // An aggregate grouped by a lump reads its input once.
    let mut analyzers = state.analyzer().rules.clone();
    if !analyzers
        .iter()
        .any(|r| r.name() == "lump_aggregate_fusion")
    {
        analyzers.push(Arc::new(lump::LumpAggregateFusion));
    }
    *state = SessionStateBuilder::new_from_existing(state.clone())
        .with_analyzer_rules(analyzers)
        .with_optimizer_rules(rules)
        .with_query_planner(Arc::new(table_function::TransformsQueryPlanner))
        .build();
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(BinWindow::default())))?;
    state.register_udf(Arc::new(ScalarUDF::new_from_impl(BinParamsUdf::default())))?;
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(StackWindow::default())))?;
    state.register_udwf(Arc::new(WindowUDF::new_from_impl(
        timeunit::TimeUnitWindow::default(),
    )))?;
    Ok(())
}
