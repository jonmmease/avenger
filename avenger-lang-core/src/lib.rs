//! Dependency-light frontend contracts for the Avenger chart language.
//!
//! This crate owns source text, spans, diagnostics, source loading, and—once
//! implemented—the strict parser and semantic frontend. It intentionally does
//! not depend on chart construction, rendering, application, filesystem, or
//! DataFusion crates. Keeping that boundary makes the frontend reusable by
//! browser analysis and future editor tooling.
#![forbid(unsafe_code)]

/// The language major implemented by this frontend.
pub const LANGUAGE_MAJOR: u32 = 1;
