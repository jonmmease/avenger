//! The closures through which built-in widgets read and change app state.

use std::sync::Arc;

use avenger_eventstream::stream::Rebuild;

/// Reads a widget's value, or whether it is enabled, from app state.
pub(crate) type Get<State, T> = Arc<dyn Fn(&State) -> T + Send + Sync>;

/// Changes app state for a widget's new value, and returns the scenes to rebuild.
pub(crate) type Set<State, T> = Arc<dyn Fn(&mut State, T) -> Rebuild + Send + Sync>;

/// Evaluate `enabled`, defaulting to true when no closure is set.
pub(crate) fn enabled<State>(enabled: &Option<Get<State, bool>>, app: &State) -> bool {
    enabled.as_ref().is_none_or(|enabled| enabled(app))
}
