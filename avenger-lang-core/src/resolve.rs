//! Registry-directed semantic resolution for parsed Avenger projects.
//!
//! This phase deliberately stops before DataFusion planning and native chart
//! construction. Its output contains no unresolved authored names: SQL
//! placeholders, typed references, state l-values, structural paths, and
//! definition imports are all bound to opaque semantic identities.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use avenger_chart_schema::{
    BodyMode, KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    ValueShape,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlparser::ast::{
    AccessExpr, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, Subscript,
    Value as SqlValue, Visit, Visitor,
};

use crate::{
    Diagnostic, ExpansionOrImportFrame, LANGUAGE_MAJOR, PhysicalField, PhysicalType, SourceFile,
    SourceId, SourceLabel, SourceMap, SourceSpan,
    ast::{
        AstNodeRole, BindingKind, BindingTime, Decl, Name, RefKind, SqlExpression, Value,
        Visibility,
    },
    expand::ExpansionSourceMap,
    module_graph::{ModuleId, ModuleImportEdge, ParsedModule, ParsedModuleGraph, SourceModuleId},
    sort_diagnostics,
};

macro_rules! semantic_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

/// Virtual declaration-path segment used for declarations authored inside a
/// schema-owned `overlay:` mark block.
///
/// The generic AST correctly keeps these declarations inside the property
/// value. Resolution still assigns them ordinary declaration identities and
/// scopes so they use the complete mark pipeline.
const MARK_BLOCK_PATH_SEGMENT: usize = usize::MAX;

semantic_id!(DeclarationId);
semantic_id!(ParamId);
semantic_id!(StoreId);
semantic_id!(SelectionId);
semantic_id!(MarkId);
semantic_id!(ToolId);
semantic_id!(WidgetId);
semantic_id!(EventId);
semantic_id!(StateMigrationKey);
semantic_id!(DefinitionLocalSeed);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeclarationKey(String);

impl DeclarationKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModuleItemId {
    pub module: SourceModuleId,
    pub declaration: DeclarationKey,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResolvedRelationId {
    pub defining_item: ModuleItemId,
    pub nested_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "selector", content = "name")]
pub enum ChartSelector {
    Anonymous,
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChartEntrypointId {
    pub module: SourceModuleId,
    pub selector: ChartSelector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "category", content = "namespace")]
pub enum BindingCategory {
    NativeKind(NativeKindNamespace),
    Chart,
    Data,
    ModuleNamespace,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModuleExportId {
    pub module: ModuleId,
    pub name: String,
    pub category: BindingCategory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "binding", content = "identity")]
pub enum ResolvedKindBinding {
    LanguageCore(String),
    Builtin(NativeKindKey),
    Native {
        export: ModuleExportId,
        implementation: NativeKindKey,
    },
    Definition(ModuleItemId),
    Structural(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedImport {
    pub source: ModuleId,
    pub specifier: String,
    pub clause: crate::ast::ImportClause,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleExportIndex {
    pub exports: BTreeMap<String, ModuleExportId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleBindingEnvironment {
    pub local: BTreeMap<(BindingCategory, String), ModuleExportId>,
    pub namespaces: BTreeMap<String, ModuleId>,
}

/// The reusable declaration family authored by a top-level `define` item.
///
/// This describes an item, never a source-module/file classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Mark,
    Tool,
    Transform,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum StateSharing {
    #[default]
    Shared,
    Free,
    Level(u32),
}

/// The source of a scalar parameter's physical Arrow type.
///
/// Authored scalar params are inferred later by the DataFusion-backed compiler.
/// Native widget/tool state can retain a schema-fixed physical contract without
/// making the dependency-light resolver approximate SQL expression types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "type", rename_all = "snake_case")]
pub enum ParamTypeContract {
    Inferred,
    SchemaFixed(PhysicalType),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedParam {
    pub id: ParamId,
    pub declaration: DeclarationId,
    pub source_name: String,
    pub type_contract: ParamTypeContract,
    pub initializer: ResolvedValue,
    pub sharing: StateSharing,
    pub migration_key: Option<StateMigrationKey>,
    pub definition_local_seed: Option<DefinitionLocalSeed>,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
    pub table_owner: Option<DeclarationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedStore {
    pub id: StoreId,
    pub declaration: DeclarationId,
    pub source_name: String,
    pub fields: Vec<PhysicalField>,
    pub primary_key: Vec<String>,
    pub rows: Vec<BTreeMap<String, ResolvedValue>>,
    pub sharing: StateSharing,
    pub migration_key: Option<StateMigrationKey>,
    pub definition_local_seed: Option<DefinitionLocalSeed>,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSelection {
    pub id: SelectionId,
    pub declaration: DeclarationId,
    pub source_name: String,
    pub empty: ResolvedSelectionEmpty,
    pub combine: ResolvedSelectionCombine,
    pub migration_key: Option<StateMigrationKey>,
    pub definition_local_seed: Option<DefinitionLocalSeed>,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedSelectionEmpty {
    All,
    #[default]
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedSelectionCombine {
    #[default]
    Union,
    Intersect,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedStateOrigin {
    pub declaration: DeclarationId,
    pub export_role: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionSchema {
    pub item: ModuleItemId,
    pub declaration: DeclarationId,
    pub local_seed: DefinitionLocalSeed,
    pub kind: DefinitionKind,
    pub source_name: String,
    pub slot_order: Vec<String>,
    pub slots: BTreeMap<String, DefinitionSlot>,
    pub channels: BTreeMap<String, DefinitionChannel>,
    pub outputs: BTreeMap<String, Option<ResolvedValue>>,
    pub exports: BTreeMap<String, DefinitionExport>,
    pub parts: BTreeMap<String, DefinitionPart>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionExport {
    pub path: Vec<String>,
    pub target_kind: DefinitionExportKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamTypeRequirement {
    pub target: ResolvedTarget,
    pub expected: PhysicalType,
    pub declaration: DeclarationId,
    pub span: SourceSpan,
    pub role: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionExportKind {
    Param,
    Store,
    Selection,
    Mark,
    Tool,
    Widget,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionPart {
    pub alias: String,
    pub declaration_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionSlot {
    pub shape: String,
    pub required: bool,
    pub default: Option<ResolvedValue>,
    pub enum_values: Vec<String>,
    pub reference_kind: Option<String>,
    pub exposes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionChannel {
    pub required: bool,
    pub physical_channel: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedModuleGraph {
    pub language_major: u32,
    pub registry_schema_major: u32,
    pub registry_schema_minor: u32,
    pub registry_profile_label: String,
    pub source_fingerprint: String,
    pub sources: SourceMap,
    pub source_modules: BTreeMap<SourceModuleId, ResolvedModule>,
    pub items: BTreeMap<ModuleItemId, ResolvedModuleItem>,
    pub entrypoints: BTreeMap<ChartEntrypointId, ResolvedChartEntrypoint>,
    pub item_dependencies: ItemDependencyGraph,
    pub authoring_items: AuthoringItemGraph,
    pub charts: Vec<DeclarationId>,
    pub definitions: BTreeMap<ModuleItemId, DefinitionSchema>,
    pub params: BTreeMap<ParamId, ResolvedParam>,
    pub stores: BTreeMap<StoreId, ResolvedStore>,
    pub selections: BTreeMap<SelectionId, ResolvedSelection>,
    pub public_targets: BTreeMap<String, ResolvedTarget>,
    pub param_initializer_order: Vec<ParamId>,
    pub param_type_requirements: Vec<ParamTypeRequirement>,
    /// Catalog tables keyed by their declaration-local SQL path. Import aliases
    /// are applied when a project analysis environment exposes a data pack;
    /// queries inside the pack continue to use these local paths.
    pub catalog_tables: BTreeMap<ResolvedRelationId, ResolvedCatalogTable>,
    pub table_order: Vec<DeclarationId>,
    pub definition_import_order: Vec<ModuleItemId>,
    /// Empty for ordinary projects; populated by the compiler when imported
    /// definitions were expanded before final semantic resolution.
    pub expansion_source_map: ExpansionSourceMap,
}

impl ResolvedModuleGraph {
    /// Find the authored source for a declaration span, including declarations
    /// reparsed from canonical definition-expanded source.
    pub fn authored_source(&self, span: SourceSpan) -> Option<&SourceFile> {
        let authored = self.expansion_source_map.authored_span(span);
        self.sources.get(authored.source)
    }
}

/// Planning metadata for one catalog table declaration.
///
/// This stays DataFusion-independent so resolution can run in lightweight
/// tooling. The compiler attaches providers and exact Arrow schemas later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCatalogTable {
    pub relation: ResolvedRelationId,
    pub id: DeclarationId,
    pub file: SourceModuleId,
    pub source: SourceId,
    pub span: SourceSpan,
    pub path: Vec<String>,
    pub kind: String,
    pub params: Vec<ParamId>,
    pub dependencies: Vec<DeclarationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModule {
    pub id: SourceModuleId,
    pub source: SourceId,
    pub imports: BTreeMap<String, SourceModuleId>,
    pub roots: Vec<ResolvedDeclaration>,
    pub local_bindings: ModuleBindingEnvironment,
    pub exports: ModuleExportIndex,
    pub resolved_imports: Vec<ResolvedImport>,
    pub item_order: Vec<ModuleItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModuleItem {
    pub id: ModuleItemId,
    pub exported: bool,
    pub declaration: DeclarationId,
    pub category: BindingCategory,
    pub source_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemDependencyCause {
    DefinitionUse,
    RelationUse,
    NativeCapability,
    PrivateHelper,
    ExportedClosure,
    ChartReference,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemDependencyEdge {
    pub from: ModuleItemId,
    pub to: ModuleItemId,
    pub cause: ItemDependencyCause,
    pub site: SourceSpan,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemDependencyGraph {
    pub edges: Vec<ItemDependencyEdge>,
    pub transitive_closures: BTreeMap<ModuleItemId, BTreeSet<ModuleItemId>>,
}

/// Stable authoring-level item inventory retained after definition expansion.
///
/// Expansion may remove imported definitions from the executable graph.
/// Incremental compilation still needs their original dependency closure so a
/// definition edit invalidates consumers without invalidating sibling charts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoringItemGraph {
    pub item_order: BTreeMap<SourceModuleId, Vec<ModuleItemId>>,
    pub chart_closures: BTreeMap<ChartEntrypointId, BTreeSet<ModuleItemId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedChartEntrypoint {
    pub id: ChartEntrypointId,
    pub item: ModuleItemId,
    pub declaration: DeclarationId,
    pub params: BTreeMap<ParamId, ResolvedParam>,
    pub stores: BTreeMap<StoreId, ResolvedStore>,
    pub selections: BTreeMap<SelectionId, ResolvedSelection>,
    pub public_targets: BTreeMap<String, ResolvedTarget>,
    pub param_initializer_order: Vec<ParamId>,
    pub reachable_items: BTreeSet<ModuleItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDeclaration {
    pub id: DeclarationId,
    pub source: SourceId,
    pub span: SourceSpan,
    pub keyword: String,
    pub kind: Option<String>,
    pub kind_binding: Option<ResolvedKindBinding>,
    pub name: Option<String>,
    pub visibility: Visibility,
    pub coordinate: Option<String>,
    pub component_kind: Option<String>,
    pub properties: BTreeMap<String, ResolvedValue>,
    pub relation_references: Vec<ResolvedRelationReference>,
    /// Definition-authored logical channel property names retain their bound
    /// interface identity until Phase 7 substitutes an instance mapping.
    pub property_channels: BTreeMap<String, ResolvedTarget>,
    pub children: Vec<ResolvedDeclaration>,
    pub runtime_target: Option<ResolvedTarget>,
    pub migration_key: Option<StateMigrationKey>,
    pub definition_local_seed: Option<DefinitionLocalSeed>,
    pub public_path: Option<String>,
    pub parts: BTreeMap<String, ResolvedPart>,
    pub exports: BTreeMap<String, ResolvedTarget>,
    pub transform_outputs: BTreeMap<String, ResolvedOutputHandle>,
    pub event_binding: Option<ResolvedEventBinding>,
    pub state_lvalue: Option<ResolvedStateLValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEventBinding {
    pub event_type: String,
    pub targets: Vec<ResolvedTarget>,
    pub scope: ResolvedEventScope,
    pub surface: ResolvedEventSurface,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", content = "targets", rename_all = "snake_case")]
pub enum ResolvedEventScope {
    Plot(DeclarationId),
    Subplots {
        plot: DeclarationId,
        targets: Vec<ResolvedTarget>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "surface", content = "channel", rename_all = "snake_case")]
pub enum ResolvedEventSurface {
    Plot(DeclarationId),
    All(DeclarationId),
    Legend {
        plot: DeclarationId,
        channel: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedActionRoute {
    Current,
    Start,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedStateLValue {
    pub target: ResolvedTarget,
    pub route: ResolvedActionRoute,
    pub replacing_scopes: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPart {
    pub source_alias: String,
    pub runtime_kind: String,
    pub runtime_alias: Option<String>,
    pub targetable: bool,
    pub declaration: DeclarationId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResolvedOutputHandle {
    pub producer: DeclarationId,
    pub name: String,
    pub ordinal: usize,
    pub shape: ResolvedOutputShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedOutputShape {
    Expression,
    RasterDimension,
    Opaque,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDimension {
    pub target: ResolvedOutputHandle,
    pub authored_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "target", content = "id", rename_all = "snake_case")]
pub enum ResolvedTarget {
    Declaration(DeclarationId),
    Param(ParamId),
    Store(StoreId),
    Selection(SelectionId),
    Mark(MarkId),
    Tool(ToolId),
    Widget(WidgetId),
    Event(EventId),
    DefinitionParam {
        instance: DeclarationId,
        definition: DeclarationId,
        alias: String,
    },
    DefinitionStore {
        instance: DeclarationId,
        definition: DeclarationId,
        alias: String,
    },
    DefinitionSelection {
        instance: DeclarationId,
        definition: DeclarationId,
        alias: String,
    },
    DefinitionStructural {
        instance: DeclarationId,
        definition: DeclarationId,
        alias: String,
        kind: DefinitionExportKind,
    },
    DefinitionSlot {
        definition: DeclarationId,
        name: String,
    },
    DefinitionChannel {
        definition: DeclarationId,
        name: String,
    },
    Part {
        declaration: DeclarationId,
        alias: String,
    },
    Output(ResolvedOutputHandle),
    Reserved {
        namespace: String,
        path: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "value", content = "detail", rename_all = "snake_case")]
pub enum ResolvedValue {
    String(String),
    Number(String),
    Boolean(bool),
    Null,
    Atom(String),
    Column(String),
    Expression(ResolvedExpression),
    Projection(ResolvedProjection),
    Query(ResolvedQuery),
    Relation(ResolvedRelationReference),
    Binding(ResolvedBinding),
    Reference(ResolvedReference),
    Channel {
        mode: crate::ast::ChannelMode,
        expression: Box<ResolvedValue>,
    },
    ChannelValue(ResolvedChannelValue),
    Dimension(ResolvedDimension),
    Pattern(Box<ResolvedValue>),
    Environment(String),
    None,
    Array(Vec<ResolvedValue>),
    Object {
        /// A value-bearing block head, such as the expression in
        /// `x: "amount" { scale: linear; }`.
        ///
        /// Atom heads identify typed blocks and are represented by `kind`
        /// instead, so the two fields are mutually exclusive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head: Option<Box<ResolvedValue>>,
        kind: Option<String>,
        properties: BTreeMap<String, ResolvedValue>,
        children: Vec<ResolvedDeclaration>,
    },
    Call {
        function: String,
        args: Vec<ResolvedValue>,
    },
    DefinitionArgument(ResolvedTarget),
    Invalid,
}

/// One expression-bearing branch of a resolved authored channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedChannelBranch {
    pub mode: crate::ast::ChannelMode,
    pub expression: Box<ResolvedValue>,
}

/// One ordered conditional branch of a resolved authored channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedChannelCondition {
    pub predicate: Box<ResolvedValue>,
    pub branch: ResolvedChannelBranch,
    pub span: SourceSpan,
}

/// Normalized semantic form for an expression-driven channel.
///
/// The authored head remains available for source-aligned expansion and
/// bundling. `effective_fallback()` applies an authored `otherwise` when
/// present. Shared configuration excludes the structural `otherwise`
/// property, and conditions retain source order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedChannelValue {
    pub head: ResolvedChannelBranch,
    pub conditions: Vec<ResolvedChannelCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<ResolvedChannelBranch>,
    pub configuration: BTreeMap<String, ResolvedValue>,
}

impl ResolvedChannelValue {
    pub fn effective_fallback(&self) -> &ResolvedChannelBranch {
        self.otherwise.as_ref().unwrap_or(&self.head)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExpression {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contextual_accesses: Vec<ResolvedContextualAccess>,
    pub references: Vec<ResolvedSqlReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContextualAccess {
    pub kind: ResolvedContextualAccessKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "access", rename_all = "snake_case")]
pub enum ResolvedContextualAccessKind {
    DatumField {
        field: String,
    },
    MarkChannel {
        channel: ResolvedChannelMember,
    },
    EventCoord {
        channel: ResolvedChannelMember,
    },
    EventStartCoord {
        channel: ResolvedChannelMember,
    },
    EventDomainBoundary {
        channel: ResolvedChannelMember,
        boundary: ResolvedIntervalBoundary,
    },
    EventPath,
    EventFacet {
        one_based_index: u32,
    },
    EventLegendValue,
    ItemChannel {
        channel: ResolvedChannelMember,
        physical_type: PhysicalType,
    },
    ItemDataField {
        field: String,
    },
    ItemBbox {
        edge: ResolvedBboxEdge,
    },
    ViewField {
        target: ResolvedTarget,
        authored_view: Vec<String>,
        axis: ResolvedViewAxis,
        field: ResolvedViewField,
    },
}

impl ResolvedContextualAccessKind {
    pub const fn signature_pattern(&self) -> &'static str {
        match self {
            Self::MarkChannel { .. } => "channel.<channel>",
            Self::DatumField { .. } => "datum.\"<field>\"",
            Self::EventCoord { .. } => "event.coord.<channel>",
            Self::EventStartCoord { .. } => "event.start.coord.<channel>",
            Self::EventDomainBoundary {
                boundary: ResolvedIntervalBoundary::Start,
                ..
            } => "event.domain.<channel>.start",
            Self::EventDomainBoundary {
                boundary: ResolvedIntervalBoundary::End,
                ..
            } => "event.domain.<channel>.end",
            Self::EventPath => "event.path",
            Self::EventFacet { .. } => "event.facet[n]",
            Self::EventLegendValue => "event.legend.value",
            Self::ItemChannel { .. } => "item.channel.<channel>",
            Self::ItemDataField { .. } => "item.data.\"<field>\"",
            Self::ItemBbox { .. } => "item.bbox.<edge>",
            Self::ViewField {
                axis: ResolvedViewAxis::X,
                field: ResolvedViewField::DomainStart,
                ..
            } => "<view>.x.domain.start",
            Self::ViewField {
                axis: ResolvedViewAxis::X,
                field: ResolvedViewField::DomainEnd,
                ..
            } => "<view>.x.domain.end",
            Self::ViewField {
                axis: ResolvedViewAxis::Y,
                field: ResolvedViewField::DomainStart,
                ..
            } => "<view>.y.domain.start",
            Self::ViewField {
                axis: ResolvedViewAxis::Y,
                field: ResolvedViewField::DomainEnd,
                ..
            } => "<view>.y.domain.end",
            Self::ViewField {
                axis: ResolvedViewAxis::X,
                field: ResolvedViewField::Pixels,
                ..
            } => "<view>.x.pixels",
            Self::ViewField {
                axis: ResolvedViewAxis::Y,
                field: ResolvedViewField::Pixels,
                ..
            } => "<view>.y.pixels",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "snake_case")]
pub enum ResolvedChannelMember {
    Named {
        name: String,
    },
    Definition {
        target: ResolvedTarget,
        name: String,
        family_suffix: String,
    },
}

impl ResolvedChannelMember {
    pub fn authored_name(&self) -> String {
        match self {
            Self::Named { name } => name.clone(),
            Self::Definition { name, .. } => name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedIntervalBoundary {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedBboxEdge {
    Top,
    Right,
    Bottom,
    Left,
}

impl ResolvedBboxEdge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedViewAxis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedViewField {
    DomainStart,
    DomainEnd,
    Pixels,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjection {
    pub sql: String,
    pub items: Vec<ResolvedProjectionItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjectionItem {
    pub sql: String,
    pub expression: Option<ResolvedExpression>,
    pub aliases: Vec<String>,
    pub aliases_quoted: Vec<bool>,
    pub direct_column: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedQuery {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
    pub references: Vec<ResolvedSqlReference>,
    pub relations: Vec<ResolvedRelationReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRelationReference {
    pub authored_path: Vec<String>,
    pub target: ResolvedRelationTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "relation", content = "identity")]
pub enum ResolvedRelationTarget {
    Relation(ResolvedRelationId),
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSqlReference {
    pub authored_path: Vec<String>,
    pub target: ResolvedTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBinding {
    pub target: ResolvedTarget,
    pub kind: BindingKind,
    pub time: BindingTime,
    pub authored_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedReference {
    pub target: ResolvedTarget,
    pub kind: RefKind,
    pub authored_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResolvedHelper {
    pub name: String,
    pub class: HelperClass,
    pub arguments: Vec<ResolvedHelperArgument>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "argument", content = "detail", rename_all = "snake_case")]
pub enum ResolvedHelperArgument {
    Name(String),
    String(String),
    Number(String),
    DatumField(String),
    Target {
        target: ResolvedTarget,
        authored_path: Vec<String>,
    },
    DefinitionChannel {
        target: ResolvedTarget,
        family_suffix: String,
    },
    Sql(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperClass {
    Selection,
    Reserved,
}

#[derive(Clone, Debug)]
pub struct ResolveFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

#[derive(Clone, Debug)]
pub struct ResolveAttempt {
    pub result: Result<ResolvedModuleGraph, ResolveFailure>,
}

/// Resolve a parsed project against one immutable authoring registry snapshot.
pub fn resolve_module_graph(
    project: &ParsedModuleGraph,
    registry: &NativeSchemaSnapshot,
) -> ResolveAttempt {
    let mut resolver = Resolver::new(project, registry);
    resolver.run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ScopeId(usize);

#[derive(Clone, Debug, Default)]
struct Scope {
    parent: Option<ScopeId>,
    module: Option<SourceModuleId>,
    owner: Option<DeclarationId>,
    label: String,
    state_symbols: BTreeMap<String, StateSymbol>,
    structural: BTreeMap<String, DeclarationId>,
    transforms: BTreeSet<String>,
    events: BTreeMap<String, EventId>,
    definition_arguments: BTreeMap<String, ResolvedTarget>,
    event_binding: bool,
    event_has_between: bool,
}

#[derive(Clone, Debug)]
enum StateSymbol {
    Param(ParamId),
    Store(StoreId),
    Selection(SelectionId),
}

impl StateSymbol {
    fn target(&self) -> ResolvedTarget {
        match self {
            Self::Param(id) => ResolvedTarget::Param(id.clone()),
            Self::Store(id) => ResolvedTarget::Store(id.clone()),
            Self::Selection(id) => ResolvedTarget::Selection(id.clone()),
        }
    }
}

#[derive(Clone, Copy)]
enum StorePayloadShape {
    CompleteRow,
    Key,
    Patch,
}

#[derive(Clone, Debug)]
struct DeclInfo {
    id: DeclarationId,
    span: SourceSpan,
    containing_scope: ScopeId,
    child_scope: Option<ScopeId>,
    ancestry: Vec<DeclarationId>,
    runtime_target: Option<ResolvedTarget>,
}

#[derive(Clone, Debug, Default)]
struct InstanceInterface {
    child_scope: Option<ScopeId>,
    exports: BTreeMap<String, ResolvedTarget>,
    published_exports: BTreeSet<String>,
    pending_exports: BTreeMap<String, Vec<String>>,
    parts: BTreeMap<String, ResolvedPart>,
}

#[derive(Clone, Debug)]
struct ItemBinding {
    id: ModuleItemId,
    category: BindingCategory,
    name: Option<String>,
    exported: bool,
    declaration: DeclarationId,
}

#[derive(Clone, Debug, Default)]
struct ModuleIndex {
    environments: BTreeMap<SourceModuleId, ModuleBindingEnvironment>,
    exports: BTreeMap<ModuleId, ModuleExportIndex>,
    items: BTreeMap<ModuleItemId, ItemBinding>,
    item_at: BTreeMap<(SourceModuleId, usize), ModuleItemId>,
    source_export_items: BTreeMap<(SourceModuleId, String), ModuleItemId>,
    local_items: BTreeMap<(SourceModuleId, BindingCategory, String), ModuleItemId>,
    resolved_imports: BTreeMap<SourceModuleId, Vec<ResolvedImport>>,
}

struct Resolver<'a> {
    project: &'a ParsedModuleGraph,
    registry: &'a NativeSchemaSnapshot,
    diagnostics: Vec<Diagnostic>,
    module_index: ModuleIndex,
    scopes: Vec<Scope>,
    declarations: BTreeMap<(SourceModuleId, Vec<usize>), DeclInfo>,
    instances: BTreeMap<DeclarationId, InstanceInterface>,
    imports: BTreeMap<SourceModuleId, BTreeMap<String, SourceModuleId>>,
    definitions: BTreeMap<ModuleItemId, DefinitionSchema>,
    params: BTreeMap<ParamId, ResolvedParam>,
    stores: BTreeMap<StoreId, ResolvedStore>,
    selections: BTreeMap<SelectionId, ResolvedSelection>,
    param_dependencies: BTreeMap<ParamId, BTreeSet<ParamId>>,
    param_type_requirements: Vec<ParamTypeRequirement>,
    table_dependencies: BTreeMap<DeclarationId, BTreeSet<DeclarationId>>,
    relation_names: BTreeMap<(SourceModuleId, Vec<String>), ResolvedRelationId>,
    relation_declarations: BTreeMap<ResolvedRelationId, DeclarationId>,
}

impl<'a> Resolver<'a> {
    fn new(project: &'a ParsedModuleGraph, registry: &'a NativeSchemaSnapshot) -> Self {
        Self {
            project,
            registry,
            diagnostics: Vec::new(),
            module_index: ModuleIndex::default(),
            scopes: Vec::new(),
            declarations: BTreeMap::new(),
            instances: BTreeMap::new(),
            imports: BTreeMap::new(),
            definitions: BTreeMap::new(),
            params: BTreeMap::new(),
            stores: BTreeMap::new(),
            selections: BTreeMap::new(),
            param_dependencies: BTreeMap::new(),
            param_type_requirements: Vec::new(),
            table_dependencies: BTreeMap::new(),
            relation_names: BTreeMap::new(),
            relation_declarations: BTreeMap::new(),
        }
    }

    fn run(&mut self) -> ResolveAttempt {
        self.check_versions();
        self.build_module_index();
        self.build_import_bindings();
        self.extract_definition_schemas();
        self.validate_definition_templates();
        self.predeclare_project();
        self.build_table_dependencies();
        self.finish_instance_interfaces();

        let mut files = BTreeMap::new();
        let mut charts = Vec::new();
        for (file_id, file) in &self.project.source_modules {
            let roots = module_declarations(file)
                .map(|(index, declaration)| {
                    let path = vec![index];
                    let resolved = self.resolve_declaration(
                        file,
                        declaration,
                        &path,
                        None,
                        None,
                        false,
                        false,
                    );
                    if declaration.keyword.as_str() == "chart" {
                        charts.push(resolved.id.clone());
                    }
                    resolved
                })
                .collect();
            files.insert(
                file_id.clone(),
                ResolvedModule {
                    id: file_id.clone(),
                    source: file.source,
                    imports: self.imports.get(file_id).cloned().unwrap_or_default(),
                    roots,
                    local_bindings: self
                        .module_index
                        .environments
                        .get(file_id)
                        .cloned()
                        .unwrap_or_default(),
                    exports: self
                        .module_index
                        .exports
                        .get(&ModuleId::Source(file_id.clone()))
                        .cloned()
                        .unwrap_or_default(),
                    resolved_imports: self
                        .module_index
                        .resolved_imports
                        .get(file_id)
                        .cloned()
                        .unwrap_or_default(),
                    item_order: file
                        .parsed
                        .ast
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(index, _)| {
                            self.module_index
                                .item_at
                                .get(&(file_id.clone(), index))
                                .cloned()
                        })
                        .collect(),
                },
            );
        }

        self.prune_unused_lazy_exports(&mut files);
        let param_initializer_order = self.check_param_initializer_dag();
        let table_order = self.check_table_dag();
        let definition_import_order = self.definition_import_order();
        let mut public_targets = BTreeMap::new();
        for file in files.values() {
            for root in &file.roots {
                let mut root_targets = BTreeMap::new();
                let mut origins = BTreeMap::new();
                let mut collisions = Vec::new();
                collect_public_targets(root, &mut root_targets, &mut origins, &mut collisions);
                for (path, first, second) in collisions {
                    let mut diagnostic = Diagnostic::error(
                        "AVENGER-RESOLVE-128",
                        "public target path is declared more than once",
                        SourceLabel::new(second, format!("`{path}` collides at this declaration")),
                    )
                    .with_secondary(SourceLabel::new(first, "the first target is declared here"));
                    diagnostic.trace = self.import_trace_for_source(second.source);
                    self.diagnostics.push(diagnostic);
                }
                // This graph-wide map is retained until the compiler moves to
                // chart entrypoint ownership. Cross-chart names intentionally
                // do not collide; each entrypoint carries its own exact map.
                public_targets.extend(root_targets);
            }
        }
        let module_items = self
            .module_index
            .items
            .values()
            .map(|binding| {
                (
                    binding.id.clone(),
                    ResolvedModuleItem {
                        id: binding.id.clone(),
                        exported: binding.exported,
                        declaration: binding.declaration.clone(),
                        category: binding.category,
                        source_name: binding.name.clone(),
                    },
                )
            })
            .collect();
        let item_dependencies = self.build_item_dependency_graph(&files);
        let entrypoints =
            self.resolved_entrypoints(&files, &param_initializer_order, &item_dependencies);
        let authoring_items = AuthoringItemGraph {
            item_order: files
                .iter()
                .map(|(module, file)| (module.clone(), file.item_order.clone()))
                .collect(),
            chart_closures: entrypoints
                .iter()
                .map(|(id, entrypoint)| (id.clone(), entrypoint.reachable_items.clone()))
                .collect(),
        };
        sort_diagnostics(&mut self.diagnostics, &self.project.sources);
        if !self.diagnostics.is_empty() {
            return ResolveAttempt {
                result: Err(ResolveFailure {
                    diagnostics: std::mem::take(&mut self.diagnostics),
                    sources: self.project.sources.clone(),
                }),
            };
        }

        ResolveAttempt {
            result: Ok(ResolvedModuleGraph {
                language_major: LANGUAGE_MAJOR,
                registry_schema_major: self.registry.version.major,
                registry_schema_minor: self.registry.version.minor,
                registry_profile_label: self.registry.profile_label.clone(),
                source_fingerprint: self.project.fingerprint.clone(),
                sources: self.project.sources.clone(),
                source_modules: files,
                items: module_items,
                entrypoints,
                item_dependencies,
                authoring_items,
                charts,
                definitions: self.definitions.clone(),
                params: self.params.clone(),
                stores: self.stores.clone(),
                selections: self.selections.clone(),
                public_targets,
                param_initializer_order,
                param_type_requirements: self.param_type_requirements.clone(),
                catalog_tables: self.resolved_catalog_tables(),
                table_order,
                definition_import_order,
                expansion_source_map: ExpansionSourceMap::default(),
            }),
        }
    }

    fn resolved_entrypoints(
        &self,
        files: &BTreeMap<SourceModuleId, ResolvedModule>,
        param_initializer_order: &[ParamId],
        item_dependencies: &ItemDependencyGraph,
    ) -> BTreeMap<ChartEntrypointId, ResolvedChartEntrypoint> {
        let mut entrypoints = BTreeMap::new();
        for (module_id, file) in files {
            for (index, chart) in file
                .roots
                .iter()
                .enumerate()
                .filter(|(_, declaration)| declaration.keyword == "chart")
            {
                let selector = chart
                    .name
                    .as_ref()
                    .map_or(ChartSelector::Anonymous, |name| {
                        ChartSelector::Named(name.clone())
                    });
                let id = ChartEntrypointId {
                    module: module_id.clone(),
                    selector,
                };
                let Some(item) = self
                    .module_index
                    .item_at
                    .get(&(module_id.clone(), index))
                    .cloned()
                else {
                    continue;
                };
                let owns = |ancestry: &[DeclarationId], declaration: &DeclarationId| {
                    declaration == &chart.id || ancestry.contains(&chart.id)
                };
                let params = self
                    .params
                    .iter()
                    .filter(|(_, param)| owns(&param.owner_ancestry, &param.declaration))
                    .map(|(id, param)| (id.clone(), param.clone()))
                    .collect();
                let stores = self
                    .stores
                    .iter()
                    .filter(|(_, store)| owns(&store.owner_ancestry, &store.declaration))
                    .map(|(id, store)| (id.clone(), store.clone()))
                    .collect();
                let selections = self
                    .selections
                    .iter()
                    .filter(|(_, selection)| {
                        owns(&selection.owner_ancestry, &selection.declaration)
                    })
                    .map(|(id, selection)| (id.clone(), selection.clone()))
                    .collect();
                let mut public_targets = BTreeMap::new();
                let mut origins = BTreeMap::new();
                let mut collisions = Vec::new();
                collect_public_targets(chart, &mut public_targets, &mut origins, &mut collisions);
                let owned_param_order = param_initializer_order
                    .iter()
                    .filter(|param| {
                        self.params
                            .get(*param)
                            .is_some_and(|param| owns(&param.owner_ancestry, &param.declaration))
                    })
                    .cloned()
                    .collect();
                entrypoints.insert(
                    id.clone(),
                    ResolvedChartEntrypoint {
                        id,
                        item: item.clone(),
                        declaration: chart.id.clone(),
                        params,
                        stores,
                        selections,
                        public_targets,
                        param_initializer_order: owned_param_order,
                        reachable_items: std::iter::once(item.clone())
                            .chain(
                                item_dependencies
                                    .transitive_closures
                                    .get(&item)
                                    .into_iter()
                                    .flatten()
                                    .cloned(),
                            )
                            .collect(),
                    },
                );
            }
        }
        entrypoints
    }

    fn build_item_dependency_graph(
        &mut self,
        files: &BTreeMap<SourceModuleId, ResolvedModule>,
    ) -> ItemDependencyGraph {
        let mut edges = Vec::new();
        for (module_id, file) in files {
            for (index, declaration) in file.roots.iter().enumerate() {
                let Some(from) = self
                    .module_index
                    .item_at
                    .get(&(module_id.clone(), index))
                    .cloned()
                else {
                    continue;
                };
                collect_item_dependencies(declaration, &from, &mut edges);
            }
        }
        // Nested tables in one schema/catalog item may refer to one another.
        // Those are dependencies inside a single construction unit, not an
        // item-level cycle.
        edges.retain(|edge| edge.from != edge.to);
        edges.sort_by(|left, right| {
            left.from
                .cmp(&right.from)
                .then(left.to.cmp(&right.to))
                .then(left.site.cmp(&right.site))
        });
        edges.dedup_by(|left, right| {
            left.from == right.from
                && left.to == right.to
                && left.cause == right.cause
                && left.site == right.site
        });

        let mut adjacency = BTreeMap::<ModuleItemId, BTreeSet<ModuleItemId>>::new();
        for item in self.module_index.items.keys() {
            adjacency.entry(item.clone()).or_default();
        }
        for edge in &edges {
            adjacency
                .entry(edge.from.clone())
                .or_default()
                .insert(edge.to.clone());
        }
        if let Err(cycle) = topological_order(&adjacency) {
            let site = cycle
                .windows(2)
                .find_map(|pair| {
                    edges
                        .iter()
                        .find(|edge| edge.from == pair[0] && edge.to == pair[1])
                        .map(|edge| edge.site)
                })
                .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0));
            self.error(
                "AVENGER-RESOLVE-059",
                "module item dependency cycle",
                site,
                format!(
                    "cycle: {}",
                    cycle
                        .iter()
                        .map(|item| {
                            self.module_index
                                .items
                                .get(item)
                                .and_then(|binding| binding.name.as_deref())
                                .map_or_else(
                                    || module_item_display(item),
                                    |name| format!("{}::{name}", item.module.as_str()),
                                )
                        })
                        .collect::<Vec<_>>()
                        .join(" -> ")
                ),
            );
        }
        let transitive_closures = adjacency
            .keys()
            .map(|item| {
                let mut closure = BTreeSet::new();
                collect_item_closure(item, &adjacency, &mut closure);
                closure.remove(item);
                (item.clone(), closure)
            })
            .collect();
        let graph = ItemDependencyGraph {
            edges,
            transitive_closures,
        };
        self.validate_definition_data_isolation(&graph, &adjacency);
        graph
    }

    fn validate_definition_data_isolation(
        &mut self,
        graph: &ItemDependencyGraph,
        adjacency: &BTreeMap<ModuleItemId, BTreeSet<ModuleItemId>>,
    ) {
        let definitions = self.definitions.clone();
        for (item, definition) in definitions {
            if !matches!(definition.kind, DefinitionKind::Mark | DefinitionKind::Tool) {
                continue;
            }
            let Some(data) = graph
                .transitive_closures
                .get(&item)
                .into_iter()
                .flatten()
                .find(|dependency| {
                    self.module_index
                        .items
                        .get(*dependency)
                        .is_some_and(|binding| binding.category == BindingCategory::Data)
                })
                .cloned()
            else {
                continue;
            };
            let path = dependency_path(&item, &data, adjacency);
            let site = path
                .windows(2)
                .find_map(|pair| {
                    graph
                        .edges
                        .iter()
                        .find(|edge| edge.from == pair[0] && edge.to == pair[1])
                        .map(|edge| edge.site)
                })
                .unwrap_or_else(|| {
                    self.module_index
                        .items
                        .get(&item)
                        .and_then(|binding| {
                            self.declarations
                                .values()
                                .find(|info| info.id == binding.declaration)
                        })
                        .map_or_else(|| SourceSpan::empty(SourceId::new(0), 0), |info| info.span)
                });
            self.error(
                "AVENGER-RESOLVE-280",
                match definition.kind {
                    DefinitionKind::Mark => "defined marks cannot capture datasets",
                    DefinitionKind::Tool => "defined tools cannot capture datasets",
                    DefinitionKind::Transform => unreachable!(),
                },
                site,
                format!(
                    "dependency path: {}",
                    path.iter()
                        .map(|item| {
                            self.module_index
                                .items
                                .get(item)
                                .and_then(|binding| binding.name.clone())
                                .unwrap_or_else(|| module_item_display(item))
                        })
                        .collect::<Vec<_>>()
                        .join(" -> ")
                ),
            );
        }
    }

    fn prune_unused_lazy_exports(&mut self, files: &mut BTreeMap<SourceModuleId, ResolvedModule>) {
        let mut used = BTreeSet::new();
        for file in files.values() {
            for root in &file.roots {
                collect_referenced_targets(root, &mut used);
            }
        }
        for file in files.values_mut() {
            for root in &mut file.roots {
                self.prune_declaration_lazy_exports(root, &used);
            }
        }
    }

    fn prune_declaration_lazy_exports(
        &mut self,
        declaration: &mut ResolvedDeclaration,
        used: &BTreeSet<ResolvedTarget>,
    ) {
        if declaration.keyword == "widget"
            && let Some(kind) = declaration.kind.as_deref()
        {
            let key = NativeKindKey::new(NativeKindNamespace::Widget, kind);
            let lazy = self
                .registry
                .entries
                .get(&key)
                .map(|schema| {
                    schema
                        .exports
                        .values()
                        .filter(|export| export.lazy)
                        .map(|export| export.alias.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for alias in lazy {
                let Some(target) = declaration.exports.get(&alias).cloned() else {
                    continue;
                };
                if used.contains(&target) {
                    continue;
                }
                declaration.exports.remove(&alias);
                match target {
                    ResolvedTarget::Param(id) => {
                        self.params.remove(&id);
                        self.param_dependencies.remove(&id);
                    }
                    ResolvedTarget::Store(id) => {
                        self.stores.remove(&id);
                    }
                    ResolvedTarget::Selection(id) => {
                        self.selections.remove(&id);
                    }
                    _ => {}
                }
            }
        }
        for child in &mut declaration.children {
            self.prune_declaration_lazy_exports(child, used);
        }
    }

    fn check_versions(&mut self) {
        for file in self.project.source_modules.values() {
            if file.parsed.ast.version != LANGUAGE_MAJOR {
                self.error(
                    "AVENGER-RESOLVE-001",
                    "unsupported language major",
                    root_span(file),
                    format!(
                        "source requests language {}, but this compiler implements language {LANGUAGE_MAJOR}",
                        file.parsed.ast.version
                    ),
                );
            }
        }
        if self.registry.version.major != LANGUAGE_MAJOR {
            let span = self
                .project
                .source_modules
                .values()
                .next()
                .map(root_span)
                .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0));
            self.error(
                "AVENGER-RESOLVE-002",
                "native authoring registry is incompatible with the language",
                span,
                format!(
                    "registry schema major {} does not match language major {LANGUAGE_MAJOR}",
                    self.registry.version.major
                ),
            );
        }
    }

    fn build_module_index(&mut self) {
        let modules = self
            .project
            .source_modules
            .iter()
            .map(|(id, module)| (id.clone(), module.clone()))
            .collect::<Vec<_>>();

        for (module_id, module) in &modules {
            let mut environment = ModuleBindingEnvironment::default();
            let mut export_index = ModuleExportIndex::default();
            let chart_count = module
                .parsed
                .ast
                .items
                .iter()
                .filter(|item| item.declaration.keyword.as_str() == "chart")
                .count();
            for (index, item) in module.parsed.ast.items.iter().enumerate() {
                let declaration = &item.declaration;
                let Some(category) = module_item_category(declaration) else {
                    continue;
                };
                let name = declaration.name.as_ref().map(ToString::to_string);
                let span = module_item_span(module, index);
                if requires_module_item_name(declaration) && name.is_none() {
                    self.error(
                        "AVENGER-RESOLVE-050",
                        "module item requires a name",
                        span,
                        format!(
                            "`{}` module items must use `as <name>`",
                            declaration.keyword
                        ),
                    );
                }
                if declaration.keyword.as_str() == "chart" {
                    if chart_count > 1 && name.is_none() {
                        self.error(
                            "AVENGER-RESOLVE-051",
                            "every chart in a multi-chart module must be named",
                            span,
                            "add `as <name>` to this chart",
                        );
                    }
                    if item.exported && name.is_none() {
                        self.error(
                            "AVENGER-RESOLVE-052",
                            "an exported chart must be named",
                            span,
                            "add `as <name>` before exporting this chart",
                        );
                    }
                }

                let stable_name = name
                    .as_deref()
                    .map(str::to_owned)
                    .unwrap_or_else(|| "<anonymous>".to_owned());
                let declaration_key = DeclarationKey(semantic_hash(&[
                    "module-item",
                    module_id.as_str(),
                    binding_category_label(category),
                    &stable_name,
                ]));
                let id = ModuleItemId {
                    module: module_id.clone(),
                    declaration: declaration_key,
                };
                let declaration_id = declaration_id(module, &[index]);
                let binding = ItemBinding {
                    id: id.clone(),
                    category,
                    name: name.clone(),
                    exported: item.exported,
                    declaration: declaration_id,
                };
                self.module_index
                    .item_at
                    .insert((module_id.clone(), index), id.clone());
                self.module_index.items.insert(id.clone(), binding);

                if let Some(name) = &name {
                    let local_key = (category, name.clone());
                    let export_id = ModuleExportId {
                        module: ModuleId::Source(module_id.clone()),
                        name: name.clone(),
                        category,
                    };
                    if environment
                        .local
                        .insert(local_key, export_id.clone())
                        .is_some()
                    {
                        self.error(
                            "AVENGER-RESOLVE-053",
                            "duplicate module-local binding",
                            span,
                            format!(
                                "`{name}` is already declared in the {} category",
                                binding_category_label(category)
                            ),
                        );
                    }
                    if let BindingCategory::NativeKind(namespace) = category
                        && self.registry.entries.keys().any(|key| {
                            key.namespace == namespace && key.kind.as_str() == name.as_str()
                        })
                    {
                        self.error(
                            "AVENGER-RESOLVE-054",
                            "module-local definition collides with a built-in kind",
                            span,
                            format!(
                                "`{name}` is already a built-in {} kind",
                                binding_category_label(category)
                            ),
                        );
                    }
                    self.module_index
                        .local_items
                        .insert((module_id.clone(), category, name.clone()), id.clone());
                    if item.exported {
                        if export_index
                            .exports
                            .insert(name.clone(), export_id)
                            .is_some()
                        {
                            self.error(
                                "AVENGER-RESOLVE-055",
                                "duplicate module export name",
                                span,
                                format!(
                                    "`{name}` is exported by more than one module item; export names are category-independent"
                                ),
                            );
                        }
                        self.module_index
                            .source_export_items
                            .insert((module_id.clone(), name.clone()), id.clone());
                    }
                }
            }
            self.module_index
                .environments
                .insert(module_id.clone(), environment);
            self.module_index
                .exports
                .insert(ModuleId::Source(module_id.clone()), export_index);
        }

        for (id, module) in &self.registry.modules {
            let exports = module
                .exports
                .iter()
                .map(|(name, export)| {
                    (
                        name.clone(),
                        ModuleExportId {
                            module: ModuleId::Native(id.clone()),
                            name: name.clone(),
                            category: BindingCategory::NativeKind(export.category),
                        },
                    )
                })
                .collect();
            self.module_index
                .exports
                .insert(ModuleId::Native(id.clone()), ModuleExportIndex { exports });
        }

        let edges = self.project.imports.clone();
        for edge in edges {
            if !self.project.source_modules.contains_key(&edge.importer) {
                continue;
            }
            let mut environment = self
                .module_index
                .environments
                .remove(&edge.importer)
                .unwrap_or_default();
            self.module_index
                .resolved_imports
                .entry(edge.importer.clone())
                .or_default()
                .push(ResolvedImport {
                    source: edge.imported.clone(),
                    specifier: edge.specifier.clone(),
                    clause: edge.clause.clone(),
                });
            match &edge.clause {
                crate::ast::ImportClause::Namespace(local) => {
                    let local = local.to_string();
                    let collides = environment.namespaces.contains_key(&local)
                        || environment.local.keys().any(|(_, name)| name == &local);
                    if collides {
                        self.error(
                            "AVENGER-RESOLVE-056",
                            "module namespace alias collides with another binding",
                            edge.site,
                            format!("`{local}` is already bound in this module"),
                        );
                    } else {
                        environment.namespaces.insert(local, edge.imported.clone());
                    }
                }
                crate::ast::ImportClause::Named(specifiers) => {
                    for specifier in specifiers {
                        let imported = specifier.imported.to_string();
                        let local = specifier.local.to_string();
                        let Some(export) = self
                            .module_index
                            .exports
                            .get(&edge.imported)
                            .and_then(|exports| exports.exports.get(&imported))
                            .cloned()
                        else {
                            let available = self
                                .module_index
                                .exports
                                .get(&edge.imported)
                                .map(|exports| {
                                    exports
                                        .exports
                                        .keys()
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default();
                            self.error(
                                "AVENGER-RESOLVE-057",
                                "import names a missing or private export",
                                edge.site,
                                if available.is_empty() {
                                    format!("`{imported}` is not exported by `{}`", edge.specifier)
                                } else {
                                    format!(
                                        "`{imported}` is not exported by `{}`; available exports: {available}",
                                        edge.specifier
                                    )
                                },
                            );
                            continue;
                        };
                        if environment.namespaces.contains_key(&local) {
                            self.error(
                                "AVENGER-RESOLVE-056",
                                "named import collides with a module namespace alias",
                                edge.site,
                                format!("`{local}` is already a namespace alias"),
                            );
                            continue;
                        }
                        let key = (export.category, local.clone());
                        if environment.local.insert(key, export).is_some() {
                            self.error(
                                "AVENGER-RESOLVE-058",
                                "duplicate imported binding",
                                edge.site,
                                format!(
                                    "`{local}` is already bound in the {} category",
                                    binding_category_label(
                                        environment
                                            .local
                                            .keys()
                                            .find(|(_, name)| name == &local)
                                            .map(|(category, _)| *category)
                                            .unwrap_or(BindingCategory::Data)
                                    )
                                ),
                            );
                        }
                    }
                }
            }
            self.module_index
                .environments
                .insert(edge.importer.clone(), environment);
        }
    }

    fn build_import_bindings(&mut self) {
        for edge in &self.project.imports {
            let importer = &edge.importer;
            let ModuleId::Source(imported) = &edge.imported else {
                continue;
            };
            let bindings = self.imports.entry(importer.clone()).or_default();
            match &edge.clause {
                crate::ast::ImportClause::Named(specifiers) => {
                    for specifier in specifiers {
                        bindings.insert(specifier.local.to_string(), imported.clone());
                    }
                }
                crate::ast::ImportClause::Namespace(local) => {
                    bindings.insert(local.to_string(), imported.clone());
                }
            }
        }
    }

    fn extract_definition_schemas(&mut self) {
        for (file_id, file) in &self.project.source_modules {
            for (root_index, declaration) in module_declarations(file)
                .filter(|(_, declaration)| declaration.keyword.as_str() == "define")
            {
                let Some(kind) = definition_kind(declaration) else {
                    continue;
                };
                let Some(item_id) = self
                    .module_index
                    .item_at
                    .get(&(file_id.clone(), root_index))
                    .cloned()
                else {
                    continue;
                };
                let id = declaration_id(file, &[root_index]);
                let mut slots = BTreeMap::new();
                let mut slot_order = Vec::new();
                let mut channels = BTreeMap::new();
                let mut outputs = BTreeMap::new();
                let mut exports = BTreeMap::new();
                let mut parts = BTreeMap::new();
                let mut interface_names = BTreeMap::<String, &'static str>::new();
                let all_slot_names = declaration
                    .children
                    .iter()
                    .filter(|child| {
                        child.keyword.as_str() == "slot"
                            && child
                                .kind
                                .as_ref()
                                .is_none_or(|kind| kind.as_str() != "channel")
                    })
                    .filter_map(|child| child.name.as_ref().map(ToString::to_string))
                    .collect::<BTreeSet<_>>();
                for child in &declaration.children {
                    match child.keyword.as_str() {
                        "slot" => {
                            let Some(name) = child.name.as_ref() else {
                                continue;
                            };
                            let shape = child
                                .kind
                                .as_ref()
                                .map_or("", |kind| kind.as_str())
                                .to_owned();
                            if shape == "channel" {
                                self.check_definition_interface_name(
                                    &mut interface_names,
                                    name.as_str(),
                                    "channel",
                                    file,
                                );
                                for (property, _) in child.props.iter() {
                                    if property.as_str() != "default" {
                                        self.error(
                                        "AVENGER-RESOLVE-004",
                                        "invalid definition channel slot property",
                                        root_span(file),
                                        format!(
                                            "channel slot `{name}` does not support `{property}:`"
                                        ),
                                    );
                                    }
                                }
                                if !child.children.is_empty() {
                                    self.error(
                                        "AVENGER-RESOLVE-004",
                                        "invalid definition channel slot body",
                                        root_span(file),
                                        format!(
                                            "channel slot `{name}` cannot contain declarations"
                                        ),
                                    );
                                }
                                let physical_channel = child
                                    .props
                                    .get("default")
                                    .and_then(value_atom)
                                    .map(str::to_owned);
                                if child.props.get("default").is_some()
                                    && physical_channel.is_none()
                                {
                                    self.error(
                                    "AVENGER-RESOLVE-004",
                                    "invalid definition channel default",
                                    root_span(file),
                                    format!(
                                        "channel slot `{name}` requires one bare physical channel name"
                                    ),
                                );
                                }
                                channels.insert(
                                    name.to_string(),
                                    DefinitionChannel {
                                        required: physical_channel.is_none(),
                                        physical_channel,
                                    },
                                );
                                continue;
                            }
                            if !matches!(
                                shape.as_str(),
                                "expr"
                                    | "expr_list"
                                    | "literal"
                                    | "number"
                                    | "string"
                                    | "boolean"
                                    | "enum"
                                    | "ref"
                                    | "block"
                                    | "outputs"
                            ) {
                                self.error(
                                    "AVENGER-RESOLVE-004",
                                    "unknown definition slot shape",
                                    root_span(file),
                                    format!("slot `{name}` uses unsupported shape `{shape}`"),
                                );
                            }
                            self.check_definition_interface_name(
                                &mut interface_names,
                                name.as_str(),
                                "slot",
                                file,
                            );
                            let default = child
                                .props
                                .get("default")
                                .map(|value| definition_value(value, &id, &all_slot_names));
                            let enum_values = value_names(child.props.get("values"));
                            let reference_kind = child
                                .props
                                .get("kind")
                                .and_then(value_atom)
                                .map(str::to_owned);
                            let exposes = value_names(child.props.get("exposes"));
                            self.validate_slot_declaration(
                                child,
                                &shape,
                                &enum_values,
                                &slot_order,
                                &all_slot_names,
                                file,
                            );
                            slot_order.push(name.to_string());
                            let slot_schema = DefinitionSlot {
                                required: shape == "outputs" || default.is_none(),
                                shape,
                                default,
                                enum_values,
                                reference_kind,
                                exposes,
                            };
                            if let Some(default) = slot_schema.default.as_ref() {
                                self.validate_definition_value(
                                    default,
                                    &slot_schema,
                                    "default",
                                    root_span(file),
                                );
                            }
                            slots.insert(name.to_string(), slot_schema);
                        }
                        "channel" => {}
                        "output" => {
                            if let Some(name) = child.name.as_ref() {
                                self.check_definition_interface_name(
                                    &mut interface_names,
                                    name.as_str(),
                                    "output",
                                    file,
                                );
                                outputs.insert(
                                    name.to_string(),
                                    child
                                        .props
                                        .get("value")
                                        .map(|value| definition_value(value, &id, &all_slot_names)),
                                );
                            }
                        }
                        "export" => {
                            if let Some(path) = value_path(child.props.get("source")) {
                                let alias = child
                                    .name
                                    .as_ref()
                                    .map(ToString::to_string)
                                    .or_else(|| path.last().cloned());
                                if let Some(alias) = alias {
                                    self.check_definition_interface_name(
                                        &mut interface_names,
                                        &alias,
                                        "export",
                                        file,
                                    );
                                    let target = find_definition_target(declaration, &path);
                                    if target.is_none() {
                                        self.error(
                                            "AVENGER-RESOLVE-005",
                                            "definition export path does not exist",
                                            root_span(file),
                                            format!(
                                                "export `{alias}` cannot resolve `{}`",
                                                path.join(".")
                                            ),
                                        );
                                    }
                                    let target_kind = target
                                        .map(definition_export_kind)
                                        .unwrap_or(DefinitionExportKind::Unknown);
                                    let export = DefinitionExport { path, target_kind };
                                    if exports.insert(alias.clone(), export).is_some() {
                                        self.error(
                                            "AVENGER-RESOLVE-003",
                                            "duplicate definition export",
                                            root_span(file),
                                            format!(
                                                "export alias `{alias}` is declared more than once"
                                            ),
                                        );
                                    }
                                    if target_kind == DefinitionExportKind::Mark {
                                        let path = exports
                                            .get(&alias)
                                            .map(|export| export.path.clone())
                                            .unwrap_or_default();
                                        parts.insert(
                                            alias.clone(),
                                            DefinitionPart {
                                                alias,
                                                declaration_path: path,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        "part" => {
                            if let Some(alias) = child.name.as_ref() {
                                self.check_definition_interface_name(
                                    &mut interface_names,
                                    alias.as_str(),
                                    "part",
                                    file,
                                );
                                parts.insert(
                                    alias.to_string(),
                                    DefinitionPart {
                                        alias: alias.to_string(),
                                        declaration_path: vec![alias.to_string()],
                                    },
                                );
                            }
                        }
                        _ => {}
                    }
                }
                self.definitions.insert(
                    item_id.clone(),
                    DefinitionSchema {
                        item: item_id,
                        declaration: id.clone(),
                        local_seed: DefinitionLocalSeed(semantic_hash(&[
                            "definition-local",
                            file_id.as_str(),
                            id.as_str(),
                        ])),
                        kind,
                        source_name: declaration
                            .name
                            .as_ref()
                            .map_or_else(|| file_id.as_str().to_owned(), ToString::to_string),
                        slot_order,
                        slots,
                        channels,
                        outputs,
                        exports,
                        parts,
                    },
                );
            }
        }
    }

    fn check_definition_interface_name(
        &mut self,
        names: &mut BTreeMap<String, &'static str>,
        name: &str,
        role: &'static str,
        file: &ParsedModule,
    ) {
        if let Some(previous) = names.insert(name.to_owned(), role) {
            self.error(
                "AVENGER-RESOLVE-006",
                "duplicate definition interface name",
                root_span(file),
                format!("`{name}` is declared as both {previous} and {role}"),
            );
        }
    }

    fn validate_definition_templates(&mut self) {
        let definitions = self
            .definitions
            .values()
            .filter_map(|schema| {
                let file = self.project.source_modules.get(&schema.item.module)?;
                let index =
                    self.module_index
                        .item_at
                        .iter()
                        .find_map(|((module, index), item)| {
                            (module == &schema.item.module && item == &schema.item)
                                .then_some(*index)
                        })?;
                let root = &file.parsed.ast.items.get(index)?.declaration;
                Some((file.clone(), root.clone(), index, schema.clone()))
            })
            .collect::<Vec<_>>();

        for (file, root, index, schema) in definitions {
            let mut splice_counts = BTreeMap::<String, usize>::new();
            self.validate_definition_node(&file, &root, &[index], &schema, &mut splice_counts);
            for (name, slot) in &schema.slots {
                let count = splice_counts.get(name).copied().unwrap_or(0);
                if slot.shape == "block" && count != 1 {
                    self.error(
                        "AVENGER-RESOLVE-153",
                        "block slot requires exactly one splice point",
                        root_span(&file),
                        format!(
                            "block slot `{name}` has {count} splice points; expected exactly one"
                        ),
                    );
                } else if slot.shape != "block" && count > 0 {
                    self.error(
                        "AVENGER-RESOLVE-154",
                        "only block slots may be spliced",
                        root_span(&file),
                        format!("`{name}` is a `{}` slot, not a block slot", slot.shape),
                    );
                }
                if slot.shape == "block" {
                    for exposed in &slot.exposes {
                        if find_definition_target(&root, std::slice::from_ref(exposed)).is_none() {
                            self.error(
                                "AVENGER-RESOLVE-155",
                                "block slot exposes an unknown internal handle",
                                root_span(&file),
                                format!("slot `{name}` cannot expose unknown name `{exposed}`"),
                            );
                        }
                    }
                }
                if slot.shape == "outputs" {
                    if schema.kind != DefinitionKind::Transform {
                        self.error(
                            "AVENGER-RESOLVE-166",
                            "outputs slots are transform-only",
                            root_span(&file),
                            format!("slot `{name}` is only valid in `define transform`"),
                        );
                    }
                    let uses = count_definition_value_uses(&root, name);
                    if uses != 1 {
                        self.error(
                            "AVENGER-RESOLVE-167",
                            "outputs slot requires exactly one value use",
                            root_span(&file),
                            format!(
                                "outputs slot `{name}` is used {uses} times; expected exactly once"
                            ),
                        );
                    } else if count_definition_output_projection_uses(&root, name) != 1 {
                        self.error(
                            "AVENGER-RESOLVE-170",
                            "outputs slot is used outside a projection splice",
                            root_span(&file),
                            format!(
                                "use outputs slot `{name}` as a whole `expressions:` value or one whole SQL `SELECT` item"
                            ),
                        );
                    }
                }
            }
            let output_slots = schema
                .slots
                .values()
                .filter(|slot| slot.shape == "outputs")
                .count();
            if output_slots > 1 {
                self.error(
                    "AVENGER-RESOLVE-168",
                    "transform definition has multiple outputs slots",
                    root_span(&file),
                    "a transform definition may declare at most one `slot outputs`",
                );
            }
        }
    }

    fn validate_definition_node(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        path: &[usize],
        schema: &DefinitionSchema,
        splice_counts: &mut BTreeMap<String, usize>,
    ) {
        let span = declaration_span(file, path).unwrap_or_else(|| root_span(file));
        if declaration.props.get("data").is_some() {
            self.error(
                "AVENGER-RESOLVE-156",
                "definitions cannot declare data",
                span,
                "the expanded component inherits data from its instantiation site",
            );
        }
        if declaration.keyword.as_str() == "theme" {
            self.error(
                "AVENGER-RESOLVE-157",
                "definitions cannot declare themes",
                span,
                "themes belong to chart files and are not definition-local state",
            );
        }
        if declaration.keyword.as_str() == "splice"
            && let Some(name) = declaration.name.as_ref()
        {
            *splice_counts.entry(name.to_string()).or_default() += 1;
            if !schema.slots.contains_key(name.as_str()) {
                self.error(
                    "AVENGER-RESOLVE-158",
                    "splice references an unknown definition slot",
                    span,
                    format!("definition has no slot `{name}`"),
                );
            }
        }
        if declaration.keyword.as_str() == "match" {
            self.validate_definition_match(declaration, span, schema);
        }
        for (index, child) in declaration.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            self.validate_definition_node(file, child, &child_path, schema, splice_counts);
        }
    }

    fn validate_definition_match(
        &mut self,
        declaration: &Decl,
        span: SourceSpan,
        schema: &DefinitionSchema,
    ) {
        let Some(name) = declaration.name.as_ref() else {
            return;
        };
        let Some(slot) = schema.slots.get(name.as_str()) else {
            self.error(
                "AVENGER-RESOLVE-159",
                "match target is not a definition slot",
                span,
                format!("definition has no slot `{name}`"),
            );
            return;
        };
        if slot.shape != "enum" {
            self.error(
                "AVENGER-RESOLVE-160",
                "match target must be an enum slot",
                span,
                format!("slot `{name}` has shape `{}`", slot.shape),
            );
            return;
        }
        let mut arms = BTreeSet::new();
        for arm in &declaration.children {
            let Some(arm_name) = arm.name.as_ref().map(ToString::to_string) else {
                continue;
            };
            if !arms.insert(arm_name.clone()) {
                self.error(
                    "AVENGER-RESOLVE-161",
                    "duplicate match arm",
                    span,
                    format!("match `{name}` repeats arm `{arm_name}`"),
                );
            }
            if !slot.enum_values.contains(&arm_name) {
                self.error(
                    "AVENGER-RESOLVE-162",
                    "match arm is outside the enum domain",
                    span,
                    format!("`{arm_name}` is not a value of enum slot `{name}`"),
                );
            }
        }
        let missing = slot
            .enum_values
            .iter()
            .filter(|value| !arms.contains(*value))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            self.error(
                "AVENGER-RESOLVE-163",
                "match does not cover the enum domain",
                span,
                format!("match `{name}` is missing arms: {}", missing.join(", ")),
            );
        }
    }

    fn validate_slot_declaration(
        &mut self,
        slot: &Decl,
        shape: &str,
        enum_values: &[String],
        earlier_slots: &[String],
        all_slots: &BTreeSet<String>,
        file: &ParsedModule,
    ) {
        let span = root_span(file);
        let name = slot.name.as_ref().map_or("<unnamed>", Name::as_str);
        let allowed = match shape {
            "enum" => &["default", "values"][..],
            "ref" => &["default", "kind"][..],
            "block" => &["default", "exposes"][..],
            "outputs" => &[][..],
            _ => &["default"][..],
        };
        for (property, _) in slot.props.iter() {
            if !allowed.contains(&property.as_str()) {
                self.error(
                    "AVENGER-RESOLVE-151",
                    "property is invalid for this definition slot shape",
                    span,
                    format!("slot `{name}` of shape `{shape}` cannot declare `{property}:`"),
                );
            }
        }
        if shape == "enum" {
            if enum_values.is_empty() {
                self.error(
                    "AVENGER-RESOLVE-007",
                    "enum slot requires values",
                    span,
                    format!("slot `{name}` must declare a non-empty `values:` array"),
                );
            }
            let unique = enum_values.iter().collect::<BTreeSet<_>>();
            if unique.len() != enum_values.len() {
                self.error(
                    "AVENGER-RESOLVE-008",
                    "enum slot values must be unique",
                    span,
                    format!("slot `{name}` repeats an enum value"),
                );
            }
            if let Some(default) = slot.props.get("default")
                && !value_atom(default).is_some_and(|default| {
                    enum_values.iter().any(|candidate| candidate == default)
                        || earlier_slots.iter().any(|slot| slot == default)
                })
            {
                self.error(
                    "AVENGER-RESOLVE-009",
                    "enum slot default is not a declared value",
                    span,
                    format!("slot `{name}` must default to one of its `values:` atoms"),
                );
            }
        }
        if shape == "ref"
            && !slot
                .props
                .get("kind")
                .and_then(value_atom)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "mark" | "param" | "selection" | "store" | "tool" | "widget" | "resource"
                    )
                })
        {
            self.error(
                "AVENGER-RESOLVE-018",
                "reference slot requires a valid target kind",
                span,
                format!("slot `{name}` has a missing or unsupported `kind:`"),
            );
        }
        if shape == "block"
            && let Some(exposes) = slot.props.get("exposes")
        {
            let names = value_names(Some(exposes));
            let valid_shape = matches!(exposes, Value::Array(values)
                if values.iter().all(|value| matches!(value, Value::Atom(_))));
            if !valid_shape || names.iter().collect::<BTreeSet<_>>().len() != names.len() {
                self.error(
                    "AVENGER-RESOLVE-152",
                    "block-slot exposes list is invalid",
                    span,
                    format!("slot `{name}` requires a duplicate-free array of internal names"),
                );
            }
        }
        let Some(default) = slot.props.get("default") else {
            return;
        };
        let mut dependencies = BTreeSet::new();
        collect_definition_slot_dependencies(default, all_slots, &mut dependencies);
        for dependency in dependencies {
            if !earlier_slots.contains(&dependency) {
                self.error(
                    "AVENGER-RESOLVE-019",
                    "slot default references a later slot",
                    span,
                    format!(
                        "slot `{name}` default cannot read `{dependency}` before it is declared"
                    ),
                );
            }
        }
    }

    fn predeclare_project(&mut self) {
        for (file_id, file) in &self.project.source_modules {
            let file_scope = self.new_scope(None, format!("file:{}", file_id.as_str()));
            self.scopes[file_scope.0].module = Some(file_id.clone());
            for (index, declaration) in module_declarations(file) {
                self.predeclare_declaration(file, declaration, vec![index], file_scope, Vec::new());
            }
        }
    }

    fn build_table_dependencies(&mut self) {
        for (module_id, file) in &self.project.source_modules {
            for (index, declaration) in module_declarations(file) {
                let Some(item) = self
                    .module_index
                    .item_at
                    .get(&(module_id.clone(), index))
                    .cloned()
                else {
                    continue;
                };
                self.collect_relation_names(file, declaration, &[index], &[], &item);
            }
        }
        let entries = self
            .relation_declarations
            .iter()
            .map(|(relation, id)| (relation.clone(), id.clone()))
            .collect::<Vec<_>>();
        for (relation, id) in entries {
            self.table_dependencies.entry(id.clone()).or_default();
            let relations = {
                let Some((_, declaration)) = self.declaration_source(&id) else {
                    continue;
                };
                let Some(Value::Query(query)) = declaration.props.get("sql") else {
                    continue;
                };
                relation_paths(query.ast())
            };
            let prefix = relation
                .nested_path
                .split_last()
                .map_or(&[][..], |(_, prefix)| prefix);
            for authored in relations {
                let mut candidates = Vec::new();
                if authored.len() == 1 {
                    let mut relative = vec![
                        self.module_index
                            .items
                            .get(&relation.defining_item)
                            .and_then(|item| item.name.clone())
                            .unwrap_or_default(),
                    ];
                    relative.extend(prefix.iter().cloned());
                    relative.extend(authored.iter().cloned());
                    candidates.push(relative);
                }
                candidates.push(authored.clone());
                let dependency = candidates.into_iter().find_map(|path| {
                    self.resolve_relation_path(
                        &relation.defining_item.module,
                        &path,
                        false,
                        self.declarations
                            .values()
                            .find(|info| info.id == id)
                            .map_or_else(
                                || SourceSpan::empty(SourceId::new(0), 0),
                                |info| info.span,
                            ),
                    )
                    .and_then(|target| match target {
                        ResolvedRelationTarget::Relation(id) => {
                            self.relation_declarations.get(&id).cloned()
                        }
                        ResolvedRelationTarget::Input => None,
                    })
                });
                if let Some(dependency) = dependency {
                    self.table_dependencies
                        .entry(id.clone())
                        .or_default()
                        .insert(dependency);
                }
            }
        }
    }

    fn collect_relation_names(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        path: &[usize],
        prefix: &[String],
        item: &ModuleItemId,
    ) {
        let mut nested_prefix = prefix.to_vec();
        if matches!(declaration.keyword.as_str(), "catalog" | "schema")
            && let Some(name) = declaration.name.as_ref()
        {
            nested_prefix.push(name.to_string());
        }
        if declaration.keyword.as_str() == "table"
            && let Some(name) = declaration.name.as_ref()
        {
            let mut table_path = nested_prefix.clone();
            table_path.push(name.to_string());
            let id = declaration_id(file, path);
            let relation = ResolvedRelationId {
                defining_item: item.clone(),
                nested_path: table_path.iter().skip(1).cloned().collect(),
            };
            if self
                .relation_names
                .insert((file.id.clone(), table_path.clone()), relation.clone())
                .is_some()
            {
                self.error(
                    "AVENGER-RESOLVE-102",
                    "duplicate catalog table path",
                    declaration_span(file, path).unwrap_or_else(|| root_span(file)),
                    format!(
                        "table `{}` is declared more than once",
                        table_path.join(".")
                    ),
                );
            }
            self.relation_declarations.insert(relation, id);
        }
        for (index, child) in declaration.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            self.collect_relation_names(file, child, &child_path, &nested_prefix, item);
        }
    }

    fn resolve_relation_path(
        &mut self,
        module: &SourceModuleId,
        path: &[String],
        allow_input: bool,
        span: SourceSpan,
    ) -> Option<ResolvedRelationTarget> {
        let first = path.first()?;
        if first == "input" {
            if allow_input && path.len() == 1 {
                return Some(ResolvedRelationTarget::Input);
            }
            self.error(
                "AVENGER-DATA-100",
                "`input` is only available inside transform definitions",
                span,
                "`input` must be the complete relation name in a transform-definition query",
            );
            return None;
        }

        let environment = self
            .module_index
            .environments
            .get(module)
            .cloned()
            .unwrap_or_default();
        let (item, nested_path) = if let Some(imported_module) = environment.namespaces.get(first) {
            let Some(member) = path.get(1) else {
                self.error(
                    "AVENGER-DATA-101",
                    "module namespace is not a relation",
                    span,
                    format!("select an exported dataset from `{first}`"),
                );
                return None;
            };
            let Some(export) = self
                .module_index
                .exports
                .get(imported_module)
                .and_then(|exports| exports.exports.get(member))
                .cloned()
            else {
                self.error(
                    "AVENGER-DATA-102",
                    "module-qualified relation is not exported",
                    span,
                    format!("`{first}.{member}` does not name an exported dataset"),
                );
                return None;
            };
            if export.category != BindingCategory::Data {
                self.error(
                    "AVENGER-DATA-103",
                    "module member is not a dataset",
                    span,
                    format!("`{first}.{member}` is in the wrong binding category"),
                );
                return None;
            }
            let Some(item) = self.source_item_for_export(&export) else {
                self.error(
                    "AVENGER-DATA-104",
                    "native modules cannot export dataset values",
                    span,
                    format!("`{first}.{member}` has no source dataset identity"),
                );
                return None;
            };
            (item, path[2..].to_vec())
        } else if let Some(export) = environment
            .local
            .get(&(BindingCategory::Data, first.clone()))
            .cloned()
        {
            let Some(item) = self.source_item_for_export(&export).or_else(|| {
                self.module_index
                    .local_items
                    .get(&(module.clone(), BindingCategory::Data, first.clone()))
                    .cloned()
            }) else {
                self.error(
                    "AVENGER-DATA-105",
                    "dataset binding has no source identity",
                    span,
                    format!("`{first}` cannot be planned as a relation"),
                );
                return None;
            };
            (item, path[1..].to_vec())
        } else {
            let ambient = self
                .project
                .ambient_data_modules
                .iter()
                .filter_map(|ambient_module| {
                    self.module_index
                        .exports
                        .get(&ModuleId::Source(ambient_module.clone()))
                        .and_then(|exports| exports.exports.get(first))
                        .filter(|export| export.category == BindingCategory::Data)
                        .and_then(|export| self.source_item_for_export(export))
                })
                .collect::<Vec<_>>();
            match ambient.as_slice() {
                [item] => (item.clone(), path[1..].to_vec()),
                [] => {
                    self.error(
                        "AVENGER-DATA-106",
                        "unknown relation",
                        span,
                        format!(
                            "`{}` is not a local, imported, standard, or ambient dataset",
                            path.join(".")
                        ),
                    );
                    return None;
                }
                _ => {
                    self.error(
                        "AVENGER-DATA-107",
                        "ambient relation is ambiguous",
                        span,
                        format!("`{first}` is exported by more than one ambient module"),
                    );
                    return None;
                }
            }
        };

        let relation = ResolvedRelationId {
            defining_item: item,
            nested_path,
        };
        if !self.relation_declarations.contains_key(&relation)
            && !self.provider_discovered_relation(&relation)
        {
            self.error(
                "AVENGER-DATA-108",
                "dataset path does not name a table",
                span,
                format!("`{}` does not resolve to a concrete table", path.join(".")),
            );
            return None;
        }
        Some(ResolvedRelationTarget::Relation(relation))
    }

    fn provider_discovered_relation(&self, relation: &ResolvedRelationId) -> bool {
        if relation.nested_path.len() < 2 {
            return false;
        }
        let declaration = self
            .module_index
            .items
            .get(&relation.defining_item)
            .and_then(|item| self.declaration_source(&item.declaration))
            .map(|(_, declaration)| declaration);
        declaration.is_some_and(|declaration| {
            declaration.keyword.as_str() == "catalog"
                && !matches!(
                    declaration.kind.as_ref().map(|kind| kind.as_str()),
                    None | Some("memory" | "schemas")
                )
        })
    }

    fn source_item_for_export(&self, export: &ModuleExportId) -> Option<ModuleItemId> {
        let ModuleId::Source(module) = &export.module else {
            return None;
        };
        self.module_index
            .source_export_items
            .get(&(module.clone(), export.name.clone()))
            .cloned()
            .or_else(|| {
                self.module_index
                    .local_items
                    .get(&(module.clone(), export.category, export.name.clone()))
                    .cloned()
            })
    }

    fn declaration_source(&self, id: &DeclarationId) -> Option<(&ParsedModule, &Decl)> {
        let ((file_id, path), _) = self.declarations.iter().find(|(_, info)| &info.id == id)?;
        let file = self.project.source_modules.get(file_id)?;
        let declaration = declaration_at(file, path)?;
        Some((file, declaration))
    }

    fn predeclare_declaration(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        path: Vec<usize>,
        containing_scope: ScopeId,
        ancestry: Vec<DeclarationId>,
    ) {
        let id = declaration_id(file, &path);
        let span = declaration_span(file, &path).unwrap_or_else(|| root_span(file));
        let runtime_target = runtime_target(declaration, &id);
        let owns_scope = owns_lexical_scope(declaration);
        let mut child_ancestry = ancestry.clone();
        if is_instance_boundary(declaration) {
            child_ancestry.push(id.clone());
        }
        let child_scope = owns_scope.then(|| {
            self.new_scope(
                Some(containing_scope),
                format!("{}:{}", file.id.as_str(), path_text(&path)),
            )
        });
        if let Some(scope) = child_scope {
            self.scopes[scope.0].owner = Some(id.clone());
        }
        if let Some(scope) = child_scope
            && declaration.keyword.as_str() == "on"
        {
            self.scopes[scope.0].event_binding = true;
            self.scopes[scope.0].event_has_between = declaration.props.get("between").is_some();
        }

        self.declarations.insert(
            (file.id.clone(), path.clone()),
            DeclInfo {
                id: id.clone(),
                span,
                containing_scope,
                child_scope,
                ancestry: ancestry.clone(),
                runtime_target: runtime_target.clone(),
            },
        );
        if let Some(name) = declaration.name.as_ref() {
            let binding_scope = if declaration.keyword.as_str() == "view" {
                child_scope.unwrap_or(containing_scope)
            } else {
                containing_scope
            };
            self.predeclare_name(
                file,
                declaration,
                binding_scope,
                name.as_str(),
                &id,
                runtime_target.clone(),
                &ancestry,
                span,
            );
        }
        if is_structural(declaration) {
            self.instances.entry(id.clone()).or_default().child_scope = child_scope;
        }

        let nested_scope = child_scope.unwrap_or(containing_scope);
        for (index, child) in declaration.children.iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(index);
            self.predeclare_declaration(
                file,
                child,
                child_path,
                nested_scope,
                child_ancestry.clone(),
            );
        }
        for (block_ordinal, body) in overlay_mark_blocks(declaration).into_iter().enumerate() {
            let block_scope = self.new_scope(
                Some(nested_scope),
                format!(
                    "{}:{}:legend-overlay:{block_ordinal}",
                    file.id.as_str(),
                    path_text(&path)
                ),
            );
            for (index, child) in body.children.iter().enumerate() {
                let mut child_path = path.clone();
                child_path.extend([MARK_BLOCK_PATH_SEGMENT, block_ordinal, index]);
                self.predeclare_declaration(
                    file,
                    child,
                    child_path,
                    block_scope,
                    child_ancestry.clone(),
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn predeclare_name(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        scope: ScopeId,
        name: &str,
        id: &DeclarationId,
        runtime_target: Option<ResolvedTarget>,
        ancestry: &[DeclarationId],
        span: SourceSpan,
    ) {
        match declaration.keyword.as_str() {
            "catalog" => {
                let kind = declaration
                    .kind
                    .as_ref()
                    .map(|kind| kind.as_str())
                    .unwrap_or("");
                if matches!(kind, "schemas" | "memory") {
                    self.validate_core_property_names(declaration, &[], span);
                    if declaration
                        .children
                        .iter()
                        .any(|child| child.keyword.as_str() != "schema")
                    {
                        self.error(
                            "AVENGER-RESOLVE-170",
                            "invalid inline catalog child",
                            span,
                            "inline catalogs contain only `schema` declarations",
                        );
                    }
                }
            }
            "schema" => {
                let kind = declaration
                    .kind
                    .as_ref()
                    .map(|kind| kind.as_str())
                    .unwrap_or("");
                match kind {
                    "tables" => {
                        self.validate_core_property_names(declaration, &[], span);
                        if declaration
                            .children
                            .iter()
                            .any(|child| child.keyword.as_str() != "table")
                        {
                            self.error(
                                "AVENGER-RESOLVE-171",
                                "invalid inline schema child",
                                span,
                                "`schema tables` contains only `table` declarations",
                            );
                        }
                    }
                    "namespace" => {
                        self.validate_core_property_names(declaration, &["path"], span);
                        if !matches!(declaration.props.get("path"), Some(Value::Array(values)) if !values.is_empty() && values.iter().all(|value| matches!(value, Value::Str(_) | Value::Atom(_))))
                        {
                            self.error(
                                "AVENGER-RESOLVE-172",
                                "incomplete provider namespace",
                                span,
                                "`schema namespace` requires a non-empty `path:` array",
                            );
                        }
                        if !declaration.children.is_empty() {
                            self.error(
                                "AVENGER-RESOLVE-173",
                                "provider namespaces do not contain declarations",
                                span,
                                "tables are discovered by the catalog provider",
                            );
                        }
                    }
                    _ => {}
                }
            }
            "table" => {
                let kind = declaration
                    .kind
                    .as_ref()
                    .map(|kind| kind.as_str())
                    .unwrap_or("");
                let (allowed, required): (&[&str], Option<&str>) = match kind {
                    "sql" => (&["sql", "materialize"], Some("sql")),
                    "inline" => (&["values", "materialize"], Some("values")),
                    "csv" | "json" | "parquet" | "arrow" | "ipc" => {
                        (&["path", "options", "materialize"], Some("path"))
                    }
                    "delta" => (&["uri", "options", "materialize"], Some("uri")),
                    // Custom table providers own their option surface.
                    _ => (&[], None),
                };
                if required.is_some() || !allowed.is_empty() {
                    self.validate_core_property_names(declaration, allowed, span);
                }
                if let Some(required) = required
                    && declaration.props.get(required).is_none()
                {
                    self.error(
                        "AVENGER-RESOLVE-174",
                        "incomplete table declaration",
                        span,
                        format!("`table {kind}` requires `{required}:`"),
                    );
                }
                if declaration.props.get("materialize").is_some_and(|value| {
                    !value_atom(value).is_some_and(|value| matches!(value, "logical" | "session"))
                }) {
                    self.error(
                        "AVENGER-RESOLVE-175",
                        "invalid table materialization mode",
                        span,
                        "`materialize:` must be `logical` or `session`",
                    );
                }
                for param in declaration
                    .children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "param")
                {
                    if param.name.as_ref().is_some_and(|name| {
                        matches!(name.as_str(), "table" | "sql" | "url" | "values")
                    }) {
                        self.error(
                            "AVENGER-RESOLVE-176",
                            "reserved table parameter name",
                            span,
                            "table params cannot be named `table`, `sql`, `url`, or `values`",
                        );
                    }
                }
            }
            "param" => {
                let param_id = ParamId(semantic_hash(&[
                    "param-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                self.insert_state_symbol(scope, name, StateSymbol::Param(param_id.clone()), span);
                self.set_runtime_target(id, ResolvedTarget::Param(param_id));
            }
            "store" => {
                let store_id = StoreId(semantic_hash(&[
                    "store-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                self.insert_state_symbol(scope, name, StateSymbol::Store(store_id.clone()), span);
                self.set_runtime_target(id, ResolvedTarget::Store(store_id));
            }
            "selection" => {
                let selection_id = SelectionId(semantic_hash(&[
                    "selection-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                self.insert_state_symbol(
                    scope,
                    name,
                    StateSymbol::Selection(selection_id.clone()),
                    span,
                );
                self.set_runtime_target(id, ResolvedTarget::Selection(selection_id));
            }
            "on" => {
                let event_id = match runtime_target {
                    Some(ResolvedTarget::Event(event)) => event,
                    _ => EventId(semantic_hash(&["event", id.as_str()])),
                };
                if self.scopes[scope.0]
                    .events
                    .insert(name.to_owned(), event_id)
                    .is_some()
                {
                    self.error(
                        "AVENGER-RESOLVE-012",
                        "duplicate event binding",
                        span,
                        format!("event binder `{name}` is already declared in this scope"),
                    );
                }
            }
            "transform" => {
                if !self.scopes[scope.0].transforms.insert(name.to_owned()) {
                    self.error(
                        "AVENGER-RESOLVE-014",
                        "duplicate transform alias",
                        span,
                        format!("transform alias `{name}` is declared more than once"),
                    );
                }
            }
            "slot" | "channel" => {
                let Some(definition) = self
                    .definitions
                    .values()
                    .find(|schema| ancestry.contains(&schema.declaration))
                    .map(|schema| schema.declaration.clone())
                else {
                    return;
                };
                let target = if declaration.keyword.as_str() == "channel"
                    || declaration
                        .kind
                        .as_ref()
                        .is_some_and(|kind| kind.as_str() == "channel")
                {
                    ResolvedTarget::DefinitionChannel {
                        definition,
                        name: name.to_owned(),
                    }
                } else {
                    ResolvedTarget::DefinitionSlot {
                        definition,
                        name: name.to_owned(),
                    }
                };
                self.scopes[scope.0]
                    .definition_arguments
                    .insert(name.to_owned(), target);
            }
            _ if is_structural(declaration) => {
                if self.scopes[scope.0]
                    .structural
                    .insert(name.to_owned(), id.clone())
                    .is_some()
                {
                    self.error(
                        "AVENGER-RESOLVE-013",
                        "duplicate structural name",
                        span,
                        format!("`{name}` is already declared in this scope"),
                    );
                }
                let _ = runtime_target;
                self.instances.entry(id.clone()).or_default();
            }
            _ => {}
        }
    }

    fn set_runtime_target(&mut self, id: &DeclarationId, target: ResolvedTarget) {
        if let Some(info) = self.declarations.values_mut().find(|info| &info.id == id) {
            info.runtime_target = Some(target);
        }
    }

    fn insert_state_symbol(
        &mut self,
        scope: ScopeId,
        name: &str,
        symbol: StateSymbol,
        span: SourceSpan,
    ) {
        if name == "cursor" {
            self.error(
                "AVENGER-RESOLVE-011",
                "state parameter name `cursor` is reserved",
                span,
                "`cursor` is reserved for the write-only cursor effect",
            );
            return;
        }
        if self.scopes[scope.0].state_symbols.contains_key(name) {
            self.error(
                "AVENGER-RESOLVE-010",
                "duplicate state parameter binding",
                span,
                format!(
                    "`{name}` is already bound by a scalar, store, or selection param in this scope"
                ),
            );
        } else {
            self.scopes[scope.0]
                .state_symbols
                .insert(name.to_owned(), symbol);
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>, label: String) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        let module = parent.and_then(|parent| self.scopes[parent.0].module.clone());
        self.scopes.push(Scope {
            parent,
            module,
            label,
            ..Scope::default()
        });
        id
    }

    fn finish_instance_interfaces(&mut self) {
        // Native parts and generated exports are available from predeclared
        // instances, independently of declaration source order.
        let keys = self.declarations.keys().cloned().collect::<Vec<_>>();
        for (file_id, path) in keys {
            let Some(file) = self.project.source_modules.get(&file_id) else {
                continue;
            };
            let Some(declaration) = declaration_at(file, &path) else {
                continue;
            };
            let Some(info) = self
                .declarations
                .get(&(file_id.clone(), path.clone()))
                .cloned()
            else {
                continue;
            };
            let coordinate = coordinate_at_path(file, &path);
            let kind_binding = self.kind_binding(
                file,
                declaration,
                coordinate.as_deref(),
                inside_definition(file, &path),
            );
            if let Some(schema) =
                self.native_schema(declaration, coordinate.as_deref(), kind_binding.as_ref())
            {
                self.install_native_interface(file, declaration, &info, &schema);
            }
            if let Some(schema) = declaration
                .kind
                .as_ref()
                .and_then(|kind| {
                    self.imported_definition(
                        file,
                        kind.as_str(),
                        declaration_kind_category(declaration)?,
                    )
                })
                .cloned()
            {
                self.install_definition_interface(&info, &schema);
            }
            let mut duplicate_exports = Vec::new();
            if let Some(interface) = self.instances.get_mut(&info.id) {
                for (index, child) in declaration.children.iter().enumerate() {
                    if child.keyword.as_str() != "export" {
                        continue;
                    }
                    if let Some(source) = value_path(child.props.get("source")) {
                        let alias = child
                            .name
                            .as_ref()
                            .map(ToString::to_string)
                            .or_else(|| source.last().cloned());
                        if let Some(alias) = alias {
                            match interface.pending_exports.entry(alias) {
                                Entry::Occupied(entry) => {
                                    let mut child_path = path.clone();
                                    child_path.push(index);
                                    duplicate_exports.push((
                                        entry.key().clone(),
                                        declaration_span(file, &child_path)
                                            .unwrap_or_else(|| root_span(file)),
                                    ));
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(source);
                                }
                            }
                        }
                    }
                }
            }
            for (alias, span) in duplicate_exports {
                self.error(
                    "AVENGER-RESOLVE-130",
                    "explicit export alias collides with the component interface",
                    span,
                    format!("`{alias}` is exported more than once"),
                );
            }
        }
        self.install_lexical_interfaces();
        self.resolve_pending_exports();
        self.install_component_parts();
    }

    fn install_lexical_interfaces(&mut self) {
        let instances = self
            .declarations
            .iter()
            .filter_map(|((file_id, path), info)| {
                self.instances.contains_key(&info.id).then_some((
                    file_id.clone(),
                    path.clone(),
                    info.id.clone(),
                ))
            })
            .collect::<Vec<_>>();
        for (file_id, path, instance) in instances {
            let Some(file) = self.project.source_modules.get(&file_id) else {
                continue;
            };
            let Some(declaration) = declaration_at(file, &path) else {
                continue;
            };
            if inside_definition(file, &path) {
                continue;
            }
            if declaration.keyword.as_str() == "tool"
                && declaration
                    .kind
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == "behavior")
            {
                continue;
            }
            let mut exports = Vec::new();
            for (index, child) in declaration.children.iter().enumerate() {
                let mut child_path = path.clone();
                child_path.push(index);
                self.collect_lexical_exports(file, child, &child_path, false, &mut exports);
            }
            for (alias, target, span) in exports {
                let collision = self.instances.get(&instance).is_some_and(|interface| {
                    interface.exports.contains_key(&alias) || interface.parts.contains_key(&alias)
                });
                if collision {
                    let mut diagnostic = Diagnostic::error(
                        "AVENGER-RESOLVE-129",
                        "component interface name collision",
                        SourceLabel::new(
                            span,
                            format!("`{alias}` collides with another export or part"),
                        ),
                    );
                    diagnostic.trace = self.import_trace_for_source(span.source);
                    self.diagnostics.push(diagnostic);
                } else if let Some(interface) = self.instances.get_mut(&instance) {
                    // Ordinary lexical children participate in qualified
                    // traversal, but only schema or explicit exports are
                    // published as component interface exports.
                    interface.exports.insert(alias, target);
                }
            }
        }
    }

    fn collect_lexical_exports(
        &self,
        file: &ParsedModule,
        declaration: &Decl,
        path: &[usize],
        inside_private: bool,
        output: &mut Vec<(String, ResolvedTarget, SourceSpan)>,
    ) {
        let explicitly_public = declaration.visibility == Visibility::Public;
        let hidden =
            declaration.visibility == Visibility::Private || (inside_private && !explicitly_public);
        if !hidden
            && declaration.keyword.as_str() != "view"
            && declaration.keyword.as_str() != "on"
            && declaration.keyword.as_str() != "transform"
            && let Some(name) = declaration.name.as_ref()
            && let Some(info) = self.declarations.get(&(file.id.clone(), path.to_vec()))
            && let Some(target) = info.runtime_target.clone()
        {
            output.push((name.to_string(), target, info.span));
        }
        let descend_private =
            declaration.visibility == Visibility::Private || (inside_private && !explicitly_public);
        if descend_private {
            for (index, child) in declaration.children.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(index);
                self.collect_lexical_exports(file, child, &child_path, true, output);
            }
        }
    }

    fn install_component_parts(&mut self) {
        let declarations = self
            .declarations
            .iter()
            .filter_map(|((file_id, path), info)| {
                let file = self.project.source_modules.get(file_id)?;
                let declaration = declaration_at(file, path)?;
                let component_kind = declaration
                    .props
                    .get("component_kind")
                    .and_then(value_atom)?
                    .to_owned();
                Some((info.id.clone(), component_kind))
            })
            .collect::<Vec<_>>();
        for (id, component_kind) in declarations {
            let Some(interface) = self.instances.get_mut(&id) else {
                continue;
            };
            let exports = interface
                .published_exports
                .iter()
                .filter_map(|alias| {
                    interface
                        .exports
                        .get(alias)
                        .cloned()
                        .map(|target| (alias.clone(), target))
                })
                .collect::<Vec<_>>();
            for (alias, target) in exports {
                if matches!(
                    target,
                    ResolvedTarget::Mark(_)
                        | ResolvedTarget::Part { .. }
                        | ResolvedTarget::DefinitionStructural {
                            kind: DefinitionExportKind::Mark,
                            ..
                        }
                ) {
                    interface.parts.insert(
                        alias.clone(),
                        ResolvedPart {
                            source_alias: alias.clone(),
                            runtime_kind: component_kind.clone(),
                            runtime_alias: Some(alias),
                            targetable: true,
                            declaration: id.clone(),
                        },
                    );
                }
            }
        }
    }

    fn install_native_interface(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        info: &DeclInfo,
        schema: &KindSchema,
    ) {
        let Some(interface) = self.instances.get_mut(&info.id) else {
            return;
        };
        for part in schema.parts.values() {
            interface.parts.insert(
                part.alias.clone(),
                ResolvedPart {
                    source_alias: part.alias.clone(),
                    runtime_kind: part.runtime_kind.clone(),
                    runtime_alias: part.runtime_alias.clone(),
                    targetable: part.targetable,
                    declaration: info.id.clone(),
                },
            );
        }
        let export_entries = schema
            .exports
            .values()
            .map(|export| {
                (
                    export.alias.clone(),
                    export.value_kind.clone(),
                    export.binding_property.clone(),
                    export.default_property.clone(),
                )
            })
            .collect::<Vec<_>>();
        let _ = interface;
        for (alias, value_kind, binding_property, default_property) in export_entries {
            if let Some(property) = binding_property
                && let Some(authored) = declaration.props.get(&property)
            {
                if let Some(target) = self.resolve_existing_export(
                    info.containing_scope,
                    authored,
                    &value_kind,
                    info.span,
                ) {
                    if let Some(expected) = export_physical_type(declaration, &value_kind) {
                        self.param_type_requirements.push(ParamTypeRequirement {
                            target: target.clone(),
                            expected,
                            declaration: info.id.clone(),
                            span: info.span,
                            role: format!("export `{alias}`"),
                        });
                    }
                    self.instances
                        .get_mut(&info.id)
                        .expect("instance exists")
                        .exports
                        .insert(alias.clone(), target);
                    self.instances
                        .get_mut(&info.id)
                        .expect("instance exists")
                        .published_exports
                        .insert(alias);
                }
                continue;
            }
            let target = self.generated_state_target(
                file,
                declaration,
                info,
                &alias,
                &value_kind,
                default_property.as_deref(),
            );
            if let Some(target) = target {
                self.instances
                    .get_mut(&info.id)
                    .expect("instance exists")
                    .exports
                    .insert(alias.clone(), target);
                self.instances
                    .get_mut(&info.id)
                    .expect("instance exists")
                    .published_exports
                    .insert(alias);
            }
        }
    }

    fn resolve_existing_export(
        &mut self,
        scope: ScopeId,
        authored: &Value,
        value_kind: &str,
        span: SourceSpan,
    ) -> Option<ResolvedTarget> {
        if value_kind.starts_with("param<") {
            let Value::Binding { path, .. } = authored else {
                return None;
            };
            let path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
            return self.resolve_binding_path(scope, &path, Some(BindingKind::Param), span, false);
        }
        if value_kind == "store" || value_kind.starts_with("store<") {
            let Value::Binding { path, .. } = authored else {
                return None;
            };
            let path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
            return self.resolve_binding_path(scope, &path, Some(BindingKind::Store), span, false);
        }
        if value_kind == "selection" || value_kind.starts_with("selection<") {
            return match authored {
                Value::Ref { kind, path } => self
                    .resolve_reference(scope, *kind, path, span)
                    .map(|reference| reference.target),
                Value::Atom(name) => {
                    let path = vec![name.to_string()];
                    self.resolve_typed_reference_path(scope, &path, RefKind::Selection, span)
                }
                _ => None,
            };
        }
        None
    }

    fn install_definition_interface(&mut self, info: &DeclInfo, schema: &DefinitionSchema) {
        let Some(interface) = self.instances.get_mut(&info.id) else {
            return;
        };
        for (alias, export) in &schema.exports {
            let target = match export.target_kind {
                DefinitionExportKind::Param => ResolvedTarget::DefinitionParam {
                    instance: info.id.clone(),
                    definition: schema.declaration.clone(),
                    alias: alias.clone(),
                },
                DefinitionExportKind::Store => ResolvedTarget::DefinitionStore {
                    instance: info.id.clone(),
                    definition: schema.declaration.clone(),
                    alias: alias.clone(),
                },
                DefinitionExportKind::Selection => ResolvedTarget::DefinitionSelection {
                    instance: info.id.clone(),
                    definition: schema.declaration.clone(),
                    alias: alias.clone(),
                },
                kind => ResolvedTarget::DefinitionStructural {
                    instance: info.id.clone(),
                    definition: schema.declaration.clone(),
                    alias: alias.clone(),
                    kind,
                },
            };
            interface.exports.insert(alias.clone(), target);
            interface.published_exports.insert(alias.clone());
        }
        for (alias, part) in &schema.parts {
            interface.parts.insert(
                alias.clone(),
                ResolvedPart {
                    source_alias: part.alias.clone(),
                    runtime_kind: schema.source_name.clone(),
                    runtime_alias: Some(alias.clone()),
                    targetable: true,
                    declaration: info.id.clone(),
                },
            );
        }
    }

    fn generated_state_target(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        info: &DeclInfo,
        alias: &str,
        value_kind: &str,
        default_property: Option<&str>,
    ) -> Option<ResolvedTarget> {
        let origin = GeneratedStateOrigin {
            declaration: info.id.clone(),
            export_role: alias.to_owned(),
        };
        let (migration_key, definition_local_seed) =
            self.state_identity(file, info, &format!("generated:{alias}"));
        if let Some(inner) = value_kind
            .strip_prefix("param<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let data_type = if inner == "item_scalar" {
                infer_widget_item_type(declaration)
            } else {
                parse_type_text(inner)
            };
            let Some(data_type) = data_type else {
                self.error(
                    "AVENGER-RESOLVE-031",
                    "cannot determine generated param type",
                    info.span,
                    format!("schema export `{alias}` declares `{value_kind}`"),
                );
                return None;
            };
            let id = ParamId(semantic_hash(&[
                "generated-param",
                file.id.as_str(),
                info.id.as_str(),
                alias,
                &ancestry_text(&info.ancestry),
            ]));
            let source_default =
                default_property.and_then(|property| declaration.props.get(property));
            let initializer = source_default
                .map(unresolved_value)
                .unwrap_or(ResolvedValue::Null);
            self.params.insert(
                id.clone(),
                ResolvedParam {
                    id: id.clone(),
                    declaration: info.id.clone(),
                    source_name: format!("{}.{}", declaration.name_string(), alias),
                    type_contract: ParamTypeContract::SchemaFixed(data_type),
                    initializer,
                    sharing: StateSharing::Shared,
                    migration_key,
                    definition_local_seed,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
                    table_owner: None,
                },
            );
            return Some(ResolvedTarget::Param(id));
        }
        if value_kind == "selection" || value_kind.starts_with("selection<") {
            let id = SelectionId(semantic_hash(&[
                "generated-selection",
                file.id.as_str(),
                info.id.as_str(),
                alias,
                &ancestry_text(&info.ancestry),
            ]));
            self.selections.insert(
                id.clone(),
                ResolvedSelection {
                    id: id.clone(),
                    declaration: info.id.clone(),
                    source_name: format!("{}.{}", declaration.name_string(), alias),
                    empty: ResolvedSelectionEmpty::None,
                    combine: ResolvedSelectionCombine::Union,
                    migration_key,
                    definition_local_seed,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
                },
            );
            return Some(ResolvedTarget::Selection(id));
        }
        if let Some(inner) = value_kind
            .strip_prefix("store<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let Some(PhysicalType::Struct(fields)) = parse_type_text(inner) else {
                self.error(
                    "AVENGER-RESOLVE-031",
                    "cannot determine generated store schema",
                    info.span,
                    format!("schema export `{alias}` must contain a struct Arrow type"),
                );
                return None;
            };
            let id = StoreId(semantic_hash(&[
                "generated-store",
                file.id.as_str(),
                info.id.as_str(),
                alias,
                &ancestry_text(&info.ancestry),
            ]));
            self.stores.insert(
                id.clone(),
                ResolvedStore {
                    id: id.clone(),
                    declaration: info.id.clone(),
                    source_name: format!("{}.{}", declaration.name_string(), alias),
                    fields,
                    primary_key: Vec::new(),
                    rows: Vec::new(),
                    sharing: StateSharing::Shared,
                    migration_key,
                    definition_local_seed,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
                },
            );
            return Some(ResolvedTarget::Store(id));
        }
        if value_kind == "store" {
            self.error(
                "AVENGER-RESOLVE-031",
                "generated store export is missing an Arrow schema",
                info.span,
                format!("schema export `{alias}` must use `store<struct(...)>`"),
            );
        }
        None
    }

    fn state_identity(
        &self,
        file: &ParsedModule,
        info: &DeclInfo,
        role: &str,
    ) -> (Option<StateMigrationKey>, Option<DefinitionLocalSeed>) {
        if self.definitions.values().any(|schema| {
            schema.declaration == info.id || info.ancestry.contains(&schema.declaration)
        }) {
            return (
                None,
                Some(DefinitionLocalSeed(semantic_hash(&[
                    "definition-local-state",
                    file.id.as_str(),
                    info.id.as_str(),
                    role,
                ]))),
            );
        }
        (
            Some(StateMigrationKey(semantic_hash(&[
                "state-migration",
                file.id.as_str(),
                info.id.as_str(),
                role,
                &ancestry_text(&info.ancestry),
            ]))),
            None,
        )
    }

    fn resolve_sql_paths(
        &mut self,
        scope: ScopeId,
        paths: Vec<Vec<String>>,
        span: SourceSpan,
        diagnose_transform_paths: bool,
    ) -> Vec<ResolvedSqlReference> {
        let mut output = Vec::new();
        for path in paths {
            let target = if path.first().is_some_and(|name| name == "repeat") {
                Some(ResolvedTarget::Reserved {
                    namespace: "repeat".to_owned(),
                    path: path[1..].to_vec(),
                })
            } else {
                self.resolve_any_path(scope, &path, span, false)
            };
            if let Some(target @ (ResolvedTarget::Output(_) | ResolvedTarget::Reserved { .. })) =
                target
            {
                output.push(ResolvedSqlReference {
                    authored_path: path,
                    target,
                });
            } else if let Some(
                target @ (ResolvedTarget::DefinitionSlot { .. }
                | ResolvedTarget::DefinitionChannel { .. }),
            ) = target
            {
                output.push(ResolvedSqlReference {
                    authored_path: path,
                    target,
                });
            } else if diagnose_transform_paths
                && let Some(first) = path.first()
                && let Some(transform_scope) = self.scope_with_transform(scope, first)
            {
                let message = if self.scopes[transform_scope.0]
                    .structural
                    .contains_key(first)
                {
                    "transform output handle is not declared"
                } else {
                    "transform alias is not visible before its stage"
                };
                self.error(
                    "AVENGER-RESOLVE-083",
                    message,
                    span,
                    format!(
                        "`{}` cannot be resolved at this source position",
                        path.join(".")
                    ),
                );
            }
        }
        output.sort_by(|left, right| left.authored_path.cmp(&right.authored_path));
        output.dedup_by(|left, right| left.authored_path == right.authored_path);
        output
    }

    fn resolve_query_relations(
        &mut self,
        scope: ScopeId,
        paths: Vec<Vec<String>>,
        span: SourceSpan,
        owner: &Decl,
    ) -> Vec<ResolvedRelationReference> {
        let Some(module) = self.scopes[scope.0].module.clone() else {
            return Vec::new();
        };
        let allow_input =
            owner.keyword.as_str() == "transform" || self.scope_allows_input_relation(scope);
        paths
            .into_iter()
            .filter_map(|authored_path| {
                let relative = (authored_path.len() == 1)
                    .then(|| self.current_relation_context(scope))
                    .flatten()
                    .and_then(|current| {
                        let mut nested_path = current.nested_path;
                        nested_path.pop();
                        nested_path.extend(authored_path.iter().cloned());
                        let candidate = ResolvedRelationId {
                            defining_item: current.defining_item,
                            nested_path,
                        };
                        self.relation_declarations
                            .contains_key(&candidate)
                            .then_some(ResolvedRelationTarget::Relation(candidate))
                    });
                relative
                    .or_else(|| {
                        self.resolve_relation_path(&module, &authored_path, allow_input, span)
                    })
                    .map(|target| ResolvedRelationReference {
                        authored_path,
                        target,
                    })
            })
            .collect()
    }

    fn current_relation_context(&self, scope: ScopeId) -> Option<ResolvedRelationId> {
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(owner) = &self.scopes[id.0].owner
                && let Some((relation, _)) = self
                    .relation_declarations
                    .iter()
                    .find(|(_, declaration)| *declaration == owner)
            {
                return Some(relation.clone());
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    fn scope_allows_input_relation(&self, scope: ScopeId) -> bool {
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(owner) = &self.scopes[id.0].owner
                && self.definitions.values().any(|definition| {
                    definition.kind == DefinitionKind::Transform && &definition.declaration == owner
                })
            {
                return true;
            }
            if let Some(owner) = &self.scopes[id.0].owner
                && self
                    .declaration_source(owner)
                    .is_some_and(|(_, declaration)| declaration.keyword.as_str() == "transform")
            {
                // Definition expansion emits the exact transform pipeline at
                // the instantiation site. Its virtual `input` relation must
                // remain valid after the defining `define transform` wrapper
                // is no longer present in source.
                return true;
            }
            cursor = self.scopes[id.0].parent;
        }
        false
    }

    fn resolve_helpers(
        &mut self,
        scope: ScopeId,
        calls: Vec<RawHelperCall>,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
    ) -> Vec<ResolvedHelper> {
        let (scope_is_event, scope_has_between) = self.scope_event_context(scope);
        let in_event = in_event || scope_is_event;
        let mut output = Vec::new();
        for call in calls {
            let Some(class) = helper_class(&call.name) else {
                continue;
            };
            let expected = helper_arity(&call.name);
            if expected.is_some_and(|expected| expected != call.args.len()) {
                self.error(
                    "AVENGER-RESOLVE-109",
                    "reserved helper has the wrong arity",
                    span,
                    format!(
                        "`{}(...)` expects {} arguments, found {}",
                        call.name,
                        expected.unwrap(),
                        call.args.len()
                    ),
                );
            }
            if !in_event {
                self.error(
                    "AVENGER-RESOLVE-110",
                    "reserved operation is outside an event context",
                    span,
                    format!(
                        "`{}(...)` requires an event binding or event effect",
                        call.name
                    ),
                );
            }
            let _ = (scope_has_between, owner);

            let mut arguments = Vec::new();
            for (index, argument) in call.args.iter().enumerate() {
                let mut resolved = helper_argument(argument);
                if call.name == "selection_contains"
                    && index == 1
                    && let Some(field) = datum_field(argument)
                {
                    resolved = ResolvedHelperArgument::DatumField(field);
                }
                let expects_target = class == HelperClass::Selection && index == 0;
                if expects_target
                    && let Some(path) = helper_argument_path(argument)
                    && let Some(target) =
                        self.resolve_typed_reference_path(scope, &path, RefKind::Selection, span)
                {
                    let valid = matches!(
                        target,
                        ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. }
                    );
                    if !valid {
                        self.error(
                            "AVENGER-RESOLVE-112",
                            "reserved helper argument has the wrong target kind",
                            span,
                            format!("`{}` is not a valid {:?} target", path.join("."), class),
                        );
                    }
                    resolved = ResolvedHelperArgument::Target {
                        target,
                        authored_path: path,
                    };
                }
                arguments.push(resolved);
            }
            output.push(ResolvedHelper {
                name: call.name,
                class,
                arguments,
            });
        }
        output
    }

    fn helper_channel_exists(
        &self,
        scope: ScopeId,
        owner: &Decl,
        helper: &str,
        channel: &str,
    ) -> bool {
        let coordinate = self.visible_coordinate(scope);
        if helper == "channel"
            && owner.keyword.as_str() == "mark"
            && let Some(kind) = owner.kind.as_ref()
        {
            return coordinate
                .as_deref()
                .and_then(|coordinate| {
                    self.registry
                        .entries
                        .get(&NativeKindKey::mark(coordinate, kind.as_str()))
                        .cloned()
                })
                .or_else(|| self.definition_mark_schema(kind.as_str()))
                .is_some_and(|schema| schema.channels.contains_key(channel));
        }
        self.registry.entries.values().any(|schema| {
            schema.key.namespace == NativeKindNamespace::Mark
                && coordinate
                    .as_deref()
                    .is_none_or(|coordinate| schema.key.coordinate.as_deref() == Some(coordinate))
                && schema.channels.contains_key(channel)
        })
    }

    fn visible_coordinate(&self, scope: ScopeId) -> Option<String> {
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(owner) = self.scopes[id.0].owner.as_ref()
                && let Some((_, declaration)) = self.declaration_source(owner)
                && matches!(
                    declaration.keyword.as_str(),
                    "chart" | "cell" | "plot" | "view"
                )
                && let Some(kind) = declaration.kind.as_ref()
            {
                return Some(kind.to_string());
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    fn scope_with_transform(&self, scope: ScopeId, name: &str) -> Option<ScopeId> {
        let mut cursor = Some(scope);
        while let Some(scope) = cursor {
            if self.scopes[scope.0].transforms.contains(name) {
                return Some(scope);
            }
            cursor = self.scopes[scope.0].parent;
        }
        None
    }

    fn scope_event_context(&self, scope: ScopeId) -> (bool, bool) {
        let mut cursor = Some(scope);
        while let Some(scope) = cursor {
            if self.scopes[scope.0].event_binding {
                return (true, self.scopes[scope.0].event_has_between);
            }
            cursor = self.scopes[scope.0].parent;
        }
        (false, false)
    }

    fn resolve_pending_exports(&mut self) {
        let ids = self.instances.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let (scope, pending) = self.instances.get(&id).map_or_else(
                || (None, BTreeMap::new()),
                |interface| (interface.child_scope, interface.pending_exports.clone()),
            );
            let Some(scope) = scope else {
                continue;
            };
            for (alias, path) in pending {
                let span = self
                    .declarations
                    .values()
                    .find(|info| info.id == id)
                    .map_or_else(|| SourceSpan::empty(SourceId::new(0), 0), |info| info.span);
                if let Some(target) = self.resolve_internal_path(scope, &path) {
                    let collision = self.instances.get(&id).is_some_and(|interface| {
                        interface.exports.contains_key(&alias)
                            || interface.parts.contains_key(&alias)
                    });
                    if collision {
                        self.error(
                            "AVENGER-RESOLVE-130",
                            "explicit export alias collides with the component interface",
                            span,
                            format!("`{alias}` is already a child, export, or part"),
                        );
                    } else {
                        self.instances
                            .get_mut(&id)
                            .expect("instance exists")
                            .exports
                            .insert(alias.clone(), target);
                        self.instances
                            .get_mut(&id)
                            .expect("instance exists")
                            .published_exports
                            .insert(alias);
                    }
                } else {
                    self.error(
                        "AVENGER-RESOLVE-131",
                        "explicit export source cannot be resolved",
                        span,
                        format!("`{}` is not an exact internal path", path.join(".")),
                    );
                }
            }
        }
    }

    fn resolve_internal_path(&self, scope: ScopeId, path: &[String]) -> Option<ResolvedTarget> {
        let first = path.first()?;
        let mut cursor = Some(scope);
        let mut target = None;
        while let Some(id) = cursor {
            if let Some(symbol) = self.scopes[id.0].state_symbols.get(first) {
                target = Some(symbol.target());
                break;
            }
            if let Some(declaration) = self.scopes[id.0].structural.get(first) {
                target = self
                    .declarations
                    .values()
                    .find(|info| &info.id == declaration)
                    .and_then(|info| info.runtime_target.clone())
                    .or_else(|| Some(ResolvedTarget::Declaration(declaration.clone())));
                break;
            }
            cursor = self.scopes[id.0].parent;
        }
        let mut target = target?;
        for segment in path.iter().skip(1) {
            let declaration = match &target {
                ResolvedTarget::Declaration(id) => id.clone(),
                _ => self
                    .declarations
                    .values()
                    .find(|info| info.runtime_target.as_ref() == Some(&target))?
                    .id
                    .clone(),
            };
            let interface = self.instances.get(&declaration)?;
            let child_scope = interface.child_scope?;
            if let Some(symbol) = self.scopes[child_scope.0].state_symbols.get(segment) {
                target = symbol.target();
                continue;
            }
            if let Some(child) = self.scopes[child_scope.0].structural.get(segment) {
                target = self
                    .declarations
                    .values()
                    .find(|info| &info.id == child)
                    .and_then(|info| info.runtime_target.clone())
                    .unwrap_or_else(|| ResolvedTarget::Declaration(child.clone()));
                continue;
            }
            if let Some(export) = interface.exports.get(segment) {
                target = export.clone();
                continue;
            }
            return None;
        }
        Some(target)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_declaration(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        path: &[usize],
        inherited_coordinate: Option<&str>,
        parent_public_path: Option<&str>,
        inside_private: bool,
        in_event: bool,
    ) -> ResolvedDeclaration {
        let info = self
            .declarations
            .get(&(file.id.clone(), path.to_vec()))
            .cloned()
            .unwrap_or_else(|| DeclInfo {
                id: declaration_id(file, path),
                span: root_span(file),
                containing_scope: ScopeId(0),
                child_scope: None,
                ancestry: Vec::new(),
                runtime_target: runtime_target(declaration, &declaration_id(file, path)),
            });
        let mut coordinate = declaration_coordinate(declaration, inherited_coordinate);
        let parent = parent_declaration(file, path);
        self.validate_placement(declaration, parent, info.span);
        self.validate_visibility(file, path, declaration, info.span);

        let in_definition = inside_definition(file, path);
        let in_transform_pipeline = parent.is_some_and(|parent| {
            parent.keyword.as_str() == "transform"
                && parent
                    .kind
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == "pipeline")
        });
        self.validate_compiler_owned_names(
            declaration,
            info.span,
            in_definition,
            inside_private,
            in_transform_pipeline,
            path.len() == 1,
        );
        let kind_binding =
            self.kind_binding(file, declaration, coordinate.as_deref(), in_definition);
        if let Some(ResolvedKindBinding::Native { implementation, .. }) = &kind_binding
            && implementation.namespace == NativeKindNamespace::Coordinate
        {
            coordinate = Some(implementation.kind.clone());
        }
        let native_schema =
            self.native_schema(declaration, coordinate.as_deref(), kind_binding.as_ref());
        let definition_schema = declaration
            .kind
            .as_ref()
            .and_then(|kind| {
                self.imported_definition(
                    file,
                    kind.as_str(),
                    declaration_kind_category(declaration)?,
                )
            })
            .cloned();
        if native_schema.is_none()
            && definition_schema.is_none()
            && requires_registered_kind(declaration)
        {
            self.error(
                "AVENGER-RESOLVE-020",
                "unknown declaration kind",
                info.span,
                format!(
                    "no registered or imported {} kind `{}` is available here",
                    declaration.keyword,
                    declaration
                        .kind
                        .as_ref()
                        .map_or("<missing>", |kind| kind.as_str())
                ),
            );
        }

        if let Some(schema) = native_schema.as_ref() {
            self.validate_native_declaration(
                declaration,
                schema,
                info.containing_scope,
                parent.map(|parent| parent.keyword.as_str()),
                coordinate.as_deref(),
                info.span,
                in_definition,
            );
        }
        if let Some(schema) = definition_schema.as_ref() {
            self.validate_definition_instance(declaration, schema, info.span);
        }
        self.validate_core_declaration(declaration, info.span, in_event);

        let value_scope = info.child_scope.unwrap_or(info.containing_scope);
        let event_context = in_event || declaration.keyword.as_str() == "on";
        let public_path = declaration_public_path(declaration, parent_public_path, inside_private);
        let child_inside_private = match declaration.visibility {
            Visibility::Private => true,
            Visibility::Public => false,
            Visibility::Default => inside_private,
        };
        let mut resolved_children = vec![None; declaration.children.len()];

        // A mark-owned inline view is the producer for raster/materialization
        // output handles consumed by the owning mark's properties. Resolve that
        // one child first, then publish only its transform outputs into the mark
        // value scope. The view itself and its local names remain non-public.
        if declaration.keyword.as_str() == "mark" {
            for (index, child) in declaration.children.iter().enumerate() {
                if child.keyword.as_str() != "view" {
                    continue;
                }
                let mut child_path = path.to_vec();
                child_path.push(index);
                let resolved = self.resolve_declaration(
                    file,
                    child,
                    &child_path,
                    coordinate.as_deref(),
                    public_path.as_deref().or(parent_public_path),
                    child_inside_private,
                    event_context,
                );
                for (source, output) in child.children.iter().zip(&resolved.children) {
                    if source.keyword.as_str() == "transform" {
                        self.install_sequential_transform(value_scope, source, output, info.span);
                    }
                }
                resolved_children[index] = Some(resolved);
            }
        }
        let mut properties = BTreeMap::new();
        let mut property_channels = BTreeMap::new();
        let mut mark_block_ordinal = 0usize;
        for (name, value) in declaration.props.iter() {
            let definition_channel =
                self.visible_definition_channel_property(value_scope, name.as_str());
            if let Some((target, _)) = definition_channel.clone() {
                property_channels.insert(name.to_string(), target);
            }
            let definition_slot = definition_schema
                .as_ref()
                .and_then(|schema| schema.slots.get(name.as_str()));
            let native_channel = native_schema
                .as_ref()
                .and_then(|schema| {
                    schema.channels.get(name.as_str()).or_else(|| {
                        definition_channel
                            .as_ref()
                            .and_then(|(_, physical)| physical.as_deref())
                            .and_then(|physical| schema.channels.get(physical))
                    })
                })
                .cloned();
            let mut resolved = match definition_slot {
                Some(slot) if slot.shape == "block" => {
                    // Caller-owned block content is hygienically resolved only
                    // after slot substitution, exposure rewriting, and splice
                    // placement. Resolving it in the caller scope here would
                    // either reject declared exposures or accidentally bind a
                    // same-named caller symbol before expansion.
                    unresolved_value(value)
                }
                Some(slot) if slot.shape == "ref" => self.resolve_definition_ref_value(
                    value_scope,
                    value,
                    slot,
                    info.span,
                    event_context,
                    declaration,
                ),
                _ => self.resolve_value_with_mark_blocks(
                    file,
                    path,
                    value_scope,
                    value,
                    info.span,
                    event_context,
                    declaration,
                    &mut mark_block_ordinal,
                ),
            };
            if let Some(channel) = native_channel {
                self.normalize_definition_arguments(
                    value_scope,
                    value,
                    &mut resolved,
                    &channel.shape,
                    info.span,
                    event_context,
                    declaration,
                );
                self.validate_channel_value(
                    &resolved,
                    &channel.shape,
                    channel.required,
                    name.as_str(),
                    info.span,
                );
                self.normalize_resolved_channel(&mut resolved);
            } else if let Some(shape) = native_schema
                .as_ref()
                .and_then(|schema| {
                    schema_property(schema, name.as_str()).or_else(|| {
                        definition_channel
                            .as_ref()
                            .and_then(|(_, physical)| physical.as_deref())
                            .and_then(|physical| schema_property(schema, physical))
                    })
                })
                .cloned()
            {
                self.normalize_definition_arguments(
                    value_scope,
                    value,
                    &mut resolved,
                    &shape,
                    info.span,
                    event_context,
                    declaration,
                );
                self.validate_value_shape(&resolved, &shape, name.as_str(), info.span);
                self.normalize_channels_for_shape(&mut resolved, &shape);
            } else if let Some(slot) = definition_schema
                .as_ref()
                .and_then(|schema| schema.slots.get(name.as_str()))
            {
                self.validate_definition_value(&resolved, slot, name.as_str(), info.span);
            } else if let Some(channel) = definition_schema
                .as_ref()
                .and_then(|schema| schema.channels.get(name.as_str()))
            {
                self.validate_definition_channel(
                    &resolved,
                    channel,
                    coordinate.as_deref(),
                    name.as_str(),
                    info.span,
                );
            } else if in_definition {
                self.normalize_expression_argument(value_scope, &mut resolved, info.span);
            }
            properties.insert(name.to_string(), resolved);
        }
        if let Some(schema) = native_schema.as_ref() {
            for (name, property) in &schema.properties {
                if !properties.contains_key(name)
                    && let Some(default) = &property.default
                {
                    properties.insert(
                        name.clone(),
                        resolved_schema_default(default, &property.shape),
                    );
                }
            }
        }
        if let Some(schema) = definition_schema.as_ref() {
            for (name, slot) in &schema.slots {
                if !properties.contains_key(name)
                    && let Some(default) = &slot.default
                {
                    properties.insert(name.clone(), default.clone());
                }
            }
            for (name, channel) in &schema.channels {
                if !properties.contains_key(name)
                    && let Some(physical) = &channel.physical_channel
                {
                    let value = ResolvedValue::Atom(physical.clone());
                    self.validate_definition_channel(
                        &value,
                        channel,
                        coordinate.as_deref(),
                        name,
                        info.span,
                    );
                    properties.insert(name.clone(), value);
                }
            }
        }
        if declaration.keyword.as_str() == "on" {
            self.validate_event_properties(declaration, &properties, info.span);
        }
        let event_binding = (declaration.keyword.as_str() == "on")
            .then(|| self.resolve_event_binding(value_scope, declaration, &properties, info.span));

        self.resolve_state_declaration(file, declaration, &info, &properties);

        let pipeline = declaration.keyword.as_str() == "transform"
            && declaration
                .kind
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "pipeline");
        let transform_definition = declaration.keyword.as_str() == "define"
            && declaration
                .kind
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "transform");
        let deferred_outputs = pipeline || transform_definition;
        let resolution_order = (0..declaration.children.len())
            .filter(|index| {
                !deferred_outputs || declaration.children[*index].keyword.as_str() != "output"
            })
            .chain((0..declaration.children.len()).filter(|index| {
                deferred_outputs && declaration.children[*index].keyword.as_str() == "output"
            }))
            .collect::<Vec<_>>();
        let mut transform_outputs = BTreeMap::new();
        for index in resolution_order {
            if resolved_children[index].is_some() {
                continue;
            }
            let child = &declaration.children[index];
            let mut child_path = path.to_vec();
            child_path.push(index);
            let resolved = self.resolve_declaration(
                file,
                child,
                &child_path,
                coordinate.as_deref(),
                public_path.as_deref().or(parent_public_path),
                child_inside_private,
                event_context,
            );
            if child.keyword.as_str() == "transform" {
                self.install_sequential_transform(value_scope, child, &resolved, info.span);
                for (name, output) in &resolved.transform_outputs {
                    transform_outputs.insert(name.clone(), output.clone());
                }
            }
            resolved_children[index] = Some(resolved);
        }
        let mut children = resolved_children
            .into_iter()
            .map(|child| child.expect("every child resolves exactly once"))
            .collect::<Vec<_>>();
        if declaration.keyword.as_str() == "transform" {
            transform_outputs =
                self.transform_outputs(declaration, &info, &children, definition_schema.as_ref());
        }
        self.validate_event_actions(declaration, &mut children, info.span);

        let interface = self.instances.get(&info.id).cloned().unwrap_or_default();
        let published_exports = interface
            .published_exports
            .iter()
            .filter_map(|alias| {
                interface
                    .exports
                    .get(alias)
                    .cloned()
                    .map(|target| (alias.clone(), target))
            })
            .collect();
        let (migration_key, definition_local_seed) = if declaration.keyword.as_str() == "on" {
            self.state_identity(file, &info, "event")
        } else {
            (None, None)
        };
        let relation_references = Self::declaration_relation_references(&properties);
        ResolvedDeclaration {
            id: info.id.clone(),
            source: file.source,
            span: info.span,
            keyword: declaration.keyword.to_string(),
            kind: match &kind_binding {
                Some(ResolvedKindBinding::Native { implementation, .. }) => {
                    Some(implementation.kind.clone())
                }
                _ => declaration.kind.as_ref().map(ToString::to_string),
            },
            kind_binding,
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate,
            component_kind: declaration
                .props
                .get("component_kind")
                .and_then(value_atom)
                .map(str::to_owned),
            properties,
            relation_references,
            property_channels,
            children,
            runtime_target: info.runtime_target,
            migration_key,
            definition_local_seed,
            public_path,
            parts: interface.parts,
            exports: published_exports,
            transform_outputs,
            event_binding,
            state_lvalue: None,
        }
    }

    fn declaration_relation_references(
        properties: &BTreeMap<String, ResolvedValue>,
    ) -> Vec<ResolvedRelationReference> {
        let mut references = Vec::new();
        for value in properties.values() {
            Self::collect_property_relation_references(value, &mut references);
        }
        references.sort_by(|left, right| {
            left.authored_path
                .cmp(&right.authored_path)
                .then(format!("{:?}", left.target).cmp(&format!("{:?}", right.target)))
        });
        references.dedup();
        references
    }

    fn collect_property_relation_references(
        value: &ResolvedValue,
        output: &mut Vec<ResolvedRelationReference>,
    ) {
        if let ResolvedValue::Relation(reference) = value {
            output.push(reference.clone());
            return;
        }
        match value {
            ResolvedValue::Query(query) => output.extend(query.relations.iter().cloned()),
            ResolvedValue::Channel {
                expression: value, ..
            }
            | ResolvedValue::Pattern(value) => {
                Self::collect_property_relation_references(value, output)
            }
            ResolvedValue::ChannelValue(channel) => {
                Self::collect_property_relation_references(&channel.head.expression, output);
                if let Some(otherwise) = &channel.otherwise {
                    Self::collect_property_relation_references(&otherwise.expression, output);
                }
                for condition in &channel.conditions {
                    Self::collect_property_relation_references(&condition.predicate, output);
                    Self::collect_property_relation_references(
                        &condition.branch.expression,
                        output,
                    );
                }
                for value in channel.configuration.values() {
                    Self::collect_property_relation_references(value, output);
                }
            }
            ResolvedValue::Array(values) => {
                for value in values {
                    Self::collect_property_relation_references(value, output);
                }
            }
            ResolvedValue::Object {
                head,
                properties,
                children,
                ..
            } => {
                if let Some(head) = head {
                    Self::collect_property_relation_references(head, output);
                }
                for value in properties.values() {
                    Self::collect_property_relation_references(value, output);
                }
                for child in children {
                    output.extend(child.relation_references.iter().cloned());
                }
            }
            ResolvedValue::Call { args, .. } => {
                for value in args {
                    Self::collect_property_relation_references(value, output);
                }
            }
            _ => {}
        }
    }

    fn native_schema(
        &self,
        declaration: &Decl,
        coordinate: Option<&str>,
        binding: Option<&ResolvedKindBinding>,
    ) -> Option<KindSchema> {
        if is_mark_group(declaration) {
            return Some(core_mark_group_schema(coordinate));
        }
        match binding? {
            ResolvedKindBinding::Builtin(key)
            | ResolvedKindBinding::Native {
                implementation: key,
                ..
            } => self.registry.entries.get(key).cloned(),
            ResolvedKindBinding::Structural(kind) if declaration.keyword.as_str() == "mark" => {
                self.definition_mark_schema(kind)
            }
            ResolvedKindBinding::LanguageCore(_)
            | ResolvedKindBinding::Definition(_)
            | ResolvedKindBinding::Structural(_) => None,
        }
    }

    /// Definition bodies are checked before an instance supplies a concrete
    /// coordinate system. Merge every registered coordinate-specific variant
    /// of a mark into a conservative authoring schema for that template pass.
    /// Phase 7 validates the substituted declaration against the exact
    /// instance coordinate again.
    fn definition_mark_schema(&self, kind: &str) -> Option<KindSchema> {
        let candidates = self
            .registry
            .entries
            .values()
            .filter(|schema| {
                schema.key.namespace == NativeKindNamespace::Mark && schema.key.kind == kind
            })
            .cloned()
            .collect::<Vec<_>>();
        merge_definition_mark_schemas(candidates)
    }

    fn imported_definition(
        &self,
        file: &ParsedModule,
        kind: &str,
        category: BindingCategory,
    ) -> Option<&DefinitionSchema> {
        let item = self.definition_item(file, kind, category)?;
        self.definitions.get(item)
    }

    fn definition_item(
        &self,
        file: &ParsedModule,
        kind: &str,
        category: BindingCategory,
    ) -> Option<&ModuleItemId> {
        let environment = self.module_index.environments.get(&file.id)?;
        let segments = kind.split('.').collect::<Vec<_>>();
        let export = match segments.as_slice() {
            [name] => environment.local.get(&(category, (*name).to_owned()))?,
            [namespace, member] => {
                let module = environment.namespaces.get(*namespace)?;
                self.module_index
                    .exports
                    .get(module)?
                    .exports
                    .get(*member)
                    .filter(|export| export.category == category)?
            }
            _ => return None,
        };
        let ModuleId::Source(module) = &export.module else {
            return None;
        };
        self.module_index
            .local_items
            .get(&(module.clone(), category, export.name.clone()))
            .or_else(|| {
                self.module_index
                    .source_export_items
                    .get(&(module.clone(), export.name.clone()))
            })
    }

    fn kind_binding(
        &self,
        file: &ParsedModule,
        declaration: &Decl,
        coordinate: Option<&str>,
        in_definition: bool,
    ) -> Option<ResolvedKindBinding> {
        let kind = declaration.kind.as_ref()?;
        if is_mark_group(declaration) {
            return Some(ResolvedKindBinding::LanguageCore("mark.group".to_owned()));
        }
        if declaration.keyword.as_str() == "tool" && kind.as_str() == "behavior" {
            return Some(ResolvedKindBinding::LanguageCore(
                "tool.behavior".to_owned(),
            ));
        }
        let category = declaration_kind_category(declaration)?;
        if let Some(item) = self.definition_item(file, kind.as_str(), category) {
            return Some(ResolvedKindBinding::Definition(item.clone()));
        }

        let environment = self.module_index.environments.get(&file.id)?;
        let segments = kind.segments();
        let export = match segments {
            [name] => environment
                .local
                .get(&(category, name.to_string()))
                .filter(|export| matches!(export.module, ModuleId::Native(_))),
            [namespace, member] => {
                let module = environment.namespaces.get(namespace.as_str())?;
                self.module_index
                    .exports
                    .get(module)?
                    .exports
                    .get(member.as_str())
                    .filter(|export| export.category == category)
            }
            _ => None,
        };
        if let Some(export) = export {
            let ModuleId::Native(module) = &export.module else {
                return None;
            };
            let implementation = self
                .registry
                .modules
                .get(module)?
                .exports
                .get(&export.name)?
                .implementation
                .clone();
            return Some(ResolvedKindBinding::Native {
                export: export.clone(),
                implementation,
            });
        }

        let simple = kind.simple()?;
        let key = match declaration.keyword.as_str() {
            "chart" | "cell" | "plot" => {
                NativeKindKey::new(NativeKindNamespace::Coordinate, simple.as_str())
            }
            "view" => NativeKindKey::new(NativeKindNamespace::View, simple.as_str()),
            "mark" => {
                if let Some(coordinate) = coordinate {
                    NativeKindKey::mark(coordinate, simple.as_str())
                } else if in_definition {
                    return Some(ResolvedKindBinding::Structural(simple.to_string()));
                } else {
                    return None;
                }
            }
            "adjust" => NativeKindKey::new(NativeKindNamespace::Adjust, simple.as_str()),
            "transform" => NativeKindKey::new(NativeKindNamespace::Transform, simple.as_str()),
            "tool" => NativeKindKey::new(NativeKindNamespace::Tool, simple.as_str()),
            "widget" => NativeKindKey::new(NativeKindNamespace::Widget, simple.as_str()),
            "resource" => NativeKindKey::new(NativeKindNamespace::Resource, simple.as_str()),
            _ => return None,
        };
        if self.registry.modules.values().any(|module| {
            module
                .exports
                .values()
                .any(|export| export.implementation == key)
        }) {
            // Host-native declarations participate in authoring only through
            // an explicit named or namespace import. Installation alone must
            // never make an unqualified kind visible.
            return None;
        }
        Some(ResolvedKindBinding::Builtin(key))
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_native_declaration(
        &mut self,
        declaration: &Decl,
        schema: &KindSchema,
        scope: ScopeId,
        parent: Option<&str>,
        coordinate: Option<&str>,
        span: SourceSpan,
        definition_template: bool,
    ) {
        if schema.body_mode == BodyMode::Properties
            && declaration.children.iter().any(|child| {
                !definition_template || !matches!(child.keyword.as_str(), "match" | "splice")
            })
        {
            self.error(
                "AVENGER-RESOLVE-015",
                "native declaration requires a property-only body",
                span,
                format!("`{}` does not accept child declarations", schema.key.kind),
            );
        }
        if !schema.allowed_parents.is_empty()
            && parent.is_none_or(|parent| !schema.allowed_parents.contains(parent))
        {
            self.error(
                "AVENGER-RESOLVE-016",
                "native declaration is not allowed at this placement",
                span,
                format!(
                    "`{}` requires one of these parents: {}",
                    schema.key.kind,
                    schema
                        .allowed_parents
                        .iter()
                        .map(|parent| format!("`{parent}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        let accepted = schema
            .properties
            .keys()
            .chain(schema.channels.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for (name, _) in declaration.props.iter() {
            let logical_channel = self
                .visible_definition_channel_property(scope, name.as_str())
                .is_some();
            if !accepted.contains(name.as_str())
                && schema.additional_properties.is_none()
                && !logical_channel
                && !core_property(declaration, name.as_str())
            {
                self.error(
                    "AVENGER-RESOLVE-021",
                    "unknown declaration property",
                    span,
                    format!("`{}` has no property or channel `{name}`", schema.key.kind),
                );
            }
        }
        for (name, property) in &schema.properties {
            if property.required && declaration.props.get(name).is_none() {
                self.error(
                    "AVENGER-RESOLVE-022",
                    "missing required property",
                    span,
                    format!("`{}` requires `{name}:`", schema.key.kind),
                );
            }
        }
        for (name, channel) in &schema.channels {
            let supplied_by_logical_channel = declaration.props.iter().any(|(property, _)| {
                self.visible_definition_channel_property(scope, property.as_str())
                    .is_some_and(|(_, physical)| physical.as_deref() == Some(name.as_str()))
            });
            let has_unbound_logical_channel = declaration.props.iter().any(|(property, _)| {
                self.visible_definition_channel_property(scope, property.as_str())
                    .is_some_and(|(_, physical)| physical.is_none())
            });
            if channel.required
                && declaration.props.get(name).is_none()
                && !supplied_by_logical_channel
                && !has_unbound_logical_channel
            {
                self.error(
                    "AVENGER-RESOLVE-022",
                    "missing required channel",
                    span,
                    format!("`{}` requires channel `{name}:`", schema.key.kind),
                );
            }
        }
        if schema.key.namespace == NativeKindNamespace::Adjust && declaration.name.is_none() {
            self.error(
                "AVENGER-RESOLVE-168",
                "transform adjustment requires a binder",
                span,
                "add `as <name>` so `apply:` can reference the adjustment's output handles",
            );
        }
        if !schema.compatible_coordinates.is_empty()
            && let Some(coordinate) = coordinate
            && !schema.compatible_coordinates.contains(coordinate)
        {
            self.error(
                "AVENGER-RESOLVE-023",
                "declaration is incompatible with its coordinate system",
                span,
                format!("`{}` does not support `{coordinate}`", schema.key.kind),
            );
        }
        let counts = declaration
            .children
            .iter()
            .filter(|child| {
                !definition_template || !matches!(child.keyword.as_str(), "match" | "splice")
            })
            .fold(BTreeMap::<String, usize>::new(), |mut counts, child| {
                *counts.entry(child.keyword.to_string()).or_default() += 1;
                counts
            });
        for rule in &schema.child_rules {
            let count = counts.get(&rule.role).copied().unwrap_or(0);
            if count < rule.min || rule.max.is_some_and(|maximum| count > maximum) {
                self.error(
                    "AVENGER-RESOLVE-024",
                    "native child multiplicity is invalid",
                    span,
                    format!(
                        "role `{}` occurs {count} times; expected {}{}",
                        rule.role,
                        rule.min,
                        rule.max.map_or_else(String::new, |max| format!("..={max}"))
                    ),
                );
            }
        }
        for effect in ["adjust", "derive"] {
            if counts.get(effect).copied().unwrap_or(0) > 0
                && !schema.child_rules.iter().any(|rule| rule.role == effect)
            {
                self.error(
                    "AVENGER-RESOLVE-024",
                    "native child role is not supported",
                    span,
                    format!("`{}` does not support `{effect}` children", schema.key.kind),
                );
            }
        }
    }

    fn validate_definition_instance(
        &mut self,
        declaration: &Decl,
        schema: &DefinitionSchema,
        span: SourceSpan,
    ) {
        let expected_kind = match schema.kind {
            DefinitionKind::Mark => "mark",
            DefinitionKind::Tool => "tool",
            DefinitionKind::Transform => "transform",
        };
        if declaration.keyword.as_str() != expected_kind {
            self.error(
                "AVENGER-RESOLVE-025",
                "imported definition used with the wrong declaration kind",
                span,
                format!(
                    "this import defines a {expected_kind}, not a {}",
                    declaration.keyword
                ),
            );
            return;
        }
        let accepted = schema
            .slots
            .keys()
            .chain(schema.channels.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for (name, _) in declaration.props.iter() {
            let generic = match schema.kind {
                DefinitionKind::Mark => matches!(
                    name.as_str(),
                    "visible" | "details" | "zindex" | "facet_data_scope" | "geometry_space"
                ),
                DefinitionKind::Transform => name.as_str() == "scope",
                DefinitionKind::Tool => false,
            };
            if !accepted.contains(name.as_str()) && !generic {
                self.error(
                    "AVENGER-RESOLVE-026",
                    "unknown definition argument",
                    span,
                    format!("definition has no slot or channel `{name}`"),
                );
            }
        }
        for (name, slot) in &schema.slots {
            if slot.required && declaration.props.get(name).is_none() {
                self.error(
                    "AVENGER-RESOLVE-027",
                    "missing required definition slot",
                    span,
                    format!("definition requires `{name}:`"),
                );
            }
        }
        for (name, channel) in &schema.channels {
            if channel.required && declaration.props.get(name).is_none() {
                self.error(
                    "AVENGER-RESOLVE-027",
                    "missing required definition channel",
                    span,
                    format!("definition requires channel `{name}:`"),
                );
            }
        }
        if !schema.outputs.is_empty() && declaration.name.is_none() {
            self.error(
                "AVENGER-RESOLVE-028",
                "output-bearing definition instance requires a binder",
                span,
                "add `as <name>` so output handles have a qualified owner",
            );
        }
        if schema.kind == DefinitionKind::Tool && declaration.name.is_none() {
            self.error(
                "AVENGER-RESOLVE-164",
                "defined tool instance requires a binder",
                span,
                "add `as <name>` to preserve tool ownership and state identity",
            );
        }
        let mut parts = BTreeSet::new();
        for child in declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "part")
        {
            if !child.children.is_empty() {
                self.error(
                    "AVENGER-RESOLVE-167",
                    "definition part override requires a property-only body",
                    span,
                    "`part` accepts configured property values, including channel conditions, but no direct child declarations",
                );
            }
            let Some(alias) = child.name.as_ref().map(ToString::to_string) else {
                continue;
            };
            if schema.kind != DefinitionKind::Mark || !schema.parts.contains_key(&alias) {
                self.error(
                    "AVENGER-RESOLVE-165",
                    "unknown definition part override",
                    span,
                    format!("definition does not export a mark part named `{alias}`"),
                );
            }
            if !parts.insert(alias.clone()) {
                self.error(
                    "AVENGER-RESOLVE-166",
                    "duplicate definition part override",
                    span,
                    format!("part `{alias}` is overridden more than once"),
                );
            }
        }
    }

    fn validate_core_declaration(&mut self, declaration: &Decl, span: SourceSpan, in_event: bool) {
        match declaration.keyword.as_str() {
            "param" => {
                self.validate_core_property_names(declaration, &["value", "sharing"], span);
                if !declaration.children.is_empty() {
                    self.error(
                        "AVENGER-RESOLVE-045",
                        "declaration requires a property-only body",
                        span,
                        "`param` does not accept child declarations",
                    );
                }
                if declaration.props.get("value").is_none() {
                    self.error(
                        "AVENGER-RESOLVE-032",
                        "incomplete param declaration",
                        span,
                        "every scalar param requires an initializer before `as`",
                    );
                }
            }
            "store" => {
                self.validate_core_property_names(declaration, &["primary_key", "sharing"], span);
                if declaration
                    .children
                    .iter()
                    .any(|child| !matches!(child.keyword.as_str(), "field" | "row"))
                {
                    self.error(
                        "AVENGER-RESOLVE-034",
                        "invalid store child",
                        span,
                        "stores accept only ordered `field` and `row` children",
                    );
                }
                if let Some(primary_key) = declaration.props.get("primary_key") {
                    let valid = matches!(primary_key, Value::Array(values)
                        if !values.is_empty()
                            && values.iter().all(|value| matches!(value, Value::Atom(_)))
                            && values
                                .iter()
                                .filter_map(value_atom)
                                .collect::<BTreeSet<_>>()
                                .len()
                                == values.len());
                    if !valid {
                        self.error(
                            "AVENGER-RESOLVE-142",
                            "invalid store primary-key declaration",
                            span,
                            "`primary_key:` requires a non-empty, duplicate-free array of field names",
                        );
                    }
                }
            }
            "selection" => {
                self.validate_core_property_names(declaration, &["empty", "combine"], span);
                if declaration.props.get("empty").is_some_and(|value| {
                    !matches!(value, Value::None)
                        && value_atom(value).is_none_or(|value| value != "all")
                }) {
                    self.error(
                        "AVENGER-RESOLVE-143",
                        "invalid empty-selection behavior",
                        span,
                        "`empty:` must be `none` or `all`",
                    );
                }
                if declaration.props.get("combine").is_some_and(|value| {
                    !value_atom(value).is_some_and(|value| matches!(value, "union" | "intersect"))
                }) {
                    self.error(
                        "AVENGER-RESOLVE-144",
                        "invalid selection combination",
                        span,
                        "`combine:` must be `union` or `intersect`",
                    );
                }
                if !declaration.children.is_empty() {
                    self.error(
                        "AVENGER-RESOLVE-045",
                        "declaration requires a property-only body",
                        span,
                        "`selection` does not accept child declarations",
                    );
                }
            }
            "view" => {
                if declaration.visibility != Visibility::Default {
                    self.error(
                        "AVENGER-RESOLVE-035",
                        "inline views cannot be public or private",
                        span,
                        "view binders are local helper scopes, not public identities",
                    );
                }
                if contains_descendant(declaration, "view") {
                    self.error(
                        "AVENGER-RESOLVE-036",
                        "nested inline views are not supported",
                        span,
                        "move the inner view-dependent chain into this view",
                    );
                }
                if declaration
                    .children
                    .iter()
                    .any(|child| child.keyword.as_str() == "export")
                {
                    self.error(
                        "AVENGER-RESOLVE-037",
                        "inline views cannot export names",
                        span,
                        "view identity is local to its body",
                    );
                }
            }
            "mark"
                if declaration
                    .children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "view")
                    .count()
                    > 1 =>
            {
                self.error(
                    "AVENGER-RESOLVE-044",
                    "a mark may own at most one inline view",
                    span,
                    "combine the dependent transform/render chain into one view scope",
                );
            }
            "widget" if !declaration.children.is_empty() => self.error(
                "AVENGER-RESOLVE-038",
                "registered widgets have property-only bodies",
                span,
                "widget implementations and expansion are supplied by the Rust registry",
            ),
            "set" if !in_event => self.error(
                "AVENGER-RESOLVE-039",
                "state actions are only valid in event or widget-action bodies",
                span,
                "move this ordered action into an event binding",
            ),
            "scale" | "axis" | "legend" if !declaration.children.is_empty() => {
                self.error(
                    "AVENGER-RESOLVE-045",
                    "declaration requires a property-only body",
                    span,
                    format!(
                        "`{}` does not accept child declarations",
                        declaration.keyword
                    ),
                );
            }
            _ => {}
        }
    }

    fn validate_compiler_owned_names(
        &mut self,
        declaration: &Decl,
        span: SourceSpan,
        in_definition: bool,
        inside_private: bool,
        in_transform_pipeline: bool,
        module_item: bool,
    ) {
        if declaration_binds_name(declaration)
            && let Some(name) = declaration.name.as_ref().map(Name::as_str)
            && name.starts_with("__av_")
            && !((declaration.visibility == Visibility::Private || inside_private)
                && is_generated_private_binder(name))
            && !(module_item && is_generated_bundle_binder(name))
        {
            self.error(
                "AVENGER-RESOLVE-181",
                "declaration uses the compiler-owned name prefix",
                span,
                format!("`{name}` begins with reserved `__av_`; rename the authored binding"),
            );
        }

        let mut identifiers = BTreeSet::new();
        for (_, value) in declaration.props.iter() {
            collect_sql_identifier_values(value, &mut identifiers);
        }
        for identifier in identifiers {
            if identifier.starts_with("__av_")
                && declaration.visibility != Visibility::Private
                && !inside_private
                && !(in_transform_pipeline && is_generated_private_column(&identifier))
            {
                self.error(
                    "AVENGER-RESOLVE-181",
                    "SQL uses the compiler-owned column prefix",
                    span,
                    format!(
                        "column `{identifier}` begins with reserved `__av_`; rename or project it before this declaration"
                    ),
                );
            } else if identifier.starts_with("__private_") && !in_definition {
                self.error(
                    "AVENGER-RESOLVE-182",
                    "definition-private SQL column is used outside a definition",
                    span,
                    format!(
                        "`{identifier}` is a private-column marker and is valid only inside a definition"
                    ),
                );
            } else if identifier == "__private_" {
                self.error(
                    "AVENGER-RESOLVE-182",
                    "definition-private SQL column has no name",
                    span,
                    "add a non-empty suffix after `__private_`",
                );
            }
        }
    }

    fn validate_core_property_names(
        &mut self,
        declaration: &Decl,
        allowed: &[&str],
        span: SourceSpan,
    ) {
        for (name, _) in declaration.props.iter() {
            if !allowed.contains(&name.as_str()) {
                self.error(
                    "AVENGER-RESOLVE-141",
                    "unknown core declaration property",
                    span,
                    format!("`{}` does not have `{name}:`", declaration.keyword),
                );
            }
        }
    }

    fn validate_placement(&mut self, declaration: &Decl, parent: Option<&Decl>, span: SourceSpan) {
        let Some(parent) = parent else {
            return;
        };
        let valid = if is_mark_group(parent) {
            ordinary_plot_child(declaration.keyword.as_str())
                || matches!(
                    declaration.keyword.as_str(),
                    "export" | "set" | "match" | "splice"
                )
        } else {
            placement_allowed(parent.keyword.as_str(), declaration.keyword.as_str())
        };
        if !valid {
            self.error(
                "AVENGER-RESOLVE-040",
                "declaration is not valid in this body",
                span,
                format!(
                    "`{}` cannot be a direct child of `{}`",
                    declaration.keyword, parent.keyword
                ),
            );
        }
    }

    fn validate_visibility(
        &mut self,
        file: &ParsedModule,
        path: &[usize],
        declaration: &Decl,
        span: SourceSpan,
    ) {
        if declaration.visibility == Visibility::Default {
            return;
        }
        if inside_definition(file, path) {
            self.error(
                "AVENGER-RESOLVE-043",
                "visibility modifiers are invalid inside definitions",
                span,
                "definition bodies are private as a unit; publish exact names with header `export` declarations",
            );
            return;
        }
        let has_public_identity = is_structural(declaration)
            || matches!(
                declaration.keyword.as_str(),
                "param" | "store" | "selection" | "transform"
            );
        if declaration.name.is_none() || !has_public_identity {
            self.error(
                "AVENGER-RESOLVE-041",
                "visibility requires a named public identity",
                span,
                "only named structure, params, stores, and selections can be public or private",
            );
        }
        let has_private_ancestor = (1..path.len()).any(|length| {
            declaration_at(file, &path[..length]).is_some_and(|ancestor| {
                ancestor.visibility == Visibility::Private && is_structural(ancestor)
            })
        });
        if declaration.visibility == Visibility::Public && !has_private_ancestor {
            self.error(
                "AVENGER-RESOLVE-042",
                "public declaration requires a private structural ancestor",
                span,
                "`public` is the constrained hoist form used beneath private structure",
            );
        }
    }

    fn validate_value_shape(
        &mut self,
        value: &ResolvedValue,
        shape: &ValueShape,
        property: &str,
        span: SourceSpan,
    ) {
        // Name/binding resolution already emitted the actionable diagnostic.
        // Do not add a derivative shape error for the same authored value.
        if resolved_value_contains_invalid(value) {
            return;
        }
        if matches!(
            value,
            ResolvedValue::DefinitionArgument(
                ResolvedTarget::DefinitionSlot { .. } | ResolvedTarget::DefinitionChannel { .. }
            )
        ) {
            return;
        }
        if value_matches_shape(value, shape) {
            match (value, shape) {
                (
                    ResolvedValue::Object {
                        head: Some(head),
                        properties,
                        ..
                    },
                    ValueShape::ConfiguredExpression(fields),
                ) => {
                    self.validate_value_shape(head, &ValueShape::SqlExpression, property, span);
                    self.validate_object_fields(properties, fields, property, span);
                }
                (
                    ResolvedValue::Object {
                        head: Some(head),
                        properties,
                        ..
                    },
                    ValueShape::ConfiguredReference {
                        namespaces,
                        properties: fields,
                    },
                ) => {
                    self.validate_value_shape(
                        head,
                        &ValueShape::TypedReference {
                            namespaces: namespaces.clone(),
                        },
                        property,
                        span,
                    );
                    self.validate_object_fields(properties, fields, property, span);
                }
                (ResolvedValue::Object { properties, .. }, ValueShape::Object(fields)) => {
                    self.validate_object_fields(properties, fields, property, span);
                }
                (ResolvedValue::Array(values), ValueShape::OneOrMany(inner)) => {
                    for value in values {
                        self.validate_value_shape(value, inner, property, span);
                    }
                }
                (value, ValueShape::OneOrMany(inner)) => {
                    self.validate_value_shape(value, inner, property, span);
                }
                (ResolvedValue::Object { properties, .. }, ValueShape::Map(inner)) => {
                    for value in properties.values() {
                        self.validate_value_shape(value, inner, property, span);
                    }
                }
                (ResolvedValue::Object { properties, .. }, ValueShape::ChannelMap) => {
                    for value in properties.values() {
                        self.validate_channel_value(
                            value,
                            &ValueShape::SqlExpression,
                            false,
                            property,
                            span,
                        );
                    }
                }
                (value, ValueShape::Union(shapes)) => {
                    if let Some(shape) = shapes
                        .iter()
                        .find(|shape| value_matches_shape(value, shape))
                    {
                        self.validate_value_shape(value, shape, property, span);
                    }
                }
                _ => {}
            }
            return;
        }
        if matches!(value, ResolvedValue::Channel { .. })
            || matches!(
                value,
                ResolvedValue::Object {
                    head: Some(head),
                    ..
                } if matches!(head.as_ref(), ResolvedValue::Channel { .. })
            )
        {
            self.error(
                "AVENGER-RESOLVE-202",
                "channel mode is not valid for an ordinary property",
                span,
                format!("remove the mode qualifier from `{property}`"),
            );
            return;
        }
        self.error(
            "AVENGER-RESOLVE-050",
            "property has the wrong value shape",
            span,
            format!(
                "`{property}` expects {}, found {}",
                shape_name(shape),
                resolved_shape(value)
            ),
        );
    }

    fn validate_channel_value(
        &mut self,
        value: &ResolvedValue,
        shape: &ValueShape,
        required: bool,
        property: &str,
        span: SourceSpan,
    ) {
        if resolved_value_contains_invalid(value) {
            return;
        }
        if matches!(shape, ValueShape::ChannelConfig) {
            self.validate_value_shape(value, shape, property, span);
            return;
        }
        if matches!(value, ResolvedValue::None) {
            if required {
                self.error(
                    "AVENGER-RESOLVE-194",
                    "required channel cannot be absent",
                    span,
                    format!("`{property}` requires an encoded or direct value"),
                );
            }
            return;
        }
        if matches!(shape, ValueShape::RasterDimensionChannel) {
            self.validate_value_shape(value, shape, property, span);
            return;
        }
        if matches!(shape, ValueShape::PatternChannel) && matches!(value, ResolvedValue::Pattern(_))
        {
            return;
        }

        let (fallback, properties, children) = match value {
            ResolvedValue::Channel { mode, expression } => {
                self.validate_value_shape(expression, &ValueShape::SqlExpression, property, span);
                ((*mode, expression.as_ref()), None, &[][..])
            }
            ResolvedValue::Object {
                head: Some(head),
                kind: None,
                properties,
                children,
            } => {
                let ResolvedValue::Channel { mode, expression } = head.as_ref() else {
                    self.error(
                        "AVENGER-RESOLVE-195",
                        "channel mode must be explicit",
                        span,
                        format!("prefix `{property}` with `encoded` or `direct`"),
                    );
                    return;
                };
                self.validate_value_shape(expression, &ValueShape::SqlExpression, property, span);
                (
                    (*mode, expression.as_ref()),
                    Some(properties),
                    children.as_slice(),
                )
            }
            _ => {
                self.error(
                    "AVENGER-RESOLVE-195",
                    "channel mode must be explicit",
                    span,
                    format!("use `encoded <expression>` or `direct <expression>` for `{property}`"),
                );
                return;
            }
        };

        let mut has_encoded = fallback.0 == crate::ast::ChannelMode::Encoded;
        let mut has_otherwise = false;
        if let Some(properties) = properties
            && let Some(otherwise) = properties.get("otherwise")
        {
            has_otherwise = true;
            has_encoded = self
                .validate_conditional_channel_branch(otherwise, "otherwise", property, span)
                .is_some_and(|mode| mode == crate::ast::ChannelMode::Encoded);
        }
        for child in children {
            if child.keyword.as_str() != "when" {
                self.error(
                    "AVENGER-RESOLVE-196",
                    "invalid channel child declaration",
                    child.span,
                    "channel bodies only accept ordered `when` declarations",
                );
                continue;
            }
            if !child.properties.contains_key("predicate") {
                self.error(
                    "AVENGER-RESOLVE-197",
                    "conditional channel branch is missing its predicate",
                    child.span,
                    "add `predicate: <boolean expression>;`",
                );
            } else if let Some(predicate) = child.properties.get("predicate") {
                self.validate_value_shape(
                    predicate,
                    &ValueShape::SqlExpression,
                    "predicate",
                    child.span,
                );
            }
            let branch = ResolvedValue::Object {
                head: None,
                kind: None,
                properties: child.properties.clone(),
                children: Vec::new(),
            };
            if self
                .validate_conditional_channel_branch(&branch, "when", property, child.span)
                .is_some_and(|mode| mode == crate::ast::ChannelMode::Encoded)
            {
                has_encoded = true;
            }
        }

        if !has_encoded
            && let Some(properties) = properties
            && let Some(name) = properties.keys().find(|name| {
                matches!(
                    name.as_str(),
                    "scale" | "axis" | "legend" | "domain_contribution" | "band"
                )
            })
        {
            self.error(
                "AVENGER-RESOLVE-198",
                "direct channel cannot configure encoding policy",
                span,
                format!(
                    "`{name}:` has no encoded effective branch{}",
                    if has_otherwise {
                        " after `otherwise` replaces the channel head"
                    } else {
                        ""
                    }
                ),
            );
        }
    }

    fn normalize_channels_for_shape(&self, value: &mut ResolvedValue, shape: &ValueShape) {
        match (value, shape) {
            (ResolvedValue::Object { properties, .. }, ValueShape::ChannelMap) => {
                for value in properties.values_mut() {
                    self.normalize_resolved_channel(value);
                }
            }
            (ResolvedValue::Array(values), ValueShape::OneOrMany(inner)) => {
                for value in values {
                    self.normalize_channels_for_shape(value, inner);
                }
            }
            (value, ValueShape::OneOrMany(inner)) => {
                self.normalize_channels_for_shape(value, inner);
            }
            _ => {}
        }
    }

    fn normalize_resolved_channel(&self, value: &mut ResolvedValue) {
        let normalized = match value {
            ResolvedValue::Channel { mode, expression } => Some(ResolvedChannelValue {
                head: ResolvedChannelBranch {
                    mode: *mode,
                    expression: expression.clone(),
                },
                conditions: Vec::new(),
                otherwise: None,
                configuration: BTreeMap::new(),
            }),
            ResolvedValue::Object {
                head: Some(head),
                kind: None,
                properties,
                children,
            } => {
                let ResolvedValue::Channel { mode, expression } = head.as_ref() else {
                    return;
                };
                let head = ResolvedChannelBranch {
                    mode: *mode,
                    expression: expression.clone(),
                };
                let otherwise = properties
                    .get("otherwise")
                    .and_then(resolved_channel_branch);
                let conditions = children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "when")
                    .filter_map(|child| {
                        Some(ResolvedChannelCondition {
                            predicate: Box::new(child.properties.get("predicate")?.clone()),
                            branch: resolved_channel_branch_from_properties(&child.properties)?,
                            span: child.span,
                        })
                    })
                    .collect();
                Some(ResolvedChannelValue {
                    head,
                    conditions,
                    otherwise,
                    configuration: properties
                        .iter()
                        .filter(|(name, _)| name.as_str() != "otherwise")
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                })
            }
            _ => None,
        };
        if let Some(normalized) = normalized {
            *value = ResolvedValue::ChannelValue(normalized);
        }
    }

    fn validate_conditional_channel_branch(
        &mut self,
        value: &ResolvedValue,
        branch: &str,
        channel: &str,
        span: SourceSpan,
    ) -> Option<crate::ast::ChannelMode> {
        let ResolvedValue::Object {
            head: None,
            kind: None,
            properties,
            children,
        } = value
        else {
            self.error(
                "AVENGER-RESOLVE-199",
                "conditional channel branch must be a property block",
                span,
                format!("`{branch}` on `{channel}` requires `encoded:` or `direct:`"),
            );
            return None;
        };
        if !children.is_empty() {
            self.error(
                "AVENGER-RESOLVE-199",
                "conditional channel branch cannot contain declarations",
                span,
                format!("remove declarations from `{branch}` on `{channel}`"),
            );
        }
        let encoded = properties.get("encoded");
        let direct = properties.get("direct");
        let removed_mode_count = usize::from(properties.contains_key("scaled"))
            + usize::from(properties.contains_key("value"));
        let mode = match (encoded, direct) {
            (Some(value), None) => Some((crate::ast::ChannelMode::Encoded, value)),
            (None, Some(value)) => Some((crate::ast::ChannelMode::Direct, value)),
            _ => {
                if removed_mode_count != 1 || encoded.is_some() || direct.is_some() {
                    self.error(
                        "AVENGER-RESOLVE-200",
                        "conditional channel branch requires exactly one mode",
                        span,
                        format!("`{branch}` on `{channel}` needs one of `encoded:` or `direct:`"),
                    );
                }
                None
            }
        };
        for name in properties.keys() {
            let allowed = matches!(name.as_str(), "encoded" | "direct")
                || (name.as_str() == "predicate" && branch == "when");
            if !allowed {
                let (message, note) = match name.as_str() {
                    "scaled" => (
                        "removed conditional channel mode `scaled`",
                        format!("replace `scaled:` with `encoded:` in `{branch}` on `{channel}`"),
                    ),
                    "value" => (
                        "removed conditional channel mode `value`",
                        format!("replace `value:` with `direct:` in `{branch}` on `{channel}`"),
                    ),
                    _ => (
                        "unknown conditional channel property",
                        format!("`{name}:` is not valid in `{branch}` on `{channel}`"),
                    ),
                };
                self.error("AVENGER-RESOLVE-201", message, span, note);
            }
        }
        if let Some((mode, expression)) = mode {
            self.validate_value_shape(expression, &ValueShape::SqlExpression, mode.as_str(), span);
            Some(mode)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn normalize_definition_arguments(
        &mut self,
        scope: ScopeId,
        source: &Value,
        resolved: &mut ResolvedValue,
        shape: &ValueShape,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
    ) {
        match shape {
            ValueShape::SqlProjection { .. } => {
                let Value::Projection(projection) = source else {
                    return;
                };
                let [
                    sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::Identifier(
                        identifier,
                    )),
                ] = projection.items()
                else {
                    return;
                };
                let path = vec![identifier.value.clone()];
                if let Some(target @ ResolvedTarget::DefinitionSlot { .. }) =
                    self.resolve_any_path(scope, &path, span, false)
                {
                    *resolved = ResolvedValue::DefinitionArgument(target);
                }
            }
            ValueShape::SqlExpression => {
                if let (
                    Value::Channel {
                        mode: source_mode,
                        expression: source_expression,
                    },
                    ResolvedValue::Channel {
                        mode: resolved_mode,
                        expression: resolved_expression,
                    },
                ) = (source, &mut *resolved)
                    && source_mode == resolved_mode
                {
                    self.normalize_definition_arguments(
                        scope,
                        source_expression,
                        resolved_expression,
                        &ValueShape::SqlExpression,
                        span,
                        in_event,
                        owner,
                    );
                } else if let (
                    Value::Block {
                        head: Some(source_head),
                        ..
                    },
                    ResolvedValue::Object {
                        head: Some(resolved_head),
                        ..
                    },
                ) = (source, &mut *resolved)
                {
                    self.normalize_definition_arguments(
                        scope,
                        source_head,
                        resolved_head,
                        &ValueShape::SqlExpression,
                        span,
                        in_event,
                        owner,
                    );
                } else if matches!(source, Value::Call { .. })
                    && let Ok(expression) =
                        crate::ast::SqlExpression::parse(&crate::print::print_value(source))
                {
                    *resolved = self.resolve_value(
                        scope,
                        &Value::Expr(Box::new(expression)),
                        span,
                        in_event,
                        owner,
                    );
                }
                self.normalize_expression_argument(scope, resolved, span);
            }
            ValueShape::ConfiguredExpression(fields) => {
                if let (
                    Value::Block {
                        head: Some(source_head),
                        body,
                    },
                    ResolvedValue::Object {
                        head: Some(resolved_head),
                        properties,
                        ..
                    },
                ) = (source, resolved)
                {
                    self.normalize_definition_arguments(
                        scope,
                        source_head,
                        resolved_head,
                        &ValueShape::SqlExpression,
                        span,
                        in_event,
                        owner,
                    );
                    for (name, field) in fields {
                        if let (Some(source), Some(value)) =
                            (body.props.get(name), properties.get_mut(name))
                        {
                            self.normalize_definition_arguments(
                                scope,
                                source,
                                value,
                                &field.shape,
                                span,
                                in_event,
                                owner,
                            );
                        }
                    }
                }
            }
            ValueShape::ConfiguredReference {
                namespaces,
                properties: fields,
            } => {
                if let Value::Block {
                    head: Some(source_head),
                    body,
                } = source
                    && let ResolvedValue::Object {
                        head,
                        kind,
                        properties,
                        ..
                    } = resolved
                {
                    if head.is_none()
                        && let Some(atom) = kind.take()
                    {
                        *head = Some(Box::new(ResolvedValue::Atom(atom)));
                    }
                    let Some(resolved_head) = head.as_deref_mut() else {
                        return;
                    };
                    self.normalize_definition_arguments(
                        scope,
                        source_head,
                        resolved_head,
                        &ValueShape::TypedReference {
                            namespaces: namespaces.clone(),
                        },
                        span,
                        in_event,
                        owner,
                    );
                    for (name, field) in fields {
                        if let (Some(source), Some(value)) =
                            (body.props.get(name), properties.get_mut(name))
                        {
                            self.normalize_definition_arguments(
                                scope,
                                source,
                                value,
                                &field.shape,
                                span,
                                in_event,
                                owner,
                            );
                        }
                    }
                }
            }
            ValueShape::SelectionBinding => {
                if let Value::Atom(name) = source {
                    let authored_path = vec![name.to_string()];
                    *resolved = self
                        .resolve_typed_reference_path(
                            scope,
                            &authored_path,
                            RefKind::Selection,
                            span,
                        )
                        .map(|target| {
                            ResolvedValue::Reference(ResolvedReference {
                                target,
                                kind: RefKind::Selection,
                                authored_path,
                            })
                        })
                        .unwrap_or(ResolvedValue::Invalid);
                }
            }
            ValueShape::TypedReference { namespaces } => {
                if let Value::Atom(name) = source {
                    let kinds = namespaces
                        .iter()
                        .filter_map(native_namespace_ref_kind)
                        .collect::<Vec<_>>();
                    if let Some(kind) = kinds
                        .first()
                        .copied()
                        .filter(|kind| kinds.iter().all(|candidate| candidate == kind))
                    {
                        let authored_path = vec![name.to_string()];
                        *resolved = self
                            .resolve_typed_reference_path(scope, &authored_path, kind, span)
                            .map(|target| {
                                ResolvedValue::Reference(ResolvedReference {
                                    target,
                                    kind,
                                    authored_path,
                                })
                            })
                            .unwrap_or(ResolvedValue::Invalid);
                    }
                }
            }
            ValueShape::Array(inner) => {
                if let (Value::Array(sources), ResolvedValue::Array(values)) = (source, resolved) {
                    for (source, value) in sources.iter().zip(values) {
                        self.normalize_definition_arguments(
                            scope, source, value, inner, span, in_event, owner,
                        );
                    }
                }
            }
            ValueShape::OneOrMany(inner) => {
                if let (Value::Array(sources), ResolvedValue::Array(values)) =
                    (source, &mut *resolved)
                {
                    for (source, value) in sources.iter().zip(values) {
                        self.normalize_definition_arguments(
                            scope, source, value, inner, span, in_event, owner,
                        );
                    }
                } else {
                    self.normalize_definition_arguments(
                        scope, source, resolved, inner, span, in_event, owner,
                    );
                }
            }
            ValueShape::Object(fields) => {
                if let (Value::Block { body, .. }, ResolvedValue::Object { properties, .. }) =
                    (source, resolved)
                {
                    for (name, field) in fields {
                        if let (Some(source), Some(value)) =
                            (body.props.get(name), properties.get_mut(name))
                        {
                            self.normalize_definition_arguments(
                                scope,
                                source,
                                value,
                                &field.shape,
                                span,
                                in_event,
                                owner,
                            );
                        }
                    }
                }
            }
            ValueShape::Map(inner) => {
                if let (Value::Block { body, .. }, ResolvedValue::Object { properties, .. }) =
                    (source, resolved)
                {
                    for (name, source) in body.props.iter() {
                        if let Some(value) = properties.get_mut(name.as_str()) {
                            self.normalize_definition_arguments(
                                scope, source, value, inner, span, in_event, owner,
                            );
                        }
                    }
                }
            }
            ValueShape::ChannelMap => {
                if let (Value::Block { body, .. }, ResolvedValue::Object { properties, .. }) =
                    (source, resolved)
                {
                    for (name, source) in body.props.iter() {
                        if let Some(value) = properties.get_mut(name.as_str()) {
                            self.normalize_definition_arguments(
                                scope,
                                source,
                                value,
                                &ValueShape::SqlExpression,
                                span,
                                in_event,
                                owner,
                            );
                        }
                    }
                }
            }
            ValueShape::Union(shapes) => {
                if let Some(shape) = shapes
                    .iter()
                    .find(|shape| value_matches_shape(resolved, shape))
                {
                    self.normalize_definition_arguments(
                        scope, source, resolved, shape, span, in_event, owner,
                    );
                }
            }
            _ => {}
        }
    }

    fn normalize_expression_argument(
        &mut self,
        scope: ScopeId,
        value: &mut ResolvedValue,
        span: SourceSpan,
    ) {
        match value {
            ResolvedValue::Atom(name) => {
                let path = vec![name.clone()];
                if let Some(
                    target @ (ResolvedTarget::DefinitionSlot { .. }
                    | ResolvedTarget::DefinitionChannel { .. }),
                ) = self.resolve_any_path(scope, &path, span, false)
                {
                    *value = ResolvedValue::DefinitionArgument(target);
                }
            }
            ResolvedValue::Call { args, .. } | ResolvedValue::Array(args) => {
                for argument in args {
                    self.normalize_expression_argument(scope, argument, span);
                }
            }
            ResolvedValue::Channel {
                expression: value, ..
            }
            | ResolvedValue::Pattern(value) => {
                self.normalize_expression_argument(scope, value, span);
            }
            ResolvedValue::Object {
                head,
                properties,
                children,
                ..
            } => {
                if let Some(head) = head {
                    self.normalize_expression_argument(scope, head, span);
                }
                for value in properties.values_mut() {
                    self.normalize_expression_argument(scope, value, span);
                }
                for child in children {
                    for value in child.properties.values_mut() {
                        self.normalize_expression_argument(scope, value, span);
                    }
                }
            }
            _ => {}
        }
    }

    fn validate_object_fields(
        &mut self,
        values: &BTreeMap<String, ResolvedValue>,
        fields: &BTreeMap<String, PropertySchema>,
        property: &str,
        span: SourceSpan,
    ) {
        for name in values.keys() {
            if !fields.contains_key(name) {
                self.error(
                    "AVENGER-RESOLVE-051",
                    "unknown object field",
                    span,
                    format!("`{property}` has no field `{name}`"),
                );
            }
        }
        for (name, field) in fields {
            if field.required && !values.contains_key(name) {
                self.error(
                    "AVENGER-RESOLVE-052",
                    "missing required object field",
                    span,
                    format!("`{property}` requires field `{name}:`"),
                );
            }
            if let Some(value) = values.get(name) {
                self.validate_value_shape(value, &field.shape, name, span);
            }
        }
    }

    fn validate_definition_value(
        &mut self,
        value: &ResolvedValue,
        slot: &DefinitionSlot,
        property: &str,
        span: SourceSpan,
    ) {
        if resolved_value_contains_invalid(value) {
            return;
        }
        if matches!(
            value,
            ResolvedValue::DefinitionArgument(ResolvedTarget::DefinitionSlot { .. })
        ) {
            return;
        }
        let valid = match slot.shape.as_str() {
            "expr" => is_expression_value(value),
            "expr_list" => {
                matches!(value, ResolvedValue::Array(values) if values.iter().all(is_expression_value))
            }
            "literal" => is_literal_value(value),
            "number" => matches!(
                value,
                ResolvedValue::Number(_) | ResolvedValue::Expression(_) | ResolvedValue::Binding(_)
            ),
            "string" => matches!(
                value,
                ResolvedValue::String(_) | ResolvedValue::Expression(_) | ResolvedValue::Binding(_)
            ),
            "boolean" => matches!(
                value,
                ResolvedValue::Boolean(_)
                    | ResolvedValue::Expression(_)
                    | ResolvedValue::Binding(_)
            ),
            "enum" => {
                matches!(value, ResolvedValue::Atom(atom) if slot.enum_values.iter().any(|candidate| candidate == atom))
            }
            "ref" => {
                matches!(
                    value,
                    ResolvedValue::Reference(reference)
                        if slot.reference_kind.as_deref().is_none_or(|kind| {
                            target_matches_definition_ref_kind(&reference.target, kind)
                        })
                ) || matches!(
                    value,
                    ResolvedValue::Binding(binding)
                        if slot.reference_kind.as_deref().is_some_and(|kind| {
                            target_matches_definition_ref_kind(&binding.target, kind)
                        })
                )
            }
            "block" => matches!(value, ResolvedValue::Object { .. }),
            "outputs" => value_matches_shape(
                value,
                &ValueShape::SqlProjection {
                    policy: avenger_chart_schema::ProjectionPolicy::Named,
                    expression_mode: avenger_chart_schema::ProjectionExpressionMode::Scalar,
                },
            ),
            _ => false,
        };
        if !valid {
            self.error(
                "AVENGER-RESOLVE-053",
                "definition slot has the wrong shape",
                span,
                format!("`{property}` expects slot shape `{}`", slot.shape),
            );
        }
    }

    fn validate_definition_channel(
        &mut self,
        value: &ResolvedValue,
        channel: &DefinitionChannel,
        coordinate: Option<&str>,
        property: &str,
        span: SourceSpan,
    ) {
        let Some(physical) = (match value {
            ResolvedValue::Atom(value) => Some(value.as_str()),
            _ => None,
        }) else {
            self.error(
                "AVENGER-RESOLVE-054",
                "definition channel argument has the wrong shape",
                span,
                format!("`{property}` requires a bare physical channel name"),
            );
            return;
        };
        if let Some(coordinate) = coordinate
            && !self.registry.entries.values().any(|schema| {
                schema.key.namespace == NativeKindNamespace::Mark
                    && schema.key.coordinate.as_deref() == Some(coordinate)
                    && schema.channels.contains_key(physical)
            })
        {
            self.error(
                "AVENGER-RESOLVE-055",
                "definition channel is incompatible with the coordinate system",
                span,
                format!("`{physical}` is not a registered `{coordinate}` channel"),
            );
        }
        let _ = channel;
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_definition_ref_value(
        &mut self,
        scope: ScopeId,
        value: &Value,
        slot: &DefinitionSlot,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
    ) -> ResolvedValue {
        if let Value::Atom(name) = value
            && let Some(kind) = slot.reference_kind.as_deref().and_then(definition_ref_kind)
        {
            let authored_path = vec![name.to_string()];
            return self
                .resolve_typed_reference_path(scope, &authored_path, kind, span)
                .map(|target| {
                    ResolvedValue::Reference(ResolvedReference {
                        target,
                        kind,
                        authored_path,
                    })
                })
                .unwrap_or(ResolvedValue::Invalid);
        }
        self.resolve_value(scope, value, span, in_event, owner)
    }

    fn resolve_value(
        &mut self,
        scope: ScopeId,
        value: &Value,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
    ) -> ResolvedValue {
        match value {
            Value::Str(value) => ResolvedValue::String(value.clone()),
            Value::Num(value) => ResolvedValue::Number(value.as_str().to_owned()),
            Value::Bool(value) => ResolvedValue::Boolean(*value),
            Value::Null => ResolvedValue::Null,
            Value::Column(value) => ResolvedValue::Column(value.clone()),
            Value::Atom(value) => ResolvedValue::Atom(value.to_string()),
            Value::Expr(expression) => {
                let bindings = expression
                    .bindings()
                    .iter()
                    .filter_map(|binding| {
                        self.resolve_sql_binding(
                            scope,
                            binding.kind,
                            &binding.path,
                            binding.time,
                            span,
                            in_event,
                            owner,
                        )
                    })
                    .collect();
                let sql = expression.canonical_sql();
                let references =
                    self.resolve_sql_paths(scope, expression_paths(expression.ast()), span, true);
                let contextual_accesses = self.resolve_contextual_accesses(
                    scope,
                    expression.ast(),
                    span,
                    in_event,
                    owner,
                );
                let helpers = self.resolve_helpers(
                    scope,
                    helper_calls(expression.ast()),
                    span,
                    in_event,
                    owner,
                );
                ResolvedValue::Expression(ResolvedExpression {
                    helpers,
                    sql,
                    bindings,
                    references,
                    contextual_accesses,
                })
            }
            Value::Projection(projection) => {
                let bindings = projection
                    .bindings()
                    .iter()
                    .filter_map(|binding| {
                        self.resolve_sql_binding(
                            scope,
                            binding.kind,
                            &binding.path,
                            binding.time,
                            span,
                            in_event,
                            owner,
                        )
                    })
                    .collect::<Vec<_>>();
                let items = projection
                    .items()
                    .iter()
                    .map(|item| {
                        let (expression, aliases, aliases_quoted) = match item {
                            sqlparser::ast::SelectItem::UnnamedExpr(expression) => {
                                (Some(expression), Vec::new(), Vec::new())
                            }
                            sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => (
                                Some(expr),
                                vec![alias.value.clone()],
                                vec![alias.quote_style.is_some()],
                            ),
                            sqlparser::ast::SelectItem::ExprWithAliases { expr, aliases } => (
                                Some(expr),
                                aliases.iter().map(|alias| alias.value.clone()).collect(),
                                aliases
                                    .iter()
                                    .map(|alias| alias.quote_style.is_some())
                                    .collect(),
                            ),
                            sqlparser::ast::SelectItem::QualifiedWildcard(_, _)
                            | sqlparser::ast::SelectItem::Wildcard(_) => {
                                (None, Vec::new(), Vec::new())
                            }
                        };
                        let direct_column = expression.is_some_and(|expression| {
                            matches!(
                                expression,
                                sqlparser::ast::Expr::Identifier(_)
                                    | sqlparser::ast::Expr::CompoundIdentifier(_)
                            )
                        });
                        let expression = expression.map(|expression| ResolvedExpression {
                            sql: crate::ast::restore_bindings(
                                expression.to_string(),
                                projection.bindings(),
                            ),
                            bindings: bindings.clone(),
                            helpers: self.resolve_helpers(
                                scope,
                                helper_calls(expression),
                                span,
                                in_event,
                                owner,
                            ),
                            references: self.resolve_sql_paths(
                                scope,
                                expression_paths(expression),
                                span,
                                true,
                            ),
                            contextual_accesses: Vec::new(),
                        });
                        ResolvedProjectionItem {
                            sql: crate::ast::restore_bindings(
                                item.to_string(),
                                projection.bindings(),
                            ),
                            expression,
                            aliases,
                            aliases_quoted,
                            direct_column,
                        }
                    })
                    .collect();
                ResolvedValue::Projection(ResolvedProjection {
                    sql: projection.canonical_sql(),
                    items,
                })
            }
            Value::Query(query) => {
                let bindings = query
                    .bindings()
                    .iter()
                    .filter_map(|binding| {
                        self.resolve_sql_binding(
                            scope,
                            binding.kind,
                            &binding.path,
                            binding.time,
                            span,
                            in_event,
                            owner,
                        )
                    })
                    .collect();
                let sql = query.canonical_sql();
                let references =
                    self.resolve_sql_paths(scope, query_paths(query.ast()), span, false);
                let store_placeholders = query
                    .bindings()
                    .iter()
                    .filter(|binding| binding.kind == BindingKind::Store)
                    .map(|binding| binding.synthetic_identifier.as_str())
                    .collect::<BTreeSet<_>>();
                let relation_paths = relation_paths(query.ast())
                    .into_iter()
                    .filter(|path| {
                        !matches!(
                            path.as_slice(),
                            [name] if store_placeholders.contains(name.as_str())
                        )
                    })
                    .collect();
                let relations = self.resolve_query_relations(scope, relation_paths, span, owner);
                let helpers = self.resolve_helpers(
                    scope,
                    query_helper_calls(query.ast()),
                    span,
                    in_event,
                    owner,
                );
                ResolvedValue::Query(ResolvedQuery {
                    helpers,
                    sql,
                    bindings,
                    references,
                    relations,
                })
            }
            Value::Relation(path) => {
                let authored_path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
                let Some(module) = self.scopes[scope.0].module.clone() else {
                    return ResolvedValue::Invalid;
                };
                self.resolve_relation_path(&module, &authored_path, false, span)
                    .map(|target| {
                        ResolvedValue::Relation(ResolvedRelationReference {
                            authored_path,
                            target,
                        })
                    })
                    .unwrap_or(ResolvedValue::Invalid)
            }
            Value::Binding { kind, path, time } => self
                .resolve_sql_binding(scope, *kind, path, *time, span, in_event, owner)
                .map(ResolvedValue::Binding)
                .unwrap_or(ResolvedValue::Invalid),
            Value::Ref { kind, path } => self
                .resolve_reference(scope, *kind, path, span)
                .map(ResolvedValue::Reference)
                .unwrap_or(ResolvedValue::Invalid),
            Value::Channel { mode, expression } => ResolvedValue::Channel {
                mode: *mode,
                expression: Box::new(self.resolve_value(scope, expression, span, in_event, owner)),
            },
            Value::Dim(path) => self
                .resolve_dimension(scope, path, span)
                .map(ResolvedValue::Dimension)
                .unwrap_or(ResolvedValue::Invalid),
            Value::Pattern(value) => ResolvedValue::Pattern(Box::new(
                self.resolve_value(scope, value, span, in_event, owner),
            )),
            Value::Env(value) => ResolvedValue::Environment(value.clone()),
            Value::None => ResolvedValue::None,
            Value::Array(values) => ResolvedValue::Array(
                values
                    .iter()
                    .map(|value| self.resolve_value(scope, value, span, in_event, owner))
                    .collect(),
            ),
            Value::Block { head, body } => ResolvedValue::Object {
                kind: head.as_deref().and_then(value_atom).map(str::to_owned),
                head: head
                    .as_deref()
                    .filter(|head| value_atom(head).is_none())
                    .map(|head| Box::new(self.resolve_value(scope, head, span, in_event, owner))),
                properties: body
                    .props
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.to_string(),
                            self.resolve_value(scope, value, span, in_event, owner),
                        )
                    })
                    .collect(),
                // Inline block declarations (notably widget action bodies) are
                // resolved by event validation and remain ordered here.
                children: body
                    .children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        self.resolve_inline_declaration(
                            scope,
                            child,
                            span,
                            index,
                            in_event,
                            owner.keyword.as_str() == "widget",
                        )
                    })
                    .collect(),
            },
            Value::Call { function, args } => ResolvedValue::Call {
                function: {
                    let name = function.as_str().to_ascii_lowercase();
                    if is_removed_contextual_call(&name) {
                        let (code, replacement) = legacy_contextual_replacement(&name);
                        self.error(
                            code,
                            "function-style contextual access was removed",
                            span,
                            replacement,
                        );
                    }
                    function.to_string()
                },
                args: args
                    .iter()
                    .map(|value| self.resolve_value(scope, value, span, in_event, owner))
                    .collect(),
            },
        }
    }

    fn resolve_contextual_accesses(
        &mut self,
        scope: ScopeId,
        expression: &Expr,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
    ) -> Vec<ResolvedContextualAccess> {
        let (scope_is_event, scope_has_between) = self.scope_event_context(scope);
        let in_event = in_event || scope_is_event;
        let item_effect = matches!(owner.keyword.as_str(), "adjust" | "derive");
        let uses = contextual_uses(expression);
        let mut output = Vec::new();
        for contextual_use in uses {
            let kind = match contextual_use {
                RawContextualUse::DatumField { field } if in_event => {
                    Some(ResolvedContextualAccessKind::DatumField { field })
                }
                RawContextualUse::DatumField { .. } => {
                    self.error(
                        "AVENGER-RESOLVE-110",
                        "event datum is outside an event context",
                        span,
                        "`datum.\"field\"` requires an event binding or event effect",
                    );
                    None
                }
                RawContextualUse::MarkChannel { channel } if owner.keyword.as_str() == "mark" => {
                    self.resolve_contextual_channel(scope, owner, "channel", &channel, span)
                        .map(|channel| ResolvedContextualAccessKind::MarkChannel { channel })
                }
                RawContextualUse::MarkChannel { .. } => {
                    self.error(
                        "AVENGER-RESOLVE-111",
                        "channel access is outside a mark channel",
                        span,
                        "`channel.<name>` requires a mark encoding context",
                    );
                    None
                }
                RawContextualUse::EventCoord { channel } if in_event => self
                    .resolve_contextual_channel(scope, owner, "event_coord", &channel, span)
                    .map(|channel| ResolvedContextualAccessKind::EventCoord { channel }),
                RawContextualUse::EventStartCoord { channel } if in_event && scope_has_between => {
                    self.resolve_contextual_channel(scope, owner, "start_coord", &channel, span)
                        .map(|channel| ResolvedContextualAccessKind::EventStartCoord { channel })
                }
                RawContextualUse::EventStartCoord { .. } if in_event => {
                    self.error(
                        "AVENGER-RESOLVE-132",
                        "gesture-start access requires a between interaction",
                        span,
                        "`event.start.coord.<channel>` has no start event in this binding",
                    );
                    None
                }
                RawContextualUse::EventDomainBoundary { channel, boundary } if in_event => self
                    .resolve_contextual_channel(scope, owner, "event_domain", &channel, span)
                    .map(
                        |channel| ResolvedContextualAccessKind::EventDomainBoundary {
                            channel,
                            boundary,
                        },
                    ),
                RawContextualUse::EventPath if in_event && scope_has_between => {
                    Some(ResolvedContextualAccessKind::EventPath)
                }
                RawContextualUse::EventPath if in_event => {
                    self.error(
                        "AVENGER-RESOLVE-132",
                        "gesture-start access requires a between interaction",
                        span,
                        "`event.path` has no start event in this binding",
                    );
                    None
                }
                RawContextualUse::EventFacet { one_based_index } if in_event => {
                    Some(ResolvedContextualAccessKind::EventFacet { one_based_index })
                }
                RawContextualUse::EventLegendValue if in_event => {
                    Some(ResolvedContextualAccessKind::EventLegendValue)
                }
                RawContextualUse::EventCoord { .. }
                | RawContextualUse::EventStartCoord { .. }
                | RawContextualUse::EventDomainBoundary { .. }
                | RawContextualUse::EventPath
                | RawContextualUse::EventFacet { .. }
                | RawContextualUse::EventLegendValue => {
                    self.error(
                        "AVENGER-RESOLVE-110",
                        "event access is outside an event context",
                        span,
                        "the `event` contextual namespace requires an event binding or effect",
                    );
                    None
                }
                RawContextualUse::ItemChannel { channel } if item_effect => {
                    let resolved = self.resolve_contextual_channel(
                        scope,
                        owner,
                        "item_channel",
                        &channel,
                        span,
                    );
                    let physical_type = self.item_channel_physical_type(scope, &channel);
                    match (resolved, physical_type) {
                        (Some(channel), Some(physical_type)) => {
                            Some(ResolvedContextualAccessKind::ItemChannel {
                                channel,
                                physical_type,
                            })
                        }
                        (Some(_), None) => {
                            self.error(
                                "AVENGER-RESOLVE-191",
                                "item channel is not available in the evaluated item frame",
                                span,
                                format!(
                                    "`item.channel.{channel}` has no registered physical item type"
                                ),
                            );
                            None
                        }
                        _ => None,
                    }
                }
                RawContextualUse::ItemDataField { field } if item_effect => {
                    Some(ResolvedContextualAccessKind::ItemDataField { field })
                }
                RawContextualUse::ItemBbox { edge } if item_effect => {
                    Some(ResolvedContextualAccessKind::ItemBbox { edge })
                }
                RawContextualUse::ItemChannel { .. }
                | RawContextualUse::ItemDataField { .. }
                | RawContextualUse::ItemBbox { .. } => {
                    self.error(
                        "AVENGER-RESOLVE-185",
                        "item access is outside an item-frame context",
                        span,
                        "the `item` contextual namespace is valid only in `adjust` and `derive` expressions",
                    );
                    None
                }
                RawContextualUse::ViewField {
                    authored_view,
                    axis,
                    field,
                } => {
                    let target = self.resolve_any_path(scope, &authored_view, span, true);
                    match target {
                        Some(target @ ResolvedTarget::Declaration(_)) => {
                            Some(ResolvedContextualAccessKind::ViewField {
                                target,
                                authored_view,
                                axis,
                                field,
                            })
                        }
                        Some(_) => {
                            self.error(
                                "AVENGER-RESOLVE-112",
                                "inline-view access has the wrong target kind",
                                span,
                                format!(
                                    "`{}` is not an inline view binder",
                                    authored_view.join(".")
                                ),
                            );
                            None
                        }
                        None => None,
                    }
                }
                RawContextualUse::LegacyCall { name } => {
                    let (code, replacement) = legacy_contextual_replacement(&name);
                    self.error(
                        code,
                        "function-style contextual access was removed",
                        span,
                        replacement,
                    );
                    None
                }
                RawContextualUse::Invalid { root, detail } => {
                    self.error(
                        "AVENGER-RESOLVE-186",
                        "contextual access has an invalid shape",
                        span,
                        format!("invalid `{root}` access; {detail}"),
                    );
                    None
                }
            };
            if let Some(kind) = kind
                && output
                    .iter()
                    .all(|access: &ResolvedContextualAccess| access.kind != kind)
            {
                output.push(ResolvedContextualAccess { kind });
            }
        }
        output
    }

    fn resolve_contextual_channel(
        &mut self,
        scope: ScopeId,
        owner: &Decl,
        access: &str,
        channel: &str,
        span: SourceSpan,
    ) -> Option<ResolvedChannelMember> {
        if let Some((target, _)) = self.visible_definition_channel_property(scope, channel) {
            let family_suffix = match &target {
                ResolvedTarget::DefinitionChannel { name, .. } => channel
                    .strip_prefix(name)
                    .filter(|suffix| *suffix == "2")
                    .unwrap_or("")
                    .to_owned(),
                _ => String::new(),
            };
            return Some(ResolvedChannelMember::Definition {
                target,
                name: channel.to_owned(),
                family_suffix,
            });
        }
        if self.helper_channel_exists(scope, owner, access, channel) {
            Some(ResolvedChannelMember::Named {
                name: channel.to_owned(),
            })
        } else {
            self.error(
                "AVENGER-RESOLVE-154",
                "contextual access references an unknown channel",
                span,
                format!("`{channel}` is not a registered channel in this context"),
            );
            None
        }
    }

    fn item_channel_physical_type(&self, scope: ScopeId, channel: &str) -> Option<PhysicalType> {
        let coordinate = self.visible_coordinate(scope)?;
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(owner) = self.scopes[id.0].owner.as_ref()
                && let Some((_, declaration)) = self.declaration_source(owner)
                && declaration.keyword.as_str() == "mark"
                && let Some(kind) = declaration.kind.as_ref()
                && let Some(item_type) = self
                    .registry
                    .entries
                    .get(&NativeKindKey::mark(coordinate.clone(), kind.as_str()))
                    .and_then(|schema| schema.channels.get(channel))
                    .and_then(|channel| channel.item_type.as_deref())
            {
                return parse_type_text(item_type);
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_value_with_mark_blocks(
        &mut self,
        file: &ParsedModule,
        owner_path: &[usize],
        scope: ScopeId,
        value: &Value,
        span: SourceSpan,
        in_event: bool,
        owner: &Decl,
        block_ordinal: &mut usize,
    ) -> ResolvedValue {
        if !contains_overlay_mark_block(value) {
            return self.resolve_value(scope, value, span, in_event, owner);
        }
        match value {
            Value::Block { head, body } => {
                let resolved_head = head.as_deref().map(|head| {
                    Box::new(self.resolve_value_with_mark_blocks(
                        file,
                        owner_path,
                        scope,
                        head,
                        span,
                        in_event,
                        owner,
                        block_ordinal,
                    ))
                });
                let properties = body
                    .props
                    .iter()
                    .map(|(name, value)| {
                        let resolved = if name.as_str() == "overlay"
                            && matches!(value, Value::Block { .. })
                        {
                            let ordinal = *block_ordinal;
                            *block_ordinal += 1;
                            self.resolve_overlay_mark_block(file, owner_path, value, span, ordinal)
                        } else {
                            self.resolve_value_with_mark_blocks(
                                file,
                                owner_path,
                                scope,
                                value,
                                span,
                                in_event,
                                owner,
                                block_ordinal,
                            )
                        };
                        (name.to_string(), resolved)
                    })
                    .collect();
                ResolvedValue::Object {
                    kind: head.as_deref().and_then(value_atom).map(str::to_owned),
                    head: resolved_head.filter(|_| {
                        head.as_deref()
                            .is_some_and(|head| value_atom(head).is_none())
                    }),
                    properties,
                    children: body
                        .children
                        .iter()
                        .enumerate()
                        .map(|(index, child)| {
                            self.resolve_inline_declaration(
                                scope, child, span, index, in_event, false,
                            )
                        })
                        .collect(),
                }
            }
            Value::Array(values) => ResolvedValue::Array(
                values
                    .iter()
                    .map(|value| {
                        self.resolve_value_with_mark_blocks(
                            file,
                            owner_path,
                            scope,
                            value,
                            span,
                            in_event,
                            owner,
                            block_ordinal,
                        )
                    })
                    .collect(),
            ),
            Value::Call { function, args } => ResolvedValue::Call {
                function: function.to_string(),
                args: args
                    .iter()
                    .map(|value| {
                        self.resolve_value_with_mark_blocks(
                            file,
                            owner_path,
                            scope,
                            value,
                            span,
                            in_event,
                            owner,
                            block_ordinal,
                        )
                    })
                    .collect(),
            },
            Value::Channel { mode, expression } => ResolvedValue::Channel {
                mode: *mode,
                expression: Box::new(self.resolve_value_with_mark_blocks(
                    file,
                    owner_path,
                    scope,
                    expression,
                    span,
                    in_event,
                    owner,
                    block_ordinal,
                )),
            },
            Value::Pattern(value) => {
                ResolvedValue::Pattern(Box::new(self.resolve_value_with_mark_blocks(
                    file,
                    owner_path,
                    scope,
                    value,
                    span,
                    in_event,
                    owner,
                    block_ordinal,
                )))
            }
            _ => self.resolve_value(scope, value, span, in_event, owner),
        }
    }

    fn resolve_overlay_mark_block(
        &mut self,
        file: &ParsedModule,
        owner_path: &[usize],
        value: &Value,
        span: SourceSpan,
        block_ordinal: usize,
    ) -> ResolvedValue {
        let Value::Block { head, body } = value else {
            return ResolvedValue::Invalid;
        };
        if head.is_some() {
            self.error(
                "AVENGER-RESOLVE-177",
                "legend overlay block cannot have a value head",
                span,
                "use `overlay: { mark ... }`",
            );
        }
        if !body.props.is_empty() {
            self.error(
                "AVENGER-RESOLVE-178",
                "legend overlay block accepts only mark declarations",
                span,
                "move data and transforms into an inner `mark group`",
            );
        }
        if body.children.is_empty() {
            self.error(
                "AVENGER-RESOLVE-179",
                "legend overlay block is empty",
                span,
                "`overlay:` requires at least one mark",
            );
        }
        for child in &body.children {
            if child.keyword.as_str() != "mark" {
                self.error(
                    "AVENGER-RESOLVE-178",
                    "legend overlay block accepts only mark declarations",
                    span,
                    format!(
                        "move `{}` into an inner `mark group` or remove it",
                        child.keyword
                    ),
                );
            }
            if child.visibility != Visibility::Default {
                self.error(
                    "AVENGER-RESOLVE-180",
                    "legend overlay marks are local",
                    span,
                    "remove `public` or `private`; overlay declarations never publish paths",
                );
            }
        }

        let children = body
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let mut path = owner_path.to_vec();
                path.extend([MARK_BLOCK_PATH_SEGMENT, block_ordinal, index]);
                self.resolve_declaration(file, child, &path, Some("cartesian"), None, true, false)
            })
            .collect();
        ResolvedValue::Object {
            kind: None,
            head: None,
            properties: BTreeMap::new(),
            children,
        }
    }

    fn resolve_inline_declaration(
        &mut self,
        scope: ScopeId,
        declaration: &Decl,
        span: SourceSpan,
        index: usize,
        in_event: bool,
        widget_action: bool,
    ) -> ResolvedDeclaration {
        self.validate_core_declaration(declaration, span, in_event || widget_action);
        let id = DeclarationId(semantic_hash(&[
            "inline-declaration",
            &self.scopes[scope.0].label,
            &index.to_string(),
            declaration.keyword.as_str(),
        ]));
        let properties = declaration
            .props
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    self.resolve_value(scope, value, span, in_event, declaration),
                )
            })
            .collect();
        let mut resolved = ResolvedDeclaration {
            id,
            source: span.source,
            span,
            keyword: declaration.keyword.to_string(),
            kind: declaration.kind.as_ref().map(ToString::to_string),
            kind_binding: None,
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate: None,
            component_kind: declaration
                .props
                .get("component_kind")
                .and_then(value_atom)
                .map(str::to_owned),
            properties,
            relation_references: Vec::new(),
            property_channels: BTreeMap::new(),
            children: declaration
                .children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    self.resolve_inline_declaration(
                        scope,
                        child,
                        span,
                        index,
                        in_event,
                        widget_action,
                    )
                })
                .collect(),
            runtime_target: None,
            migration_key: None,
            definition_local_seed: None,
            public_path: None,
            parts: BTreeMap::new(),
            exports: BTreeMap::new(),
            transform_outputs: BTreeMap::new(),
            event_binding: None,
            state_lvalue: None,
        };
        if declaration.keyword.as_str() == "set" && widget_action {
            self.validate_action(declaration, &mut resolved, None, span, scope, true);
        }
        resolved
    }

    fn visible_definition_argument(&self, scope: ScopeId, name: &str) -> Option<ResolvedTarget> {
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(target) = self.scopes[id.0].definition_arguments.get(name) {
                return Some(target.clone());
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    fn definition_channel_schema(&self, target: &ResolvedTarget) -> Option<&DefinitionChannel> {
        let ResolvedTarget::DefinitionChannel { definition, name } = target else {
            return None;
        };
        self.definitions
            .values()
            .find(|schema| &schema.declaration == definition)
            .and_then(|schema| schema.channels.get(name))
    }

    fn definition_slot_schema(&self, target: &ResolvedTarget) -> Option<&DefinitionSlot> {
        let ResolvedTarget::DefinitionSlot { definition, name } = target else {
            return None;
        };
        self.definitions
            .values()
            .find(|schema| &schema.declaration == definition)
            .and_then(|schema| schema.slots.get(name))
    }

    fn visible_definition_channel_property(
        &self,
        scope: ScopeId,
        property: &str,
    ) -> Option<(ResolvedTarget, Option<String>)> {
        if let Some(target) = self.visible_definition_argument(scope, property)
            && let Some(channel) = self.definition_channel_schema(&target)
        {
            return Some((target, channel.physical_channel.clone()));
        }
        let logical = property.strip_suffix('2')?;
        let target = self.visible_definition_argument(scope, logical)?;
        let channel = self.definition_channel_schema(&target)?;
        let physical = channel
            .physical_channel
            .as_ref()
            .map(|physical| format!("{physical}2"));
        Some((target, physical))
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_sql_binding(
        &mut self,
        scope: ScopeId,
        kind: BindingKind,
        path: &[Name],
        time: BindingTime,
        span: SourceSpan,
        in_event: bool,
        _owner: &Decl,
    ) -> Option<ResolvedBinding> {
        let path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
        let target = self.resolve_binding_path(scope, &path, Some(kind), span, true)?;
        let (scope_is_event, scope_has_between) = self.scope_event_context(scope);
        let in_event = in_event || scope_is_event;
        if time != BindingTime::Current && !in_event {
            self.error(
                "AVENGER-RESOLVE-060",
                "temporal state read is outside an event binding",
                span,
                "`@start` and `@previous` are defined only for event invocations",
            );
        }
        if time == BindingTime::Start && !scope_has_between {
            self.error(
                "AVENGER-RESOLVE-061",
                "`@start` requires a between interaction",
                span,
                "the containing event must declare `between:`",
            );
        }
        if time != BindingTime::Current
            && matches!(
                target,
                ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. }
            )
        {
            self.error(
                "AVENGER-RESOLVE-062",
                "temporal qualifiers apply only to params",
                span,
                "stores are table bindings and have no `@start` or `@previous` scalar snapshot",
            );
        }
        Some(ResolvedBinding {
            target,
            kind,
            time,
            authored_path: path,
        })
    }

    fn resolve_binding_path(
        &mut self,
        scope: ScopeId,
        path: &[String],
        expected: Option<BindingKind>,
        span: SourceSpan,
        diagnose: bool,
    ) -> Option<ResolvedTarget> {
        let target = self.resolve_any_path(scope, path, span, diagnose)?;
        let actual = match target {
            ResolvedTarget::Param(_) | ResolvedTarget::DefinitionParam { .. } => BindingKind::Param,
            ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. } => BindingKind::Store,
            ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. } => {
                if diagnose {
                    self.error(
                        "AVENGER-RESOLVE-063",
                        "selection is not a SQL value binding",
                        span,
                        format!(
                            "`${}` resolves to selection state; use a typed selection reference",
                            path.join(".")
                        ),
                    );
                }
                return None;
            }
            _ => {
                if diagnose {
                    self.error(
                        "AVENGER-RESOLVE-063",
                        "path is not a param/store value binding",
                        span,
                        format!(
                            "`${}` does not resolve to scalar or table state",
                            path.join(".")
                        ),
                    );
                }
                return None;
            }
        };
        if expected.is_some_and(|expected| expected != actual) {
            if diagnose {
                self.error(
                    "AVENGER-RESOLVE-064",
                    "value binding has the wrong scalar/table role",
                    span,
                    format!(
                        "`${}` resolves as {actual:?}, not {:?}",
                        path.join("."),
                        expected.unwrap()
                    ),
                );
            }
            return None;
        }
        Some(target)
    }

    fn resolve_reference(
        &mut self,
        scope: ScopeId,
        kind: RefKind,
        path: &[Name],
        span: SourceSpan,
    ) -> Option<ResolvedReference> {
        let authored_path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
        let target = self.resolve_typed_reference_path(scope, &authored_path, kind, span)?;
        if !reference_kind_matches(&target, kind) {
            self.error(
                "AVENGER-RESOLVE-065",
                "typed reference resolves to the wrong target kind",
                span,
                format!(
                    "`{kind:?} {}` resolves to {target:?}",
                    authored_path.join(".")
                ),
            );
            return None;
        }
        Some(ResolvedReference {
            target,
            kind,
            authored_path,
        })
    }

    fn resolve_dimension(
        &mut self,
        scope: ScopeId,
        path: &[Name],
        span: SourceSpan,
    ) -> Option<ResolvedDimension> {
        let authored_path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
        let target = self.resolve_any_path(scope, &authored_path, span, true)?;
        match target {
            ResolvedTarget::Output(target)
                if target.shape == ResolvedOutputShape::RasterDimension =>
            {
                Some(ResolvedDimension {
                    target,
                    authored_path,
                })
            }
            ResolvedTarget::Output(target) => {
                self.error(
                    "AVENGER-RESOLVE-151",
                    "dimension path does not resolve to a raster dimension",
                    span,
                    format!(
                        "`dim {}` resolves to output `{}` with shape {:?}",
                        authored_path.join("."),
                        target.name,
                        target.shape
                    ),
                );
                None
            }
            _ => {
                self.error(
                    "AVENGER-RESOLVE-151",
                    "dimension path does not resolve to a raster dimension",
                    span,
                    format!(
                        "`dim {}` must name a registered raster-dimension transform output",
                        authored_path.join(".")
                    ),
                );
                None
            }
        }
    }

    fn resolve_typed_reference_path(
        &mut self,
        scope: ScopeId,
        path: &[String],
        kind: RefKind,
        span: SourceSpan,
    ) -> Option<ResolvedTarget> {
        let definition_argument = path.first().filter(|_| path.len() == 1).and_then(|name| {
            let target = self.visible_definition_argument(scope, name)?;
            let slot = self.definition_slot_schema(&target)?;
            (slot.shape == "ref"
                && slot.reference_kind.as_deref().and_then(definition_ref_kind) == Some(kind))
            .then_some(target)
        });
        let target = if definition_argument.is_some() {
            definition_argument
        } else if kind == RefKind::Selection && path.len() == 1 {
            let name = path.first()?;
            let mut cursor = Some(scope);
            let mut found = None;
            while let Some(id) = cursor {
                if let Some(symbol) = self.scopes[id.0].state_symbols.get(name) {
                    found = Some(symbol.target());
                    break;
                }
                cursor = self.scopes[id.0].parent;
            }
            found
        } else if path.len() == 1 {
            let name = path.first()?;
            let mut cursor = Some(scope);
            let mut found = None;
            while let Some(id) = cursor {
                if let Some(declaration) = self.scopes[id.0].structural.get(name) {
                    found = self
                        .declarations
                        .values()
                        .find(|info| &info.id == declaration)
                        .and_then(|info| info.runtime_target.clone())
                        .or_else(|| Some(ResolvedTarget::Declaration(declaration.clone())));
                    break;
                }
                cursor = self.scopes[id.0].parent;
            }
            found
        } else {
            self.resolve_qualified_structural_path(scope, path)
        };
        if target.is_none() {
            self.error(
                "AVENGER-RESOLVE-066",
                "unresolved authored path",
                span,
                format!(
                    "`{}` is not visible for a {kind:?} reference",
                    path.join(".")
                ),
            );
        }
        target
    }

    fn resolve_qualified_structural_path(
        &self,
        scope: ScopeId,
        path: &[String],
    ) -> Option<ResolvedTarget> {
        let first = path.first()?;
        let mut cursor = Some(scope);
        let mut current = loop {
            let id = cursor?;
            if let Some(found) = self.scopes[id.0].structural.get(first) {
                break found.clone();
            }
            cursor = self.scopes[id.0].parent;
        };
        for (index, segment) in path.iter().enumerate().skip(1) {
            let interface = self.instances.get(&current)?;
            let target = interface.exports.get(segment).cloned().or_else(|| {
                interface
                    .parts
                    .contains_key(segment)
                    .then(|| ResolvedTarget::Part {
                        declaration: current.clone(),
                        alias: segment.clone(),
                    })
            })?;
            if index == path.len() - 1 {
                return Some(target);
            }
            current = match &target {
                ResolvedTarget::Declaration(id) => id.clone(),
                _ => self
                    .declarations
                    .values()
                    .find(|info| info.runtime_target.as_ref() == Some(&target))?
                    .id
                    .clone(),
            };
        }
        None
    }

    fn resolve_any_path(
        &mut self,
        scope: ScopeId,
        path: &[String],
        span: SourceSpan,
        diagnose: bool,
    ) -> Option<ResolvedTarget> {
        let first = path.first()?;
        if path.len() == 1 {
            let mut cursor = Some(scope);
            while let Some(id) = cursor {
                if let Some(symbol) = self.scopes[id.0].state_symbols.get(first) {
                    return Some(symbol.target());
                }
                if let Some(target) = self.scopes[id.0].definition_arguments.get(first) {
                    return Some(target.clone());
                }
                if let Some(id) = self.scopes[id.0].structural.get(first) {
                    return self
                        .declarations
                        .values()
                        .find(|info| &info.id == id)
                        .and_then(|info| info.runtime_target.clone())
                        .or_else(|| Some(ResolvedTarget::Declaration(id.clone())));
                }
                cursor = self.scopes[id.0].parent;
            }
        } else if let Some(target) = self.resolve_qualified_structural_path(scope, path) {
            return Some(target);
        }
        if diagnose {
            self.error(
                "AVENGER-RESOLVE-066",
                "unresolved authored path",
                span,
                format!(
                    "`{}` is not visible from this lexical scope",
                    path.join(".")
                ),
            );
        }
        None
    }

    fn resolve_state_declaration(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        info: &DeclInfo,
        properties: &BTreeMap<String, ResolvedValue>,
    ) {
        let Some(name) = declaration.name.as_ref().map(ToString::to_string) else {
            return;
        };
        match declaration.keyword.as_str() {
            "param" => {
                let symbol = self.scopes[info.containing_scope.0]
                    .state_symbols
                    .get(&name)
                    .cloned();
                let Some(StateSymbol::Param(id)) = symbol else {
                    return;
                };
                let initializer = properties
                    .get("value")
                    .cloned()
                    .unwrap_or(ResolvedValue::Invalid);
                let sharing =
                    parse_sharing(properties.get("sharing"), info.span, &mut self.diagnostics);
                let (migration_key, definition_local_seed) =
                    self.state_identity(file, info, "param");
                let dependencies = resolved_param_dependencies(&initializer);
                let table_owner = self.table_owner(&info.id);
                if table_owner.is_some() && !is_self_contained_row_free(&initializer) {
                    self.error(
                        "AVENGER-RESOLVE-135",
                        "catalog-table param initializer must be a self-contained row-free expression",
                        info.span,
                        "table params may use scalar SQL but cannot read params, columns, relations, outputs, helpers, or contextual values",
                    );
                }
                self.param_dependencies.insert(
                    id.clone(),
                    if table_owner.is_some() {
                        BTreeSet::new()
                    } else {
                        dependencies
                    },
                );
                self.params.insert(
                    id.clone(),
                    ResolvedParam {
                        id,
                        declaration: info.id.clone(),
                        source_name: name,
                        type_contract: ParamTypeContract::Inferred,
                        initializer,
                        sharing,
                        migration_key,
                        definition_local_seed,
                        lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                        owner_ancestry: info.ancestry.clone(),
                        generated_by: None,
                        table_owner,
                    },
                );
            }
            "store" => self.resolve_store(file, declaration, info, &name, properties),
            "selection" => {
                let Some(StateSymbol::Selection(id)) = self.scopes[info.containing_scope.0]
                    .state_symbols
                    .get(&name)
                    .cloned()
                else {
                    return;
                };
                let (migration_key, definition_local_seed) =
                    self.state_identity(file, info, "selection");
                self.selections.insert(
                    id.clone(),
                    ResolvedSelection {
                        id,
                        declaration: info.id.clone(),
                        source_name: name,
                        empty: match properties.get("empty") {
                            Some(ResolvedValue::Atom(value)) if value == "all" => {
                                ResolvedSelectionEmpty::All
                            }
                            _ => ResolvedSelectionEmpty::None,
                        },
                        combine: match properties.get("combine") {
                            Some(ResolvedValue::Atom(value)) if value == "intersect" => {
                                ResolvedSelectionCombine::Intersect
                            }
                            _ => ResolvedSelectionCombine::Union,
                        },
                        migration_key,
                        definition_local_seed,
                        lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                        owner_ancestry: info.ancestry.clone(),
                        generated_by: None,
                    },
                );
            }
            _ => {}
        }
    }

    fn table_owner(&self, declaration: &DeclarationId) -> Option<DeclarationId> {
        let ((file_id, path), _) = self
            .declarations
            .iter()
            .find(|(_, info)| &info.id == declaration)?;
        if path.len() < 2 {
            return None;
        }
        let file = self.project.source_modules.get(file_id)?;
        let parent_path = &path[..path.len() - 1];
        let parent = declaration_at(file, parent_path)?;
        (parent.keyword.as_str() == "table").then(|| declaration_id(file, parent_path))
    }

    fn resolve_store(
        &mut self,
        file: &ParsedModule,
        declaration: &Decl,
        info: &DeclInfo,
        name: &str,
        properties: &BTreeMap<String, ResolvedValue>,
    ) {
        let Some(StateSymbol::Store(id)) = self.scopes[info.containing_scope.0]
            .state_symbols
            .get(name)
            .cloned()
        else {
            return;
        };
        let mut fields = Vec::new();
        let mut field_names = BTreeSet::new();
        for field in declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "field")
        {
            let Some(field_name) = field.name.as_ref().map(ToString::to_string) else {
                continue;
            };
            if field_name.starts_with("__avenger_store_") {
                self.error(
                    "AVENGER-RESOLVE-145",
                    "store field uses the reserved runtime prefix",
                    info.span,
                    "field names beginning with `__avenger_store_` are reserved",
                );
            }
            if !field_names.insert(field_name.clone()) {
                self.error(
                    "AVENGER-RESOLVE-070",
                    "duplicate store field",
                    info.span,
                    format!("field `{field_name}` is declared more than once"),
                );
                continue;
            }
            match field.props.get("type").map(PhysicalType::parse) {
                Some(Ok(data_type)) => fields.push(PhysicalField {
                    name: field_name,
                    data_type,
                    nullable: matches!(field.props.get("nullable"), Some(Value::Bool(true))),
                }),
                Some(Err(error)) => self.error(
                    "AVENGER-RESOLVE-071",
                    "invalid store field type",
                    info.span,
                    error.to_string(),
                ),
                None => self.error(
                    "AVENGER-RESOLVE-071",
                    "store field is missing a type",
                    info.span,
                    format!("field `{field_name}` requires `: <arrow_type>`"),
                ),
            }
        }
        let primary_key = resolved_string_array(properties.get("primary_key"));
        for key in &primary_key {
            match fields.iter().find(|field| &field.name == key) {
                None => self.error(
                    "AVENGER-RESOLVE-072",
                    "primary key references an unknown field",
                    info.span,
                    format!("primary-key field `{key}` is not declared"),
                ),
                Some(field) if field.nullable => self.error(
                    "AVENGER-RESOLVE-073",
                    "primary key field cannot be nullable",
                    info.span,
                    format!("field `{key}` is declared nullable"),
                ),
                Some(_) => {}
            }
        }
        let mut rows = Vec::new();
        let mut static_keys = BTreeSet::new();
        for row in declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "row")
        {
            let mut values = row
                .props
                .iter()
                .map(|(name, value)| (name.to_string(), unresolved_value(value)))
                .collect::<BTreeMap<_, _>>();
            for name in values.keys() {
                if !field_names.contains(name) {
                    self.error(
                        "AVENGER-RESOLVE-074",
                        "store row contains an unknown field",
                        info.span,
                        format!("row field `{name}` is not declared"),
                    );
                }
            }
            for field in &fields {
                match row.props.get(&field.name) {
                    Some(_) => {}
                    None if !field.nullable => self.error(
                        "AVENGER-RESOLVE-075",
                        "store row is missing a non-nullable field",
                        info.span,
                        format!("row requires field `{}`", field.name),
                    ),
                    None => {
                        values.insert(field.name.clone(), ResolvedValue::Null);
                    }
                }
            }
            for key in &primary_key {
                if matches!(row.props.get(key), Some(Value::Null)) {
                    self.error(
                        "AVENGER-RESOLVE-079",
                        "store primary-key value cannot be null",
                        info.span,
                        format!("row field `{key}` is part of the primary key"),
                    );
                }
            }
            if let Some(key) = static_store_key(row, &primary_key)
                && !static_keys.insert(key)
            {
                self.error(
                    "AVENGER-RESOLVE-125",
                    "store initial rows contain a duplicate primary key",
                    info.span,
                    "statically known key tuples must be unique",
                );
            }
            rows.push(values);
        }
        let (migration_key, definition_local_seed) = self.state_identity(file, info, "store");
        self.stores.insert(
            id.clone(),
            ResolvedStore {
                id,
                declaration: info.id.clone(),
                source_name: name.to_owned(),
                fields,
                primary_key,
                rows,
                sharing: parse_sharing(properties.get("sharing"), info.span, &mut self.diagnostics),
                migration_key,
                definition_local_seed,
                lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                owner_ancestry: info.ancestry.clone(),
                generated_by: None,
            },
        );
    }

    fn install_sequential_transform(
        &mut self,
        scope: ScopeId,
        source: &Decl,
        resolved: &ResolvedDeclaration,
        span: SourceSpan,
    ) {
        let Some(name) = source.name.as_ref().map(ToString::to_string) else {
            return;
        };
        if self.scopes[scope.0].structural.contains_key(&name) {
            self.error(
                "AVENGER-RESOLVE-080",
                "duplicate transform alias",
                span,
                format!("transform alias `{name}` is already visible"),
            );
            return;
        }
        self.scopes[scope.0]
            .structural
            .insert(name, resolved.id.clone());
        self.instances.insert(
            resolved.id.clone(),
            InstanceInterface {
                exports: resolved
                    .transform_outputs
                    .iter()
                    .map(|(name, output)| (name.clone(), ResolvedTarget::Output(output.clone())))
                    .collect(),
                ..InstanceInterface::default()
            },
        );
    }

    fn transform_outputs(
        &mut self,
        declaration: &Decl,
        info: &DeclInfo,
        children: &[ResolvedDeclaration],
        definition: Option<&DefinitionSchema>,
    ) -> BTreeMap<String, ResolvedOutputHandle> {
        let mut output_names = Vec::new();
        if let Some(schema) = declaration.kind.as_ref().and_then(|kind| {
            self.registry.entries.get(&NativeKindKey::new(
                NativeKindNamespace::Transform,
                kind.to_string(),
            ))
        }) {
            output_names.extend(
                schema
                    .outputs
                    .values()
                    .filter(|output| {
                        output
                            .condition_property
                            .as_ref()
                            .is_none_or(|property| declaration.props.get(property).is_some())
                    })
                    .map(|output| (output.name.clone(), resolved_output_shape(&output.shape))),
            );
        }
        if let Some(definition) = definition {
            output_names.extend(
                definition
                    .outputs
                    .keys()
                    .cloned()
                    .map(|name| (name, ResolvedOutputShape::Expression)),
            );
            for (name, slot) in &definition.slots {
                if slot.shape != "outputs" {
                    continue;
                }
                if let Some(Value::Projection(projection)) = declaration.props.get(name) {
                    output_names.extend(projection.items().iter().filter_map(|item| {
                        let sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } = item else {
                            return None;
                        };
                        Some((alias.value.clone(), ResolvedOutputShape::Expression))
                    }));
                }
            }
            if definition
                .slots
                .values()
                .any(|slot| slot.shape == "outputs")
                && declaration.name.is_none()
            {
                self.error(
                    "AVENGER-RESOLVE-169",
                    "dynamic-output transform instance requires a binder",
                    info.span,
                    "add `as <name>` so caller-authored output handles have a namespace",
                );
            }
        }
        if let Some(schema) = declaration.kind.as_ref().and_then(|kind| {
            self.registry.entries.get(&NativeKindKey::new(
                NativeKindNamespace::Transform,
                kind.to_string(),
            ))
        }) {
            for dynamic in &schema.dynamic_outputs {
                match &dynamic.source {
                    avenger_chart_schema::DynamicOutputSource::ArrayObjectField {
                        property,
                        field,
                    } => {
                        if let Some(Value::Array(values)) = declaration.props.get(property) {
                            output_names.extend(values.iter().filter_map(|value| {
                                let Value::Block { body, .. } = value else {
                                    return None;
                                };
                                match body.props.get(field) {
                                    Some(Value::Str(name)) => {
                                        Some((name.clone(), resolved_output_shape(&dynamic.shape)))
                                    }
                                    Some(Value::Atom(name)) => Some((
                                        name.to_string(),
                                        resolved_output_shape(&dynamic.shape),
                                    )),
                                    _ => None,
                                }
                            }));
                        }
                    }
                    avenger_chart_schema::DynamicOutputSource::ArrayValueNames { property } => {
                        if let Some(Value::Array(values)) = declaration.props.get(property) {
                            output_names.extend(values.iter().filter_map(|value| match value {
                                Value::Str(name) => {
                                    Some((name.clone(), resolved_output_shape(&dynamic.shape)))
                                }
                                Value::Atom(name) => {
                                    Some((name.to_string(), resolved_output_shape(&dynamic.shape)))
                                }
                                _ => None,
                            }));
                        }
                    }
                    avenger_chart_schema::DynamicOutputSource::ProjectionAliases { property } => {
                        if let Some(Value::Projection(projection)) = declaration.props.get(property)
                        {
                            output_names.extend(projection.items().iter().filter_map(|item| {
                                let sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } = item
                                else {
                                    return None;
                                };
                                Some((alias.value.clone(), resolved_output_shape(&dynamic.shape)))
                            }));
                        }
                    }
                }
            }
        }
        if declaration
            .kind
            .as_ref()
            .is_some_and(|kind| kind.as_str() == "pipeline")
        {
            if !declaration
                .children
                .iter()
                .any(|child| child.keyword.as_str() == "transform")
            {
                self.error(
                    "AVENGER-RESOLVE-081",
                    "transform pipeline is empty",
                    info.span,
                    "a pipeline requires at least one ordered transform stage",
                );
            }
            for child in declaration
                .children
                .iter()
                .filter(|child| child.keyword.as_str() == "output")
            {
                if let Some(name) = child.name.as_ref() {
                    output_names.push((name.to_string(), ResolvedOutputShape::Expression));
                }
            }
            if !output_names.is_empty() && declaration.name.is_none() {
                self.error(
                    "AVENGER-RESOLVE-082",
                    "output-bearing pipeline requires a binder",
                    info.span,
                    "add `as <name>` to qualify its output handles",
                );
            }
        }
        let producer = children
            .iter()
            .rev()
            .find(|child| child.keyword == "transform")
            .map_or_else(|| info.id.clone(), |child| child.id.clone());
        let mut unique_names = Vec::new();
        let mut seen = BTreeSet::new();
        for (name, shape) in output_names {
            if seen.insert(name.clone()) {
                unique_names.push((name, shape));
            } else {
                self.error(
                    "AVENGER-RESOLVE-150",
                    "duplicate transform output handle",
                    info.span,
                    format!("output `{name}` is declared more than once"),
                );
            }
        }
        unique_names
            .into_iter()
            .enumerate()
            .map(|(ordinal, (name, shape))| {
                (
                    name.clone(),
                    ResolvedOutputHandle {
                        producer: producer.clone(),
                        name,
                        ordinal,
                        shape,
                    },
                )
            })
            .collect()
    }

    fn validate_event_actions(
        &mut self,
        declaration: &Decl,
        children: &mut [ResolvedDeclaration],
        span: SourceSpan,
    ) {
        if declaration.keyword.as_str() != "on" {
            return;
        }
        let mut seen_action = false;
        for (source_child, child) in declaration.children.iter().zip(children) {
            if child.keyword == "set" {
                seen_action = true;
                let scope = self
                    .declarations
                    .values()
                    .find(|info| info.id == child.id)
                    .map_or(ScopeId(0), |info| info.containing_scope);
                self.validate_action(source_child, child, Some(declaration), span, scope, false);
            } else if seen_action && child.keyword != "set" {
                self.error(
                    "AVENGER-RESOLVE-090",
                    "event actions must remain in authored order",
                    span,
                    "non-action declarations cannot interrupt the ordered action vector",
                );
            }
        }
    }

    fn validate_action(
        &mut self,
        source: &Decl,
        action: &mut ResolvedDeclaration,
        event: Option<&Decl>,
        span: SourceSpan,
        scope: ScopeId,
        widget_action: bool,
    ) {
        let kind = action.kind.clone().unwrap_or_default();
        if kind == "cursor" {
            let value = action.properties.get("value");
            if value.is_none() {
                self.error(
                    "AVENGER-RESOLVE-087",
                    "cursor action is missing its value",
                    span,
                    "use `set cursor = <utf8 expression>`",
                );
            }
            if value.is_some_and(|value| {
                !matches!(
                    value,
                    ResolvedValue::String(_)
                        | ResolvedValue::Atom(_)
                        | ResolvedValue::Number(_)
                        | ResolvedValue::Boolean(_)
                        | ResolvedValue::Null
                        | ResolvedValue::Column(_)
                        | ResolvedValue::Call { .. }
                        | ResolvedValue::Expression(_)
                        | ResolvedValue::Binding(_)
                        | ResolvedValue::Channel { .. }
                )
            }) {
                self.error(
                    "AVENGER-RESOLVE-091",
                    "cursor action requires a scalar SQL expression",
                    span,
                    "the expression is strictly cast to utf8 before cursor-style validation",
                );
            }
            if let Some(ResolvedValue::String(style) | ResolvedValue::Atom(style)) = value
                && !matches!(
                    style.as_str(),
                    "default"
                        | "pointer"
                        | "text"
                        | "crosshair"
                        | "grab"
                        | "grabbing"
                        | "resize_horizontal"
                        | "resize_vertical"
                        | "resize_nw_se"
                        | "resize_ne_sw"
                )
            {
                self.error(
                    "AVENGER-RESOLVE-153",
                    "unknown cursor style literal",
                    span,
                    format!("`{style}` is not a registered cursor style"),
                );
            }
            for property in action.properties.keys() {
                if property != "value" {
                    self.error(
                        "AVENGER-RESOLVE-086",
                        "cursor action has an invalid l-value modifier",
                        span,
                        format!("`{property}` is not valid for the write-only cursor effect"),
                    );
                }
            }
            return;
        }
        let path = action
            .properties
            .get("target")
            .and_then(resolved_path)
            .unwrap_or_default();
        let target = self.resolve_any_path(scope, &path, span, true);
        let kind = match target.as_ref() {
            Some(ResolvedTarget::Param(_) | ResolvedTarget::DefinitionParam { .. }) => "param",
            Some(ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. }) => "store",
            Some(ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. }) => {
                "selection"
            }
            Some(_) => {
                self.error(
                    "AVENGER-RESOLVE-089",
                    "state action target has the wrong kind",
                    span,
                    format!(
                        "`{}` is not a scalar, store, or selection parameter",
                        path.join(".")
                    ),
                );
                return;
            }
            None => return,
        };
        action.kind = Some(kind.to_owned());
        if !action.properties.contains_key("value") {
            self.error(
                "AVENGER-RESOLVE-085",
                "state action is missing its update value",
                span,
                "every param, store, and selection action requires `= <value>`",
            );
        }
        for property in action.properties.keys() {
            if !matches!(
                property.as_str(),
                "target" | "value" | "at" | "replacing_scopes"
            ) {
                self.error(
                    "AVENGER-RESOLVE-134",
                    "unknown state action property",
                    span,
                    format!("state l-values do not have a `{property}` modifier"),
                );
            }
        }
        if let Some(ResolvedValue::Atom(at)) = action.properties.get("at")
            && !matches!(at.as_str(), "current" | "start")
        {
            self.error(
                "AVENGER-RESOLVE-084",
                "state action has an invalid owner route",
                span,
                "use `at current` or `at start`",
            );
        }
        if let Some(target) = target.clone() {
            action.state_lvalue = Some(ResolvedStateLValue {
                target: target.clone(),
                route: if matches!(
                    action.properties.get("at"),
                    Some(ResolvedValue::Atom(at)) if at == "start"
                ) {
                    ResolvedActionRoute::Start
                } else {
                    ResolvedActionRoute::Current
                },
                replacing_scopes: action.properties.contains_key("replacing_scopes"),
            });
            if widget_action && !self.target_is_shared(&target) {
                self.error(
                    "AVENGER-RESOLVE-124",
                    "widget action target must be shared",
                    span,
                    "button-style actions cannot select a scoped owner without an event route",
                );
            }
        }
        if kind == "store"
            && let Some(target) = target.as_ref()
        {
            self.validate_store_action(source, action, target, span);
        }
        if kind == "selection" {
            self.validate_selection_action(source, action, span, scope);
        }
        if matches!(action.properties.get("at"), Some(ResolvedValue::Atom(at)) if at == "start")
            && event.is_none_or(|event| event.props.get("between").is_none())
        {
            self.error(
                "AVENGER-RESOLVE-093",
                "`at start` action target requires `between:`",
                span,
                "the modifier routes the l-value to the gesture-start owner",
            );
        }
        if action.properties.contains_key("replacing_scopes") && kind == "selection" {
            self.error(
                "AVENGER-RESOLVE-094",
                "`replacing scopes` is invalid for selections",
                span,
                "the modifier is defined only for params and stores",
            );
        }
    }

    fn target_is_shared(&self, target: &ResolvedTarget) -> bool {
        match target {
            ResolvedTarget::Param(id) => self
                .params
                .get(id)
                .is_none_or(|param| param.sharing == StateSharing::Shared),
            ResolvedTarget::Store(id) => self
                .stores
                .get(id)
                .is_none_or(|store| store.sharing == StateSharing::Shared),
            // Definition exports are instantiated later; their resolved
            // interface contract is shared unless expansion says otherwise.
            ResolvedTarget::DefinitionParam { .. }
            | ResolvedTarget::DefinitionStore { .. }
            | ResolvedTarget::Selection(_)
            | ResolvedTarget::DefinitionSelection { .. } => true,
            _ => false,
        }
    }

    fn validate_event_properties(
        &mut self,
        declaration: &Decl,
        properties: &BTreeMap<String, ResolvedValue>,
        span: SourceSpan,
    ) {
        const EVENT_TYPES: &[&str] = &[
            "mouse_down",
            "mouse_up",
            "click",
            "double_click",
            "mouse_wheel",
            "key_press",
            "key_release",
            "cursor_moved",
            "mark_mouse_enter",
            "mark_mouse_leave",
            "window_resize",
            "window_resize_settled",
            "canvas_resize",
            "canvas_resize_settled",
            "window_moved",
            "window_focused",
            "window_close_requested",
        ];
        if !declaration
            .kind
            .as_ref()
            .is_some_and(|kind| EVENT_TYPES.contains(&kind.as_str()))
        {
            self.error(
                "AVENGER-RESOLVE-095",
                "unknown event type",
                span,
                format!(
                    "`{}` is not an exposed v1 event",
                    declaration
                        .kind
                        .as_ref()
                        .map_or("<missing>", |kind| kind.as_str())
                ),
            );
        }
        let allowed = [
            "target",
            "scope",
            "surface",
            "filter",
            "throttle_ms",
            "consume",
            "mode",
            "settle_exact",
            "between",
        ];
        for name in properties.keys() {
            if !allowed.contains(&name.as_str()) {
                self.error(
                    "AVENGER-RESOLVE-096",
                    "unknown event binding property",
                    span,
                    format!("event bindings do not have `{name}:`"),
                );
            }
        }
        for (name, valid) in [
            (
                "filter",
                properties.get("filter").is_none_or(is_expression_value),
            ),
            (
                "throttle_ms",
                properties.get("throttle_ms").is_none_or(|value| {
                    matches!(value, ResolvedValue::Number(value) if value.parse::<u64>().is_ok())
                }),
            ),
            (
                "consume",
                properties
                    .get("consume")
                    .is_none_or(|value| matches!(value, ResolvedValue::Boolean(_))),
            ),
            (
                "mode",
                properties.get("mode").is_none_or(|value| {
                    matches!(value, ResolvedValue::Atom(value) if matches!(value.as_str(), "preview" | "exact"))
                }),
            ),
            (
                "settle_exact",
                properties
                    .get("settle_exact")
                    .is_none_or(|value| matches!(value, ResolvedValue::Boolean(_))),
            ),
            (
                "between",
                properties
                    .get("between")
                    .is_none_or(|value| matches!(value, ResolvedValue::Object { .. })),
            ),
        ] {
            if !valid {
                self.error(
                    "AVENGER-RESOLVE-146",
                    "event binding property has the wrong shape",
                    span,
                    format!("`{name}:` has an invalid value"),
                );
            }
        }
        if let Some(target) = properties.get("target")
            && !self.valid_event_target_value(target)
        {
            self.error(
                "AVENGER-RESOLVE-097",
                "invalid event target filter",
                span,
                "`target:` requires one or more distinct targetable mark paths",
            );
        }
        if let Some(scope) = properties.get("scope")
            && !valid_event_scope(scope)
        {
            self.error(
                "AVENGER-RESOLVE-098",
                "invalid event scope",
                span,
                "use `plot`, `subplot <path>`, or a non-empty `subplots [...]` list",
            );
        }
        if let Some(surface) = properties.get("surface")
            && !valid_event_surface(surface)
        {
            self.error(
                "AVENGER-RESOLVE-099",
                "invalid event surface",
                span,
                "use `plot`, `all`, or `legend <channel>`",
            );
        }
        if let Some(ResolvedValue::Object { properties, .. }) = properties.get("between") {
            for required in ["start", "end"] {
                if !properties.contains_key(required) {
                    self.error(
                        "AVENGER-RESOLVE-103",
                        "incomplete between interaction",
                        span,
                        format!("`between:` requires `{required}: <event> {{ ... }}`"),
                    );
                }
            }
            for (role, stream) in properties {
                if !matches!(role.as_str(), "start" | "end") {
                    self.error(
                        "AVENGER-RESOLVE-104",
                        "unknown between stream role",
                        span,
                        format!("`between:` has no `{role}:` role"),
                    );
                }
                if let ResolvedValue::Object {
                    kind,
                    properties: stream_properties,
                    ..
                } = stream
                {
                    if !kind
                        .as_deref()
                        .is_some_and(|kind| EVENT_TYPES.contains(&kind))
                    {
                        self.error(
                            "AVENGER-RESOLVE-147",
                            "between stream has an unknown event type",
                            span,
                            format!(
                                "`{}` is not an exposed v1 event",
                                kind.as_deref().unwrap_or("<missing>")
                            ),
                        );
                    }
                    for property in stream_properties.keys() {
                        if !matches!(property.as_str(), "target" | "filter" | "scope" | "surface") {
                            self.error(
                                "AVENGER-RESOLVE-148",
                                "unknown between-stream property",
                                span,
                                format!("between streams do not have `{property}:`"),
                            );
                        }
                    }
                    if stream_properties.contains_key("scope")
                        || stream_properties.contains_key("surface")
                    {
                        self.error(
                            "AVENGER-RESOLVE-105",
                            "between streams inherit scope and surface",
                            span,
                            "remove nested `scope:`/`surface:` and configure the outer event",
                        );
                    }
                    if let Some(target) = stream_properties.get("target")
                        && !self.valid_event_target_value(target)
                    {
                        self.error(
                            "AVENGER-RESOLVE-097",
                            "invalid event target filter",
                            span,
                            "nested `target:` requires one or more distinct targetable mark paths",
                        );
                    }
                    if let Some(filter) = stream_properties.get("filter") {
                        if !is_expression_value(filter) {
                            self.error(
                                "AVENGER-RESOLVE-146",
                                "event binding property has the wrong shape",
                                span,
                                "nested `filter:` requires a SQL scalar expression",
                            );
                        }
                        if resolved_value_has_forbidden_stream_binding(filter) {
                            self.error(
                                "AVENGER-RESOLVE-106",
                                "stream filter uses unavailable state",
                                span,
                                "start/end filters may read current params, but not stores or temporal snapshots",
                            );
                        }
                    }
                } else {
                    self.error(
                        "AVENGER-RESOLVE-149",
                        "between stream has the wrong shape",
                        span,
                        format!("`{role}:` requires an event block"),
                    );
                }
            }
        }
    }

    fn valid_event_target_value(&self, value: &ResolvedValue) -> bool {
        let targets = match value {
            ResolvedValue::Reference(reference) => vec![&reference.target],
            ResolvedValue::Array(values) => values
                .iter()
                .filter_map(|value| match value {
                    ResolvedValue::Reference(reference) => Some(&reference.target),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let expected_len = match value {
            ResolvedValue::Reference(_) => 1,
            ResolvedValue::Array(values) => values.len(),
            _ => 0,
        };
        !targets.is_empty()
            && targets.len() == expected_len
            && targets
                .iter()
                .all(|target| self.is_targetable_event_target(target))
            && targets.iter().copied().collect::<BTreeSet<_>>().len() == targets.len()
    }

    fn is_targetable_event_target(&self, target: &ResolvedTarget) -> bool {
        match target {
            ResolvedTarget::Mark(_)
            | ResolvedTarget::DefinitionStructural {
                kind: DefinitionExportKind::Mark,
                ..
            } => true,
            target @ ResolvedTarget::DefinitionSlot { .. } => {
                self.definition_slot_schema(target).is_some_and(|slot| {
                    slot.shape == "ref" && slot.reference_kind.as_deref() == Some("mark")
                })
            }
            ResolvedTarget::Part { declaration, alias } => self
                .instances
                .get(declaration)
                .and_then(|interface| interface.parts.get(alias))
                .is_some_and(|part| part.targetable),
            _ => false,
        }
    }

    fn resolve_event_binding(
        &mut self,
        scope: ScopeId,
        declaration: &Decl,
        properties: &BTreeMap<String, ResolvedValue>,
        span: SourceSpan,
    ) -> ResolvedEventBinding {
        let plot = self.containing_plot(scope).unwrap_or_else(|| {
            self.declarations
                .values()
                .find(|info| info.child_scope == Some(scope))
                .map_or_else(
                    || DeclarationId(semantic_hash(&["missing-event-plot"])),
                    |info| info.id.clone(),
                )
        });
        let targets = match properties.get("target") {
            Some(ResolvedValue::Reference(reference)) => vec![reference.target.clone()],
            Some(ResolvedValue::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    ResolvedValue::Reference(reference) => Some(reference.target.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let scope = match properties.get("scope") {
            None => ResolvedEventScope::Plot(plot.clone()),
            Some(ResolvedValue::Atom(value)) if value == "plot" => {
                ResolvedEventScope::Plot(plot.clone())
            }
            Some(value) => {
                let calls = match value {
                    ResolvedValue::Call { function, args } if function == "subplot" => {
                        vec![args.as_slice()]
                    }
                    ResolvedValue::Array(values) => values
                        .iter()
                        .filter_map(|value| match value {
                            ResolvedValue::Call { function, args } if function == "subplot" => {
                                Some(args.as_slice())
                            }
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let mut targets = Vec::new();
                for args in calls {
                    let path = args
                        .iter()
                        .filter_map(|value| match value {
                            ResolvedValue::Atom(value) => Some(value.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if let Some(target) = self.resolve_any_path(scope, &path, span, true) {
                        if matches!(target, ResolvedTarget::Declaration(_)) {
                            targets.push(target);
                        } else {
                            self.error(
                                "AVENGER-RESOLVE-113",
                                "event subplot scope resolves to a non-structural target",
                                span,
                                format!("`{}` is not a subplot path", path.join(".")),
                            );
                        }
                    }
                }
                ResolvedEventScope::Subplots {
                    plot: plot.clone(),
                    targets,
                }
            }
        };
        let surface = match properties.get("surface") {
            Some(ResolvedValue::Atom(value)) if value == "all" => {
                ResolvedEventSurface::All(plot.clone())
            }
            Some(ResolvedValue::Call { function, args }) if function == "legend" => args
                .first()
                .and_then(|value| match value {
                    ResolvedValue::Atom(channel) => Some(ResolvedEventSurface::Legend {
                        plot: plot.clone(),
                        channel: channel.clone(),
                    }),
                    _ => None,
                })
                .unwrap_or_else(|| ResolvedEventSurface::Plot(plot.clone())),
            _ => ResolvedEventSurface::Plot(plot.clone()),
        };
        ResolvedEventBinding {
            event_type: declaration
                .kind
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            targets,
            scope,
            surface,
        }
    }

    fn containing_plot(&self, scope: ScopeId) -> Option<DeclarationId> {
        let mut cursor = Some(scope);
        while let Some(id) = cursor {
            if let Some(owner) = self.scopes[id.0].owner.as_ref()
                && self
                    .declaration_source(owner)
                    .is_some_and(|(_, declaration)| {
                        matches!(declaration.keyword.as_str(), "chart" | "cell" | "plot")
                    })
            {
                return Some(owner.clone());
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    fn validate_store_action(
        &mut self,
        source: &Decl,
        action: &ResolvedDeclaration,
        target: &ResolvedTarget,
        span: SourceSpan,
    ) {
        if matches!(action.properties.get("value"), Some(ResolvedValue::Atom(operation)) if operation == "clear")
        {
            return;
        }
        let operation = match action.properties.get("value") {
            Some(ResolvedValue::Object {
                kind: Some(operation),
                ..
            }) => matches!(
                operation.as_str(),
                "insert_rows"
                    | "replace_rows"
                    | "upsert_rows"
                    | "update_by_key"
                    | "delete_by_key"
                    | "toggle_rows"
            )
            .then_some(operation.as_str()),
            _ => None,
        };
        let Some(operation) = operation else {
            self.error(
                "AVENGER-RESOLVE-107",
                "invalid store update operation",
                span,
                "use clear, insert_rows, replace_rows, upsert_rows, update_by_key, delete_by_key, or toggle_rows",
            );
            return;
        };
        let Some((fields, primary_key)) = self.store_shape(target) else {
            return;
        };
        if matches!(
            operation,
            "upsert_rows" | "update_by_key" | "delete_by_key" | "toggle_rows"
        ) && primary_key.is_empty()
        {
            self.error(
                "AVENGER-RESOLVE-114",
                "store operation requires a primary key",
                span,
                format!("`{operation}` cannot target an unkeyed store"),
            );
        }
        let (
            Some(Value::Block {
                body: source_body, ..
            }),
            Some(ResolvedValue::Object { children, .. }),
        ) = (source.props.get("value"), action.properties.get("value"))
        else {
            return;
        };
        match operation {
            "insert_rows" | "replace_rows" | "upsert_rows" | "toggle_rows" => {
                let source_rows = source_body
                    .children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "row")
                    .collect::<Vec<_>>();
                let resolved_rows = children
                    .iter()
                    .filter(|child| child.keyword == "row")
                    .collect::<Vec<_>>();
                if source_rows.is_empty() || source_rows.len() != source_body.children.len() {
                    self.error(
                        "AVENGER-RESOLVE-115",
                        "store row operation requires only row payloads",
                        span,
                        format!("`{operation}` requires one or more `row {{ ... }}` children"),
                    );
                }
                for (source_row, row) in source_rows.into_iter().zip(resolved_rows) {
                    self.validate_store_payload_fields(
                        source_row,
                        row,
                        &fields,
                        &primary_key,
                        StorePayloadShape::CompleteRow,
                        span,
                    );
                }
                let mut keys = BTreeSet::new();
                for row in source_body
                    .children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "row")
                {
                    if let Some(key) = static_store_key(row, &primary_key)
                        && !keys.insert(key)
                    {
                        self.error(
                            "AVENGER-RESOLVE-126",
                            "store update payload contains a duplicate primary key",
                            span,
                            "all statically known payload key tuples must be unique before mutation",
                        );
                    }
                }
            }
            "update_by_key" => {
                self.validate_key_patch_payload(source_body, children, &fields, &primary_key, span);
            }
            "delete_by_key" => {
                self.validate_key_only_payload(source_body, children, &fields, &primary_key, span);
            }
            _ => {}
        }
    }

    fn store_shape(&self, target: &ResolvedTarget) -> Option<(Vec<PhysicalField>, Vec<String>)> {
        let ResolvedTarget::Store(id) = target else {
            return None;
        };
        if let Some(store) = self.stores.get(id) {
            return Some((store.fields.clone(), store.primary_key.clone()));
        }
        let info = self
            .declarations
            .values()
            .find(|info| info.runtime_target.as_ref() == Some(target))?;
        let (_, declaration) = self.declaration_source(&info.id)?;
        let fields = declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "field")
            .filter_map(|field| {
                Some(PhysicalField {
                    name: field.name.as_ref()?.to_string(),
                    data_type: PhysicalType::parse(field.props.get("type")?).ok()?,
                    nullable: matches!(field.props.get("nullable"), Some(Value::Bool(true))),
                })
            })
            .collect();
        Some((fields, value_names(declaration.props.get("primary_key"))))
    }

    fn validate_store_payload_fields(
        &mut self,
        source: &Decl,
        resolved: &ResolvedDeclaration,
        fields: &[PhysicalField],
        primary_key: &[String],
        shape: StorePayloadShape,
        span: SourceSpan,
    ) {
        for name in resolved.properties.keys() {
            if !fields.iter().any(|field| &field.name == name) {
                self.error(
                    "AVENGER-RESOLVE-116",
                    "store update references an unknown field",
                    span,
                    format!("field `{name}` is not declared by the target store"),
                );
                continue;
            }
            if matches!(shape, StorePayloadShape::Key) && !primary_key.contains(name) {
                self.error(
                    "AVENGER-RESOLVE-117",
                    "store key payload contains a non-key field",
                    span,
                    format!("field `{name}` is not part of the primary key"),
                );
            }
            if matches!(shape, StorePayloadShape::Patch) && primary_key.contains(name) {
                self.error(
                    "AVENGER-RESOLVE-118",
                    "store patch cannot modify a primary-key field",
                    span,
                    format!("delete and insert/upsert to change `{name}`"),
                );
            }
            if primary_key.contains(name) && matches!(source.props.get(name), Some(Value::Null)) {
                self.error(
                    "AVENGER-RESOLVE-127",
                    "store action primary-key value cannot be null",
                    span,
                    format!("field `{name}` is part of the primary key"),
                );
            }
        }
        match shape {
            StorePayloadShape::CompleteRow => {
                for field in fields {
                    if !field.nullable && !resolved.properties.contains_key(&field.name) {
                        self.error(
                            "AVENGER-RESOLVE-119",
                            "store action row is incomplete",
                            span,
                            format!("row requires non-nullable field `{}`", field.name),
                        );
                    }
                }
            }
            StorePayloadShape::Key => {
                for key in primary_key {
                    if !resolved.properties.contains_key(key) {
                        self.error(
                            "AVENGER-RESOLVE-120",
                            "store action key is incomplete",
                            span,
                            format!("key requires field `{key}`"),
                        );
                    }
                }
            }
            StorePayloadShape::Patch if resolved.properties.is_empty() => self.error(
                "AVENGER-RESOLVE-121",
                "store action patch is empty",
                span,
                "`fields { ... }` must update at least one non-key field",
            ),
            StorePayloadShape::Patch => {}
        }
    }

    fn validate_key_patch_payload(
        &mut self,
        source: &crate::ast::Body,
        resolved: &[ResolvedDeclaration],
        fields: &[PhysicalField],
        primary_key: &[String],
        span: SourceSpan,
    ) {
        let source_key = unique_child(&source.children, "key");
        let source_fields = unique_child(&source.children, "fields");
        let resolved_key = unique_resolved_child(resolved, "key");
        let resolved_fields = unique_resolved_child(resolved, "fields");
        if source_key.is_none()
            || source_fields.is_none()
            || source.children.len() != 2
            || resolved_key.is_none()
            || resolved_fields.is_none()
        {
            self.error(
                "AVENGER-RESOLVE-122",
                "update_by_key requires one key and one fields block",
                span,
                "use `key { ... } fields { ... }` with no other children",
            );
            return;
        }
        self.validate_store_payload_fields(
            source_key.unwrap(),
            resolved_key.unwrap(),
            fields,
            primary_key,
            StorePayloadShape::Key,
            span,
        );
        self.validate_store_payload_fields(
            source_fields.unwrap(),
            resolved_fields.unwrap(),
            fields,
            primary_key,
            StorePayloadShape::Patch,
            span,
        );
    }

    fn validate_key_only_payload(
        &mut self,
        source: &crate::ast::Body,
        resolved: &[ResolvedDeclaration],
        fields: &[PhysicalField],
        primary_key: &[String],
        span: SourceSpan,
    ) {
        let source_key = unique_child(&source.children, "key");
        let resolved_key = unique_resolved_child(resolved, "key");
        if source_key.is_none() || source.children.len() != 1 || resolved_key.is_none() {
            self.error(
                "AVENGER-RESOLVE-123",
                "delete_by_key requires exactly one key block",
                span,
                "use `key { ... }` with no other children",
            );
            return;
        }
        self.validate_store_payload_fields(
            source_key.unwrap(),
            resolved_key.unwrap(),
            fields,
            primary_key,
            StorePayloadShape::Key,
            span,
        );
    }

    fn validate_selection_action(
        &mut self,
        source: &Decl,
        action: &mut ResolvedDeclaration,
        span: SourceSpan,
        scope: ScopeId,
    ) {
        let valid = match action.properties.get("value") {
            Some(ResolvedValue::Atom(operation)) => operation == "clear",
            Some(ResolvedValue::Object {
                kind: Some(operation),
                ..
            }) => matches!(
                operation.as_str(),
                "clear_in_scope"
                    | "replace_all_clauses"
                    | "replace_clauses_in_scope"
                    | "upsert_clauses"
                    | "toggle_clauses"
                    | "delete_clauses"
                    | "delete_clauses_in_scope"
                    | "replace_all_from_scene_query"
                    | "replace_from_scene_query_in_scope"
                    | "upsert_from_scene_query"
                    | "toggle_from_scene_query"
            ),
            _ => false,
        };
        if !valid {
            self.error(
                "AVENGER-RESOLVE-108",
                "invalid selection update operation",
                span,
                "use one of the closed v1 clause or scene-query update operations",
            );
            return;
        }
        let (
            Some(Value::Block {
                body: source_body, ..
            }),
            Some(ResolvedValue::Object {
                kind: Some(operation),
                properties,
                children,
                ..
            }),
        ) = (
            source.props.get("value"),
            action.properties.get_mut("value"),
        )
        else {
            return;
        };
        if operation == "clear_in_scope" && !properties.contains_key("scope") {
            self.error(
                "AVENGER-RESOLVE-136",
                "clear_in_scope requires a selection scope",
                span,
                "add `scope: level(n)` to the update payload",
            );
        }
        if matches!(
            operation.as_str(),
            "replace_clauses_in_scope" | "delete_clauses_in_scope"
        ) && !properties.contains_key("scope")
        {
            self.error(
                "AVENGER-RESOLVE-158",
                "scoped selection update requires a scope",
                span,
                format!("add `scope:` to `{operation}`"),
            );
        }
        if matches!(
            operation.as_str(),
            "replace_all_clauses"
                | "replace_clauses_in_scope"
                | "upsert_clauses"
                | "toggle_clauses"
        ) {
            let clauses = children
                .iter()
                .filter(|child| child.keyword == "clause")
                .collect::<Vec<_>>();
            if clauses.is_empty() || clauses.len() != children.len() {
                self.error(
                    "AVENGER-RESOLVE-137",
                    "selection clause update requires only clause payloads",
                    span,
                    format!("`{operation}` requires one or more `clause {{ ... }}` children"),
                );
            }
            let source_clauses = source_body
                .children
                .iter()
                .filter(|child| child.keyword.as_str() == "clause");
            for (source_clause, clause) in source_clauses.zip(clauses) {
                let source_id = source_clause.props.get("id");
                let resolved_id = clause.properties.get("id");
                if source_id.is_none() || resolved_id.is_none() {
                    self.error(
                        "AVENGER-RESOLVE-138",
                        "selection clause requires an id",
                        span,
                        "clause ids are exact non-empty utf8 values",
                    );
                    continue;
                }
                if matches!(source_id, Some(Value::Null))
                    || matches!(source_id, Some(Value::Str(value)) if value.is_empty())
                {
                    self.error(
                        "AVENGER-RESOLVE-139",
                        "selection clause id must be non-empty utf8",
                        span,
                        "NULL and the empty string are not stable clause identities",
                    );
                }
            }
        }
        if matches!(
            operation.as_str(),
            "delete_clauses" | "delete_clauses_in_scope"
        ) {
            match (source_body.props.get("ids"), properties.get("ids")) {
                (Some(Value::Array(source_ids)), Some(ResolvedValue::Array(ids)))
                    if !source_ids.is_empty() && source_ids.len() == ids.len() =>
                {
                    let _ = (source_ids, ids);
                }
                _ => self.error(
                    "AVENGER-RESOLVE-159",
                    "selection clause deletion requires ids",
                    span,
                    format!("`{operation}` requires a non-empty `ids: [...]` array"),
                ),
            }
        }
        if operation.contains("scene_query") {
            for required in ["geometry", "policy", "marks", "fields"] {
                if !properties.contains_key(required) {
                    self.error(
                        "AVENGER-RESOLVE-140",
                        "scene-query selection update is incomplete",
                        span,
                        format!("`{operation}` requires `{required}:`"),
                    );
                }
            }
            if !matches!(
                properties.get("geometry"),
                Some(ResolvedValue::Call { function, args })
                    if crate::intrinsic_operation_signature(function).is_some_and(|signature| {
                        signature.arguments.len() == args.len()
                            && signature
                                .contexts
                                .contains(&crate::IntrinsicOperationContext::SceneGeometry)
                    })
            ) {
                self.error(
                    "AVENGER-RESOLVE-160",
                    "scene-query geometry is invalid",
                    span,
                    "use `polygon(points)`, `rect(x0, y0, x1, y1)`, or `circle(cx, cy, radius)`",
                );
            }
            if !matches!(
                properties.get("policy"),
                Some(ResolvedValue::Atom(value) | ResolvedValue::String(value))
                    if matches!(
                        value.as_str(),
                        "intersects"
                            | "geometry_intersects"
                            | "envelope_intersects"
                            | "contained"
                            | "geometry_contained"
                            | "anchor_inside"
                            | "centroid_inside"
                    )
            ) {
                self.error(
                    "AVENGER-RESOLVE-161",
                    "scene-query hit policy is invalid",
                    span,
                    "use intersects, envelope_intersects, contained, anchor_inside, or centroid_inside",
                );
            }
            if !matches!(
                properties.get("fields"),
                Some(ResolvedValue::Array(fields)) if !fields.is_empty()
            ) {
                self.error(
                    "AVENGER-RESOLVE-162",
                    "scene-query selection requires captured fields",
                    span,
                    "add one or more `{ id: 'name'; field: \"column\"; }` entries to `fields:`",
                );
            }
            if let Some(source_marks) = source_body.props.get("marks") {
                properties.insert(
                    "marks".to_owned(),
                    self.resolve_scene_query_targets(scope, source_marks, span),
                );
            }
        }
    }

    fn resolve_scene_query_targets(
        &mut self,
        scope: ScopeId,
        source: &Value,
        span: SourceSpan,
    ) -> ResolvedValue {
        let Value::Array(values) = source else {
            self.error(
                "AVENGER-RESOLVE-155",
                "scene-query marks must be a non-empty path list",
                span,
                "use `marks: [mark_name, group.mark_name]`",
            );
            return ResolvedValue::Invalid;
        };
        if values.is_empty() {
            self.error(
                "AVENGER-RESOLVE-155",
                "scene-query marks must be a non-empty path list",
                span,
                "add at least one targetable mark or exported mark part",
            );
            return ResolvedValue::Invalid;
        }

        let mut seen = BTreeSet::new();
        let mut resolved = Vec::with_capacity(values.len());
        for value in values {
            let Some(authored_path) = authored_bare_path(value) else {
                self.error(
                    "AVENGER-RESOLVE-155",
                    "scene-query mark target is not an authored path",
                    span,
                    "mark targets are bare or qualified structural paths, not computed SQL expressions",
                );
                resolved.push(ResolvedValue::Invalid);
                continue;
            };
            let Some(target) =
                self.resolve_typed_reference_path(scope, &authored_path, RefKind::Mark, span)
            else {
                resolved.push(ResolvedValue::Invalid);
                continue;
            };
            if !reference_kind_matches(&target, RefKind::Mark)
                || !self.is_targetable_event_target(&target)
            {
                self.error(
                    "AVENGER-RESOLVE-156",
                    "scene-query target is not a targetable mark",
                    span,
                    format!(
                        "`{}` resolves to a non-mark or non-targetable part",
                        authored_path.join(".")
                    ),
                );
                resolved.push(ResolvedValue::Invalid);
                continue;
            }
            if !seen.insert(target.clone()) {
                self.error(
                    "AVENGER-RESOLVE-157",
                    "duplicate scene-query mark target",
                    span,
                    format!(
                        "`{}` names an already listed target",
                        authored_path.join(".")
                    ),
                );
                resolved.push(ResolvedValue::Invalid);
                continue;
            }
            resolved.push(ResolvedValue::Reference(ResolvedReference {
                target,
                kind: RefKind::Mark,
                authored_path,
            }));
        }
        ResolvedValue::Array(resolved)
    }

    fn check_param_initializer_dag(&mut self) -> Vec<ParamId> {
        match topological_order(&self.param_dependencies) {
            Ok(order) => order,
            Err(cycle) => {
                let span = self
                    .project
                    .source_modules
                    .values()
                    .next()
                    .map(root_span)
                    .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0));
                self.error(
                    "AVENGER-RESOLVE-100",
                    "param initializer dependency cycle",
                    span,
                    format!(
                        "cycle: {}",
                        cycle
                            .iter()
                            .map(|id| self.param_display_name(id))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    ),
                );
                Vec::new()
            }
        }
    }

    fn check_table_dag(&mut self) -> Vec<DeclarationId> {
        match topological_order(&self.table_dependencies) {
            Ok(order) => order,
            Err(cycle) => {
                let span = self
                    .project
                    .source_modules
                    .values()
                    .next()
                    .map(root_span)
                    .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0));
                self.error(
                    "AVENGER-RESOLVE-101",
                    "table dependency cycle",
                    span,
                    format!(
                        "cycle: {}",
                        cycle
                            .iter()
                            .map(|id| self.table_display_name(id))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    ),
                );
                Vec::new()
            }
        }
    }

    fn definition_import_order(&self) -> Vec<ModuleItemId> {
        let mut dependencies = BTreeMap::<ModuleItemId, BTreeSet<ModuleItemId>>::new();
        for item in self.definitions.keys() {
            let imported_modules = self
                .project
                .imports
                .iter()
                .filter(|edge| edge.importer == item.module)
                .filter_map(|edge| match &edge.imported {
                    ModuleId::Source(module) => Some(module),
                    ModuleId::Native(_) => None,
                })
                .collect::<BTreeSet<_>>();
            dependencies.entry(item.clone()).or_default().extend(
                self.definitions
                    .keys()
                    .filter(|candidate| imported_modules.contains(&candidate.module))
                    .cloned(),
            );
        }
        topological_order(&dependencies).unwrap_or_default()
    }

    fn param_display_name(&self, id: &ParamId) -> String {
        self.params.get(id).map_or_else(
            || id.as_str().to_owned(),
            |param| format!("{}::{}", param.lexical_scope, param.source_name),
        )
    }

    fn table_display_name(&self, id: &DeclarationId) -> String {
        self.relation_declarations
            .iter()
            .find_map(|(relation, candidate)| {
                (candidate == id).then(|| {
                    let root = self
                        .module_index
                        .items
                        .get(&relation.defining_item)
                        .and_then(|item| item.name.clone())
                        .unwrap_or_else(|| "<data>".to_owned());
                    std::iter::once(root)
                        .chain(relation.nested_path.iter().cloned())
                        .collect::<Vec<_>>()
                        .join(".")
                })
            })
            .unwrap_or_else(|| id.as_str().to_owned())
    }

    fn resolved_catalog_tables(&self) -> BTreeMap<ResolvedRelationId, ResolvedCatalogTable> {
        self.relation_declarations
            .iter()
            .filter_map(|(relation, id)| {
                let ((file_id, _), info) =
                    self.declarations.iter().find(|(_, info)| &info.id == id)?;
                let (_, declaration) = self.declaration_source(id)?;
                let params = self
                    .params
                    .values()
                    .filter(|param| param.table_owner.as_ref() == Some(id))
                    .map(|param| param.id.clone())
                    .collect::<Vec<_>>();
                Some((
                    relation.clone(),
                    ResolvedCatalogTable {
                        relation: relation.clone(),
                        id: id.clone(),
                        file: file_id.clone(),
                        source: info.span.source,
                        span: info.span,
                        path: std::iter::once(
                            self.module_index
                                .items
                                .get(&relation.defining_item)
                                .and_then(|item| item.name.clone())
                                .unwrap_or_else(|| "<data>".to_owned()),
                        )
                        .chain(relation.nested_path.iter().cloned())
                        .collect(),
                        kind: declaration
                            .kind
                            .as_ref()
                            .map_or_else(|| "unknown".to_owned(), ToString::to_string),
                        params,
                        dependencies: self
                            .table_dependencies
                            .get(id)
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect(),
                    },
                ))
            })
            .collect()
    }

    fn error(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        span: SourceSpan,
        label: impl Into<String>,
    ) {
        let mut diagnostic = Diagnostic::error(code, message, SourceLabel::new(span, label));
        diagnostic.trace = self.import_trace_for_source(span.source);
        self.diagnostics.push(diagnostic);
    }

    fn import_trace_for_source(&self, target: SourceId) -> Vec<ExpansionOrImportFrame> {
        let roots = self
            .project
            .requested_modules
            .iter()
            .filter_map(|file| {
                self.project
                    .source_modules
                    .get(file)
                    .map(|file| file.source)
            })
            .collect::<Vec<_>>();
        for root in roots {
            let mut visiting = BTreeSet::new();
            let mut path = Vec::new();
            if find_import_trace(
                root,
                target,
                &self.project.imports,
                &mut visiting,
                &mut path,
            ) {
                return path;
            }
        }
        Vec::new()
    }

    // The remaining resolution operations are implemented below in focused
    // helpers so validation can accumulate independent diagnostics.
}

fn merge_definition_mark_schemas(mut schemas: Vec<KindSchema>) -> Option<KindSchema> {
    let mut merged = schemas.pop()?;
    let first_coordinate = merged.key.coordinate.take();
    if let Some(coordinate) = first_coordinate {
        merged.compatible_coordinates.insert(coordinate);
    }

    for mut schema in schemas {
        if let Some(coordinate) = schema.key.coordinate.take() {
            merged.compatible_coordinates.insert(coordinate);
        }
        merged
            .compatible_coordinates
            .extend(schema.compatible_coordinates);
        if merged.allowed_parents.is_empty() || schema.allowed_parents.is_empty() {
            merged.allowed_parents.clear();
        } else {
            merged.allowed_parents.extend(schema.allowed_parents);
        }
        merged.body_mode =
            if merged.body_mode == BodyMode::Mixed || schema.body_mode == BodyMode::Mixed {
                BodyMode::Mixed
            } else {
                BodyMode::Properties
            };
        merged.stateless &= schema.stateless;
        if merged.runtime_kind != schema.runtime_kind {
            merged.runtime_kind = None;
        }
        if merged.child_rules != schema.child_rules {
            // Coordinate-specific child grammars cannot be proven until the
            // definition is instantiated. Exact validation occurs in Phase 7.
            merged.child_rules.clear();
        }

        let previous_properties = merged.properties.keys().cloned().collect::<BTreeSet<_>>();
        for name in previous_properties {
            let Some(property) = merged.properties.get_mut(&name) else {
                continue;
            };
            if let Some(candidate) = schema.properties.remove(&name) {
                if property.shape != candidate.shape {
                    property.shape = ValueShape::Any;
                }
                property.required &= candidate.required;
                if property.default != candidate.default {
                    property.default = None;
                }
            } else {
                property.required = false;
                property.default = None;
            }
        }
        for (name, mut property) in schema.properties {
            property.required = false;
            property.default = None;
            merged.properties.insert(name, property);
        }

        let previous_channels = merged.channels.keys().cloned().collect::<BTreeSet<_>>();
        for name in previous_channels {
            let Some(channel) = merged.channels.get_mut(&name) else {
                continue;
            };
            if let Some(candidate) = schema.channels.remove(&name) {
                if channel.shape != candidate.shape {
                    channel.shape = ValueShape::Any;
                }
                channel.required &= candidate.required;
            } else {
                channel.required = false;
            }
        }
        for (name, mut channel) in schema.channels {
            channel.required = false;
            merged.channels.insert(name, channel);
        }

        merged
            .parts
            .retain(|name, value| schema.parts.get(name) == Some(value));
        merged
            .exports
            .retain(|name, value| schema.exports.get(name) == Some(value));
        merged
            .outputs
            .retain(|name, value| schema.outputs.get(name) == Some(value));
        merged
            .dynamic_outputs
            .retain(|value| schema.dynamic_outputs.contains(value));
    }
    Some(merged)
}

fn collect_public_targets(
    declaration: &ResolvedDeclaration,
    output: &mut BTreeMap<String, ResolvedTarget>,
    origins: &mut BTreeMap<String, SourceSpan>,
    collisions: &mut Vec<(String, SourceSpan, SourceSpan)>,
) {
    if let (Some(path), Some(target)) = (&declaration.public_path, &declaration.runtime_target) {
        insert_public_target(path, target, declaration.span, output, origins, collisions);
    }
    for (alias, target) in &declaration.exports {
        // A targetable component part is the public event/scene identity for
        // an exported mark. Keep the typed definition export on the resolved
        // declaration, but publish exactly one canonical path here.
        if !declaration.parts.contains_key(alias)
            && let Some(path) = &declaration.public_path
        {
            insert_public_target(
                &format!("{path}.{alias}"),
                target,
                declaration.span,
                output,
                origins,
                collisions,
            );
        }
    }
    for alias in declaration.parts.keys() {
        if let Some(path) = &declaration.public_path {
            insert_public_target(
                &format!("{path}.{alias}"),
                &ResolvedTarget::Part {
                    declaration: declaration.id.clone(),
                    alias: alias.clone(),
                },
                declaration.span,
                output,
                origins,
                collisions,
            );
        }
    }
    for child in &declaration.children {
        collect_public_targets(child, output, origins, collisions);
    }
}

fn collect_item_dependencies(
    declaration: &ResolvedDeclaration,
    from: &ModuleItemId,
    output: &mut Vec<ItemDependencyEdge>,
) {
    if let Some(ResolvedKindBinding::Definition(to)) = &declaration.kind_binding {
        output.push(ItemDependencyEdge {
            from: from.clone(),
            to: to.clone(),
            cause: ItemDependencyCause::DefinitionUse,
            site: declaration.span,
        });
    }
    for reference in &declaration.relation_references {
        if let ResolvedRelationTarget::Relation(relation) = &reference.target {
            output.push(ItemDependencyEdge {
                from: from.clone(),
                to: relation.defining_item.clone(),
                cause: ItemDependencyCause::RelationUse,
                site: declaration.span,
            });
        }
    }
    for value in declaration.properties.values() {
        collect_item_dependencies_from_value(value, from, output);
    }
    for child in &declaration.children {
        collect_item_dependencies(child, from, output);
    }
}

fn collect_item_dependencies_from_value(
    value: &ResolvedValue,
    from: &ModuleItemId,
    output: &mut Vec<ItemDependencyEdge>,
) {
    match value {
        ResolvedValue::Object {
            properties,
            children,
            ..
        } => {
            for value in properties.values() {
                collect_item_dependencies_from_value(value, from, output);
            }
            for child in children {
                collect_item_dependencies(child, from, output);
            }
        }
        ResolvedValue::Array(values) => {
            for value in values {
                collect_item_dependencies_from_value(value, from, output);
            }
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_item_dependencies_from_value(value, from, output);
        }
        ResolvedValue::ChannelValue(channel) => {
            collect_item_dependencies_from_value(&channel.head.expression, from, output);
            if let Some(otherwise) = &channel.otherwise {
                collect_item_dependencies_from_value(&otherwise.expression, from, output);
            }
            for condition in &channel.conditions {
                collect_item_dependencies_from_value(&condition.predicate, from, output);
                collect_item_dependencies_from_value(&condition.branch.expression, from, output);
            }
            for value in channel.configuration.values() {
                collect_item_dependencies_from_value(value, from, output);
            }
        }
        ResolvedValue::Call { args, .. } => {
            for value in args {
                collect_item_dependencies_from_value(value, from, output);
            }
        }
        _ => {}
    }
}

fn collect_item_closure(
    item: &ModuleItemId,
    adjacency: &BTreeMap<ModuleItemId, BTreeSet<ModuleItemId>>,
    closure: &mut BTreeSet<ModuleItemId>,
) {
    if !closure.insert(item.clone()) {
        return;
    }
    for dependency in adjacency.get(item).into_iter().flatten() {
        collect_item_closure(dependency, adjacency, closure);
    }
}

fn dependency_path(
    start: &ModuleItemId,
    target: &ModuleItemId,
    adjacency: &BTreeMap<ModuleItemId, BTreeSet<ModuleItemId>>,
) -> Vec<ModuleItemId> {
    let mut pending = std::collections::VecDeque::from([start.clone()]);
    let mut parent = BTreeMap::<ModuleItemId, ModuleItemId>::new();
    let mut visited = BTreeSet::from([start.clone()]);
    while let Some(item) = pending.pop_front() {
        if &item == target {
            let mut path = vec![item.clone()];
            let mut cursor = item;
            while let Some(previous) = parent.get(&cursor).cloned() {
                path.push(previous.clone());
                cursor = previous;
            }
            path.reverse();
            return path;
        }
        for dependency in adjacency.get(&item).into_iter().flatten() {
            if visited.insert(dependency.clone()) {
                parent.insert(dependency.clone(), item.clone());
                pending.push_back(dependency.clone());
            }
        }
    }
    vec![start.clone(), target.clone()]
}

fn module_item_display(item: &ModuleItemId) -> String {
    format!("{}::{}", item.module.as_str(), item.declaration.as_str())
}

fn insert_public_target(
    path: &str,
    target: &ResolvedTarget,
    span: SourceSpan,
    output: &mut BTreeMap<String, ResolvedTarget>,
    origins: &mut BTreeMap<String, SourceSpan>,
    collisions: &mut Vec<(String, SourceSpan, SourceSpan)>,
) {
    if let Some(first) = origins.get(path) {
        collisions.push((path.to_owned(), *first, span));
        return;
    }
    origins.insert(path.to_owned(), span);
    output.insert(path.to_owned(), target.clone());
}

fn unique_child<'a>(children: &'a [Decl], keyword: &str) -> Option<&'a Decl> {
    let mut matches = children
        .iter()
        .filter(|child| child.keyword.as_str() == keyword);
    let child = matches.next()?;
    matches.next().is_none().then_some(child)
}

fn static_store_key(row: &Decl, primary_key: &[String]) -> Option<String> {
    if primary_key.is_empty() {
        return None;
    }
    let values = primary_key
        .iter()
        .map(|field| row.props.get(field))
        .collect::<Option<Vec<_>>>()?;
    if !values
        .iter()
        .all(|value| matches!(value, Value::Str(_) | Value::Num(_) | Value::Bool(_)))
    {
        return None;
    }
    serde_json::to_string(&values).ok()
}

fn unique_resolved_child<'a>(
    children: &'a [ResolvedDeclaration],
    keyword: &str,
) -> Option<&'a ResolvedDeclaration> {
    let mut matches = children.iter().filter(|child| child.keyword == keyword);
    let child = matches.next()?;
    matches.next().is_none().then_some(child)
}

fn find_import_trace(
    current: SourceId,
    target: SourceId,
    imports: &[ModuleImportEdge],
    visiting: &mut BTreeSet<SourceId>,
    path: &mut Vec<ExpansionOrImportFrame>,
) -> bool {
    if current == target {
        return true;
    }
    if !visiting.insert(current) {
        return false;
    }
    let mut edges = imports
        .iter()
        .filter(|edge| edge.importer_source == current)
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        left.specifier
            .cmp(&right.specifier)
            .then_with(|| left.site.cmp(&right.site))
    });
    for edge in edges {
        path.push(ExpansionOrImportFrame {
            span: edge.site,
            message: format!("imported from `{}`", edge.specifier),
        });
        if edge
            .imported_source
            .is_some_and(|source| find_import_trace(source, target, imports, visiting, path))
        {
            return true;
        }
        path.pop();
    }
    visiting.remove(&current);
    false
}

fn declaration_coordinate(declaration: &Decl, inherited: Option<&str>) -> Option<String> {
    match declaration.keyword.as_str() {
        "chart" | "cell" | "plot" | "view" => declaration.kind.as_ref().map(ToString::to_string),
        _ => inherited.map(str::to_owned),
    }
}

fn coordinate_at_path(file: &ParsedModule, path: &[usize]) -> Option<String> {
    let mut coordinate = None;
    for length in 1..=path.len() {
        if path[length - 1] == MARK_BLOCK_PATH_SEGMENT {
            coordinate = Some("cartesian".to_owned());
        }
        if let Some(declaration) = declaration_at(file, &path[..length]) {
            coordinate = declaration_coordinate(declaration, coordinate.as_deref());
        }
    }
    coordinate
}

fn parent_declaration<'a>(file: &'a ParsedModule, path: &[usize]) -> Option<&'a Decl> {
    if let Some(segment) = path
        .iter()
        .rposition(|value| *value == MARK_BLOCK_PATH_SEGMENT)
        && path.len() == segment + 3
    {
        return None;
    }
    (path.len() > 1).then(|| declaration_at(file, &path[..path.len() - 1]))?
}

fn requires_registered_kind(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart"
            | "cell"
            | "plot"
            | "view"
            | "mark"
            | "adjust"
            | "transform"
            | "tool"
            | "widget"
            | "resource"
    ) && !is_mark_group(declaration)
        && !matches!(
            (
                declaration.keyword.as_str(),
                declaration.kind.as_ref().map(|kind| kind.as_str())
            ),
            ("tool", Some("behavior")) | ("adjust", None)
        )
}

fn is_mark_group(declaration: &Decl) -> bool {
    declaration.keyword.as_str() == "mark"
        && declaration.kind.as_ref().map(|kind| kind.as_str()) == Some("group")
}

fn core_mark_group_schema(coordinate: Option<&str>) -> KindSchema {
    KindSchema::new(
        NativeKindKey::mark(coordinate.unwrap_or(""), "group"),
        "Logical mark group for shared data preparation and recursive mark authoring.",
    )
    .body_mode(BodyMode::Mixed)
}

/// Returns whether a core declaration keyword may appear below `parent`.
///
/// This is shared by semantic validation and editor completion so tooling
/// cannot drift from the resolver's placement rules.
pub fn placement_allowed(parent: &str, child: &str) -> bool {
    if child == "view" {
        return parent == "mark";
    }
    match parent {
        "define" => {
            matches!(
                child,
                "slot" | "channel" | "output" | "export" | "match" | "splice"
            ) || ordinary_plot_child(child)
        }
        "param" | "field" | "row" | "slot" | "channel" | "output" | "export" => false,
        "store" => matches!(child, "field" | "row"),
        "widget" => false,
        "transform" => matches!(child, "transform" | "output" | "match" | "splice"),
        "on" => matches!(child, "set" | "on" | "match" | "splice"),
        "match" => child == "arm",
        "arm" => !matches!(child, "slot" | "channel" | "output" | "export" | "arm"),
        "view" => matches!(child, "transform" | "mark"),
        "mark" => matches!(
            child,
            "view" | "plot" | "adjust" | "derive" | "part" | "match" | "splice"
        ),
        "table" => matches!(child, "param" | "field" | "row" | "key"),
        "catalog" => matches!(child, "schema"),
        "schema" => matches!(child, "table"),
        _ => ordinary_plot_child(child) || matches!(child, "export" | "set" | "match" | "splice"),
    }
}

/// Canonical declaration-keyword inventory accepted by the resolver.
pub const DECLARATION_KEYWORDS: &[&str] = &[
    "adjust",
    "catalog",
    "cell",
    "channel",
    "define",
    "derive",
    "dimension",
    "export",
    "field",
    "key",
    "layer",
    "level",
    "mark",
    "match",
    "on",
    "output",
    "param",
    "part",
    "plot",
    "resource",
    "row",
    "scale_edit",
    "scale_hint",
    "schema",
    "selection",
    "set",
    "slot",
    "store",
    "table",
    "theme",
    "tool",
    "transform",
    "variable",
    "view",
    "when",
    "widget",
];

/// Declaration keywords valid below `parent`, in deterministic lexical order.
pub fn allowed_child_declarations(parent: &str) -> impl Iterator<Item = &'static str> + '_ {
    DECLARATION_KEYWORDS
        .iter()
        .copied()
        .filter(move |child| placement_allowed(parent, child))
}

fn ordinary_plot_child(child: &str) -> bool {
    matches!(
        child,
        "param"
            | "store"
            | "selection"
            | "resource"
            | "theme"
            | "mark"
            | "transform"
            | "tool"
            | "widget"
            | "view"
            | "on"
            | "cell"
            | "plot"
            | "variable"
            | "part"
            | "level"
            | "adjust"
            | "derive"
            | "layer"
            | "when"
            | "scale_edit"
            | "scale_hint"
            | "dimension"
    )
}

fn core_property(declaration: &Decl, property: &str) -> bool {
    if is_mark_group(declaration) {
        return matches!(
            property,
            "data"
                | "component_kind"
                | "label"
                | "visible"
                | "details"
                | "zindex"
                | "facet_data_scope"
                | "geometry_space"
        );
    }
    match declaration.keyword.as_str() {
        "chart" | "plot" => {
            matches!(
                property,
                "data" | "title" | "subtitle" | "layout" | "theme" | "time" | "format" | "guide"
            )
        }
        "cell" => matches!(property, "at" | "data" | "label" | "when"),
        "view" => property == "data",
        "mark" => matches!(property, "data"),
        "tool" => matches!(property, "id"),
        "widget" => false,
        "transform" | "adjust" => false,
        _ => true,
    }
}

fn schema_property<'a>(schema: &'a KindSchema, name: &str) -> Option<&'a ValueShape> {
    schema
        .properties
        .get(name)
        .map(|property| &property.shape)
        .or_else(|| schema.channels.get(name).map(|channel| &channel.shape))
        .or_else(|| {
            schema
                .additional_properties
                .as_ref()
                .map(|property| &property.shape)
        })
}

fn resolved_output_shape(shape: &ValueShape) -> ResolvedOutputShape {
    match shape {
        ValueShape::SqlExpression | ValueShape::SqlProjection { .. } => {
            ResolvedOutputShape::Expression
        }
        ValueShape::RasterDimension => ResolvedOutputShape::RasterDimension,
        _ => ResolvedOutputShape::Opaque,
    }
}

fn declaration_public_path(
    declaration: &Decl,
    parent: Option<&str>,
    inside_private: bool,
) -> Option<String> {
    if declaration.keyword.as_str() == "on" {
        return None;
    }
    let name = declaration.name.as_ref()?.as_str();
    if declaration.visibility == Visibility::Private
        || declaration.keyword.as_str() == "view"
        || (inside_private && declaration.visibility != Visibility::Public)
    {
        return None;
    }
    Some(match parent {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_owned(),
    })
}

fn contains_descendant(declaration: &Decl, keyword: &str) -> bool {
    declaration
        .children
        .iter()
        .any(|child| child.keyword.as_str() == keyword || contains_descendant(child, keyword))
}

fn contains_overlay_mark_block(value: &Value) -> bool {
    match value {
        Value::Block { head, body } => {
            body.props.iter().any(|(name, value)| {
                (name.as_str() == "overlay" && matches!(value, Value::Block { .. }))
                    || contains_overlay_mark_block(value)
            }) || head.as_deref().is_some_and(contains_overlay_mark_block)
        }
        Value::Array(values) | Value::Call { args: values, .. } => {
            values.iter().any(contains_overlay_mark_block)
        }
        Value::Channel {
            expression: value, ..
        }
        | Value::Pattern(value) => contains_overlay_mark_block(value),
        _ => false,
    }
}

fn overlay_mark_blocks(declaration: &Decl) -> Vec<&crate::ast::Body> {
    fn collect<'a>(value: &'a Value, output: &mut Vec<&'a crate::ast::Body>) {
        match value {
            Value::Block { head, body } => {
                if let Some(head) = head {
                    collect(head, output);
                }
                for (name, value) in body.props.iter() {
                    if name.as_str() == "overlay"
                        && let Value::Block { body, .. } = value
                    {
                        output.push(body);
                    } else {
                        collect(value, output);
                    }
                }
            }
            Value::Array(values) | Value::Call { args: values, .. } => {
                for value in values {
                    collect(value, output);
                }
            }
            Value::Channel {
                expression: value, ..
            }
            | Value::Pattern(value) => collect(value, output),
            _ => {}
        }
    }

    let mut output = Vec::new();
    for (_, value) in declaration.props.iter() {
        collect(value, &mut output);
    }
    output
}

fn resolved_channel_branch(value: &ResolvedValue) -> Option<ResolvedChannelBranch> {
    let ResolvedValue::Object {
        head: None,
        kind: None,
        properties,
        children,
    } = value
    else {
        return None;
    };
    if !children.is_empty() {
        return None;
    }
    resolved_channel_branch_from_properties(properties)
}

fn resolved_channel_branch_from_properties(
    properties: &BTreeMap<String, ResolvedValue>,
) -> Option<ResolvedChannelBranch> {
    let (mode, expression) = match (properties.get("encoded"), properties.get("direct")) {
        (Some(expression), None) => (crate::ast::ChannelMode::Encoded, expression),
        (None, Some(expression)) => (crate::ast::ChannelMode::Direct, expression),
        _ => return None,
    };
    Some(ResolvedChannelBranch {
        mode,
        expression: Box::new(expression.clone()),
    })
}

fn value_matches_shape(value: &ResolvedValue, shape: &ValueShape) -> bool {
    match shape {
        ValueShape::Any => !matches!(value, ResolvedValue::Invalid),
        ValueShape::ParamChangeAction => matches!(
            value,
            ResolvedValue::Object {
                head: None,
                kind: None,
                ..
            }
        ),
        ValueShape::Boolean => matches!(value, ResolvedValue::Boolean(_)),
        ValueShape::Integer => {
            matches!(value, ResolvedValue::Number(value) if value.parse::<i64>().is_ok())
        }
        ValueShape::Number => matches!(value, ResolvedValue::Number(_)),
        ValueShape::String => matches!(value, ResolvedValue::String(_)),
        ValueShape::Identifier => {
            matches!(value, ResolvedValue::String(_) | ResolvedValue::Atom(_))
        }
        ValueShape::Atom { values } => {
            matches!(value, ResolvedValue::Atom(value) if values.iter().any(|candidate| candidate.value == *value))
        }
        ValueShape::SqlExpression => {
            is_expression_value(value)
                || matches!(
                    value,
                    ResolvedValue::Object {
                        head: Some(head),
                        kind: None,
                        ..
                    } if is_expression_value(head)
                )
        }
        ValueShape::SqlProjection { policy, .. } => {
            let ResolvedValue::Projection(projection) = value else {
                return false;
            };
            !projection.items.is_empty()
                && projection.items.iter().all(|item| {
                    if item.expression.is_none()
                        || item.aliases.len() > 1
                        || item.aliases_quoted.iter().any(|quoted| *quoted)
                        || item
                            .aliases
                            .iter()
                            .any(|alias| crate::ast::Name::new(alias.clone()).is_err())
                    {
                        return false;
                    }
                    match policy {
                        avenger_chart_schema::ProjectionPolicy::Named => item.aliases.len() == 1,
                        avenger_chart_schema::ProjectionPolicy::Select => {
                            item.aliases.len() == 1
                                || (item.aliases.is_empty() && item.direct_column)
                        }
                    }
                })
        }
        ValueShape::SqlQuery => matches!(value, ResolvedValue::Query(_)),
        ValueShape::ChannelConfig => matches!(
            value,
            ResolvedValue::Object {
                head: None,
                kind: None,
                ..
            }
        ),
        ValueShape::ConfiguredExpression(_) => matches!(
            value,
            ResolvedValue::Object {
                head: Some(_),
                kind: None,
                children,
                ..
            } if children.is_empty()
        ),
        ValueShape::ConfiguredReference { namespaces, .. } => matches!(
            value,
            ResolvedValue::Object {
                head: Some(head),
                kind: None,
                children,
                ..
            } if children.is_empty()
                && value_matches_shape(
                    head,
                    &ValueShape::TypedReference {
                        namespaces: namespaces.clone(),
                    },
                )
        ),
        ValueShape::PatternChannel => {
            matches!(value, ResolvedValue::Pattern(_))
                || value_matches_shape(value, &ValueShape::SqlExpression)
        }
        ValueShape::CoordinationScope => match value {
            ResolvedValue::Atom(value) => matches!(value.as_str(), "shared" | "free"),
            ResolvedValue::Call { function, args } => {
                function == "level"
                    && matches!(
                        args.as_slice(),
                        [ResolvedValue::Number(value)] if value.parse::<u8>().is_ok()
                    )
            }
            _ => false,
        },
        ValueShape::FacetDataScope => match value {
            ResolvedValue::Atom(value) => matches!(value.as_str(), "filtered" | "broadcast"),
            ResolvedValue::Call { function, args } => {
                function == "level"
                    && matches!(
                        args.as_slice(),
                        [ResolvedValue::Number(value)] if value.parse::<u8>().is_ok()
                    )
            }
            _ => false,
        },
        ValueShape::RasterDimension => matches!(value, ResolvedValue::Dimension(_)),
        ValueShape::RasterDimensionChannel => match value {
            ResolvedValue::Dimension(_) => true,
            ResolvedValue::Object {
                head: Some(head),
                kind: None,
                ..
            } => matches!(head.as_ref(), ResolvedValue::Dimension(_)),
            _ => false,
        },
        ValueShape::ScalarBinding => {
            matches!(
                value,
                ResolvedValue::Binding(ResolvedBinding {
                    target: ResolvedTarget::Param(_) | ResolvedTarget::DefinitionParam { .. },
                    ..
                })
            )
        }
        ValueShape::TableBinding => {
            matches!(
                value,
                ResolvedValue::Binding(ResolvedBinding {
                    target: ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. },
                    ..
                })
            )
        }
        ValueShape::SelectionBinding => matches!(
            value,
            ResolvedValue::Reference(ResolvedReference {
                target: ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. },
                ..
            })
        ),
        ValueShape::WidgetData => {
            matches!(value, ResolvedValue::Object { .. })
                || value_matches_shape(value, &ValueShape::TableBinding)
        }
        ValueShape::MarkBlock => matches!(
            value,
            ResolvedValue::Object {
                kind: None,
                head: None,
                properties,
                children,
            } if properties.is_empty()
                && !children.is_empty()
                && children.iter().all(|child| child.keyword == "mark")
        ),
        ValueShape::TypedReference { namespaces } => match value {
            ResolvedValue::Reference(reference) => namespaces
                .iter()
                .any(|namespace| namespace_matches_target(*namespace, &reference.target)),
            _ => false,
        },
        ValueShape::Union(shapes) => shapes.iter().any(|shape| value_matches_shape(value, shape)),
        ValueShape::OneOrMany(inner) => match value {
            ResolvedValue::Array(values) => {
                values.iter().all(|value| value_matches_shape(value, inner))
            }
            value => value_matches_shape(value, inner),
        },
        ValueShape::Array(inner) => {
            matches!(value, ResolvedValue::Array(values) if values.iter().all(|value| value_matches_shape(value, inner)))
        }
        ValueShape::Map(inner) => match value {
            ResolvedValue::Object { properties, .. } => properties
                .values()
                .all(|value| value_matches_shape(value, inner)),
            _ => false,
        },
        ValueShape::ChannelMap => match value {
            ResolvedValue::Object { properties, .. } => properties.values().all(|value| {
                matches!(
                    value,
                    ResolvedValue::Channel { .. }
                        | ResolvedValue::ChannelValue(_)
                        | ResolvedValue::None
                        | ResolvedValue::Pattern(_)
                        | ResolvedValue::Object {
                            head: Some(_),
                            kind: None,
                            ..
                        }
                )
            }),
            _ => false,
        },
        ValueShape::Object(_) => matches!(value, ResolvedValue::Object { .. }),
    }
}

fn is_self_contained_row_free(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::String(_)
        | ResolvedValue::Relation(_)
        | ResolvedValue::Number(_)
        | ResolvedValue::Boolean(_)
        | ResolvedValue::Null => true,
        ResolvedValue::Atom(value) => value.eq_ignore_ascii_case("null"),
        ResolvedValue::Expression(expression) => {
            expression.bindings.is_empty()
                && expression.helpers.is_empty()
                && expression.contextual_accesses.is_empty()
                && expression.references.is_empty()
                && SqlExpression::parse(&expression.sql)
                    .is_ok_and(|expression| expression.column_identifier_values().is_empty())
        }
        ResolvedValue::Array(values) => values.iter().all(is_self_contained_row_free),
        ResolvedValue::Call { args, .. } => args.iter().all(is_self_contained_row_free),
        ResolvedValue::Object {
            head: None,
            properties,
            children,
            ..
        } => children.is_empty() && properties.values().all(is_self_contained_row_free),
        ResolvedValue::ChannelValue(channel) => {
            is_self_contained_row_free(&channel.head.expression)
                && channel
                    .otherwise
                    .as_ref()
                    .is_none_or(|branch| is_self_contained_row_free(&branch.expression))
                && channel.conditions.iter().all(|condition| {
                    is_self_contained_row_free(&condition.predicate)
                        && is_self_contained_row_free(&condition.branch.expression)
                })
                && channel
                    .configuration
                    .values()
                    .all(is_self_contained_row_free)
        }
        _ => false,
    }
}

fn resolved_value_contains_invalid(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::Invalid => true,
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => resolved_value_contains_invalid(value),
        ResolvedValue::ChannelValue(channel) => {
            resolved_value_contains_invalid(&channel.head.expression)
                || channel
                    .otherwise
                    .as_ref()
                    .is_some_and(|branch| resolved_value_contains_invalid(&branch.expression))
                || channel.conditions.iter().any(|condition| {
                    resolved_value_contains_invalid(&condition.predicate)
                        || resolved_value_contains_invalid(&condition.branch.expression)
                })
                || channel
                    .configuration
                    .values()
                    .any(resolved_value_contains_invalid)
        }
        ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
            values.iter().any(resolved_value_contains_invalid)
        }
        ResolvedValue::Object {
            head,
            properties,
            children,
            ..
        } => {
            head.as_deref().is_some_and(resolved_value_contains_invalid)
                || properties.values().any(resolved_value_contains_invalid)
                || children.iter().any(|child| {
                    child
                        .properties
                        .values()
                        .any(resolved_value_contains_invalid)
                })
        }
        _ => false,
    }
}

fn namespace_matches_target(namespace: NativeKindNamespace, target: &ResolvedTarget) -> bool {
    matches!(
        (namespace, target),
        (NativeKindNamespace::Mark, ResolvedTarget::Mark(_))
            | (NativeKindNamespace::Tool, ResolvedTarget::Tool(_))
            | (NativeKindNamespace::Widget, ResolvedTarget::Widget(_))
            | (
                NativeKindNamespace::Resource,
                ResolvedTarget::Declaration(_)
            )
            | (
                NativeKindNamespace::Mark | NativeKindNamespace::Tool | NativeKindNamespace::Widget,
                ResolvedTarget::DefinitionStructural { .. }
            )
    )
}

fn native_namespace_ref_kind(namespace: &NativeKindNamespace) -> Option<RefKind> {
    Some(match namespace {
        NativeKindNamespace::Mark => RefKind::Mark,
        NativeKindNamespace::Tool => RefKind::Tool,
        NativeKindNamespace::Widget => RefKind::Widget,
        NativeKindNamespace::Resource => RefKind::Resource,
        _ => return None,
    })
}

fn reference_kind_matches(target: &ResolvedTarget, kind: RefKind) -> bool {
    matches!(
        (kind, target),
        (
            RefKind::Mark,
            ResolvedTarget::Mark(_) | ResolvedTarget::Part { .. }
        ) | (
            RefKind::Mark,
            ResolvedTarget::DefinitionStructural {
                kind: DefinitionExportKind::Mark,
                ..
            }
        ) | (
            RefKind::Selection,
            ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. }
        ) | (RefKind::Tool, ResolvedTarget::Tool(_))
            | (
                RefKind::Tool,
                ResolvedTarget::DefinitionStructural {
                    kind: DefinitionExportKind::Tool,
                    ..
                }
            )
            | (RefKind::Widget, ResolvedTarget::Widget(_))
            | (
                RefKind::Widget,
                ResolvedTarget::DefinitionStructural {
                    kind: DefinitionExportKind::Widget,
                    ..
                }
            )
            | (RefKind::Resource, ResolvedTarget::Declaration(_))
            | (_, ResolvedTarget::DefinitionSlot { .. })
    )
}

fn target_matches_definition_ref_kind(target: &ResolvedTarget, kind: &str) -> bool {
    match kind {
        "param" => matches!(
            target,
            ResolvedTarget::Param(_) | ResolvedTarget::DefinitionParam { .. }
        ),
        "store" => matches!(
            target,
            ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. }
        ),
        "selection" => matches!(
            target,
            ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. }
        ),
        "mark" => reference_kind_matches(target, RefKind::Mark),
        "tool" => reference_kind_matches(target, RefKind::Tool),
        "widget" => reference_kind_matches(target, RefKind::Widget),
        "resource" => reference_kind_matches(target, RefKind::Resource),
        _ => false,
    }
}

fn definition_ref_kind(kind: &str) -> Option<RefKind> {
    Some(match kind {
        "mark" => RefKind::Mark,
        "selection" => RefKind::Selection,
        "tool" => RefKind::Tool,
        "widget" => RefKind::Widget,
        "resource" => RefKind::Resource,
        // Param and store ref slots use `$` value-binding syntax and therefore
        // do not have a `RefKind` representation.
        "param" | "store" => return None,
        _ => return None,
    })
}

fn is_expression_value(value: &ResolvedValue) -> bool {
    matches!(
        value,
        ResolvedValue::String(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::Boolean(_)
            | ResolvedValue::Null
            | ResolvedValue::Column(_)
            | ResolvedValue::Atom(_)
            | ResolvedValue::Expression(_)
            | ResolvedValue::Binding(_)
            | ResolvedValue::Object { .. }
            | ResolvedValue::DefinitionArgument(_)
    )
}

fn is_literal_value(value: &ResolvedValue) -> bool {
    matches!(
        value,
        ResolvedValue::String(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::Boolean(_)
            | ResolvedValue::Null
    )
}

fn collect_referenced_targets(
    declaration: &ResolvedDeclaration,
    targets: &mut BTreeSet<ResolvedTarget>,
) {
    for value in declaration.properties.values() {
        collect_value_targets(value, targets);
    }
    if let Some(binding) = &declaration.event_binding {
        targets.extend(binding.targets.iter().cloned());
    }
    if let Some(lvalue) = &declaration.state_lvalue {
        targets.insert(lvalue.target.clone());
    }
    for child in &declaration.children {
        collect_referenced_targets(child, targets);
    }
}

fn collect_value_targets(value: &ResolvedValue, targets: &mut BTreeSet<ResolvedTarget>) {
    match value {
        ResolvedValue::Binding(binding) => {
            targets.insert(binding.target.clone());
        }
        ResolvedValue::Reference(reference) => {
            targets.insert(reference.target.clone());
        }
        ResolvedValue::Expression(expression) => {
            targets.extend(
                expression
                    .bindings
                    .iter()
                    .map(|binding| binding.target.clone()),
            );
            targets.extend(
                expression
                    .references
                    .iter()
                    .map(|reference| reference.target.clone()),
            );
        }
        ResolvedValue::Projection(projection) => {
            for item in &projection.items {
                if let Some(expression) = &item.expression {
                    targets.extend(
                        expression
                            .bindings
                            .iter()
                            .map(|binding| binding.target.clone()),
                    );
                    targets.extend(
                        expression
                            .references
                            .iter()
                            .map(|reference| reference.target.clone()),
                    );
                }
            }
        }
        ResolvedValue::Query(query) => {
            targets.extend(query.bindings.iter().map(|binding| binding.target.clone()));
            targets.extend(
                query
                    .references
                    .iter()
                    .map(|reference| reference.target.clone()),
            );
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_value_targets(value, targets);
        }
        ResolvedValue::ChannelValue(channel) => {
            collect_value_targets(&channel.head.expression, targets);
            if let Some(otherwise) = &channel.otherwise {
                collect_value_targets(&otherwise.expression, targets);
            }
            for condition in &channel.conditions {
                collect_value_targets(&condition.predicate, targets);
                collect_value_targets(&condition.branch.expression, targets);
            }
            for value in channel.configuration.values() {
                collect_value_targets(value, targets);
            }
        }
        ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
            for value in values {
                collect_value_targets(value, targets);
            }
        }
        ResolvedValue::Object {
            head,
            properties,
            children,
            ..
        } => {
            if let Some(head) = head {
                collect_value_targets(head, targets);
            }
            for value in properties.values() {
                collect_value_targets(value, targets);
            }
            for child in children {
                collect_referenced_targets(child, targets);
            }
        }
        ResolvedValue::DefinitionArgument(target) => {
            targets.insert(target.clone());
        }
        ResolvedValue::String(_)
        | ResolvedValue::Relation(_)
        | ResolvedValue::Number(_)
        | ResolvedValue::Boolean(_)
        | ResolvedValue::Null
        | ResolvedValue::Atom(_)
        | ResolvedValue::Column(_)
        | ResolvedValue::Dimension(_)
        | ResolvedValue::Environment(_)
        | ResolvedValue::None
        | ResolvedValue::Invalid => {}
    }
}

fn shape_name(shape: &ValueShape) -> &'static str {
    match shape {
        ValueShape::Boolean => "boolean",
        ValueShape::Integer => "integer",
        ValueShape::Number => "number",
        ValueShape::String => "string",
        ValueShape::Identifier => "identifier",
        ValueShape::Atom { .. } => "enum atom",
        ValueShape::SqlExpression => "SQL expression",
        ValueShape::SqlProjection { policy, .. } => match policy {
            avenger_chart_schema::ProjectionPolicy::Named => {
                "named SQL projection list (`expression AS name, ...`)"
            }
            avenger_chart_schema::ProjectionPolicy::Select => {
                "SQL select list (direct columns or `expression AS name`)"
            }
        },
        ValueShape::SqlQuery => "SQL query",
        ValueShape::ChannelConfig => "configuration-only channel block",
        ValueShape::ConfiguredExpression(_) => "configured SQL expression",
        ValueShape::ConfiguredReference { .. } => "configured typed reference",
        ValueShape::PatternChannel => "pattern literal or configured pattern channel",
        ValueShape::CoordinationScope => "coordination scope",
        ValueShape::FacetDataScope => "facet data scope",
        ValueShape::RasterDimension => "raster dimension",
        ValueShape::RasterDimensionChannel => "configured raster dimension",
        ValueShape::ScalarBinding => "param binding",
        ValueShape::TableBinding => "store binding",
        ValueShape::SelectionBinding => "selection reference",
        ValueShape::WidgetData => "widget data source",
        ValueShape::ParamChangeAction => "ordered parameter-change action block",
        ValueShape::MarkBlock => "mark-only block",
        ValueShape::TypedReference { .. } => "typed reference",
        ValueShape::Union(_) => "one of the allowed shapes",
        ValueShape::OneOrMany(_) => "value or array",
        ValueShape::Array(_) => "array",
        ValueShape::Map(_) => "property map",
        ValueShape::ChannelMap => "configured channel map",
        ValueShape::Object(_) => "object",
        ValueShape::Any => "value",
    }
}

fn resolved_shape(value: &ResolvedValue) -> &'static str {
    match value {
        ResolvedValue::String(_) => "string",
        ResolvedValue::Number(_) => "number",
        ResolvedValue::Boolean(_) => "boolean",
        ResolvedValue::Null => "null",
        ResolvedValue::Atom(_) => "atom",
        ResolvedValue::Column(_) => "column",
        ResolvedValue::Expression(_) => "SQL expression",
        ResolvedValue::Projection(_) => "SQL projection list",
        ResolvedValue::Query(_) => "SQL query",
        ResolvedValue::Relation(_) => "relation path",
        ResolvedValue::Binding(_) => "binding",
        ResolvedValue::Reference(_) => "reference",
        ResolvedValue::Channel { mode, .. } => match mode {
            crate::ast::ChannelMode::Encoded => "encoded channel value",
            crate::ast::ChannelMode::Direct => "direct channel value",
        },
        ResolvedValue::ChannelValue(_) => "resolved channel value",
        ResolvedValue::Dimension(_) => "dimension",
        ResolvedValue::Pattern(_) => "pattern",
        ResolvedValue::Environment(_) => "environment value",
        ResolvedValue::None => "none",
        ResolvedValue::Array(_) => "array",
        ResolvedValue::Object { .. } => "object",
        ResolvedValue::Call { .. } => "call",
        ResolvedValue::DefinitionArgument(_) => "definition argument",
        ResolvedValue::Invalid => "invalid value",
    }
}

fn resolved_json(value: &serde_json::Value) -> ResolvedValue {
    match value {
        serde_json::Value::Null => ResolvedValue::Null,
        serde_json::Value::Bool(value) => ResolvedValue::Boolean(*value),
        serde_json::Value::Number(value) => ResolvedValue::Number(value.to_string()),
        serde_json::Value::String(value) => ResolvedValue::String(value.clone()),
        serde_json::Value::Array(values) => {
            ResolvedValue::Array(values.iter().map(resolved_json).collect())
        }
        serde_json::Value::Object(values) => ResolvedValue::Object {
            head: None,
            kind: None,
            properties: values
                .iter()
                .map(|(name, value)| (name.clone(), resolved_json(value)))
                .collect(),
            children: Vec::new(),
        },
    }
}

fn resolved_schema_default(value: &serde_json::Value, shape: &ValueShape) -> ResolvedValue {
    match (value, shape) {
        (serde_json::Value::String(value), ValueShape::Atom { .. }) => {
            ResolvedValue::Atom(value.clone())
        }
        (serde_json::Value::Array(values), ValueShape::Array(inner)) => ResolvedValue::Array(
            values
                .iter()
                .map(|value| resolved_schema_default(value, inner))
                .collect(),
        ),
        (serde_json::Value::Array(values), ValueShape::OneOrMany(inner)) => ResolvedValue::Array(
            values
                .iter()
                .map(|value| resolved_schema_default(value, inner))
                .collect(),
        ),
        (_, ValueShape::OneOrMany(inner)) => resolved_schema_default(value, inner),
        (_, ValueShape::Union(shapes)) => shapes
            .iter()
            .find(|shape| json_matches_shape(value, shape))
            .map_or_else(
                || resolved_json(value),
                |shape| resolved_schema_default(value, shape),
            ),
        (serde_json::Value::Object(values), ValueShape::Map(inner)) => ResolvedValue::Object {
            head: None,
            kind: None,
            properties: values
                .iter()
                .map(|(name, value)| (name.clone(), resolved_schema_default(value, inner)))
                .collect(),
            children: Vec::new(),
        },
        (serde_json::Value::Object(values), ValueShape::ChannelMap) => ResolvedValue::Object {
            head: None,
            kind: None,
            properties: values
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        resolved_schema_default(value, &ValueShape::SqlExpression),
                    )
                })
                .collect(),
            children: Vec::new(),
        },
        (serde_json::Value::Object(values), ValueShape::Object(fields)) => ResolvedValue::Object {
            head: None,
            kind: None,
            properties: values
                .iter()
                .map(|(name, value)| {
                    let resolved = fields.get(name).map_or_else(
                        || resolved_json(value),
                        |field| resolved_schema_default(value, &field.shape),
                    );
                    (name.clone(), resolved)
                })
                .collect(),
            children: Vec::new(),
        },
        _ => resolved_json(value),
    }
}

fn json_matches_shape(value: &serde_json::Value, shape: &ValueShape) -> bool {
    match (value, shape) {
        (_, ValueShape::Any) => true,
        (serde_json::Value::Bool(_), ValueShape::Boolean) => true,
        (serde_json::Value::Number(value), ValueShape::Integer) => value.is_i64() || value.is_u64(),
        (serde_json::Value::Number(_), ValueShape::Number) => true,
        (
            serde_json::Value::String(_),
            ValueShape::String | ValueShape::Identifier | ValueShape::Atom { .. },
        ) => true,
        (serde_json::Value::Array(values), ValueShape::Array(inner)) => {
            values.iter().all(|value| json_matches_shape(value, inner))
        }
        (serde_json::Value::Array(values), ValueShape::OneOrMany(inner)) => {
            values.iter().all(|value| json_matches_shape(value, inner))
        }
        (value, ValueShape::OneOrMany(inner)) => json_matches_shape(value, inner),
        (
            serde_json::Value::Object(_),
            ValueShape::Map(_)
            | ValueShape::ChannelMap
            | ValueShape::Object(_)
            | ValueShape::MarkBlock,
        ) => true,
        (value, ValueShape::Union(shapes)) => {
            shapes.iter().any(|shape| json_matches_shape(value, shape))
        }
        _ => false,
    }
}

fn resolved_string_array(value: Option<&ResolvedValue>) -> Vec<String> {
    match value {
        Some(ResolvedValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                ResolvedValue::String(value) | ResolvedValue::Atom(value) => Some(value.clone()),
                ResolvedValue::Column(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn resolved_path(value: &ResolvedValue) -> Option<Vec<String>> {
    match value {
        ResolvedValue::Array(values) => values
            .iter()
            .map(|value| match value {
                ResolvedValue::Atom(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn authored_bare_path(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Atom(name) => Some(vec![name.to_string()]),
        Value::Expr(expression) => helper_argument_path(expression.ast()),
        _ => None,
    }
}

fn valid_event_scope(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::Atom(value) => value == "plot",
        ResolvedValue::Call { function, args } => {
            function == "subplot"
                && !args.is_empty()
                && args
                    .iter()
                    .all(|value| matches!(value, ResolvedValue::Atom(_)))
        }
        ResolvedValue::Array(values) => {
            !values.is_empty()
                && values.iter().all(|value| {
                    matches!(value, ResolvedValue::Call { function, args }
                        if function == "subplot"
                            && !args.is_empty()
                            && args.iter().all(|value| matches!(value, ResolvedValue::Atom(_))))
                })
                && !values
                    .iter()
                    .enumerate()
                    .any(|(index, value)| values[..index].contains(value))
        }
        _ => false,
    }
}

fn valid_event_surface(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::Atom(value) => matches!(value.as_str(), "plot" | "all"),
        ResolvedValue::Call { function, args } => {
            function == "legend" && matches!(args.as_slice(), [ResolvedValue::Atom(_)])
        }
        _ => false,
    }
}

fn resolved_value_has_forbidden_stream_binding(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::Binding(binding) => {
            binding.time != BindingTime::Current
                || matches!(
                    &binding.target,
                    ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. }
                )
        }
        ResolvedValue::Expression(expression) => expression.bindings.iter().any(|binding| {
            binding.time != BindingTime::Current
                || matches!(
                    &binding.target,
                    ResolvedTarget::Store(_) | ResolvedTarget::DefinitionStore { .. }
                )
        }),
        ResolvedValue::Query(_) => true,
        ResolvedValue::Array(values) => values
            .iter()
            .any(resolved_value_has_forbidden_stream_binding),
        ResolvedValue::Object {
            head,
            properties,
            children,
            ..
        } => {
            head.as_deref()
                .is_some_and(resolved_value_has_forbidden_stream_binding)
                || properties
                    .values()
                    .any(resolved_value_has_forbidden_stream_binding)
                || children.iter().any(|child| {
                    child
                        .properties
                        .values()
                        .any(resolved_value_has_forbidden_stream_binding)
                })
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => resolved_value_has_forbidden_stream_binding(value),
        ResolvedValue::ChannelValue(channel) => {
            resolved_value_has_forbidden_stream_binding(&channel.head.expression)
                || channel.otherwise.as_ref().is_some_and(|branch| {
                    resolved_value_has_forbidden_stream_binding(&branch.expression)
                })
                || channel.conditions.iter().any(|condition| {
                    resolved_value_has_forbidden_stream_binding(&condition.predicate)
                        || resolved_value_has_forbidden_stream_binding(&condition.branch.expression)
                })
                || channel
                    .configuration
                    .values()
                    .any(resolved_value_has_forbidden_stream_binding)
        }
        ResolvedValue::Call { args, .. } => {
            args.iter().any(resolved_value_has_forbidden_stream_binding)
        }
        _ => false,
    }
}

fn parse_sharing(
    value: Option<&ResolvedValue>,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) -> StateSharing {
    match value {
        None => StateSharing::Shared,
        Some(ResolvedValue::Atom(value)) if value == "shared" => StateSharing::Shared,
        Some(ResolvedValue::Atom(value)) if value == "free" => StateSharing::Free,
        Some(ResolvedValue::Call { function, args }) if function == "level" => {
            match args.as_slice() {
                [ResolvedValue::Number(level)] => level
                    .parse::<u32>()
                    .map(StateSharing::Level)
                    .unwrap_or_else(|_| {
                        diagnostics.push(Diagnostic::error(
                            "AVENGER-RESOLVE-078",
                            "invalid state sharing level",
                            SourceLabel::new(span, "`level(n)` requires a nonnegative integer"),
                        ));
                        StateSharing::Shared
                    }),
                _ => {
                    diagnostics.push(Diagnostic::error(
                        "AVENGER-RESOLVE-078",
                        "invalid state sharing level",
                        SourceLabel::new(span, "use `level(<nonnegative integer>)`"),
                    ));
                    StateSharing::Shared
                }
            }
        }
        Some(_) => {
            diagnostics.push(Diagnostic::error(
                "AVENGER-RESOLVE-078",
                "invalid state sharing mode",
                SourceLabel::new(span, "expected `shared`, `free`, or `level(n)`"),
            ));
            StateSharing::Shared
        }
    }
}

fn resolved_param_dependencies(value: &ResolvedValue) -> BTreeSet<ParamId> {
    let mut output = BTreeSet::new();
    collect_param_dependencies(value, &mut output);
    output
}

fn collect_param_dependencies(value: &ResolvedValue, output: &mut BTreeSet<ParamId>) {
    match value {
        ResolvedValue::Binding(ResolvedBinding {
            target: ResolvedTarget::Param(id),
            ..
        }) => {
            output.insert(id.clone());
        }
        ResolvedValue::Expression(expression) => {
            for binding in &expression.bindings {
                if let ResolvedTarget::Param(id) = &binding.target {
                    output.insert(id.clone());
                }
            }
        }
        ResolvedValue::Array(values) => {
            for value in values {
                collect_param_dependencies(value, output);
            }
        }
        ResolvedValue::Object {
            head,
            properties,
            children,
            ..
        } => {
            if let Some(head) = head {
                collect_param_dependencies(head, output);
            }
            for value in properties.values() {
                collect_param_dependencies(value, output);
            }
            for child in children {
                for value in child.properties.values() {
                    collect_param_dependencies(value, output);
                }
            }
        }
        ResolvedValue::Call { args, .. } => {
            for value in args {
                collect_param_dependencies(value, output);
            }
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_param_dependencies(value, output);
        }
        ResolvedValue::ChannelValue(channel) => {
            collect_param_dependencies(&channel.head.expression, output);
            if let Some(otherwise) = &channel.otherwise {
                collect_param_dependencies(&otherwise.expression, output);
            }
            for condition in &channel.conditions {
                collect_param_dependencies(&condition.predicate, output);
                collect_param_dependencies(&condition.branch.expression, output);
            }
            for value in channel.configuration.values() {
                collect_param_dependencies(value, output);
            }
        }
        _ => {}
    }
}

fn topological_order<K>(dependencies: &BTreeMap<K, BTreeSet<K>>) -> Result<Vec<K>, Vec<K>>
where
    K: Clone + Ord,
{
    fn visit<K>(
        node: &K,
        dependencies: &BTreeMap<K, BTreeSet<K>>,
        temporary: &mut BTreeSet<K>,
        permanent: &mut BTreeSet<K>,
        stack: &mut Vec<K>,
        order: &mut Vec<K>,
    ) -> Result<(), Vec<K>>
    where
        K: Clone + Ord,
    {
        if permanent.contains(node) {
            return Ok(());
        }
        if !temporary.insert(node.clone()) {
            let start = stack
                .iter()
                .position(|candidate| candidate == node)
                .unwrap_or(0);
            let mut cycle = stack[start..].to_vec();
            cycle.push(node.clone());
            return Err(cycle);
        }
        stack.push(node.clone());
        for dependency in dependencies.get(node).into_iter().flatten() {
            visit(dependency, dependencies, temporary, permanent, stack, order)?;
        }
        stack.pop();
        temporary.remove(node);
        permanent.insert(node.clone());
        order.push(node.clone());
        Ok(())
    }

    let mut nodes = dependencies.keys().cloned().collect::<BTreeSet<_>>();
    nodes.extend(dependencies.values().flatten().cloned());
    let mut temporary = BTreeSet::new();
    let mut permanent = BTreeSet::new();
    let mut stack = Vec::new();
    let mut order = Vec::new();
    for node in nodes {
        visit(
            &node,
            dependencies,
            &mut temporary,
            &mut permanent,
            &mut stack,
            &mut order,
        )?;
    }
    Ok(order)
}

fn unresolved_value(value: &Value) -> ResolvedValue {
    match value {
        Value::Str(value) => ResolvedValue::String(value.clone()),
        Value::Num(value) => ResolvedValue::Number(value.as_str().to_owned()),
        Value::Bool(value) => ResolvedValue::Boolean(*value),
        Value::Null => ResolvedValue::Null,
        Value::Column(value) => ResolvedValue::Column(value.clone()),
        Value::Atom(value) => ResolvedValue::Atom(value.to_string()),
        Value::Expr(value) => ResolvedValue::Expression(ResolvedExpression {
            sql: value.canonical_sql(),
            bindings: Vec::new(),
            helpers: helpers_in_sql(&value.canonical_sql()),
            contextual_accesses: Vec::new(),
            references: Vec::new(),
        }),
        Value::Projection(value) => ResolvedValue::Projection(ResolvedProjection {
            sql: value.canonical_sql(),
            items: value
                .items()
                .iter()
                .map(|item| {
                    let (expression, aliases, aliases_quoted) = match item {
                        sqlparser::ast::SelectItem::UnnamedExpr(expression) => {
                            (Some(expression), Vec::new(), Vec::new())
                        }
                        sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => (
                            Some(expr),
                            vec![alias.value.clone()],
                            vec![alias.quote_style.is_some()],
                        ),
                        sqlparser::ast::SelectItem::ExprWithAliases { expr, aliases } => (
                            Some(expr),
                            aliases.iter().map(|alias| alias.value.clone()).collect(),
                            aliases
                                .iter()
                                .map(|alias| alias.quote_style.is_some())
                                .collect(),
                        ),
                        sqlparser::ast::SelectItem::QualifiedWildcard(_, _)
                        | sqlparser::ast::SelectItem::Wildcard(_) => (None, Vec::new(), Vec::new()),
                    };
                    let direct_column = expression.is_some_and(|expression| {
                        matches!(
                            expression,
                            sqlparser::ast::Expr::Identifier(_)
                                | sqlparser::ast::Expr::CompoundIdentifier(_)
                        )
                    });
                    ResolvedProjectionItem {
                        sql: item.to_string(),
                        expression: expression.map(|expression| ResolvedExpression {
                            sql: expression.to_string(),
                            bindings: Vec::new(),
                            helpers: helpers_in_sql(&expression.to_string()),
                            contextual_accesses: Vec::new(),
                            references: Vec::new(),
                        }),
                        aliases,
                        aliases_quoted,
                        direct_column,
                    }
                })
                .collect(),
        }),
        Value::Query(value) => ResolvedValue::Query(ResolvedQuery {
            sql: value.canonical_sql(),
            bindings: Vec::new(),
            helpers: helpers_in_sql(&value.canonical_sql()),
            references: Vec::new(),
            relations: Vec::new(),
        }),
        Value::Relation(path) => ResolvedValue::Relation(ResolvedRelationReference {
            authored_path: path.iter().map(ToString::to_string).collect(),
            target: ResolvedRelationTarget::Input,
        }),
        Value::Binding { kind, path, time } => ResolvedValue::Binding(ResolvedBinding {
            target: ResolvedTarget::Declaration(DeclarationId("unresolved".to_owned())),
            kind: *kind,
            time: *time,
            authored_path: path.iter().map(ToString::to_string).collect(),
        }),
        Value::Ref { kind, path } => ResolvedValue::Reference(ResolvedReference {
            target: ResolvedTarget::Declaration(DeclarationId("unresolved".to_owned())),
            kind: *kind,
            authored_path: path.iter().map(ToString::to_string).collect(),
        }),
        Value::Channel { mode, expression } => ResolvedValue::Channel {
            mode: *mode,
            expression: Box::new(unresolved_value(expression)),
        },
        Value::Dim(path) => ResolvedValue::Dimension(ResolvedDimension {
            target: ResolvedOutputHandle {
                producer: DeclarationId("unresolved".to_owned()),
                name: path.last().map(ToString::to_string).unwrap_or_default(),
                ordinal: 0,
                shape: ResolvedOutputShape::RasterDimension,
            },
            authored_path: path.iter().map(ToString::to_string).collect(),
        }),
        Value::Pattern(value) => ResolvedValue::Pattern(Box::new(unresolved_value(value))),
        Value::Env(value) => ResolvedValue::Environment(value.clone()),
        Value::None => ResolvedValue::None,
        Value::Array(values) => ResolvedValue::Array(values.iter().map(unresolved_value).collect()),
        Value::Block { head, body } => ResolvedValue::Object {
            kind: head.as_deref().and_then(value_atom).map(str::to_owned),
            head: head
                .as_deref()
                .filter(|head| value_atom(head).is_none())
                .map(|head| Box::new(unresolved_value(head))),
            properties: body
                .props
                .iter()
                .map(|(name, value)| (name.to_string(), unresolved_value(value)))
                .collect(),
            children: Vec::new(),
        },
        Value::Call { function, args } => ResolvedValue::Call {
            function: function.to_string(),
            args: args.iter().map(unresolved_value).collect(),
        },
    }
}

fn select_item_expression(item: &sqlparser::ast::SelectItem) -> Option<&sqlparser::ast::Expr> {
    match item {
        sqlparser::ast::SelectItem::UnnamedExpr(expression) => Some(expression),
        sqlparser::ast::SelectItem::ExprWithAlias { expr, .. }
        | sqlparser::ast::SelectItem::ExprWithAliases { expr, .. } => Some(expr),
        sqlparser::ast::SelectItem::QualifiedWildcard(_, _)
        | sqlparser::ast::SelectItem::Wildcard(_) => None,
    }
}

fn definition_value(
    value: &Value,
    definition: &DeclarationId,
    slots: &BTreeSet<String>,
) -> ResolvedValue {
    let mut resolved = unresolved_value(value);
    normalize_definition_value_references(value, &mut resolved, definition, slots);
    resolved
}

fn normalize_definition_value_references(
    source: &Value,
    resolved: &mut ResolvedValue,
    definition: &DeclarationId,
    slots: &BTreeSet<String>,
) {
    match (source, resolved) {
        (Value::Atom(name), resolved) if slots.contains(name.as_str()) => {
            *resolved = ResolvedValue::DefinitionArgument(ResolvedTarget::DefinitionSlot {
                definition: definition.clone(),
                name: name.to_string(),
            });
        }
        (Value::Expr(expression), ResolvedValue::Expression(resolved)) => {
            resolved.references = expression_paths(expression.ast())
                .into_iter()
                .filter_map(|path| {
                    let name = path.first()?.clone();
                    slots.contains(&name).then(|| ResolvedSqlReference {
                        authored_path: path,
                        target: ResolvedTarget::DefinitionSlot {
                            definition: definition.clone(),
                            name,
                        },
                    })
                })
                .collect();
        }
        (Value::Projection(projection), ResolvedValue::Projection(resolved)) => {
            for (source, item) in projection.items().iter().zip(&mut resolved.items) {
                let Some(expression) = select_item_expression(source) else {
                    continue;
                };
                let Some(resolved_expression) = item.expression.as_mut() else {
                    continue;
                };
                resolved_expression.references = expression_paths(expression)
                    .into_iter()
                    .filter_map(|path| {
                        let name = path.first()?.clone();
                        slots.contains(&name).then(|| ResolvedSqlReference {
                            authored_path: path,
                            target: ResolvedTarget::DefinitionSlot {
                                definition: definition.clone(),
                                name,
                            },
                        })
                    })
                    .collect();
            }
        }
        (Value::Query(query), ResolvedValue::Query(resolved)) => {
            resolved.references = query_paths(query.ast())
                .into_iter()
                .filter_map(|path| {
                    let name = path.first()?.clone();
                    slots.contains(&name).then(|| ResolvedSqlReference {
                        authored_path: path,
                        target: ResolvedTarget::DefinitionSlot {
                            definition: definition.clone(),
                            name,
                        },
                    })
                })
                .collect();
        }
        (
            Value::Channel {
                mode: source_mode,
                expression: source,
            },
            ResolvedValue::Channel {
                mode: resolved_mode,
                expression: resolved,
            },
        ) if source_mode == resolved_mode => {
            normalize_definition_value_references(source, resolved, definition, slots);
        }
        (Value::Pattern(source), ResolvedValue::Pattern(resolved)) => {
            normalize_definition_value_references(source, resolved, definition, slots);
        }
        (Value::Array(sources), ResolvedValue::Array(resolved)) => {
            for (source, resolved) in sources.iter().zip(resolved) {
                normalize_definition_value_references(source, resolved, definition, slots);
            }
        }
        (
            Value::Block { head, body },
            ResolvedValue::Object {
                head: resolved_head,
                kind,
                properties,
                ..
            },
        ) => {
            if let (Some(source), Some(resolved)) = (head.as_deref(), kind.as_mut())
                && let Value::Atom(name) = source
                && slots.contains(name.as_str())
            {
                // A block head is structural syntax rather than a value slot;
                // leave its spelling intact for Phase 7 expansion.
                *resolved = name.to_string();
            }
            if let (Some(source), Some(resolved)) = (head.as_deref(), resolved_head.as_deref_mut())
            {
                normalize_definition_value_references(source, resolved, definition, slots);
            }
            for (name, source) in body.props.iter() {
                if let Some(resolved) = properties.get_mut(name.as_str()) {
                    normalize_definition_value_references(source, resolved, definition, slots);
                }
            }
        }
        (Value::Call { args: sources, .. }, ResolvedValue::Call { args: resolved, .. }) => {
            for (source, resolved) in sources.iter().zip(resolved) {
                normalize_definition_value_references(source, resolved, definition, slots);
            }
        }
        _ => {}
    }
}

fn runtime_target(declaration: &Decl, id: &DeclarationId) -> Option<ResolvedTarget> {
    Some(match declaration.keyword.as_str() {
        "mark" => ResolvedTarget::Mark(MarkId(semantic_hash(&["mark", id.as_str()]))),
        "tool" => ResolvedTarget::Tool(ToolId(semantic_hash(&["tool", id.as_str()]))),
        "widget" => ResolvedTarget::Widget(WidgetId(semantic_hash(&["widget", id.as_str()]))),
        "on" => ResolvedTarget::Event(EventId(semantic_hash(&["event", id.as_str()]))),
        _ if is_structural(declaration) => ResolvedTarget::Declaration(id.clone()),
        _ => return None,
    })
}

fn is_structural(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart"
            | "mark"
            | "tool"
            | "widget"
            | "view"
            | "cell"
            | "plot"
            | "variable"
            | "dimension"
            | "resource"
    )
}

fn declaration_binds_name(declaration: &Decl) -> bool {
    declaration.name.is_some() && declaration.keyword.as_str() != "set"
}

fn is_generated_private_binder(name: &str) -> bool {
    is_generated_prefixed_name(name, 12)
}

fn is_generated_bundle_binder(name: &str) -> bool {
    is_generated_prefixed_name(name, 10)
}

fn is_generated_prefixed_name(name: &str, identity_len: usize) -> bool {
    let Some((identity, suffix)) = name
        .strip_prefix("__av_")
        .and_then(|rest| rest.split_once('_'))
    else {
        return false;
    };
    identity.len() == identity_len
        && identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        && !suffix.is_empty()
}

fn is_generated_private_column(name: &str) -> bool {
    let Some((identity, suffix)) = name
        .strip_prefix("__av_col_")
        .and_then(|rest| rest.split_once('_'))
    else {
        return false;
    };
    identity.len() == 12
        && identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        && !suffix.is_empty()
}

fn collect_sql_identifier_values(value: &Value, output: &mut BTreeSet<String>) {
    match value {
        Value::Expr(expression) => output.extend(expression.column_identifier_values()),
        Value::Projection(projection) => {
            output.extend(projection.column_identifier_values());
        }
        Value::Query(query) => output.extend(query.column_identifier_values()),
        Value::Array(values) | Value::Call { args: values, .. } => {
            for value in values {
                collect_sql_identifier_values(value, output);
            }
        }
        Value::Block { head, body } => {
            if let Some(head) = head {
                collect_sql_identifier_values(head, output);
            }
            for (_, value) in body.props.iter() {
                collect_sql_identifier_values(value, output);
            }
        }
        Value::Channel {
            expression: value, ..
        }
        | Value::Pattern(value) => {
            collect_sql_identifier_values(value, output);
        }
        _ => {}
    }
}

fn owns_lexical_scope(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "define" | "view" | "cell" | "plot" | "on" | "table"
    ) || (matches!(declaration.keyword.as_str(), "tool" | "mark")
        && !declaration.children.is_empty())
}

fn is_instance_boundary(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "define" | "tool" | "widget" | "mark" | "view" | "cell" | "plot"
    )
}

fn declaration_id(file: &ParsedModule, path: &[usize]) -> DeclarationId {
    let stable_path = stable_declaration_path(file, path);
    DeclarationId(semantic_hash(&[
        "declaration",
        file.id.as_str(),
        &stable_path,
    ]))
}

fn stable_declaration_path(file: &ParsedModule, path: &[usize]) -> String {
    if let Some(segment) = path
        .iter()
        .rposition(|value| *value == MARK_BLOCK_PATH_SEGMENT)
    {
        let owner_path = &path[..segment];
        let block_ordinal = *path.get(segment + 1).unwrap_or(&0);
        let child_path = path.get(segment + 2..).unwrap_or_default();
        let mut components = vec![
            stable_declaration_path(file, owner_path),
            format!("legend-overlay#{block_ordinal}"),
        ];
        let Some(owner) = declaration_at(file, owner_path) else {
            components.push("missing-owner".to_owned());
            return components.join("/");
        };
        let Some(body) = overlay_mark_blocks(owner).get(block_ordinal).copied() else {
            components.push("missing-overlay".to_owned());
            return components.join("/");
        };
        let Some((first, rest)) = child_path.split_first() else {
            return components.join("/");
        };
        let Some(mut declaration) = body.children.get(*first) else {
            components.push(format!("missing:{first}"));
            return components.join("/");
        };
        let signature = declaration_identity_signature(declaration);
        let ordinal = body.children[..*first]
            .iter()
            .filter(|candidate| declaration_identity_signature(candidate) == signature)
            .count();
        components.push(format!("{signature}#{ordinal}"));
        for index in rest {
            let Some(next) = declaration.children.get(*index) else {
                components.push(format!("missing:{index}"));
                break;
            };
            let signature = declaration_identity_signature(next);
            let ordinal = declaration.children[..*index]
                .iter()
                .filter(|candidate| declaration_identity_signature(candidate) == signature)
                .count();
            components.push(format!("{signature}#{ordinal}"));
            declaration = next;
        }
        return components.join("/");
    }

    let mut components = Vec::with_capacity(path.len());
    for (depth, index) in path.iter().copied().enumerate() {
        let Some(declaration) = declaration_at(file, &path[..=depth]) else {
            components.push(format!("missing:{index}"));
            continue;
        };
        let signature = declaration_identity_signature(declaration);
        let ordinal = if depth == 0 {
            file.parsed.ast.items[..index]
                .iter()
                .filter(|item| declaration_identity_signature(&item.declaration) == signature)
                .count()
        } else {
            declaration_at(file, &path[..depth])
                .map(|parent| {
                    parent.children[..index]
                        .iter()
                        .filter(|candidate| declaration_identity_signature(candidate) == signature)
                        .count()
                })
                .unwrap_or_default()
        };
        components.push(format!("{signature}#{ordinal}"));
    }
    components.join("/")
}

fn declaration_identity_signature(declaration: &Decl) -> String {
    let name = declaration.name.as_ref().map_or("", Name::as_str);
    let kind = declaration.kind.as_ref().map_or("", |kind| kind.as_str());
    format!("{}:{}:{}", declaration.keyword, name, kind)
}

fn semantic_hash(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    let digest = hash.finalize();
    format!("av1_{}", &format!("{digest:x}")[..32])
}

fn path_text(path: &[usize]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn ancestry_text(ancestry: &[DeclarationId]) -> String {
    ancestry
        .iter()
        .map(DeclarationId::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

fn module_declarations(file: &ParsedModule) -> impl Iterator<Item = (usize, &Decl)> {
    file.parsed
        .ast
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| (index, &item.declaration))
}

fn definition_kind(declaration: &Decl) -> Option<DefinitionKind> {
    if declaration.keyword.as_str() != "define" {
        return None;
    }
    match declaration.kind.as_ref()?.as_str() {
        "mark" => Some(DefinitionKind::Mark),
        "tool" => Some(DefinitionKind::Tool),
        "transform" => Some(DefinitionKind::Transform),
        _ => None,
    }
}

fn module_item_category(declaration: &Decl) -> Option<BindingCategory> {
    match declaration.keyword.as_str() {
        "chart" => Some(BindingCategory::Chart),
        "define" => match definition_kind(declaration)? {
            DefinitionKind::Mark => Some(BindingCategory::NativeKind(NativeKindNamespace::Mark)),
            DefinitionKind::Tool => Some(BindingCategory::NativeKind(NativeKindNamespace::Tool)),
            DefinitionKind::Transform => {
                Some(BindingCategory::NativeKind(NativeKindNamespace::Transform))
            }
        },
        "table" | "schema" | "catalog" => Some(BindingCategory::Data),
        _ => None,
    }
}

fn declaration_kind_category(declaration: &Decl) -> Option<BindingCategory> {
    let namespace = match declaration.keyword.as_str() {
        "chart" | "cell" | "plot" => NativeKindNamespace::Coordinate,
        "view" => NativeKindNamespace::View,
        "mark" => NativeKindNamespace::Mark,
        "adjust" => NativeKindNamespace::Adjust,
        "transform" => NativeKindNamespace::Transform,
        "tool" => NativeKindNamespace::Tool,
        "widget" => NativeKindNamespace::Widget,
        "resource" => NativeKindNamespace::Resource,
        _ => return None,
    };
    Some(BindingCategory::NativeKind(namespace))
}

fn requires_module_item_name(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "define" | "table" | "schema" | "catalog"
    )
}

fn binding_category_label(category: BindingCategory) -> &'static str {
    match category {
        BindingCategory::NativeKind(NativeKindNamespace::Coordinate) => "coordinate",
        BindingCategory::NativeKind(NativeKindNamespace::Adjust) => "adjustment",
        BindingCategory::NativeKind(NativeKindNamespace::Mark) => "mark",
        BindingCategory::NativeKind(NativeKindNamespace::Transform) => "transform",
        BindingCategory::NativeKind(NativeKindNamespace::Tool) => "tool",
        BindingCategory::NativeKind(NativeKindNamespace::Widget) => "widget",
        BindingCategory::NativeKind(NativeKindNamespace::Scale) => "scale",
        BindingCategory::NativeKind(NativeKindNamespace::Axis) => "axis",
        BindingCategory::NativeKind(NativeKindNamespace::Legend) => "legend",
        BindingCategory::NativeKind(NativeKindNamespace::Layout) => "layout",
        BindingCategory::NativeKind(NativeKindNamespace::View) => "view",
        BindingCategory::NativeKind(NativeKindNamespace::Resource) => "resource",
        BindingCategory::Chart => "chart",
        BindingCategory::Data => "data",
        BindingCategory::ModuleNamespace => "module namespace",
    }
}

fn module_item_span(module: &ParsedModule, index: usize) -> SourceSpan {
    module
        .parsed
        .module_syntax
        .items
        .get(index)
        .map(|item| item.span)
        .unwrap_or_else(|| root_span(module))
}

fn inside_definition(file: &ParsedModule, path: &[usize]) -> bool {
    path.first()
        .and_then(|index| file.parsed.ast.items.get(*index))
        .is_some_and(|item| item.declaration.keyword.as_str() == "define")
}

fn declaration_at<'a>(file: &'a ParsedModule, path: &[usize]) -> Option<&'a Decl> {
    let (first, rest) = path.split_first()?;
    let mut declaration = &file.parsed.ast.items.get(*first)?.declaration;
    let mut cursor = 0usize;
    while cursor < rest.len() {
        if rest[cursor] == MARK_BLOCK_PATH_SEGMENT {
            let block_ordinal = *rest.get(cursor + 1)?;
            let child_index = *rest.get(cursor + 2)?;
            let body = overlay_mark_blocks(declaration)
                .get(block_ordinal)
                .copied()?;
            declaration = body.children.get(child_index)?;
            cursor += 3;
        } else {
            declaration = declaration.children.get(rest[cursor])?;
            cursor += 1;
        }
    }
    Some(declaration)
}

fn declaration_span(file: &ParsedModule, path: &[usize]) -> Option<SourceSpan> {
    let declaration = declaration_at(file, path)?;
    let mut spans = file
        .parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| match file.parsed.source_map.role(id) {
            Some(AstNodeRole::Declaration(keyword)) if *keyword == declaration.keyword => {
                Some(span)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    // Same-keyword declarations are selected by their ordinal among matching
    // declarations in source preorder.
    let mut ordinal = 0usize;
    let mut found = false;
    for item in &file.parsed.ast.items {
        declaration_preorder(std::slice::from_ref(&item.declaration), &mut |candidate| {
            if found {
                return false;
            }
            if std::ptr::eq(candidate, declaration) {
                found = true;
                return false;
            }
            if candidate.keyword == declaration.keyword {
                ordinal += 1;
            }
            true
        });
        if found {
            break;
        }
    }
    spans.get(ordinal).copied()
}

fn declaration_preorder<'a>(declarations: &'a [Decl], visit: &mut impl FnMut(&'a Decl) -> bool) {
    for declaration in declarations {
        if !visit(declaration) {
            return;
        }
        for (_, value) in declaration.props.iter() {
            value_declaration_preorder(value, visit);
        }
        declaration_preorder(&declaration.children, visit);
    }
}

fn value_declaration_preorder<'a>(value: &'a Value, visit: &mut impl FnMut(&'a Decl) -> bool) {
    match value {
        Value::Block { head, body } => {
            if let Some(head) = head {
                value_declaration_preorder(head, visit);
            }
            for (_, property) in body.props.iter() {
                value_declaration_preorder(property, visit);
            }
            declaration_preorder(&body.children, visit);
        }
        Value::Array(values) | Value::Call { args: values, .. } => {
            for value in values {
                value_declaration_preorder(value, visit);
            }
        }
        Value::Channel {
            expression: value, ..
        }
        | Value::Pattern(value) => {
            value_declaration_preorder(value, visit);
        }
        _ => {}
    }
}

fn root_span(file: &ParsedModule) -> SourceSpan {
    file.parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| {
            matches!(
                file.parsed.source_map.role(id),
                Some(AstNodeRole::Declaration(_))
            )
            .then_some(span)
        })
        .min_by_key(|span| span.range.start)
        .unwrap_or_else(|| SourceSpan::empty(file.source, 0))
}

fn value_atom(value: &Value) -> Option<&str> {
    match value {
        Value::Atom(value) => Some(value.as_str()),
        _ => None,
    }
}

fn value_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(value_atom)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn value_path(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::Array(values) => values
            .iter()
            .map(value_atom)
            .map(|value| value.map(str::to_owned))
            .collect(),
        _ => None,
    }
}

fn find_definition_target<'a>(declaration: &'a Decl, path: &[String]) -> Option<&'a Decl> {
    let (first, rest) = path.split_first()?;
    let target = declaration.children.iter().find(|child| {
        child
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == first)
    })?;
    find_named_descendant(target, rest)
}

fn find_named_descendant<'a>(declaration: &'a Decl, path: &[String]) -> Option<&'a Decl> {
    let Some((first, rest)) = path.split_first() else {
        return Some(declaration);
    };
    let child = declaration.children.iter().find(|child| {
        child
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == first)
    })?;
    find_named_descendant(child, rest)
}

fn definition_export_kind(declaration: &Decl) -> DefinitionExportKind {
    match declaration.keyword.as_str() {
        "param" => DefinitionExportKind::Param,
        "store" => DefinitionExportKind::Store,
        "selection" => DefinitionExportKind::Selection,
        "mark" => DefinitionExportKind::Mark,
        "tool" => DefinitionExportKind::Tool,
        "widget" => DefinitionExportKind::Widget,
        _ => DefinitionExportKind::Unknown,
    }
}

fn collect_definition_slot_dependencies(
    value: &Value,
    slots: &BTreeSet<String>,
    output: &mut BTreeSet<String>,
) {
    match value {
        Value::Atom(name) if slots.contains(name.as_str()) => {
            output.insert(name.to_string());
        }
        Value::Expr(expression) => {
            for path in expression_paths(expression.ast()) {
                if let Some(name) = path.first()
                    && slots.contains(name)
                {
                    output.insert(name.clone());
                }
            }
        }
        Value::Projection(projection) => {
            for item in projection.items() {
                if let Some(expression) = select_item_expression(item) {
                    for path in expression_paths(expression) {
                        if let Some(name) = path.first()
                            && slots.contains(name)
                        {
                            output.insert(name.clone());
                        }
                    }
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_definition_slot_dependencies(value, slots, output);
            }
        }
        Value::Channel {
            expression: value, ..
        }
        | Value::Pattern(value) => {
            collect_definition_slot_dependencies(value, slots, output);
        }
        Value::Block { head, body } => {
            if let Some(head) = head {
                collect_definition_slot_dependencies(head, slots, output);
            }
            for (_, value) in body.props.iter() {
                collect_definition_slot_dependencies(value, slots, output);
            }
        }
        Value::Call { args, .. } => {
            for value in args {
                collect_definition_slot_dependencies(value, slots, output);
            }
        }
        _ => {}
    }
}

fn count_definition_value_uses(declaration: &Decl, slot: &str) -> usize {
    fn count_value(value: &Value, slot: &str) -> usize {
        match value {
            Value::Atom(name) => usize::from(name.as_str() == slot),
            Value::Expr(expression) => expression.column_identifier_occurrences(slot),
            Value::Projection(projection) => projection.column_identifier_occurrences(slot),
            Value::Query(query) => query.column_identifier_occurrences(slot),
            Value::Array(values) | Value::Call { args: values, .. } => {
                values.iter().map(|value| count_value(value, slot)).sum()
            }
            Value::Block { head, body } => {
                head.as_deref().map_or(0, |head| count_value(head, slot))
                    + body
                        .props
                        .iter()
                        .map(|(_, value)| count_value(value, slot))
                        .sum::<usize>()
                    + body
                        .children
                        .iter()
                        .map(|child| count_definition_value_uses(child, slot))
                        .sum::<usize>()
            }
            Value::Channel {
                expression: value, ..
            }
            | Value::Pattern(value) => count_value(value, slot),
            _ => 0,
        }
    }

    let own = declaration
        .props
        .iter()
        .map(|(_, value)| count_value(value, slot))
        .sum::<usize>();
    own + declaration
        .children
        .iter()
        .filter(|child| child.keyword.as_str() != "slot")
        .map(|child| count_definition_value_uses(child, slot))
        .sum::<usize>()
}

fn count_definition_output_projection_uses(declaration: &Decl, slot: &str) -> usize {
    struct SelectProjectionCounter<'a> {
        slot: &'a str,
        count: usize,
    }

    impl Visitor for SelectProjectionCounter<'_> {
        type Break = ();

        fn pre_visit_select(
            &mut self,
            select: &sqlparser::ast::Select,
        ) -> std::ops::ControlFlow<Self::Break> {
            self.count += select
                .projection
                .iter()
                .filter(|item| {
                    matches!(
                        item,
                        sqlparser::ast::SelectItem::UnnamedExpr(
                            sqlparser::ast::Expr::Identifier(identifier)
                        ) if identifier.value == self.slot && identifier.quote_style.is_none()
                    )
                })
                .count();
            std::ops::ControlFlow::Continue(())
        }
    }

    fn count_value(value: &Value, slot: &str) -> usize {
        match value {
            Value::Projection(projection)
                if matches!(
                    projection.items(),
                    [sqlparser::ast::SelectItem::UnnamedExpr(
                        sqlparser::ast::Expr::Identifier(identifier)
                    )] if identifier.value == slot && identifier.quote_style.is_none()
                ) =>
            {
                1
            }
            Value::Query(query) => {
                let mut counter = SelectProjectionCounter { slot, count: 0 };
                let _ = query.ast().visit(&mut counter);
                counter.count
            }
            Value::Array(values) | Value::Call { args: values, .. } => {
                values.iter().map(|value| count_value(value, slot)).sum()
            }
            Value::Block { head, body } => {
                head.as_deref().map_or(0, |head| count_value(head, slot))
                    + body
                        .props
                        .iter()
                        .map(|(_, value)| count_value(value, slot))
                        .sum::<usize>()
                    + body
                        .children
                        .iter()
                        .map(|child| count_definition_output_projection_uses(child, slot))
                        .sum::<usize>()
            }
            Value::Channel {
                expression: value, ..
            }
            | Value::Pattern(value) => count_value(value, slot),
            _ => 0,
        }
    }

    declaration
        .props
        .iter()
        .map(|(_, value)| count_value(value, slot))
        .sum::<usize>()
        + declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() != "slot")
            .map(|child| count_definition_output_projection_uses(child, slot))
            .sum::<usize>()
}

fn infer_widget_item_type(declaration: &Decl) -> Option<PhysicalType> {
    let Value::Block { body: data, .. } = declaration.props.get("data")? else {
        return None;
    };
    let Value::Array(items) = data.props.get("values")? else {
        return None;
    };
    let first = items.first()?;
    let Value::Block { body, .. } = first else {
        return None;
    };
    let value = body.props.get("value")?;
    let inferred = match value {
        Value::Str(_) => PhysicalType::Utf8,
        Value::Bool(_) => PhysicalType::Boolean,
        Value::Num(number) if number.as_str().contains(['.', 'e']) => PhysicalType::Float64,
        Value::Num(_) => PhysicalType::Int64,
        _ => return None,
    };
    items
        .iter()
        .all(|item| match item {
            Value::Block { body, .. } => body
                .props
                .get("value")
                .is_some_and(|value| inferred.accepts_inference_literal(value).is_ok()),
            _ => false,
        })
        .then_some(inferred)
}

fn export_physical_type(declaration: &Decl, value_kind: &str) -> Option<PhysicalType> {
    let inner = value_kind.strip_prefix("param<")?.strip_suffix('>')?;
    if inner == "item_scalar" {
        infer_widget_item_type(declaration)
    } else {
        parse_type_text(inner)
    }
}

fn parse_type_text(text: &str) -> Option<PhysicalType> {
    let source = crate::SourceFile::new(
        SourceId::new(0),
        crate::SourceOrigin::Memory("<registry-type>".into()),
        format!("avenger 1; chart cartesian {{ param store as rows {{ field {text} value; }} }}"),
    );
    let parsed = crate::syntax::parse_file(&source).ok()?;
    let chart = parsed
        .ast
        .items
        .into_iter()
        .find(|item| item.declaration.keyword.as_str() == "chart")?
        .declaration;
    let store = chart.children.first()?;
    PhysicalType::parse(store.children.first()?.props.get("type")?).ok()
}

fn helpers_in_sql(sql: &str) -> Vec<ResolvedHelper> {
    let lowercase = sql.to_ascii_lowercase();
    crate::INTRINSIC_OPERATION_SIGNATURES
        .iter()
        .filter(|signature| {
            signature
                .contexts
                .contains(&crate::IntrinsicOperationContext::EventExpression)
                && lowercase.contains(&format!("{}(", signature.name))
        })
        .map(|signature| ResolvedHelper {
            name: signature.name.to_owned(),
            class: helper_class(signature.name).expect("scalar intrinsic helper class"),
            arguments: Vec::new(),
        })
        .collect()
}

fn expression_paths(expression: &Expr) -> Vec<Vec<String>> {
    let mut paths = SqlPaths::default();
    let _ = expression.visit(&mut paths);
    paths.0
}

fn query_paths(query: &sqlparser::ast::Query) -> Vec<Vec<String>> {
    let mut paths = SqlPaths::default();
    let _ = query.visit(&mut paths);
    paths.0
}

#[derive(Clone)]
struct RawHelperCall {
    name: String,
    args: Vec<Expr>,
}

fn helper_calls(expression: &Expr) -> Vec<RawHelperCall> {
    let mut helpers = HelperCalls::default();
    let _ = expression.visit(&mut helpers);
    helpers.0
}

fn query_helper_calls(query: &sqlparser::ast::Query) -> Vec<RawHelperCall> {
    let mut helpers = HelperCalls::default();
    let _ = query.visit(&mut helpers);
    helpers.0
}

#[derive(Default)]
struct HelperCalls(Vec<RawHelperCall>);

impl Visitor for HelperCalls {
    type Break = ();

    fn pre_visit_expr(&mut self, expression: &Expr) -> std::ops::ControlFlow<Self::Break> {
        let Expr::Function(function) = expression else {
            return std::ops::ControlFlow::Continue(());
        };
        let Some(name) = function.name.0.last().and_then(|part| part.as_ident()) else {
            return std::ops::ControlFlow::Continue(());
        };
        let name = name.value.to_ascii_lowercase();
        if helper_class(&name).is_none() {
            return std::ops::ControlFlow::Continue(());
        }
        let args = match &function.args {
            FunctionArguments::List(arguments) => arguments
                .args
                .iter()
                .filter_map(|argument| match argument {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(expression)) => {
                        Some(expression.clone())
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.0.push(RawHelperCall { name, args });
        std::ops::ControlFlow::Continue(())
    }
}

fn helper_class(name: &str) -> Option<HelperClass> {
    let signature = crate::intrinsic_operation_signature(name)?;
    signature
        .contexts
        .contains(&crate::IntrinsicOperationContext::EventExpression)
        .then_some(if signature.name == "selection_contains" {
            HelperClass::Selection
        } else {
            HelperClass::Reserved
        })
}

fn helper_arity(name: &str) -> Option<usize> {
    helper_class(name)?;
    crate::intrinsic_operation_signature(name).map(|signature| signature.arguments.len())
}

fn helper_argument(expression: &Expr) -> ResolvedHelperArgument {
    match expression {
        Expr::Identifier(identifier) if identifier.quote_style.is_none() => {
            ResolvedHelperArgument::Name(identifier.value.clone())
        }
        Expr::CompoundIdentifier(identifiers)
            if identifiers
                .iter()
                .all(|identifier| identifier.quote_style.is_none()) =>
        {
            ResolvedHelperArgument::Name(
                identifiers
                    .iter()
                    .map(|identifier| identifier.value.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        }
        Expr::Value(value) => match &value.value {
            SqlValue::SingleQuotedString(value) => ResolvedHelperArgument::String(value.clone()),
            SqlValue::Number(value, false) => ResolvedHelperArgument::Number(value.clone()),
            _ => ResolvedHelperArgument::Sql(expression.to_string()),
        },
        _ => ResolvedHelperArgument::Sql(expression.to_string()),
    }
}

fn datum_field(expression: &Expr) -> Option<String> {
    let Expr::CompoundIdentifier(identifiers) = expression else {
        return None;
    };
    let [namespace, field] = identifiers.as_slice() else {
        return None;
    };
    (namespace.quote_style.is_none()
        && namespace.value.eq_ignore_ascii_case("datum")
        && field.quote_style == Some('"'))
    .then(|| field.value.clone())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RawContextualUse {
    DatumField {
        field: String,
    },
    MarkChannel {
        channel: String,
    },
    EventCoord {
        channel: String,
    },
    EventStartCoord {
        channel: String,
    },
    EventDomainBoundary {
        channel: String,
        boundary: ResolvedIntervalBoundary,
    },
    EventPath,
    EventFacet {
        one_based_index: u32,
    },
    EventLegendValue,
    ItemChannel {
        channel: String,
    },
    ItemDataField {
        field: String,
    },
    ItemBbox {
        edge: ResolvedBboxEdge,
    },
    ViewField {
        authored_view: Vec<String>,
        axis: ResolvedViewAxis,
        field: ResolvedViewField,
    },
    LegacyCall {
        name: String,
    },
    Invalid {
        root: String,
        detail: &'static str,
    },
}

fn contextual_uses(expression: &Expr) -> Vec<RawContextualUse> {
    #[derive(Default)]
    struct ContextualUses {
        uses: Vec<RawContextualUse>,
        suppressed_bare_roots: BTreeMap<String, usize>,
    }

    impl Visitor for ContextualUses {
        type Break = ();

        fn pre_visit_expr(&mut self, expression: &Expr) -> std::ops::ControlFlow<Self::Break> {
            if let Some(access) = contextual_use(expression) {
                if matches!(expression, Expr::CompoundFieldAccess { .. }) {
                    *self
                        .suppressed_bare_roots
                        .entry("event".to_owned())
                        .or_default() += 1;
                }
                self.uses.push(access);
            } else if let Expr::Identifier(identifier) = expression
                && identifier.quote_style.is_none()
            {
                let name = identifier.value.to_ascii_lowercase();
                if let Some(remaining) = self.suppressed_bare_roots.get_mut(&name)
                    && *remaining > 0
                {
                    *remaining -= 1;
                } else if matches!(name.as_str(), "datum" | "channel" | "event" | "item") {
                    self.uses.push(RawContextualUse::Invalid {
                        root: name,
                        detail: "a complete property path is required",
                    });
                }
            }
            std::ops::ControlFlow::Continue(())
        }
    }

    let mut uses = ContextualUses::default();
    let _ = expression.visit(&mut uses);
    uses.uses
}

fn contextual_use(expression: &Expr) -> Option<RawContextualUse> {
    if let Expr::Function(function) = expression
        && let Some(identifier) = function.name.0.last().and_then(|part| part.as_ident())
        && identifier.quote_style.is_none()
    {
        let name = identifier.value.to_ascii_lowercase();
        if is_removed_contextual_call(&name) {
            return Some(RawContextualUse::LegacyCall { name });
        }
    }

    if let Expr::CompoundFieldAccess { root, access_chain } = expression
        && compound_field_root_matches(root, access_chain, "event", "facet")
    {
        let Some(AccessExpr::Subscript(Subscript::Index { index })) = access_chain.last() else {
            return Some(RawContextualUse::Invalid {
                root: "event".to_owned(),
                detail: "`event.facet` accepts exactly one positive literal subscript",
            });
        };
        let Expr::Value(value) = index else {
            return Some(RawContextualUse::Invalid {
                root: "event".to_owned(),
                detail: "`event.facet` uses a positive one-based integer literal",
            });
        };
        let SqlValue::Number(value, false) = &value.value else {
            return Some(RawContextualUse::Invalid {
                root: "event".to_owned(),
                detail: "`event.facet` uses a positive one-based integer literal",
            });
        };
        return match value.parse::<u32>() {
            Ok(one_based_index) if one_based_index > 0 => {
                Some(RawContextualUse::EventFacet { one_based_index })
            }
            _ => Some(RawContextualUse::Invalid {
                root: "event".to_owned(),
                detail: "`event.facet` indices start at 1",
            }),
        };
    }

    let Expr::CompoundIdentifier(identifiers) = expression else {
        return None;
    };
    let root = identifiers.first()?;
    if root.quote_style.is_some() {
        return None;
    }
    let root_name = root.value.to_ascii_lowercase();

    match root_name.as_str() {
        "datum" => {
            if let Some(field) = datum_field(expression) {
                Some(RawContextualUse::DatumField { field })
            } else {
                Some(RawContextualUse::Invalid {
                    root: root_name,
                    detail: "use exactly `datum.\"field\"`",
                })
            }
        }
        "channel" => match unquoted_path(identifiers).as_deref() {
            Some([_, channel]) => Some(RawContextualUse::MarkChannel {
                channel: channel.clone(),
            }),
            _ => Some(RawContextualUse::Invalid {
                root: root_name,
                detail: "use exactly `channel.<channel>`",
            }),
        },
        "event" => {
            let path = unquoted_path(identifiers);
            match path.as_deref() {
                Some([_, coord, channel]) if coord.eq_ignore_ascii_case("coord") => {
                    Some(RawContextualUse::EventCoord {
                        channel: channel.clone(),
                    })
                }
                Some([_, start, coord, channel])
                    if start.eq_ignore_ascii_case("start")
                        && coord.eq_ignore_ascii_case("coord") =>
                {
                    Some(RawContextualUse::EventStartCoord {
                        channel: channel.clone(),
                    })
                }
                Some([_, domain, channel, boundary]) if domain.eq_ignore_ascii_case("domain") => {
                    let boundary = match boundary.to_ascii_lowercase().as_str() {
                        "start" => ResolvedIntervalBoundary::Start,
                        "end" => ResolvedIntervalBoundary::End,
                        _ => {
                            return Some(RawContextualUse::Invalid {
                                root: root_name,
                                detail: "event domains end in `.start` or `.end`",
                            });
                        }
                    };
                    Some(RawContextualUse::EventDomainBoundary {
                        channel: channel.clone(),
                        boundary,
                    })
                }
                Some([_, path]) if path.eq_ignore_ascii_case("path") => {
                    Some(RawContextualUse::EventPath)
                }
                Some([_, legend, value])
                    if legend.eq_ignore_ascii_case("legend")
                        && value.eq_ignore_ascii_case("value") =>
                {
                    Some(RawContextualUse::EventLegendValue)
                }
                // The root of a valid `event.facet[n]` is visited separately
                // by sqlparser's visitor. The enclosing CompoundFieldAccess
                // performs the actual validation.
                Some([_, facet]) if facet.eq_ignore_ascii_case("facet") => None,
                _ => Some(RawContextualUse::Invalid {
                    root: root_name,
                    detail: "the event property path is not recognized",
                }),
            }
        }
        "item" => {
            if identifiers.len() == 3
                && identifiers[1].quote_style.is_none()
                && identifiers[1].value.eq_ignore_ascii_case("data")
                && identifiers[2].quote_style == Some('"')
            {
                return Some(RawContextualUse::ItemDataField {
                    field: identifiers[2].value.clone(),
                });
            }
            let path = unquoted_path(identifiers);
            match path.as_deref() {
                Some([_, channel, name]) if channel.eq_ignore_ascii_case("channel") => {
                    Some(RawContextualUse::ItemChannel {
                        channel: name.clone(),
                    })
                }
                Some([_, bbox, edge]) if bbox.eq_ignore_ascii_case("bbox") => {
                    let edge = match edge.to_ascii_lowercase().as_str() {
                        "top" => ResolvedBboxEdge::Top,
                        "right" => ResolvedBboxEdge::Right,
                        "bottom" => ResolvedBboxEdge::Bottom,
                        "left" => ResolvedBboxEdge::Left,
                        _ => {
                            return Some(RawContextualUse::Invalid {
                                root: root_name,
                                detail: "item bbox edges are top, right, bottom, or left",
                            });
                        }
                    };
                    Some(RawContextualUse::ItemBbox { edge })
                }
                _ => Some(RawContextualUse::Invalid {
                    root: root_name,
                    detail: "use `item.channel.<name>`, `item.data.\"field\"`, or `item.bbox.<edge>`",
                }),
            }
        }
        _ => contextual_view_use(identifiers),
    }
}

fn compound_field_root_matches(
    root: &Expr,
    access_chain: &[AccessExpr],
    namespace: &str,
    member: &str,
) -> bool {
    match root {
        Expr::CompoundIdentifier(identifiers) => {
            identifier_path_matches(identifiers, &[namespace, member]) && access_chain.len() == 1
        }
        Expr::Identifier(identifier)
            if identifier.quote_style.is_none()
                && identifier.value.eq_ignore_ascii_case(namespace) =>
        {
            matches!(
                access_chain,
                [
                    AccessExpr::Dot(Expr::Identifier(member_identifier)),
                    AccessExpr::Subscript(_)
                ] if member_identifier.quote_style.is_none()
                    && member_identifier.value.eq_ignore_ascii_case(member)
            )
        }
        _ => false,
    }
}

fn contextual_view_use(identifiers: &[sqlparser::ast::Ident]) -> Option<RawContextualUse> {
    let path = unquoted_path(identifiers)?;
    let (authored_view, axis, field) = match path.as_slice() {
        [view, axis, pixels] if pixels.eq_ignore_ascii_case("pixels") => (
            vec![view.clone()],
            resolved_view_axis(axis)?,
            ResolvedViewField::Pixels,
        ),
        [view, axis, domain, boundary] if domain.eq_ignore_ascii_case("domain") => (
            vec![view.clone()],
            resolved_view_axis(axis)?,
            match boundary.to_ascii_lowercase().as_str() {
                "start" => ResolvedViewField::DomainStart,
                "end" => ResolvedViewField::DomainEnd,
                _ => return None,
            },
        ),
        _ => return None,
    };
    Some(RawContextualUse::ViewField {
        authored_view,
        axis,
        field,
    })
}

fn resolved_view_axis(value: &str) -> Option<ResolvedViewAxis> {
    match value.to_ascii_lowercase().as_str() {
        "x" => Some(ResolvedViewAxis::X),
        "y" => Some(ResolvedViewAxis::Y),
        _ => None,
    }
}

fn unquoted_path(identifiers: &[sqlparser::ast::Ident]) -> Option<Vec<String>> {
    identifiers
        .iter()
        .map(|identifier| {
            identifier
                .quote_style
                .is_none()
                .then(|| identifier.value.clone())
        })
        .collect()
}

fn identifier_path_matches(identifiers: &[sqlparser::ast::Ident], expected: &[&str]) -> bool {
    identifiers.len() == expected.len()
        && identifiers
            .iter()
            .zip(expected)
            .all(|(identifier, expected)| {
                identifier.quote_style.is_none() && identifier.value.eq_ignore_ascii_case(expected)
            })
}

fn is_removed_contextual_call(name: &str) -> bool {
    matches!(
        name,
        "datum"
            | "channel"
            | "event_coord"
            | "start_coord"
            | "event_domain_start"
            | "event_domain_end"
            | "event_path"
            | "event_facet_value"
            | "legend_value"
            | "item_channel"
            | "item_data"
            | "item_bbox"
            | "view_x"
            | "view_y"
    )
}

fn legacy_contextual_replacement(name: &str) -> (&'static str, &'static str) {
    match name {
        "datum" => (
            "AVENGER-RESOLVE-183",
            "use `datum.\"field\"` to read a field from the event datum",
        ),
        "channel" => (
            "AVENGER-RESOLVE-187",
            "use `channel.<channel>` to reference another mark channel",
        ),
        "event_coord" => (
            "AVENGER-RESOLVE-188",
            "use `event.coord.<channel>` for the current event coordinate",
        ),
        "start_coord" => (
            "AVENGER-RESOLVE-188",
            "use `event.start.coord.<channel>` for the gesture-start coordinate",
        ),
        "event_domain_start" | "event_domain_end" => (
            "AVENGER-RESOLVE-188",
            "use `event.domain.<channel>.start` or `.end`",
        ),
        "event_path" => ("AVENGER-RESOLVE-188", "use `event.path`"),
        "event_facet_value" => ("AVENGER-RESOLVE-188", "use one-based `event.facet[index]`"),
        "legend_value" => ("AVENGER-RESOLVE-188", "use `event.legend.value`"),
        "item_channel" => (
            "AVENGER-RESOLVE-189",
            "use `item.channel.<channel>` in item-frame expressions",
        ),
        "item_data" => (
            "AVENGER-RESOLVE-189",
            "use `item.data.\"field\"` in item-frame expressions",
        ),
        "item_bbox" => (
            "AVENGER-RESOLVE-189",
            "use `item.bbox.<edge>` in item-frame expressions",
        ),
        "view_x" | "view_y" => (
            "AVENGER-RESOLVE-190",
            "use `<view>.x.<field>` or `<view>.y.<field>`",
        ),
        _ => (
            "AVENGER-RESOLVE-186",
            "use the corresponding contextual property access",
        ),
    }
}

fn helper_argument_path(expression: &Expr) -> Option<Vec<String>> {
    match expression {
        Expr::Identifier(identifier) if identifier.quote_style.is_none() => {
            Some(vec![identifier.value.clone()])
        }
        Expr::CompoundIdentifier(identifiers)
            if identifiers
                .iter()
                .all(|identifier| identifier.quote_style.is_none()) =>
        {
            Some(
                identifiers
                    .iter()
                    .map(|identifier| identifier.value.clone())
                    .collect(),
            )
        }
        _ => None,
    }
}

fn relation_paths(query: &sqlparser::ast::Query) -> Vec<Vec<String>> {
    #[derive(Default)]
    struct Relations {
        paths: Vec<Vec<String>>,
        ctes: BTreeSet<String>,
    }

    impl Visitor for Relations {
        type Break = ();

        fn pre_visit_query(
            &mut self,
            query: &sqlparser::ast::Query,
        ) -> std::ops::ControlFlow<Self::Break> {
            if let Some(with) = &query.with {
                self.ctes.extend(
                    with.cte_tables
                        .iter()
                        .map(|cte| cte.alias.name.value.clone()),
                );
            }
            std::ops::ControlFlow::Continue(())
        }

        fn pre_visit_relation(
            &mut self,
            relation: &ObjectName,
        ) -> std::ops::ControlFlow<Self::Break> {
            let path = relation
                .0
                .iter()
                .filter_map(|part| part.as_ident())
                .map(|identifier| identifier.value.clone())
                .collect::<Vec<_>>();
            if !(path.len() == 1 && self.ctes.contains(&path[0])) {
                self.paths.push(path);
            }
            std::ops::ControlFlow::Continue(())
        }
    }

    let mut relations = Relations::default();
    let _ = query.visit(&mut relations);
    relations.paths.retain(|path| !path.is_empty());
    relations.paths
}

#[derive(Default)]
struct SqlPaths(Vec<Vec<String>>);

impl Visitor for SqlPaths {
    type Break = ();

    fn pre_visit_expr(&mut self, expression: &Expr) -> std::ops::ControlFlow<Self::Break> {
        if let Expr::CompoundIdentifier(identifiers) = expression
            && identifiers.len() >= 2
            && identifiers
                .iter()
                .all(|identifier| identifier.quote_style.is_none())
            && contextual_use(expression).is_none()
            && !identifiers.first().is_some_and(|identifier| {
                identifier.value.eq_ignore_ascii_case("event")
                    && identifiers.get(1).is_some_and(|member| {
                        member.quote_style.is_none() && member.value.eq_ignore_ascii_case("facet")
                    })
            })
        {
            self.0.push(
                identifiers
                    .iter()
                    .map(|identifier| identifier.value.clone())
                    .collect(),
            );
        }
        std::ops::ControlFlow::Continue(())
    }
}

trait DeclName {
    fn name_string(&self) -> String;
}

impl DeclName for Decl {
    fn name_string(&self) -> String {
        self.name.as_ref().map_or_else(
            || {
                self.kind
                    .as_ref()
                    .map_or("anonymous", |kind| kind.as_str())
                    .to_owned()
            },
            ToString::to_string,
        )
    }
}
