//! Chart-aware compiler for the Avenger chart language.
//!
//! This crate owns asynchronous project compilation, DataFusion-backed
//! analysis, native-registry lowering, and compiled chart artifacts. Parsing
//! and source-oriented semantics belong in `avenger-lang-core`; ergonomic
//! defaults and stable re-exports belong in the `avenger-lang` facade.
#![forbid(unsafe_code)]

mod analysis;
mod artifact;
mod catalog;
mod compiler;
mod lowering;
mod options;
mod schema;
mod source_loader;
mod sql_profile;

pub use analysis::{
    AnalyzedColumn, AnalyzedDataset, AnalyzedMarkChannel, ColumnLineage, DatasetLineage,
    DatasetLineageIndex, DatasetProvenance, DatasetSchemaIndex, DatasetStageId, DatasetStageKind,
    FunctionInventory, ModuleAnalysis, ModuleDatasetId, ParamTypeIndex, ParamTypeInfo,
    ParamTypeProvenance, physical_type_to_arrow,
};
pub use artifact::{
    ArtifactCacheKey, ArtifactSerializationError, COMPILED_ARTIFACT_FORMAT_MAJOR,
    CompiledChartArtifact, CompiledChartInterface, CompiledModule, DependencyFingerprint,
    InterfaceStateBinding, ModuleDependencyFingerprints, ModuleFingerprint,
    NativeModuleRequirement, NativeRequirementSet,
};
pub use compiler::{
    CompileAttempt, CompileFailure, CompiledChartGeneration, CompiledDependency, Compiler,
    CompilerBuildError, CompilerBuilder, CompilerCacheSnapshot, DependencyRole,
    DiscoveredDependencySet, ExpandedSource, ModuleCompilationMode,
};
pub use options::{
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment,
    CompileEnvironmentError, CompileEnvironmentFactory, CompileEnvironmentRequest,
    CompileEnvironmentResourceVersion, CompilerLimits, CompilerOptions,
    DefaultCompileEnvironmentFactory, LocalResourceLimits, TableFactory, TableFactoryError,
    TableFactoryRegistry,
};
pub use schema::{LanguageHost, SemanticJsonSchema};
pub use source_loader::{DefaultSourceLoader, SourceLoaderLimits};
pub use sql_profile::{
    SQL_SEMANTIC_PROFILE, exact_numeric_scalar, normalize_sql_expression, normalize_sql_query,
};

/// Last fully implemented language/compiler plan phase.
pub const COMPILER_PHASE: u8 = 10;
