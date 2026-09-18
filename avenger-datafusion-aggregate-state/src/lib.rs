#![doc = include_str!("../README.md")]

mod aggregate;
mod families;
mod state;

pub use families::Family;

use aggregate::{Operation, StateAggregate};
use datafusion::{
    common::Result,
    logical_expr::{registry::FunctionRegistry, AggregateUDF, Expr},
};
use std::sync::Arc;

/// The aggregate `{name}_state` of `family`, which aggregates rows to a state.
pub fn state_udaf(family: Family) -> AggregateUDF {
    AggregateUDF::from(StateAggregate::new(family, Operation::State))
}

/// The aggregate `{name}_merge` of `family`, which aggregates states to the final value.
pub fn merge_udaf(family: Family) -> AggregateUDF {
    AggregateUDF::from(StateAggregate::new(family, Operation::Merge))
}

/// The aggregate `{name}_merge_state` of `family`, which aggregates states to one state.
pub fn merge_state_udaf(family: Family) -> AggregateUDF {
    AggregateUDF::from(StateAggregate::new(family, Operation::MergeState))
}

/// The state of `family` of the rows of `value`.
pub fn state(family: Family, value: Expr) -> Expr {
    state_udaf(family).call(vec![value])
}

/// The final value of `family` of the states of `states`.
pub fn merge(family: Family, states: Expr) -> Expr {
    merge_udaf(family).call(vec![states])
}

/// One state of `family` of the states of `states`.
pub fn merge_state(family: Family, states: Expr) -> Expr {
    merge_state_udaf(family).call(vec![states])
}

/// Registers the three aggregates of every family for SQL.
pub fn register_all(registry: &mut dyn FunctionRegistry) -> Result<()> {
    for family in Family::ALL {
        for udaf in [
            state_udaf(family),
            merge_udaf(family),
            merge_state_udaf(family),
        ] {
            registry.register_udaf(Arc::new(udaf))?;
        }
    }
    Ok(())
}
