//! Ported from crates/typst-library/src/engine.rs @ v0.15.1, modified for Avenger.
//!
//! Definition of the central compilation context.
//!
//! avenger: an engine is the world and a warning sink. Labels have no introspection, show rules,
//! imports or parallel layout, so there is no introspector, route, tracing, delayed error or
//! traced value. Nothing is memoized, so `Tracked` and `TrackedMut` are plain references.

use ecow::{EcoString, EcoVec};
use rustc_hash::FxHashSet;

use crate::typst_library::World;
use crate::typst_library::diag::SourceDiagnostic;
use crate::typst_syntax::DiagSpan;

/// A tracked reference, in place of comemo's.
pub type Tracked<'a, T> = &'a T;

/// A mutably tracked reference, in place of comemo's.
pub type TrackedMut<'a, T> = &'a mut T;

/// Holds all data needed during compilation.
pub struct Engine<'a> {
    /// The compilation environment.
    pub world: Tracked<'a, dyn World + 'a>,
    /// A pure sink for warnings, delayed errors, and spans under inspection.
    pub sink: TrackedMut<'a, Sink>,
}

/// A push-only sink for recorded introspections, delayed errors, warnings, and
/// traced values.
///
/// All tracked methods of this type are of the form `(&mut self, ..) -> ()`, so
/// in principle they do not need validation (though that optimization is not
/// yet implemented in comemo).
#[derive(Default, Clone)]
pub struct Sink {
    /// Warnings emitted during iteration.
    warnings: EcoVec<SourceDiagnostic>,
    /// Hashes of all warning's spans and messages for warning deduplication.
    // avenger: the spans and messages themselves, since there is no 128-bit hash.
    warnings_set: FxHashSet<(DiagSpan, EcoString)>,
}

impl Sink {
    /// Create a new empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the stored warnings.
    pub fn warnings(self) -> EcoVec<SourceDiagnostic> {
        self.warnings
    }

    /// Add a warning.
    pub fn warn(&mut self, warning: SourceDiagnostic) {
        // Check if warning is a duplicate.
        if self.warnings_set.insert((warning.span, warning.message.clone())) {
            self.warnings.push(warning);
        }
    }
}
