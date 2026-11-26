//! The parts of a node's key that a run's selections give: for each selection that the node
//! reads, the fingerprints of the clauses of the owners whose clauses it reads, and the
//! selection's resolution when there are any.

use avenger_selection::{Resolution, SelectionState};
use avenger_selection_datafusion::Owners;

/// The part of a key for one selection: the fingerprints of the clauses of the owners that the
/// node reads, in the state's order, and the state's resolution when there are any.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SelectionKey {
    pub resolution: Option<u8>,
    pub clauses: Vec<u64>,
}

impl SelectionKey {
    /// The part of a key of a node that reads the clauses of `owners` of the selection whose
    /// state is `state`.
    pub fn new(state: &SelectionState, owners: &Owners) -> Self {
        let clauses: Vec<u64> = (state.clauses.iter())
            .filter(|clause| reads_owner(owners, &clause.owner))
            .map(|clause| clause.fingerprint())
            .collect();
        // A test that keeps no clause selects every row, whatever the resolution.
        let resolution = match clauses.is_empty() {
            true => None,
            false => Some(resolution_code(state.resolution)),
        };
        Self {
            resolution,
            clauses,
        }
    }
}

/// The code of `resolution` in a key.
pub fn resolution_code(resolution: Resolution) -> u8 {
    match resolution {
        Resolution::Global => 0,
        Resolution::Union => 1,
        Resolution::Intersect => 2,
    }
}

/// Whether `owners` reads the clauses of `owner`.
pub fn reads_owner(owners: &Owners, owner: &str) -> bool {
    match owners {
        Owners::AllBut(skipped) => !skipped.contains(owner),
        Owners::Only(read) => read.contains(owner),
    }
}
