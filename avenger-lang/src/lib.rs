//! Public facade for the Avenger chart language and compiler.
//!
//! The facade remains intentionally small. Frontend implementation details
//! live in `avenger-lang-core`, while chart-aware compilation lives in
//! `avenger-lang-compiler`.
#![forbid(unsafe_code)]

use avenger_chart_lang_registry::builtins;
pub use avenger_chart_lang_registry::{
    NativeBuiltinProfileId, NativeModuleBuilder, NativeModuleExport, NativeModuleId,
    NativeModuleImplementationProfileId, NativeModuleSchema, NativeModuleSchemaProfileId,
    NativeRegistry, NativeRegistryBuilder, NativeRegistryProfileId, RegisteredNativeModule,
    RegistryError, ResolvedNativeExport,
};

pub use avenger_lang_compiler::{
    AnalyzedColumn, AnalyzedDataset, ArtifactCacheKey, COMPILER_PHASE, CatalogFactory,
    CatalogFactoryError, CatalogFactoryRegistry, CompileAttempt, CompileEnvironment,
    CompileEnvironmentError, CompileEnvironmentFactory, CompileEnvironmentRequest,
    CompileEnvironmentResourceVersion, CompileFailure, CompiledChartArtifact,
    CompiledChartGeneration, CompiledChartInterface, CompiledDependency, CompiledProject, Compiler,
    CompilerBuildError, CompilerBuilder, CompilerCacheSnapshot, CompilerLimits, CompilerOptions,
    DatasetLineageIndex, DatasetSchemaIndex, DatasetStageId, DatasetStageKind, DefaultSourceLoader,
    DependencyFingerprint, DependencyRole, DiscoveredDependencySet, LanguageHost,
    LocalResourceLimits, ProjectAnalysis, ProjectChartId, ProjectCompilationMode, ProjectDatasetId,
    ProjectDependencyFingerprints, ProjectFingerprint, SourceLoaderLimits, TableFactory,
    TableFactoryError, TableFactoryRegistry,
};
pub use avenger_lang_core::{
    AmbientDataItem, AvailableNativeModule, ByteSpan, ContentVersion, DataCapabilities,
    DefinitionKind, Diagnostic, DiagnosticCode, DiagnosticSeverity, EmptyEnvironmentProvider,
    EnvironmentProvider, ExpansionLimits, ImportCapabilities, InMemorySourceLoader, LANGUAGE_MAJOR,
    LineIndex, LoadedSource, MapEnvironmentProvider, ModuleDependency, ModuleDependencyRole,
    ModuleDependencyTarget, ModuleGraphLoadAttempt, ModuleGraphLoadFailure, ModuleGraphLoadLimits,
    ModuleGraphLoadRequest, ModuleGraphLoader, ModuleId, ModuleImportEdge, ModuleRoot,
    ParsedModule, ParsedModuleGraph, ResolveAttempt, ResolveFailure, ResolvedProject, SourceFile,
    SourceId, SourceLabel, SourceLoader, SourceMap, SourceModuleId, SourceOrigin, SourceSpan,
};
pub use avenger_lang_core::{sql::SqlParseLimits, syntax::SyntaxLimits};

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
