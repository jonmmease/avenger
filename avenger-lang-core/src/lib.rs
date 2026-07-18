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
mod expand;
pub mod interchange;
mod loader;
mod physical_type;
pub mod print;
pub mod project;
pub mod resolve;
mod semantic_schema;
mod source;
pub mod sql;
pub mod syntax;

pub use capabilities::{
    DataCapabilities, EmptyEnvironmentProvider, EnvironmentProvider, ImportCapabilities,
    MapEnvironmentProvider,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, ExpansionOrImportFrame, SourceLabel,
    render_diagnostics, sort_diagnostics,
};
pub use expand::{
    ExpandedProject, ExpansionFailure, ExpansionMapping, ExpansionSourceMap, expand_project,
};
pub use loader::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceLoaderError,
};
pub use physical_type::{
    IntervalUnit, PhysicalField, PhysicalType, PhysicalTypeError, PhysicalValueError, TimeUnit,
};
pub use project::{
    AmbientDataDeclaration, DefinitionKind, ImportEdge, ParsedProject, ProjectDependency,
    ProjectDependencyRole, ProjectFile, ProjectFileId, ProjectFileKind, ProjectLoadAttempt,
    ProjectLoadFailure, ProjectLoadRequest, ProjectLoader, ProjectRoot,
};
pub use resolve::{
    DeclarationId, DefinitionChannel, DefinitionExport, DefinitionExportKind, DefinitionLocalSeed,
    DefinitionPart, DefinitionSchema, DefinitionSlot, EventId, GeneratedStateOrigin, HelperClass,
    MarkId, ParamId, ResolveAttempt, ResolveFailure, ResolvedActionRoute, ResolvedBinding,
    ResolvedDeclaration, ResolvedDimension, ResolvedEventBinding, ResolvedEventScope,
    ResolvedEventSurface, ResolvedExpression, ResolvedFile, ResolvedHelper, ResolvedHelperArgument,
    ResolvedOutputHandle, ResolvedOutputShape, ResolvedParam, ResolvedPart, ResolvedProject,
    ResolvedQuery, ResolvedReference, ResolvedSelection, ResolvedSelectionCombine,
    ResolvedSelectionEmpty, ResolvedSqlReference, ResolvedStateLValue, ResolvedStore,
    ResolvedTarget, ResolvedValue, SelectionId, StateMigrationKey, StateSharing, StoreId, ToolId,
    WidgetId, resolve_project,
};
pub use semantic_schema::semantic_json_schema;
pub use source::{
    ByteSpan, LineIndex, SourceError, SourceFile, SourceId, SourceLocation, SourceMap,
    SourceOrigin, SourceSpan,
};

/// The language major implemented by this frontend.
pub const LANGUAGE_MAJOR: u32 = 1;
