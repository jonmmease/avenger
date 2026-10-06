//! Ported from crates/typst-library/src/symbols.rs @ v0.15.1, modified for Avenger.
//!
//! Modifiable symbols.
//!
//! avenger: no `define` or `define_math`; scopes arrive with evaluation, which looks names up
//! in codex's modules.

use crate::typst_library::foundations::Symbol;

impl From<codex::Symbol> for Symbol {
    fn from(symbol: codex::Symbol) -> Self {
        match symbol {
            codex::Symbol::Single(value) => Symbol::single(value),
            codex::Symbol::Multi(list) => Symbol::list(list),
        }
    }
}
