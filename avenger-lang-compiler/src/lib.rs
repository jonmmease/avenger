//! Chart-aware compiler for the Avenger chart language.
//!
//! This crate owns asynchronous project compilation, DataFusion-backed
//! analysis, native-registry lowering, and compiled chart artifacts. Parsing
//! and source-oriented semantics belong in `avenger-lang-core`; ergonomic
//! defaults and stable re-exports belong in the `avenger-lang` facade.
#![forbid(unsafe_code)]

mod analysis;
mod artifact;
mod compiler;
mod lowering;
mod options;
mod schema;
mod source_loader;

pub use analysis::{
    AnalyzedDataset, ColumnLineage, DatasetLineage, DatasetLineageIndex, DatasetProvenance,
    DatasetSchemaIndex, DatasetStageId, DatasetStageKind, ProjectAnalysis, ProjectDatasetId,
};
pub use artifact::{
    ArtifactCacheKey, ArtifactSerializationError, COMPILED_ARTIFACT_FORMAT_MAJOR,
    CompiledChartArtifact, CompiledChartInterface, CompiledProject, DependencyFingerprint,
    InterfaceStateBinding, ProjectChartId, ProjectFingerprint,
};
pub use compiler::{
    CompileAttempt, CompileFailure, CompiledDependency, Compiler, CompilerBuildError,
    CompilerBuilder, DependencyRole, DiscoveredDependencySet, ExpandedSource,
};
pub use options::{
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment,
    CompileEnvironmentError, CompileEnvironmentFactory, CompileEnvironmentRequest, CompilerOptions,
    DefaultCompileEnvironmentFactory,
};
pub use schema::{LanguageHost, SemanticJsonSchema};
pub use source_loader::{DefaultSourceLoader, SourceLoaderLimits};

/// Last fully implemented language/compiler plan phase.
pub const COMPILER_PHASE: u8 = 5;
