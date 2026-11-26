//! What the tests share: a session that plans selections' operators, and the frame of a plan
//! whose selections are nodes, bound by the session's states.

#![allow(dead_code)]

use std::{collections::BTreeMap, sync::Arc};

use avenger_selection::SelectionState;
use avenger_selection_datafusion::{
    with_selection_nodes, with_selection_states, QueryPlanner, SelectionFields,
};
use datafusion::{
    common::Result,
    dataframe::DataFrame,
    execution::SessionStateBuilder,
    logical_expr::LogicalPlan,
    prelude::{SessionConfig, SessionContext},
};

/// A session with `selected` and the planner of selections' operators.
pub fn session() -> SessionContext {
    session_with(SessionConfig::new())
}

/// [`session`] with `config`.
pub fn session_with(config: SessionConfig) -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_default_features()
        .with_query_planner(Arc::new(QueryPlanner))
        .build();
    let mut ctx = SessionContext::new_with_state(state);
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx
}

/// The states `states` by the names of their selections without `$`.
pub fn states(states: &[(&str, &SelectionState)]) -> BTreeMap<String, SelectionState> {
    (states.iter())
        .map(|(name, state)| (name.trim_start_matches('$').to_string(), (*state).clone()))
        .collect()
}

/// The frame of `plan`, an unoptimized plan of `ctx`, with selection nodes that are bound to
/// `states` as the frame plans.
pub fn frame(
    ctx: &SessionContext,
    plan: LogicalPlan,
    states: &[(&str, &SelectionState)],
) -> Result<DataFrame> {
    let plan = with_selection_nodes(plan, &SelectionFields::new())?;
    let state = with_selection_states(&ctx.state(), self::states(states));
    Ok(DataFrame::new(state, plan))
}
