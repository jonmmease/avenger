#![doc = include_str!("../README.md")]

mod bind;
mod functions;
mod lower;
mod reads;

pub mod preaggregate;

pub use bind::bind_selections;
pub use reads::{selection_reads, Owners, SelectionRead};

use std::sync::Arc;

use datafusion::{
    common::Result,
    logical_expr::{registry::FunctionRegistry, ScalarUDF},
};

/// Registers `selected`, which [`bind_selections`] lowers to a filter, and the function rewrite
/// that fails a plan that runs with a `selected` call, with an error that names the selection that
/// was not registered.
pub fn register(registry: &mut dyn FunctionRegistry) -> Result<()> {
    registry.register_udf(selected())?;
    registry.register_function_rewrite(Arc::new(bind::Unbound))
}

/// `selected`, as [`register`] registers it, for a planner that builds its calls itself.
pub fn selected() -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(functions::SelectedFunction::new()))
}
