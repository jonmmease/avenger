//! Dependency-light frontend contracts for the Avenger chart language.
//!
//! This crate owns source text, spans, diagnostics, source loading, and—once
//! implemented—the strict parser and semantic frontend. It intentionally does
//! not depend on chart construction, rendering, application, filesystem, or
//! DataFusion crates. Keeping that boundary makes the frontend reusable by
//! browser analysis and future editor tooling.
#![forbid(unsafe_code)]

pub mod ast;
mod capabilities;
mod diagnostic;
pub mod interchange;
mod loader;
mod source;
pub mod sql;

pub use capabilities::{
    DataCapabilities, EmptyEnvironmentProvider, EnvironmentProvider, ImportCapabilities,
    MapEnvironmentProvider,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, ExpansionOrImportFrame, SourceLabel,
    render_diagnostics, sort_diagnostics,
};
pub use loader::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceLoaderError,
};
pub use source::{
    ByteSpan, LineIndex, SourceError, SourceFile, SourceId, SourceLocation, SourceMap,
    SourceOrigin, SourceSpan,
};

/// The language major implemented by this frontend.
pub const LANGUAGE_MAJOR: u32 = 1;
