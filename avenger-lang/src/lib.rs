//! Public facade for the Avenger chart language and compiler.
//!
//! The facade remains intentionally small. Frontend implementation details
//! live in `avenger-lang-core`, while chart-aware compilation lives in
//! `avenger-lang-compiler`.
#![forbid(unsafe_code)]

use avenger_chart_lang_registry::{NativeRegistry, NativeRegistryBuilder, RegistryError, builtins};

pub use avenger_lang_compiler::{
    ArtifactCacheKey, COMPILER_PHASE, CatalogFactory, CatalogFactoryRegistry, CompileAttempt,
    CompileFailure, CompiledChartArtifact, CompiledChartInterface, CompiledDependency,
    CompiledProject, Compiler, CompilerBuildError, CompilerBuilder, CompilerOptions,
    DatasetLineageIndex, DatasetSchemaIndex, DefaultSourceLoader, DependencyFingerprint,
    DependencyRole, DiscoveredDependencySet, LanguageHost, ProjectAnalysis, ProjectChartId,
    ProjectDatasetId, ProjectFingerprint, SourceLoaderLimits,
};
pub use avenger_lang_core::{
    AmbientDataDeclaration, ByteSpan, ContentVersion, DataCapabilities, DefinitionKind, Diagnostic,
    DiagnosticCode, DiagnosticSeverity, EmptyEnvironmentProvider, EnvironmentProvider,
    ImportCapabilities, ImportEdge, InMemorySourceLoader, LANGUAGE_MAJOR, LineIndex, LoadedSource,
    MapEnvironmentProvider, ParsedProject, ProjectDependency, ProjectDependencyRole, ProjectFile,
    ProjectFileId, ProjectFileKind, ProjectLoadAttempt, ProjectLoadFailure, ProjectLoadRequest,
    ProjectLoader, ProjectRoot, ResolveAttempt, ResolveFailure, ResolvedProject, SourceFile,
    SourceId, SourceLabel, SourceLoader, SourceMap, SourceOrigin, SourceSpan,
};

/// Build the current canonical stock language registry.
///
pub fn stock_registry() -> Result<NativeRegistry, RegistryError> {
    builtins::stock_registry()
}

/// Register the current stock built-ins into an explicitly composed host.
pub fn register_builtins(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    builtins::register_stock_builtins(builder)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Build(#[from] CompilerBuildError),
    #[error(transparent)]
    Compile(#[from] CompileFailure),
}
