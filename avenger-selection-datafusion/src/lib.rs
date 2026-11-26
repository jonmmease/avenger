#![doc = include_str!("../README.md")]

mod bind;
mod call;
mod functions;
mod operator;
mod reads;

pub mod preaggregate;

pub use operator::{
    bind_selections, with_selection_nodes, with_selection_states, QueryPlanner, SelectionExec,
    SelectionExtensionPlanner, SelectionFields, SelectionNode, SelectionStates,
};
pub use reads::{selection_reads, Owners, SelectionRead};

use std::sync::Arc;

use datafusion::{
    common::Result,
    logical_expr::{registry::FunctionRegistry, ScalarUDF},
};

/// Registers `selected`, which [`with_selection_nodes`] replaces with a selection's operator, and a
/// function rewrite. The rewrite makes planning fail, with an error that names the selection, when
/// a plan still contains a `selected` call on a placeholder.
///
/// The session's query planner must also include [`SelectionExtensionPlanner`], which plans the
/// selections' operators. [`QueryPlanner`] is such a planner.
pub fn register(registry: &mut dyn FunctionRegistry) -> Result<()> {
    registry.register_udf(selected())?;
    registry.register_function_rewrite(Arc::new(bind::Unbound))
}

/// `selected`, as [`register`] registers it, for a planner that builds its calls itself.
pub fn selected() -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(functions::SelectedFunction::new()))
}
