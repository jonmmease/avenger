//! Ported from crates/typst-library/src/routines.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: only the realized content type so far. The routines themselves are called
//! directly, since the label pipeline is one crate.

use crate::typst_library::foundations::{Content, StyleChain};

/// A pair of content and a style chain that applies to it.
pub type Pair<'a> = (&'a Content, StyleChain<'a>);
