//! `MATCH_RECOGNIZE`, GoogleSQL's row pattern recognition: the pattern, its compiler, and the
//! matcher of a partition's rows. The matcher sees, for each row and each pattern variable,
//! whether the row meets the variable's condition, which a front end computes before matching,
//! since GoogleSQL's conditions read only the current row and its neighbors.

pub mod matcher;
pub mod pattern;

pub use matcher::{Match, Skip, matches};
pub use pattern::{Anchor, MAX_INSTRUCTIONS, Pattern, Program};
