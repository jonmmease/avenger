#![doc = include_str!("../README.md")]

mod bind;
mod functions;
mod lower;

pub use bind::bind_selections;

use std::sync::Arc;

use datafusion::{
    common::Result,
    logical_expr::{registry::FunctionRegistry, ScalarUDF},
};

/// Registers `selected`, which [`bind_selections`] lowers to a filter, and the function rewrite
/// that fails a plan that runs with a `selected` call, with an error that names the selection that
/// was not registered.
pub fn register(registry: &mut dyn FunctionRegistry) -> Result<()> {
    let selected = ScalarUDF::from(functions::SelectedFunction::new());
    registry.register_udf(Arc::new(selected))?;
    registry.register_function_rewrite(Arc::new(bind::Unbound))
}
