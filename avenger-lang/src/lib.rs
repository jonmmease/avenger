//! Public facade for the Avenger chart language and compiler.
//!
//! The facade remains intentionally small. Frontend implementation details
//! live in `avenger-lang-core`, while chart-aware compilation lives in
//! `avenger-lang-compiler`.
#![forbid(unsafe_code)]

use avenger_chart_lang_registry::{NativeRegistry, NativeRegistryBuilder, RegistryError, builtins};

pub use avenger_lang_compiler::{
    ArtifactCacheKey, COMPILER_PHASE, CatalogFactory, CatalogFactoryRegistry, CompileAttempt,
    CompileFailure, CompiledChartArtifact, CompiledChartInterface, CompiledProject, Compiler,
    CompilerBuildError, CompilerBuilder, CompilerOptions, DatasetLineageIndex, DatasetSchemaIndex,
    DependencyFingerprint, DiscoveredDependencySet, LanguageHost, ProjectAnalysis, ProjectChartId,
    ProjectDatasetId, ProjectFingerprint,
};
pub use avenger_lang_core::{
    ByteSpan, ContentVersion, DataCapabilities, Diagnostic, DiagnosticCode, DiagnosticSeverity,
    EmptyEnvironmentProvider, EnvironmentProvider, ImportCapabilities, InMemorySourceLoader,
    LANGUAGE_MAJOR, LineIndex, LoadedSource, MapEnvironmentProvider, SourceFile, SourceId,
    SourceLabel, SourceLoader, SourceMap, SourceOrigin, SourceSpan,
};

/// Build the current canonical stock language registry.
///
/// Phase 0 contains the bootstrap inventory; Phase 6 expands the same function
/// to the complete built-in v1 inventory without changing compiler hosts.
pub fn stock_registry() -> Result<NativeRegistry, RegistryError> {
    builtins::bootstrap_registry()
}

/// Register the current stock built-ins into an explicitly composed host.
pub fn register_builtins(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    builtins::register_bootstrap_builtins(builder)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Build(#[from] CompilerBuildError),
    #[error(transparent)]
    Compile(#[from] CompileFailure),
}
