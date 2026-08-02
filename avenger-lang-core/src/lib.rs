//! Dependency-light frontend contracts for the Avenger chart language.
//!
//! This crate owns source text, spans, diagnostics, source loading, and—once
//! implemented—the strict parser and semantic frontend. It intentionally does
//! not depend on chart construction, rendering, application, filesystem, or
//! DataFusion crates. Keeping that boundary makes the frontend reusable by
//! browser analysis and future editor tooling.
#![forbid(unsafe_code)]

pub mod ast;
mod bundle;
mod capabilities;
pub mod contextual;
mod diagnostic;
mod expand;
pub mod interchange;
pub mod intrinsic;
mod loader;
pub mod module_graph;
mod physical_type;
pub mod print;
pub mod resolve;
mod semantic_schema;
mod source;
pub mod sql;
pub mod syntax;

pub use bundle::{BundleFailure, BundleTarget, BundledSource, bundle_module_graph};
pub use capabilities::{
    DataCapabilities, EmptyEnvironmentProvider, EnvironmentProvider, ImportCapabilities,
    MapEnvironmentProvider,
};
pub use contextual::{
    CONTEXTUAL_ACCESS_SIGNATURES, ContextualAccessContext, ContextualAccessPhysicalType,
    ContextualAccessSignature, contextual_access_signature,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, ExpansionOrImportFrame, SourceLabel,
    render_diagnostics, sort_diagnostics,
};
pub use expand::{
    ExpandedModuleGraph, ExpansionFailure, ExpansionLimits, ExpansionMapping, ExpansionSourceMap,
    expand_module_graph, expand_module_graph_with_limits,
};
pub use intrinsic::{
    INTRINSIC_OPERATION_SIGNATURES, IntrinsicOperationArgumentKind, IntrinsicOperationContext,
    IntrinsicOperationResult, IntrinsicOperationSignature, intrinsic_operation_signature,
};
pub use loader::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceLoaderError,
};
pub use module_graph::{
    AmbientDataItem, AvailableNativeModule, ModuleDependency, ModuleDependencyRole,
    ModuleDependencyTarget, ModuleGraphLoadAttempt, ModuleGraphLoadFailure, ModuleGraphLoadLimits,
    ModuleGraphLoadRequest, ModuleGraphLoader, ModuleId, ModuleImportEdge, ModuleRoot,
    ParsedModule, ParsedModuleGraph, ResolvedImportTarget, SourceModuleId, resolve_import_target,
    resolve_relative_origin,
};
pub use physical_type::{IntervalUnit, PhysicalField, PhysicalType, PhysicalTypeError, TimeUnit};
pub use resolve::{
    AuthoringItemGraph, BindingCategory, ChartEntrypointId, ChartSelector, DECLARATION_KEYWORDS,
    DeclarationId, DeclarationKey, DefinitionChannel, DefinitionExport, DefinitionExportKind,
    DefinitionKind, DefinitionLocalSeed, DefinitionPart, DefinitionSchema, DefinitionSlot, EventId,
    GeneratedStateOrigin, HelperClass, ItemDependencyCause, ItemDependencyEdge,
    ItemDependencyGraph, MarkId, ModuleBindingEnvironment, ModuleExportId, ModuleExportIndex,
    ModuleItemId, ParamId, ResolveAttempt, ResolveFailure, ResolvedActionRoute, ResolvedBboxEdge,
    ResolvedBinding, ResolvedCatalogTable, ResolvedChannelMember, ResolvedChartEntrypoint,
    ResolvedContextualAccess, ResolvedContextualAccessKind, ResolvedDeclaration, ResolvedDimension,
    ResolvedEventBinding, ResolvedEventScope, ResolvedEventSurface, ResolvedExpression,
    ResolvedHelper, ResolvedHelperArgument, ResolvedImport, ResolvedIntervalBoundary,
    ResolvedKindBinding, ResolvedModule, ResolvedModuleGraph, ResolvedModuleItem,
    ResolvedOutputHandle, ResolvedOutputShape, ResolvedParam, ResolvedPart, ResolvedProjection,
    ResolvedProjectionItem, ResolvedQuery, ResolvedReference, ResolvedRelationId,
    ResolvedRelationReference, ResolvedRelationTarget, ResolvedSelection, ResolvedSelectionCombine,
    ResolvedSelectionEmpty, ResolvedSqlReference, ResolvedStateLValue, ResolvedStore,
    ResolvedTarget, ResolvedValue, ResolvedViewAxis, ResolvedViewField, SelectionId,
    StateMigrationKey, StateSharing, StoreId, ToolId, WidgetId, allowed_child_declarations,
    placement_allowed, resolve_module_graph,
};
pub use semantic_schema::semantic_json_schema;
pub use source::{
    ByteSpan, LineIndex, SourceError, SourceFile, SourceId, SourceLocation, SourceMap,
    SourceOrigin, SourceSpan,
};

/// The language major implemented by this frontend.
pub const LANGUAGE_MAJOR: u32 = 1;
