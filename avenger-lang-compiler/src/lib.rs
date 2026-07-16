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
mod options;
mod schema;

pub use analysis::{
    AnalyzedDataset, ColumnLineage, DatasetLineage, DatasetLineageIndex, DatasetProvenance,
    DatasetSchemaIndex, DatasetStageId, DatasetStageKind, ProjectAnalysis, ProjectDatasetId,
};
pub use artifact::{
    ArtifactCacheKey, CompiledChartArtifact, CompiledChartInterface, CompiledProject,
    DependencyFingerprint, InterfaceStateBinding, ProjectChartId, ProjectFingerprint,
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

/// Phase-zero marker used while the strict language frontend is built.
pub const COMPILER_PHASE: u8 = 0;
