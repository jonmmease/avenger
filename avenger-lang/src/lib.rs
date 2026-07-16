//! Public facade for the Avenger chart language and compiler.
//!
//! The facade remains intentionally small. Frontend implementation details
//! live in `avenger-lang-core`, while chart-aware compilation lives in
//! `avenger-lang-compiler`.
#![forbid(unsafe_code)]

pub use avenger_lang_compiler::COMPILER_PHASE;
pub use avenger_lang_core::LANGUAGE_MAJOR;
