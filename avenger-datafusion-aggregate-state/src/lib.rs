#![doc = include_str!("../README.md")]

mod aggregate;
mod codec;
mod families;
mod finalize;
mod state;

pub mod expr_fn;
pub mod functions;

pub use codec::{
    function_versions, AggregateStateExtensionCodec, AGGREGATE_STATE_FUNCTION_VERSION,
};

use aggregate::{Operation, StateAggregate};
use datafusion::{
    common::{plan_err, Result},
    logical_expr::{
        expr::AggregateFunction,
        planner::{ExprPlanner, PlannerResult, RawAggregateExpr},
        registry::FunctionRegistry,
        AggregateUDF, Expr, ScalarUDF,
    },
};
use families::Family;
use finalize::Finalize;
use std::sync::Arc;

/// Register all nine function families and their lowercase SQL aliases.
///
/// Repeated registration of identical functions is permitted. Conflicting names
/// produce an error before registration starts. Other registry failures can leave
/// partial registrations, following the registry's normal mutation semantics.
pub fn register_all(registry: &mut dyn FunctionRegistry) -> Result<()> {
    let aggregates: Vec<_> = Family::ALL
        .into_iter()
        .flat_map(|family| {
            [Operation::State, Operation::Merge, Operation::MergeState]
                .map(|op| Arc::new(AggregateUDF::from(StateAggregate::new(family, op))))
        })
        .collect();
    let scalars: Vec<_> = Family::ALL
        .into_iter()
        .map(|f| Arc::new(ScalarUDF::from(Finalize::new(f))))
        .collect();
    let aggregate_names = registry.udafs();
    let scalar_names = registry.udfs();
    let mut installed = true;
    for function in &aggregates {
        for name in
            std::iter::once(function.name()).chain(function.aliases().iter().map(String::as_str))
        {
            if scalar_names.contains(name)
                || aggregate_names.contains(name) && registry.udaf(name)? != *function
            {
                return plan_err!(
                    "Function name {name} is already registered with a different implementation"
                );
            }
            installed &= aggregate_names.contains(name);
        }
    }
    for function in &scalars {
        for name in
            std::iter::once(function.name()).chain(function.aliases().iter().map(String::as_str))
        {
            if aggregate_names.contains(name)
                || scalar_names.contains(name) && registry.udf(name)? != *function
            {
                return plan_err!(
                    "Function name {name} is already registered with a different implementation"
                );
            }
            installed &= scalar_names.contains(name);
        }
    }
    if installed {
        return Ok(());
    }
    for function in aggregates {
        registry.register_udaf(function)?;
    }
    for function in scalars {
        registry.register_udf(function)?;
    }
    registry.register_expr_planner(Arc::new(StatePlanner))?;
    Ok(())
}

#[derive(Debug)]
struct StatePlanner;
impl ExprPlanner for StatePlanner {
    fn plan_aggregate(&self, mut raw: RawAggregateExpr) -> Result<PlannerResult<RawAggregateExpr>> {
        if let Some(function) = raw.func.inner().downcast_ref::<StateAggregate>() {
            if raw.distinct || !raw.order_by.is_empty() || raw.null_treatment.is_some() {
                return plan_err!("Aggregate-state functions do not support DISTINCT, ORDER BY, or null-treatment modifiers");
            }
            if function.family == Family::Count
                && function.operation == Operation::State
                && raw.args.is_empty()
            {
                raw.args.push(datafusion::logical_expr::lit(1_i64));
                return Ok(PlannerResult::Planned(Expr::AggregateFunction(
                    AggregateFunction::new_udf(raw.func, raw.args, false, raw.filter, vec![], None),
                )));
            }
        }
        Ok(PlannerResult::Original(raw))
    }
}
