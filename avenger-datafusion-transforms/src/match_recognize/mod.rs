//! `MATCH_RECOGNIZE`, GoogleSQL's row pattern recognition: the pattern, its compiler, the
//! matcher of a partition's rows, and the node of a logical plan that runs it, [`plan`]. The
//! matcher sees, for each row and each pattern variable, whether the row meets the variable's
//! condition, which a front end computes before matching, since GoogleSQL's conditions read only
//! the current row and its neighbors.

pub mod matcher;
mod node;
pub mod pattern;

pub use matcher::{Match, Skip, Tail, matches, tail_matches};
pub use node::{MatchRecognizeCall, call_of, plan};
pub use pattern::{Anchor, MAX_INSTRUCTIONS, Pattern, Program};
