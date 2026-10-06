//! No-op timing shim for the vendored Typst parser.
//!
//! Upstream `typst-syntax` opens `typst_timing::TimingScope`s around parsing
//! (`crates/typst-timing/src`). Avenger labels do not record timing traces, so
//! this module keeps only the constructor the parser calls.

/// A disabled timing scope.
pub struct TimingScope;

impl TimingScope {
    /// Create a no-op timing scope.
    #[inline]
    pub fn new(_name: &'static str) -> Option<Self> {
        None
    }
}
