//! A selection's state: its resolution and its clauses.

use crate::{Clause, Resolution};

/// A selection's resolution and its clauses, in the order of their owners.
#[derive(Clone, Debug, Default)]
pub struct SelectionState {
    pub resolution: Resolution,
    pub clauses: Vec<Clause>,
}

impl SelectionState {
    /// A state of `resolution` with `clauses`.
    pub fn new(resolution: Resolution, clauses: Vec<Clause>) -> Self {
        Self {
            resolution,
            clauses,
        }
    }
}
