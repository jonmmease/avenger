//! Static label evaluation.
//!
//! This mirrors a small subset of upstream `typst-eval`: resolving parsed
//! markup, retained function calls, math expressions, and read-only external
//! parameters into label content. It intentionally excludes Typst scripting,
//! `#let`, `#set`, `#show`, imports, and document evaluation.

pub(crate) mod call;
pub(crate) mod delimiter;
pub(crate) mod markup;
pub(crate) mod math;
