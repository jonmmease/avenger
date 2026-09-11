//! No-op timing shim for the vendored Typst parser.
//!
//! Upstream `typst-syntax` instruments parsing with `typst-timing`
//! (`crates/typst-timing/src`). Avenger labels do not expose timing traces, so
//! this module preserves the tiny API shape the parser expects without
//! retaining the tracing machinery.

/// Creates a timing scope around an expression.
#[macro_export]
macro_rules! timed {
    ($name:expr, span = $span:expr, $body:expr $(,)?) => {{
        let _ = ($name, $span);
        $body
    }};
    ($name:expr, $body:expr $(,)?) => {{
        let _ = $name;
        $body
    }};
}

/// A disabled timing scope.
pub struct TimingScope;

impl TimingScope {
    /// Create a no-op timing scope.
    #[inline]
    pub fn new(_name: &'static str) -> Option<Self> {
        None
    }

    /// Create a no-op timing scope with an attached span.
    #[inline]
    pub fn with_span(_name: &'static str, _span: Option<std::num::NonZeroU64>) -> Option<Self> {
        None
    }
}
