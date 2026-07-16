//! Chart-aware compiler for the Avenger chart language.
//!
//! This crate owns asynchronous project compilation, DataFusion-backed
//! analysis, native-registry lowering, and compiled chart artifacts. Parsing
//! and source-oriented semantics belong in `avenger-lang-core`; ergonomic
//! defaults and stable re-exports belong in the `avenger-lang` facade.
#![forbid(unsafe_code)]

/// Phase-zero marker used while the strict language frontend is built.
pub const COMPILER_PHASE: u8 = 0;
