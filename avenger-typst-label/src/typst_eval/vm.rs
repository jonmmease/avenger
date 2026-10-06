//! Ported from crates/typst-eval/src/vm.rs @ v0.15.1, modified for Avenger.

use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::Scopes;

/// A virtual machine.
///
/// Holds the state needed to [evaluate](crate::eval()) Typst sources. A
/// new virtual machine is created for each module evaluation and function call.
// avenger: no control flow, tracing or context: labels have no loops, functions, IDE or
// introspection.
pub struct Vm<'a> {
    /// The underlying virtual typesetter.
    pub engine: Engine<'a>,
    /// The stack of scopes.
    pub scopes: Scopes<'a>,
}

impl<'a> Vm<'a> {
    /// Create a new virtual machine.
    pub fn new(engine: Engine<'a>, scopes: Scopes<'a>) -> Self {
        Self { engine, scopes }
    }
}
