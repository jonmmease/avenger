//! The functions of a session, as the passes and the mark planner read them: from a session
//! state before planning, or from the context provider of the mark planner during planning.

use datafusion::execution::SessionState;

/// The functions of a session.
pub(crate) trait Registry {
    /// The parameter names of the function `name`, a scalar, an aggregate, or a window function,
    /// when its signature declares them.
    fn parameter_names(&self, name: &str) -> Option<Vec<String>>;
    /// Whether `name` is an aggregate function.
    fn is_aggregate(&self, name: &str) -> bool;
    /// Whether `name` is a window function that is not an aggregate.
    fn is_window(&self, name: &str) -> bool;
}

impl Registry for SessionState {
    fn parameter_names(&self, name: &str) -> Option<Vec<String>> {
        let signature = if let Some(function) = self.scalar_functions().get(name) {
            function.signature()
        } else if let Some(function) = self.aggregate_functions().get(name) {
            function.signature()
        } else {
            self.window_functions().get(name)?.signature()
        };
        signature.parameter_names.clone()
    }

    fn is_aggregate(&self, name: &str) -> bool {
        self.aggregate_functions().contains_key(name)
    }

    fn is_window(&self, name: &str) -> bool {
        self.window_functions().contains_key(name)
    }
}
