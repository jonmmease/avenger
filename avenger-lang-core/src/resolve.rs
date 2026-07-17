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
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, Value as SqlValue, Visit,
    Visitor,
};

use crate::{
    Diagnostic, ExpansionOrImportFrame, LANGUAGE_MAJOR, PhysicalField, PhysicalType, SourceId,
    SourceLabel, SourceMap, SourceSpan,
    ast::{AstNodeRole, BindingKind, BindingTime, Decl, Name, RefKind, Root, Value, Visibility},
    project::{DefinitionKind, ParsedProject, ProjectFile, ProjectFileId, ProjectFileKind},
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedParam {
    pub id: ParamId,
    pub declaration: DeclarationId,
    pub source_name: String,
    pub data_type: PhysicalType,
    pub default: ResolvedValue,
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
    pub data_type: Option<PhysicalType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionExportKind {
    Param,
    Store,
    Selection,
    Mark,
    Group,
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
    pub function_class: Option<String>,
    pub reference_kind: Option<String>,
    pub exposes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionChannel {
    pub required: bool,
    pub physical_channel: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedProject {
    pub language_major: u32,
    pub registry_schema_major: u32,
    pub registry_schema_minor: u32,
    pub registry_profile_label: String,
    pub source_fingerprint: String,
    pub sources: SourceMap,
    pub files: BTreeMap<ProjectFileId, ResolvedFile>,
    pub charts: Vec<DeclarationId>,
    pub definitions: BTreeMap<ProjectFileId, DefinitionSchema>,
    pub params: BTreeMap<ParamId, ResolvedParam>,
    pub stores: BTreeMap<StoreId, ResolvedStore>,
    pub selections: BTreeMap<SelectionId, ResolvedSelection>,
    pub public_targets: BTreeMap<String, ResolvedTarget>,
    pub param_default_order: Vec<ParamId>,
    pub table_order: Vec<DeclarationId>,
    pub definition_import_order: Vec<ProjectFileId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedFile {
    pub id: ProjectFileId,
    pub source: SourceId,
    pub kind: ProjectFileKind,
    pub imports: BTreeMap<String, ProjectFileId>,
    pub roots: Vec<ResolvedDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDeclaration {
    pub id: DeclarationId,
    pub source: SourceId,
    pub span: SourceSpan,
    pub keyword: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub visibility: Visibility,
    pub coordinate: Option<String>,
    pub component_kind: Option<String>,
    pub properties: BTreeMap<String, ResolvedValue>,
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
    Query(ResolvedQuery),
    Binding(ResolvedBinding),
    Reference(ResolvedReference),
    Visual(Box<ResolvedValue>),
    Dimension(Vec<String>),
    Pattern(Box<ResolvedValue>),
    Environment(String),
    None,
    Array(Vec<ResolvedValue>),
    Object {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExpression {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
    pub references: Vec<ResolvedSqlReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedQuery {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
    pub references: Vec<ResolvedSqlReference>,
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
    Target(ResolvedTarget),
    DefinitionChannel {
        target: ResolvedTarget,
        family_suffix: String,
    },
    Sql(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperClass {
    Channel,
    Datum,
    Event,
    Selection,
    View,
    Reserved,
}

#[derive(Clone, Debug)]
pub struct ResolveFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

#[derive(Clone, Debug)]
pub struct ResolveAttempt {
    pub result: Result<ResolvedProject, ResolveFailure>,
}

/// Resolve a parsed project against one immutable authoring registry snapshot.
pub fn resolve_project(project: &ParsedProject, registry: &NativeSchemaSnapshot) -> ResolveAttempt {
    let mut resolver = Resolver::new(project, registry);
    resolver.run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ScopeId(usize);

#[derive(Clone, Debug, Default)]
struct Scope {
    parent: Option<ScopeId>,
    owner: Option<DeclarationId>,
    label: String,
    values: BTreeMap<String, ValueSymbol>,
    selections: BTreeMap<String, SelectionId>,
    structural: BTreeMap<String, DeclarationId>,
    transforms: BTreeSet<String>,
    events: BTreeMap<String, EventId>,
    definition_arguments: BTreeMap<String, ResolvedTarget>,
    event_binding: bool,
    event_has_between: bool,
}

#[derive(Clone, Debug)]
enum ValueSymbol {
    Param(ParamId, Option<PhysicalType>),
    Store(StoreId),
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

struct Resolver<'a> {
    project: &'a ParsedProject,
    registry: &'a NativeSchemaSnapshot,
    diagnostics: Vec<Diagnostic>,
    scopes: Vec<Scope>,
    declarations: BTreeMap<(ProjectFileId, Vec<usize>), DeclInfo>,
    instances: BTreeMap<DeclarationId, InstanceInterface>,
    imports: BTreeMap<ProjectFileId, BTreeMap<String, ProjectFileId>>,
    definitions: BTreeMap<ProjectFileId, DefinitionSchema>,
    params: BTreeMap<ParamId, ResolvedParam>,
    param_types: BTreeMap<ParamId, PhysicalType>,
    stores: BTreeMap<StoreId, ResolvedStore>,
    selections: BTreeMap<SelectionId, ResolvedSelection>,
    param_dependencies: BTreeMap<ParamId, BTreeSet<ParamId>>,
    table_dependencies: BTreeMap<DeclarationId, BTreeSet<DeclarationId>>,
    table_names: BTreeMap<String, DeclarationId>,
}

impl<'a> Resolver<'a> {
    fn new(project: &'a ParsedProject, registry: &'a NativeSchemaSnapshot) -> Self {
        Self {
            project,
            registry,
            diagnostics: Vec::new(),
            scopes: Vec::new(),
            declarations: BTreeMap::new(),
            instances: BTreeMap::new(),
            imports: BTreeMap::new(),
            definitions: BTreeMap::new(),
            params: BTreeMap::new(),
            param_types: BTreeMap::new(),
            stores: BTreeMap::new(),
            selections: BTreeMap::new(),
            param_dependencies: BTreeMap::new(),
            table_dependencies: BTreeMap::new(),
            table_names: BTreeMap::new(),
        }
    }

    fn run(&mut self) -> ResolveAttempt {
        self.check_versions();
        self.build_import_bindings();
        self.extract_definition_schemas();
        self.predeclare_project();
        self.build_table_dependencies();
        self.finish_instance_interfaces();

        let mut files = BTreeMap::new();
        let mut charts = Vec::new();
        for (file_id, file) in &self.project.files {
            let roots = file
                .parsed
                .ast
                .root
                .declarations()
                .iter()
                .enumerate()
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
                    if matches!(file.kind, ProjectFileKind::Chart) {
                        charts.push(resolved.id.clone());
                    }
                    resolved
                })
                .collect();
            files.insert(
                file_id.clone(),
                ResolvedFile {
                    id: file_id.clone(),
                    source: file.source,
                    kind: file.kind,
                    imports: self.imports.get(file_id).cloned().unwrap_or_default(),
                    roots,
                },
            );
        }

        let param_default_order = self.check_param_dag();
        let table_order = self.check_table_dag();
        let definition_import_order = self.definition_import_order();
        let mut public_targets = BTreeMap::new();
        for file in files.values() {
            let mut file_targets = BTreeMap::new();
            let mut origins = BTreeMap::new();
            let mut collisions = Vec::new();
            for root in &file.roots {
                collect_public_targets(root, &mut file_targets, &mut origins, &mut collisions);
            }
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
            public_targets.extend(file_targets);
        }
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
            result: Ok(ResolvedProject {
                language_major: LANGUAGE_MAJOR,
                registry_schema_major: self.registry.version.major,
                registry_schema_minor: self.registry.version.minor,
                registry_profile_label: self.registry.profile_label.clone(),
                source_fingerprint: self.project.fingerprint.clone(),
                sources: self.project.sources.clone(),
                files,
                charts,
                definitions: self.definitions.clone(),
                params: self.params.clone(),
                stores: self.stores.clone(),
                selections: self.selections.clone(),
                public_targets,
                param_default_order,
                table_order,
                definition_import_order,
            }),
        }
    }

    fn check_versions(&mut self) {
        for file in self.project.files.values() {
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
                .files
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

    fn build_import_bindings(&mut self) {
        let source_to_file = self
            .project
            .files
            .iter()
            .map(|(id, file)| (file.source, id.clone()))
            .collect::<BTreeMap<_, _>>();
        for edge in &self.project.imports {
            let Some(importer) = source_to_file.get(&edge.importer) else {
                continue;
            };
            let Some(imported) = source_to_file.get(&edge.imported) else {
                continue;
            };
            self.imports
                .entry(importer.clone())
                .or_default()
                .insert(edge.binding.clone(), imported.clone());
        }
    }

    fn extract_definition_schemas(&mut self) {
        for (file_id, file) in &self.project.files {
            let ProjectFileKind::Definition(kind) = file.kind else {
                continue;
            };
            let Root::Define(declaration) = &file.parsed.ast.root else {
                continue;
            };
            let id = declaration_id(file, &[0]);
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
                .filter(|child| child.keyword.as_str() == "slot")
                .filter_map(|child| child.name.as_ref().map(ToString::to_string))
                .collect::<BTreeSet<_>>();
            for child in &declaration.children {
                match child.keyword.as_str() {
                    "slot" => {
                        let Some(name) = child.name.as_ref() else {
                            continue;
                        };
                        let shape = child.kind.as_ref().map_or("", Name::as_str).to_owned();
                        if !matches!(
                            shape.as_str(),
                            "expr"
                                | "expr_list"
                                | "literal"
                                | "number"
                                | "string"
                                | "boolean"
                                | "enum"
                                | "function"
                                | "ref"
                                | "block"
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
                        let function_class = child
                            .props
                            .get("class")
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
                            shape,
                            required: default.is_none(),
                            default,
                            enum_values,
                            function_class,
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
                    "channel" => {
                        if let Some(name) = child.name.as_ref() {
                            self.check_definition_interface_name(
                                &mut interface_names,
                                name.as_str(),
                                "channel",
                                file,
                            );
                            channels.insert(
                                name.to_string(),
                                DefinitionChannel {
                                    required: child.kind.is_none(),
                                    physical_channel: child.kind.as_ref().map(ToString::to_string),
                                },
                            );
                        }
                    }
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
                                let export = DefinitionExport {
                                    path,
                                    target_kind,
                                    data_type: target.and_then(|target| {
                                        (target.keyword.as_str() == "param")
                                            .then(|| target.props.get("type"))
                                            .flatten()
                                            .and_then(|value| PhysicalType::parse(value).ok())
                                    }),
                                };
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
                file_id.clone(),
                DefinitionSchema {
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

    fn check_definition_interface_name(
        &mut self,
        names: &mut BTreeMap<String, &'static str>,
        name: &str,
        role: &'static str,
        file: &ProjectFile,
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

    fn validate_slot_declaration(
        &mut self,
        slot: &Decl,
        shape: &str,
        enum_values: &[String],
        earlier_slots: &[String],
        all_slots: &BTreeSet<String>,
        file: &ProjectFile,
    ) {
        let span = root_span(file);
        let name = slot.name.as_ref().map_or("<unnamed>", Name::as_str);
        let allowed = match shape {
            "enum" => &["default", "values"][..],
            "function" => &["default", "class"][..],
            "ref" => &["default", "kind"][..],
            "block" => &["default", "exposes"][..],
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
        if shape == "function"
            && !slot
                .props
                .get("class")
                .and_then(value_atom)
                .is_some_and(|class| matches!(class, "scalar" | "aggregate" | "window" | "table"))
        {
            self.error(
                "AVENGER-RESOLVE-017",
                "function slot requires a valid class",
                span,
                format!("slot `{name}` requires scalar, aggregate, window, or table"),
            );
        }
        if shape == "ref"
            && !slot
                .props
                .get("kind")
                .and_then(value_atom)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "mark"
                            | "group"
                            | "param"
                            | "selection"
                            | "store"
                            | "tool"
                            | "widget"
                            | "resource"
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
        for (file_id, file) in &self.project.files {
            let file_scope = self.new_scope(None, format!("file:{}", file_id.as_str()));
            for (index, declaration) in file.parsed.ast.root.declarations().iter().enumerate() {
                self.predeclare_declaration(file, declaration, vec![index], file_scope, Vec::new());
            }
        }
    }

    fn build_table_dependencies(&mut self) {
        let data_files = self
            .project
            .files
            .values()
            .filter(|file| matches!(file.kind, ProjectFileKind::Data))
            .collect::<Vec<_>>();
        for file in data_files {
            for (index, declaration) in file.parsed.ast.root.declarations().iter().enumerate() {
                self.collect_table_names(file, declaration, &[index], &[]);
            }
        }
        let entries = self
            .table_names
            .iter()
            .map(|(name, id)| (name.clone(), id.clone()))
            .collect::<Vec<_>>();
        for (name, id) in entries {
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
            let prefix = name
                .rsplit_once('.')
                .map_or("", |(prefix, _)| prefix)
                .to_owned();
            for relation in relations {
                let relation = relation.join(".");
                let qualified = if relation.contains('.') || prefix.is_empty() {
                    relation.clone()
                } else {
                    format!("{prefix}.{relation}")
                };
                if let Some(dependency) = self
                    .table_names
                    .get(&qualified)
                    .or_else(|| self.table_names.get(&relation))
                    .cloned()
                {
                    self.table_dependencies
                        .entry(id.clone())
                        .or_default()
                        .insert(dependency);
                }
            }
        }
    }

    fn collect_table_names(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        path: &[usize],
        prefix: &[String],
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
            let qualified = table_path.join(".");
            let id = declaration_id(file, path);
            if self.table_names.insert(qualified.clone(), id).is_some() {
                self.error(
                    "AVENGER-RESOLVE-102",
                    "duplicate catalog table path",
                    declaration_span(file, path).unwrap_or_else(|| root_span(file)),
                    format!("table `{qualified}` is declared more than once"),
                );
            }
        }
        for (index, child) in declaration.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            self.collect_table_names(file, child, &child_path, &nested_prefix);
        }
    }

    fn declaration_source(&self, id: &DeclarationId) -> Option<(&ProjectFile, &Decl)> {
        let ((file_id, path), _) = self.declarations.iter().find(|(_, info)| &info.id == id)?;
        let file = self.project.files.get(file_id)?;
        let declaration = declaration_at(file.parsed.ast.root.declarations(), path)?;
        Some((file, declaration))
    }

    fn predeclare_declaration(
        &mut self,
        file: &ProjectFile,
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
    }

    #[allow(clippy::too_many_arguments)]
    fn predeclare_name(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        scope: ScopeId,
        name: &str,
        id: &DeclarationId,
        runtime_target: Option<ResolvedTarget>,
        ancestry: &[DeclarationId],
        span: SourceSpan,
    ) {
        match declaration.keyword.as_str() {
            "param" => {
                let param_id = ParamId(semantic_hash(&[
                    "param-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                let data_type = declaration.props.get("type").and_then(|value| {
                    match PhysicalType::parse(value) {
                        Ok(data_type) => Some(data_type),
                        Err(error) => {
                            self.error(
                                "AVENGER-RESOLVE-030",
                                "invalid param physical Arrow type",
                                span,
                                error.to_string(),
                            );
                            None
                        }
                    }
                });
                self.insert_value_symbol(
                    scope,
                    name,
                    ValueSymbol::Param(param_id.clone(), data_type.clone()),
                    span,
                );
                if let Some(data_type) = data_type {
                    self.param_types.insert(param_id.clone(), data_type);
                }
                self.set_runtime_target(id, ResolvedTarget::Param(param_id));
            }
            "store" => {
                let store_id = StoreId(semantic_hash(&[
                    "store-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                self.insert_value_symbol(scope, name, ValueSymbol::Store(store_id.clone()), span);
                self.set_runtime_target(id, ResolvedTarget::Store(store_id));
            }
            "selection" => {
                let selection_id = SelectionId(semantic_hash(&[
                    "selection-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                if self.scopes[scope.0]
                    .selections
                    .insert(name.to_owned(), selection_id.clone())
                    .is_some()
                {
                    self.error(
                        "AVENGER-RESOLVE-011",
                        "duplicate selection name",
                        span,
                        format!("`{name}` is already declared in this scope"),
                    );
                }
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
                    .get(&file.id)
                    .map(|schema| schema.declaration.clone())
                else {
                    return;
                };
                let target = if declaration.keyword.as_str() == "slot" {
                    ResolvedTarget::DefinitionSlot {
                        definition,
                        name: name.to_owned(),
                    }
                } else {
                    ResolvedTarget::DefinitionChannel {
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

    fn insert_value_symbol(
        &mut self,
        scope: ScopeId,
        name: &str,
        symbol: ValueSymbol,
        span: SourceSpan,
    ) {
        if self.scopes[scope.0]
            .values
            .insert(name.to_owned(), symbol)
            .is_some()
        {
            self.error(
                "AVENGER-RESOLVE-010",
                "duplicate param/store value binding",
                span,
                format!("`{name}` is already bound by a param or store in this scope"),
            );
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>, label: String) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            parent,
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
            let Some(file) = self.project.files.get(&file_id) else {
                continue;
            };
            let Some(declaration) = declaration_at(file.parsed.ast.root.declarations(), &path)
            else {
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
            if let Some(schema) = self.native_schema(file, declaration, coordinate.as_deref()) {
                self.install_native_interface(file, declaration, &info, &schema);
            }
            if let Some(schema) = declaration
                .kind
                .as_ref()
                .and_then(|kind| self.imported_definition(file, kind.as_str()))
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
            let Some(file) = self.project.files.get(&file_id) else {
                continue;
            };
            if matches!(file.kind, ProjectFileKind::Definition(_)) {
                continue;
            }
            let Some(declaration) = declaration_at(file.parsed.ast.root.declarations(), &path)
            else {
                continue;
            };
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
        file: &ProjectFile,
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
                let file = self.project.files.get(file_id)?;
                let declaration = declaration_at(file.parsed.ast.root.declarations(), path)?;
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
        file: &ProjectFile,
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
                    if let Some(expected) = export_physical_type(declaration, &value_kind)
                        && self.target_physical_type(&target).as_ref() != Some(&expected)
                    {
                        self.error(
                            "AVENGER-RESOLVE-029",
                            "existing widget/tool state binding has the wrong Arrow type",
                            info.span,
                            format!(
                                "export `{alias}` requires `{expected}`, but the bound state has {}",
                                self.target_physical_type(&target)
                                    .map_or_else(|| "an unknown type".to_owned(), |actual| format!("`{actual}`"))
                            ),
                        );
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
            let Value::Ref { kind, path } = authored else {
                return None;
            };
            return self
                .resolve_reference(scope, *kind, path, span)
                .map(|reference| reference.target);
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
        file: &ProjectFile,
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
            let default = source_default
                .map(unresolved_value)
                .unwrap_or(ResolvedValue::Null);
            self.validate_typed_boundary(
                &data_type,
                source_default,
                &default,
                info.span,
                "generated state default",
            );
            self.params.insert(
                id.clone(),
                ResolvedParam {
                    id: id.clone(),
                    declaration: info.id.clone(),
                    source_name: format!("{}.{}", declaration.name_string(), alias),
                    data_type: data_type.clone(),
                    default,
                    sharing: StateSharing::Shared,
                    migration_key,
                    definition_local_seed,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
                    table_owner: None,
                },
            );
            self.param_types.insert(id.clone(), data_type);
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
        file: &ProjectFile,
        info: &DeclInfo,
        role: &str,
    ) -> (Option<StateMigrationKey>, Option<DefinitionLocalSeed>) {
        if matches!(file.kind, ProjectFileKind::Definition(_)) {
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
            if matches!(class, HelperClass::Event | HelperClass::Datum) && !in_event {
                self.error(
                    "AVENGER-RESOLVE-110",
                    "event helper is outside an event context",
                    span,
                    format!(
                        "`{}(...)` requires an event binding or event effect",
                        call.name
                    ),
                );
            }
            if class == HelperClass::Channel && owner.keyword.as_str() != "mark" {
                self.error(
                    "AVENGER-RESOLVE-111",
                    "channel helper is outside a mark channel",
                    span,
                    format!("`{}(...)` requires a mark encoding context", call.name),
                );
            }
            if matches!(call.name.as_str(), "start_coord" | "event_path") && !scope_has_between {
                self.error(
                    "AVENGER-RESOLVE-132",
                    "gesture-start helper requires a between interaction",
                    span,
                    format!("`{}(...)` has no start event in this binding", call.name),
                );
            }

            let mut arguments = Vec::new();
            for (index, argument) in call.args.iter().enumerate() {
                let mut resolved = helper_argument(argument);
                let expects_target =
                    matches!(class, HelperClass::Selection | HelperClass::View) && index == 0;
                if expects_target
                    && let Some(path) = helper_argument_path(argument)
                    && let Some(target) = if class == HelperClass::Selection {
                        self.resolve_typed_reference_path(scope, &path, RefKind::Selection, span)
                    } else {
                        self.resolve_any_path(scope, &path, span, true)
                    }
                {
                    let valid = match class {
                        HelperClass::Selection => matches!(
                            target,
                            ResolvedTarget::Selection(_)
                                | ResolvedTarget::DefinitionSelection { .. }
                        ),
                        HelperClass::View => matches!(target, ResolvedTarget::Declaration(_)),
                        HelperClass::Channel => {
                            matches!(target, ResolvedTarget::DefinitionChannel { .. })
                        }
                        _ => true,
                    };
                    if !valid {
                        self.error(
                            "AVENGER-RESOLVE-112",
                            "reserved helper argument has the wrong target kind",
                            span,
                            format!("`{}` is not a valid {:?} target", path.join("."), class),
                        );
                    }
                    resolved = ResolvedHelperArgument::Target(target);
                } else if index == 0
                    && helper_uses_channel_argument(&call.name)
                    && let Some(path) = helper_argument_path(argument)
                    && path.len() == 1
                    && let Some((target, _)) =
                        self.visible_definition_channel_property(scope, &path[0])
                {
                    let family_suffix = match &target {
                        ResolvedTarget::DefinitionChannel { name, .. } => path[0]
                            .strip_prefix(name)
                            .filter(|suffix| *suffix == "2")
                            .unwrap_or("")
                            .to_owned(),
                        _ => String::new(),
                    };
                    resolved = ResolvedHelperArgument::DefinitionChannel {
                        target,
                        family_suffix,
                    };
                }
                if index == 0
                    && helper_uses_channel_argument(&call.name)
                    && let ResolvedHelperArgument::Name(channel) = &resolved
                    && !self.helper_channel_exists(scope, owner, &call.name, channel)
                {
                    self.error(
                        "AVENGER-RESOLVE-154",
                        "reserved helper references an unknown channel",
                        span,
                        format!(
                            "`{}` is not a registered channel in this helper context",
                            channel
                        ),
                    );
                }
                self.validate_helper_argument(&call.name, index, &resolved, span);
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
                && matches!(declaration.keyword.as_str(), "chart" | "plot" | "view")
                && let Some(kind) = declaration.kind.as_ref()
            {
                return Some(kind.to_string());
            }
            cursor = self.scopes[id.0].parent;
        }
        None
    }

    fn validate_helper_argument(
        &mut self,
        helper: &str,
        index: usize,
        argument: &ResolvedHelperArgument,
        span: SourceSpan,
    ) {
        let valid = match (helper, index) {
            ("datum" | "item_data", 0) => {
                matches!(argument, ResolvedHelperArgument::String(value) if !value.is_empty())
            }
            ("event_facet_value", 0) => matches!(
                argument,
                ResolvedHelperArgument::Number(value)
                    if value.parse::<u32>().is_ok()
            ),
            ("item_bbox", 0) => matches!(
                argument,
                ResolvedHelperArgument::Name(value)
                    if matches!(value.as_str(), "top" | "right" | "bottom" | "left")
            ),
            ("view_x" | "view_y", 1) => matches!(
                argument,
                ResolvedHelperArgument::Name(value)
                    if matches!(
                        value.as_str(),
                        "pixels" | "domain_start" | "domain_end"
                    )
            ),
            _ => true,
        };
        if !valid {
            self.error(
                "AVENGER-RESOLVE-133",
                "reserved helper argument has an invalid shape",
                span,
                format!("argument {} to `{helper}(...)` is invalid", index + 1),
            );
        }
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
            if let Some(symbol) = self.scopes[id.0].values.get(first) {
                target = Some(match symbol {
                    ValueSymbol::Param(id, _) => ResolvedTarget::Param(id.clone()),
                    ValueSymbol::Store(id) => ResolvedTarget::Store(id.clone()),
                });
                break;
            }
            if let Some(selection) = self.scopes[id.0].selections.get(first) {
                target = Some(ResolvedTarget::Selection(selection.clone()));
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
            if let Some(symbol) = self.scopes[child_scope.0].values.get(segment) {
                target = match symbol {
                    ValueSymbol::Param(id, _) => ResolvedTarget::Param(id.clone()),
                    ValueSymbol::Store(id) => ResolvedTarget::Store(id.clone()),
                };
                continue;
            }
            if let Some(selection) = self.scopes[child_scope.0].selections.get(segment) {
                target = ResolvedTarget::Selection(selection.clone());
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
        file: &ProjectFile,
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
        let coordinate = declaration_coordinate(declaration, inherited_coordinate);
        let parent = parent_declaration(file, path);
        self.validate_placement(declaration, parent, info.span);
        self.validate_visibility(file, path, declaration, info.span);

        let native_schema = self.native_schema(file, declaration, coordinate.as_deref());
        let definition_schema = declaration
            .kind
            .as_ref()
            .and_then(|kind| self.imported_definition(file, kind.as_str()))
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
                    declaration.kind.as_ref().map_or("<missing>", Name::as_str)
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
            );
        }
        if let Some(schema) = definition_schema.as_ref() {
            self.validate_definition_instance(declaration, schema, info.span);
        }
        self.validate_core_declaration(declaration, info.span, in_event);

        let value_scope = info.child_scope.unwrap_or(info.containing_scope);
        let event_context = in_event || declaration.keyword.as_str() == "on";
        let mut properties = BTreeMap::new();
        let mut property_channels = BTreeMap::new();
        for (name, value) in declaration.props.iter() {
            let definition_channel =
                self.visible_definition_channel_property(value_scope, name.as_str());
            if let Some((target, _)) = definition_channel.clone() {
                property_channels.insert(name.to_string(), target);
            }
            let definition_slot = definition_schema
                .as_ref()
                .and_then(|schema| schema.slots.get(name.as_str()));
            let mut resolved = if let Some(slot) = definition_slot
                && slot.shape == "ref"
            {
                self.resolve_definition_ref_value(
                    value_scope,
                    value,
                    slot,
                    info.span,
                    event_context,
                    declaration,
                )
            } else {
                self.resolve_value(value_scope, value, info.span, event_context, declaration)
            };
            if let Some(shape) = native_schema
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
                );
                self.validate_value_shape(&resolved, &shape, name.as_str(), info.span);
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
            } else if matches!(file.kind, ProjectFileKind::Definition(_)) {
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

        let public_path = declaration_public_path(declaration, parent_public_path, inside_private);
        let child_inside_private = match declaration.visibility {
            Visibility::Private => true,
            Visibility::Public => false,
            Visibility::Default => inside_private,
        };
        self.resolve_state_declaration(file, declaration, &info, &properties);

        let pipeline = declaration.keyword.as_str() == "transform"
            && declaration
                .kind
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "pipeline");
        let resolution_order = (0..declaration.children.len())
            .filter(|index| !pipeline || declaration.children[*index].keyword.as_str() != "output")
            .chain((0..declaration.children.len()).filter(|index| {
                pipeline && declaration.children[*index].keyword.as_str() == "output"
            }))
            .collect::<Vec<_>>();
        let mut resolved_children = vec![None; declaration.children.len()];
        let mut transform_outputs = BTreeMap::new();
        for index in resolution_order {
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
        ResolvedDeclaration {
            id: info.id.clone(),
            source: file.source,
            span: info.span,
            keyword: declaration.keyword.to_string(),
            kind: declaration.kind.as_ref().map(ToString::to_string),
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate,
            component_kind: declaration
                .props
                .get("component_kind")
                .and_then(value_atom)
                .map(str::to_owned),
            properties,
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

    fn native_schema(
        &self,
        file: &ProjectFile,
        declaration: &Decl,
        coordinate: Option<&str>,
    ) -> Option<KindSchema> {
        let kind = declaration.kind.as_ref()?.as_str();
        let key = match declaration.keyword.as_str() {
            "chart" | "plot" | "view" => NativeKindKey::new(NativeKindNamespace::Coordinate, kind),
            "mark" => {
                if let Some(coordinate) = coordinate {
                    NativeKindKey::mark(coordinate, kind)
                } else if matches!(file.kind, ProjectFileKind::Definition(_)) {
                    return self.definition_mark_schema(kind);
                } else {
                    return None;
                }
            }
            "transform" if kind != "pipeline" => {
                NativeKindKey::new(NativeKindNamespace::Transform, kind)
            }
            "tool" if kind != "behavior" => NativeKindKey::new(NativeKindNamespace::Tool, kind),
            "widget" => NativeKindKey::new(NativeKindNamespace::Widget, kind),
            _ => return None,
        };
        self.registry.entries.get(&key).cloned().or_else(|| {
            // Tool schemas may be registered coordinate-independently and use
            // `compatible_coordinates` as their placement constraint.
            (declaration.keyword.as_str() == "tool")
                .then(|| {
                    self.registry
                        .entries
                        .get(&NativeKindKey::new(NativeKindNamespace::Tool, kind))
                })
                .flatten()
                .cloned()
        })
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

    fn imported_definition(&self, file: &ProjectFile, kind: &str) -> Option<&DefinitionSchema> {
        let imported = self.imports.get(&file.id)?.get(kind)?;
        self.definitions.get(imported)
    }

    fn validate_native_declaration(
        &mut self,
        declaration: &Decl,
        schema: &KindSchema,
        scope: ScopeId,
        parent: Option<&str>,
        coordinate: Option<&str>,
        span: SourceSpan,
    ) {
        if schema.body_mode == BodyMode::Properties && !declaration.children.is_empty() {
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
        let counts = declaration.children.iter().fold(
            BTreeMap::<String, usize>::new(),
            |mut counts, child| {
                *counts.entry(child.keyword.to_string()).or_default() += 1;
                counts
            },
        );
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
            if !accepted.contains(name.as_str()) {
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
    }

    fn validate_core_declaration(&mut self, declaration: &Decl, span: SourceSpan, in_event: bool) {
        match declaration.keyword.as_str() {
            "param" => {
                self.validate_core_property_names(
                    declaration,
                    &["type", "default", "sharing", "kind"],
                    span,
                );
                if !declaration.children.is_empty() {
                    self.error(
                        "AVENGER-RESOLVE-045",
                        "declaration requires a property-only body",
                        span,
                        "`param` does not accept child declarations",
                    );
                }
                for required in ["type", "default"] {
                    if declaration.props.get(required).is_none() {
                        self.error(
                            "AVENGER-RESOLVE-032",
                            "incomplete param declaration",
                            span,
                            format!("every param requires `{required}:`"),
                        );
                    }
                }
                if declaration.props.get("kind").is_some() {
                    self.error(
                        "AVENGER-RESOLVE-033",
                        "params do not have a behavioral kind",
                        span,
                        "remove `kind:` and retain the explicit physical `type:`",
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
            "mark" | "group"
                if declaration
                    .children
                    .iter()
                    .filter(|child| child.keyword.as_str() == "view")
                    .count()
                    > 1 =>
            {
                self.error(
                    "AVENGER-RESOLVE-044",
                    "a mark or group may own at most one inline view",
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
        let valid = placement_allowed(parent.keyword.as_str(), declaration.keyword.as_str());
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
        file: &ProjectFile,
        path: &[usize],
        declaration: &Decl,
        span: SourceSpan,
    ) {
        if declaration.visibility == Visibility::Default {
            return;
        }
        if matches!(file.kind, ProjectFileKind::Definition(_)) {
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
                "param" | "store" | "selection"
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
            declaration_at(file.parsed.ast.root.declarations(), &path[..length]).is_some_and(
                |ancestor| ancestor.visibility == Visibility::Private && is_structural(ancestor),
            )
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
        if value_matches_shape(value, shape) {
            if let (ResolvedValue::Object { properties, .. }, ValueShape::Object(fields)) =
                (value, shape)
            {
                self.validate_object_fields(properties, fields, property, span);
            }
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

    fn normalize_definition_arguments(
        &mut self,
        scope: ScopeId,
        source: &Value,
        resolved: &mut ResolvedValue,
        shape: &ValueShape,
        span: SourceSpan,
    ) {
        match shape {
            ValueShape::SqlExpression => {
                self.normalize_expression_argument(scope, resolved, span);
            }
            ValueShape::Array(inner) => {
                if let (Value::Array(sources), ResolvedValue::Array(values)) = (source, resolved) {
                    for (source, value) in sources.iter().zip(values) {
                        self.normalize_definition_arguments(scope, source, value, inner, span);
                    }
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
                            );
                        }
                    }
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
            ResolvedValue::Visual(value) | ResolvedValue::Pattern(value) => {
                self.normalize_expression_argument(scope, value, span);
            }
            ResolvedValue::Object {
                properties,
                children,
                ..
            } => {
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
            "function" => matches!(value, ResolvedValue::Call { .. }),
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
                })
            }
            Value::Binding { kind, path, time } => self
                .resolve_sql_binding(scope, *kind, path, *time, span, in_event, owner)
                .map(ResolvedValue::Binding)
                .unwrap_or(ResolvedValue::Invalid),
            Value::Ref { kind, path } => self
                .resolve_reference(scope, *kind, path, span)
                .map(ResolvedValue::Reference)
                .unwrap_or(ResolvedValue::Invalid),
            Value::Visual(value) => ResolvedValue::Visual(Box::new(
                self.resolve_value(scope, value, span, in_event, owner),
            )),
            Value::Dim(path) => {
                ResolvedValue::Dimension(path.iter().map(ToString::to_string).collect())
            }
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
                function: function.to_string(),
                args: args
                    .iter()
                    .map(|value| self.resolve_value(scope, value, span, in_event, owner))
                    .collect(),
            },
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
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate: None,
            component_kind: declaration
                .props
                .get("component_kind")
                .and_then(value_atom)
                .map(str::to_owned),
            properties,
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

    fn resolve_typed_reference_path(
        &mut self,
        scope: ScopeId,
        path: &[String],
        kind: RefKind,
        span: SourceSpan,
    ) -> Option<ResolvedTarget> {
        let target = if kind == RefKind::Selection && path.len() == 1 {
            let name = path.first()?;
            let mut cursor = Some(scope);
            let mut found = None;
            while let Some(id) = cursor {
                if let Some(selection) = self.scopes[id.0].selections.get(name) {
                    found = Some(ResolvedTarget::Selection(selection.clone()));
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
                if let Some(symbol) = self.scopes[id.0].values.get(first) {
                    return Some(match symbol {
                        ValueSymbol::Param(id, _) => ResolvedTarget::Param(id.clone()),
                        ValueSymbol::Store(id) => ResolvedTarget::Store(id.clone()),
                    });
                }
                if let Some(target) = self.scopes[id.0].definition_arguments.get(first) {
                    return Some(target.clone());
                }
                if let Some(id) = self.scopes[id.0].selections.get(first) {
                    return Some(ResolvedTarget::Selection(id.clone()));
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
        file: &ProjectFile,
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
                    .values
                    .get(&name)
                    .cloned();
                let Some(ValueSymbol::Param(id, Some(data_type))) = symbol else {
                    return;
                };
                let default = properties
                    .get("default")
                    .cloned()
                    .unwrap_or(ResolvedValue::Invalid);
                self.validate_typed_boundary(
                    &data_type,
                    declaration.props.get("default"),
                    &default,
                    info.span,
                    "param default",
                );
                let sharing =
                    parse_sharing(properties.get("sharing"), info.span, &mut self.diagnostics);
                let (migration_key, definition_local_seed) =
                    self.state_identity(file, info, "param");
                let dependencies = resolved_param_dependencies(&default);
                let table_owner = self.table_owner(&info.id);
                if table_owner.is_some()
                    && !declaration.props.get("default").is_some_and(|value| {
                        matches!(
                            value,
                            Value::Str(_) | Value::Num(_) | Value::Bool(_) | Value::Null
                        )
                    })
                {
                    self.error(
                        "AVENGER-RESOLVE-135",
                        "catalog-table param default must be a scalar literal",
                        info.span,
                        "table params are self-contained plan defaults and cannot read other params",
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
                        data_type,
                        default,
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
                let Some(id) = self.scopes[info.containing_scope.0]
                    .selections
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
        let file = self.project.files.get(file_id)?;
        let parent_path = &path[..path.len() - 1];
        let parent = declaration_at(file.parsed.ast.root.declarations(), parent_path)?;
        (parent.keyword.as_str() == "table").then(|| declaration_id(file, parent_path))
    }

    fn resolve_store(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        info: &DeclInfo,
        name: &str,
        properties: &BTreeMap<String, ResolvedValue>,
    ) {
        let Some(ValueSymbol::Store(id)) = self.scopes[info.containing_scope.0]
            .values
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
                    Some(source_value) => {
                        let resolved = values.get(&field.name).expect("row value exists");
                        self.validate_typed_boundary(
                            &field.data_type,
                            Some(source_value),
                            resolved,
                            info.span,
                            "store row field",
                        );
                    }
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

    fn validate_typed_boundary(
        &mut self,
        data_type: &PhysicalType,
        source: Option<&Value>,
        resolved: &ResolvedValue,
        span: SourceSpan,
        boundary: &str,
    ) {
        if let Some(source) = source
            && let Err(error) = data_type.accepts_literal(source)
            && ast_value_is_literal(source)
        {
            self.error(
                "AVENGER-RESOLVE-076",
                "typed value boundary mismatch",
                span,
                format!("{boundary}: {error}"),
            );
        }
        if let ResolvedValue::Binding(binding) = resolved
            && let Some(actual) = self.target_physical_type(&binding.target)
            && &actual != data_type
        {
            self.error(
                "AVENGER-RESOLVE-077",
                "typed value boundary requires exact Arrow type equality",
                span,
                format!(
                    "{boundary} expects `{data_type}`, but referenced param has `{}`; author an explicit CAST",
                    actual
                ),
            );
        }
    }

    fn target_physical_type(&self, target: &ResolvedTarget) -> Option<PhysicalType> {
        match target {
            ResolvedTarget::Param(id) => self.param_types.get(id).cloned(),
            ResolvedTarget::DefinitionParam {
                definition, alias, ..
            } => self
                .definitions
                .values()
                .find(|schema| &schema.declaration == definition)
                .and_then(|schema| schema.exports.get(alias))
                .and_then(|export| export.data_type.clone()),
            _ => None,
        }
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
            output_names.extend(schema.outputs.keys().cloned());
        }
        if let Some(definition) = definition {
            output_names.extend(definition.outputs.keys().cloned());
        }
        if declaration
            .kind
            .as_ref()
            .is_some_and(|kind| kind.as_str() == "aggregate")
            && let Some(Value::Array(measures)) = declaration.props.get("measures")
        {
            output_names.extend(measures.iter().filter_map(|measure| {
                let Value::Block { body, .. } = measure else {
                    return None;
                };
                match body.props.get("name") {
                    Some(Value::Str(name)) => Some(name.clone()),
                    Some(Value::Atom(name)) => Some(name.to_string()),
                    _ => None,
                }
            }));
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
                    output_names.push(name.to_string());
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
        for name in output_names {
            if seen.insert(name.clone()) {
                unique_names.push(name);
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
            .map(|(ordinal, name)| {
                (
                    name.clone(),
                    ResolvedOutputHandle {
                        producer: producer.clone(),
                        name,
                        ordinal,
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
                        | ResolvedValue::Null
                        | ResolvedValue::Column(_)
                        | ResolvedValue::Call { .. }
                        | ResolvedValue::Expression(_)
                        | ResolvedValue::Binding(_)
                )
            }) {
                self.error(
                    "AVENGER-RESOLVE-091",
                    "cursor action requires a UTF-8 scalar expression",
                    span,
                    "cursor is a write-only peer of params and stores",
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
            if let Some(ResolvedValue::Binding(binding)) = value
                && self.target_physical_type(&binding.target) != Some(PhysicalType::Utf8)
            {
                self.error(
                    "AVENGER-RESOLVE-088",
                    "cursor action binding must have exact utf8 type",
                    span,
                    "cast or bind a utf8 param before calling set_cursor",
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
        let expected = match kind.as_str() {
            "param" => Some(BindingKind::Param),
            "store" => Some(BindingKind::Store),
            "selection" => None,
            _ => {
                self.error(
                    "AVENGER-RESOLVE-089",
                    "unknown state action target kind",
                    span,
                    format!("`set {kind}` is not a param, store, selection, or cursor action"),
                );
                return;
            }
        };
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
        let target = if kind == "selection" {
            self.resolve_typed_reference_path(scope, &path, RefKind::Selection, span)
        } else {
            self.resolve_binding_path(scope, &path, expected, span, true)
        };
        if kind == "selection"
            && !matches!(
                target,
                Some(ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. })
            )
        {
            self.error(
                "AVENGER-RESOLVE-092",
                "selection action target has the wrong kind",
                span,
                format!("`{}` is not a selection", path.join(".")),
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
        if kind == "param"
            && let Some(target) = target.as_ref()
            && let Some(data_type) = self.target_physical_type(target)
        {
            self.validate_typed_boundary(
                &data_type,
                source.props.get("value"),
                action
                    .properties
                    .get("value")
                    .unwrap_or(&ResolvedValue::Invalid),
                span,
                "param action assignment",
            );
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
                    declaration.kind.as_ref().map_or("<missing>", Name::as_str)
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
                        matches!(declaration.keyword.as_str(), "chart" | "plot")
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
            let Some(field) = fields.iter().find(|field| &field.name == name) else {
                self.error(
                    "AVENGER-RESOLVE-116",
                    "store update references an unknown field",
                    span,
                    format!("field `{name}` is not declared by the target store"),
                );
                continue;
            };
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
            if let Some(source_value) = source.props.get(name)
                && let Some(resolved_value) = resolved.properties.get(name)
            {
                self.validate_typed_boundary(
                    &field.data_type,
                    Some(source_value),
                    resolved_value,
                    span,
                    "store action field",
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
            }),
        ) = (source.props.get("value"), action.properties.get_mut("value"))
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
                self.validate_typed_boundary(
                    &PhysicalType::Utf8,
                    source_id,
                    resolved_id.expect("checked"),
                    span,
                    "selection clause id",
                );
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
        if operation.contains("scene_query") {
            for required in ["geometry", "policy", "marks"] {
                if !properties.contains_key(required) {
                    self.error(
                        "AVENGER-RESOLVE-140",
                        "scene-query selection update is incomplete",
                        span,
                        format!("`{operation}` requires `{required}:`"),
                    );
                }
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
                    format!("`{}` names an already listed target", authored_path.join(".")),
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

    fn check_param_dag(&mut self) -> Vec<ParamId> {
        match topological_order(&self.param_dependencies) {
            Ok(order) => order,
            Err(cycle) => {
                let span = self
                    .project
                    .files
                    .values()
                    .next()
                    .map(root_span)
                    .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0));
                self.error(
                    "AVENGER-RESOLVE-100",
                    "param default dependency cycle",
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
                    .files
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

    fn definition_import_order(&self) -> Vec<ProjectFileId> {
        let mut dependencies = BTreeMap::<ProjectFileId, BTreeSet<ProjectFileId>>::new();
        for (file, imports) in &self.imports {
            if !self.definitions.contains_key(file) {
                continue;
            }
            dependencies.entry(file.clone()).or_default().extend(
                imports
                    .values()
                    .filter(|imported| self.definitions.contains_key(*imported))
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
        self.table_names
            .iter()
            .find_map(|(name, candidate)| (candidate == id).then_some(name.clone()))
            .unwrap_or_else(|| id.as_str().to_owned())
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
            .chart_roots
            .iter()
            .filter_map(|file| self.project.files.get(file).map(|file| file.source))
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
    imports: &[crate::project::ImportEdge],
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
        .filter(|edge| edge.importer == current)
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        left.binding
            .cmp(&right.binding)
            .then_with(|| left.site.cmp(&right.site))
    });
    for edge in edges {
        path.push(ExpansionOrImportFrame {
            span: edge.site,
            message: format!("imported through binding `{}`", edge.binding),
        });
        if find_import_trace(edge.imported, target, imports, visiting, path) {
            return true;
        }
        path.pop();
    }
    visiting.remove(&current);
    false
}

fn declaration_coordinate(declaration: &Decl, inherited: Option<&str>) -> Option<String> {
    match declaration.keyword.as_str() {
        "chart" | "plot" | "view" => declaration.kind.as_ref().map(ToString::to_string),
        _ => inherited.map(str::to_owned),
    }
}

fn coordinate_at_path(file: &ProjectFile, path: &[usize]) -> Option<String> {
    let mut coordinate = None;
    for length in 1..=path.len() {
        let declaration = declaration_at(file.parsed.ast.root.declarations(), &path[..length])?;
        coordinate = declaration_coordinate(declaration, coordinate.as_deref());
    }
    coordinate
}

fn parent_declaration<'a>(file: &'a ProjectFile, path: &[usize]) -> Option<&'a Decl> {
    (path.len() > 1)
        .then(|| declaration_at(file.parsed.ast.root.declarations(), &path[..path.len() - 1]))
        .flatten()
}

fn requires_registered_kind(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "plot" | "view" | "mark" | "transform" | "tool" | "widget"
    ) && !matches!(
        (
            declaration.keyword.as_str(),
            declaration.kind.as_ref().map(Name::as_str)
        ),
        ("transform", Some("pipeline")) | ("tool", Some("behavior"))
    )
}

fn placement_allowed(parent: &str, child: &str) -> bool {
    if child == "view" {
        return matches!(parent, "mark" | "group");
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
        "transform" => matches!(child, "transform" | "output"),
        "on" => matches!(child, "set" | "on"),
        "view" => matches!(child, "transform" | "mark" | "group"),
        "mark" => matches!(child, "view"),
        "table" => matches!(child, "param" | "field" | "row" | "key"),
        "catalog" => matches!(child, "schema"),
        "schema" => matches!(child, "table"),
        _ => ordinary_plot_child(child) || matches!(child, "export" | "set"),
    }
}

fn ordinary_plot_child(child: &str) -> bool {
    matches!(
        child,
        "param"
            | "store"
            | "selection"
            | "resource"
            | "theme"
            | "group"
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
            | "overlay"
            | "layer"
            | "when"
            | "scale_edit"
            | "scale_hint"
            | "dimension"
    )
}

fn core_property(declaration: &Decl, property: &str) -> bool {
    match declaration.keyword.as_str() {
        "chart" | "plot" => {
            matches!(property, "data" | "title" | "subtitle" | "layout" | "theme")
        }
        "view" => property == "data",
        "group" => matches!(property, "data" | "component_kind" | "label"),
        "mark" => matches!(property, "data"),
        "tool" => matches!(property, "id"),
        "widget" => false,
        "transform" => false,
        _ => true,
    }
}

fn schema_property<'a>(schema: &'a KindSchema, name: &str) -> Option<&'a ValueShape> {
    schema
        .properties
        .get(name)
        .map(|property| &property.shape)
        .or_else(|| schema.channels.get(name).map(|channel| &channel.shape))
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

fn value_matches_shape(value: &ResolvedValue, shape: &ValueShape) -> bool {
    match shape {
        ValueShape::Any => !matches!(value, ResolvedValue::Invalid),
        ValueShape::Boolean => matches!(value, ResolvedValue::Boolean(_)),
        ValueShape::Integer => {
            matches!(value, ResolvedValue::Number(value) if value.parse::<i64>().is_ok())
        }
        ValueShape::Number => matches!(value, ResolvedValue::Number(_)),
        ValueShape::String => matches!(value, ResolvedValue::String(_)),
        ValueShape::Atom { values } => {
            matches!(value, ResolvedValue::Atom(value) if values.iter().any(|candidate| candidate.value == *value))
        }
        ValueShape::SqlExpression => is_expression_value(value),
        ValueShape::SqlQuery => matches!(value, ResolvedValue::Query(_)),
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
        ValueShape::TypedReference { namespaces } => match value {
            ResolvedValue::Reference(reference) => namespaces
                .iter()
                .any(|namespace| namespace_matches_target(*namespace, &reference.target)),
            _ => false,
        },
        ValueShape::Array(inner) => {
            matches!(value, ResolvedValue::Array(values) if values.iter().all(|value| value_matches_shape(value, inner)))
        }
        ValueShape::Object(_) => matches!(value, ResolvedValue::Object { .. }),
    }
}

fn resolved_value_contains_invalid(value: &ResolvedValue) -> bool {
    match value {
        ResolvedValue::Invalid => true,
        ResolvedValue::Visual(value) | ResolvedValue::Pattern(value) => {
            resolved_value_contains_invalid(value)
        }
        ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
            values.iter().any(resolved_value_contains_invalid)
        }
        ResolvedValue::Object {
            properties,
            children,
            ..
        } => {
            properties.values().any(resolved_value_contains_invalid)
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
                NativeKindNamespace::Mark | NativeKindNamespace::Tool | NativeKindNamespace::Widget,
                ResolvedTarget::DefinitionStructural { .. }
            )
    )
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
        ) | (RefKind::Group, ResolvedTarget::Declaration(_))
            | (
                RefKind::Group,
                ResolvedTarget::DefinitionStructural {
                    kind: DefinitionExportKind::Group,
                    ..
                }
            )
            | (
                RefKind::Selection,
                ResolvedTarget::Selection(_) | ResolvedTarget::DefinitionSelection { .. }
            )
            | (RefKind::Tool, ResolvedTarget::Tool(_))
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
        "group" => reference_kind_matches(target, RefKind::Group),
        "tool" => reference_kind_matches(target, RefKind::Tool),
        "widget" => reference_kind_matches(target, RefKind::Widget),
        "resource" => reference_kind_matches(target, RefKind::Resource),
        _ => false,
    }
}

fn definition_ref_kind(kind: &str) -> Option<RefKind> {
    Some(match kind {
        "mark" => RefKind::Mark,
        "group" => RefKind::Group,
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
            | ResolvedValue::Visual(_)
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

fn ast_value_is_literal(value: &Value) -> bool {
    match value {
        Value::Str(_) | Value::Num(_) | Value::Bool(_) | Value::Null => true,
        Value::Array(values) => values.iter().all(ast_value_is_literal),
        Value::Block { head: None, body } => {
            body.children.is_empty()
                && body
                    .props
                    .iter()
                    .all(|(_, value)| ast_value_is_literal(value))
        }
        _ => false,
    }
}

fn shape_name(shape: &ValueShape) -> &'static str {
    match shape {
        ValueShape::Boolean => "boolean",
        ValueShape::Integer => "integer",
        ValueShape::Number => "number",
        ValueShape::String => "string",
        ValueShape::Atom { .. } => "enum atom",
        ValueShape::SqlExpression => "SQL expression",
        ValueShape::SqlQuery => "SQL query",
        ValueShape::ScalarBinding => "param binding",
        ValueShape::TableBinding => "store binding",
        ValueShape::TypedReference { .. } => "typed reference",
        ValueShape::Array(_) => "array",
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
        ResolvedValue::Query(_) => "SQL query",
        ResolvedValue::Binding(_) => "binding",
        ResolvedValue::Reference(_) => "reference",
        ResolvedValue::Visual(_) => "visual value",
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
        (serde_json::Value::Object(values), ValueShape::Object(fields)) => ResolvedValue::Object {
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
            properties,
            children,
            ..
        } => {
            properties
                .values()
                .any(resolved_value_has_forbidden_stream_binding)
                || children.iter().any(|child| {
                    child
                        .properties
                        .values()
                        .any(resolved_value_has_forbidden_stream_binding)
                })
        }
        ResolvedValue::Visual(value) | ResolvedValue::Pattern(value) => {
            resolved_value_has_forbidden_stream_binding(value)
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
            properties,
            children,
            ..
        } => {
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
        ResolvedValue::Visual(value) | ResolvedValue::Pattern(value) => {
            collect_param_dependencies(value, output);
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
            references: Vec::new(),
        }),
        Value::Query(value) => ResolvedValue::Query(ResolvedQuery {
            sql: value.canonical_sql(),
            bindings: Vec::new(),
            helpers: helpers_in_sql(&value.canonical_sql()),
            references: Vec::new(),
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
        Value::Visual(value) => ResolvedValue::Visual(Box::new(unresolved_value(value))),
        Value::Dim(path) => {
            ResolvedValue::Dimension(path.iter().map(ToString::to_string).collect())
        }
        Value::Pattern(value) => ResolvedValue::Pattern(Box::new(unresolved_value(value))),
        Value::Env(value) => ResolvedValue::Environment(value.clone()),
        Value::None => ResolvedValue::None,
        Value::Array(values) => ResolvedValue::Array(values.iter().map(unresolved_value).collect()),
        Value::Block { head, body } => ResolvedValue::Object {
            kind: head.as_deref().and_then(value_atom).map(str::to_owned),
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
        (Value::Visual(source), ResolvedValue::Visual(resolved))
        | (Value::Pattern(source), ResolvedValue::Pattern(resolved)) => {
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
                kind, properties, ..
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
            | "group"
            | "mark"
            | "tool"
            | "widget"
            | "view"
            | "cell"
            | "plot"
            | "variable"
            | "dimension"
            | "overlay"
    )
}

fn owns_lexical_scope(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "define" | "group" | "view" | "cell" | "plot" | "overlay" | "on" | "table"
    ) || (matches!(declaration.keyword.as_str(), "tool" | "mark")
        && !declaration.children.is_empty())
}

fn is_instance_boundary(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "group" | "tool" | "widget" | "mark" | "view" | "cell" | "plot"
    )
}

fn declaration_id(file: &ProjectFile, path: &[usize]) -> DeclarationId {
    let stable_path = stable_declaration_path(file, path);
    DeclarationId(semantic_hash(&[
        "declaration",
        file.id.as_str(),
        &stable_path,
    ]))
}

fn stable_declaration_path(file: &ProjectFile, path: &[usize]) -> String {
    let mut components = Vec::with_capacity(path.len());
    for (depth, index) in path.iter().copied().enumerate() {
        let siblings = if depth == 0 {
            file.parsed.ast.root.declarations()
        } else {
            declaration_at(file.parsed.ast.root.declarations(), &path[..depth])
                .map_or(&[][..], |parent| parent.children.as_slice())
        };
        let Some(declaration) = siblings.get(index) else {
            components.push(format!("missing:{index}"));
            continue;
        };
        let signature = declaration_identity_signature(declaration);
        let ordinal = siblings[..index]
            .iter()
            .filter(|candidate| declaration_identity_signature(candidate) == signature)
            .count();
        components.push(format!("{signature}#{ordinal}"));
    }
    components.join("/")
}

fn declaration_identity_signature(declaration: &Decl) -> String {
    let name = declaration.name.as_ref().map_or("", Name::as_str);
    let kind = declaration.kind.as_ref().map_or("", Name::as_str);
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

fn declaration_at<'a>(roots: &'a [Decl], path: &[usize]) -> Option<&'a Decl> {
    let (first, rest) = path.split_first()?;
    let mut declaration = roots.get(*first)?;
    for index in rest {
        declaration = declaration.children.get(*index)?;
    }
    Some(declaration)
}

fn declaration_span(file: &ProjectFile, path: &[usize]) -> Option<SourceSpan> {
    let declaration = declaration_at(file.parsed.ast.root.declarations(), path)?;
    let mut spans = file
        .parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| match file.parsed.source_map.role(id) {
            Some(AstNodeRole::Declaration(keyword)) if keyword == &declaration.keyword => {
                Some(span)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    // Same-keyword declarations are selected by their ordinal among matching
    // declarations in source preorder.
    let mut ordinal = 0usize;
    declaration_preorder(file.parsed.ast.root.declarations(), &mut |candidate| {
        if std::ptr::eq(candidate, declaration) {
            return false;
        }
        if candidate.keyword == declaration.keyword {
            ordinal += 1;
        }
        true
    });
    spans.get(ordinal).copied()
}

fn declaration_preorder<'a>(declarations: &'a [Decl], visit: &mut impl FnMut(&'a Decl) -> bool) {
    for declaration in declarations {
        if !visit(declaration) {
            return;
        }
        declaration_preorder(&declaration.children, visit);
    }
}

fn root_span(file: &ProjectFile) -> SourceSpan {
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
        "group" => DefinitionExportKind::Group,
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
        Value::Array(values) => {
            for value in values {
                collect_definition_slot_dependencies(value, slots, output);
            }
        }
        Value::Visual(value) | Value::Pattern(value) => {
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

fn infer_widget_item_type(declaration: &Decl) -> Option<PhysicalType> {
    let Value::Array(items) = declaration.props.get("items")? else {
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
                .is_some_and(|value| inferred.accepts_literal(value).is_ok()),
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
        format!(
            "avenger 1; chart cartesian {{ param as value {{ type: {text}; default: NULL; }} }}"
        ),
    );
    let parsed = crate::syntax::parse_file(&source).ok()?;
    let Root::Chart(chart) = parsed.ast.root else {
        return None;
    };
    PhysicalType::parse(chart.children.first()?.props.get("type")?).ok()
}

fn helpers_in_sql(sql: &str) -> Vec<ResolvedHelper> {
    const HELPERS: &[(&str, HelperClass)] = &[
        ("datum", HelperClass::Datum),
        ("event_coord", HelperClass::Event),
        ("event_path", HelperClass::Event),
        ("start_coord", HelperClass::Event),
        ("selection_test", HelperClass::Selection),
        ("view_x", HelperClass::View),
        ("view_y", HelperClass::View),
        ("scaled", HelperClass::Channel),
        ("unscaled", HelperClass::Channel),
    ];
    HELPERS
        .iter()
        .filter(|(name, _)| sql.contains(&format!("{name}(")))
        .map(|(name, class)| ResolvedHelper {
            name: (*name).to_owned(),
            class: *class,
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
    Some(match name {
        "channel" => HelperClass::Channel,
        "datum" | "item_data" => HelperClass::Datum,
        "event_coord" | "start_coord" | "event_domain_start" | "event_domain_end"
        | "event_path" | "event_facet_value" | "legend_value" | "item_channel" | "item_bbox" => {
            HelperClass::Event
        }
        "selection_contains" => HelperClass::Selection,
        "view_x" | "view_y" => HelperClass::View,
        "span" | "span_ordered" | "polygon" => HelperClass::Reserved,
        _ => return None,
    })
}

fn helper_arity(name: &str) -> Option<usize> {
    Some(match name {
        "event_path" | "legend_value" => 0,
        "channel" | "datum" | "event_coord" | "start_coord" | "event_domain_start"
        | "event_domain_end" | "event_facet_value" | "item_channel" | "item_data" | "item_bbox"
        | "polygon" => 1,
        "selection_contains" | "view_x" | "view_y" | "span" | "span_ordered" => 2,
        _ => return None,
    })
}

fn helper_uses_channel_argument(name: &str) -> bool {
    matches!(
        name,
        "channel"
            | "event_coord"
            | "start_coord"
            | "event_domain_start"
            | "event_domain_end"
            | "item_channel"
    )
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
    struct Relations(Vec<Vec<String>>);

    impl Visitor for Relations {
        type Break = ();

        fn pre_visit_relation(
            &mut self,
            relation: &ObjectName,
        ) -> std::ops::ControlFlow<Self::Break> {
            self.0.push(
                relation
                    .0
                    .iter()
                    .filter_map(|part| part.as_ident())
                    .map(|identifier| identifier.value.clone())
                    .collect(),
            );
            std::ops::ControlFlow::Continue(())
        }
    }

    let mut relations = Relations::default();
    let _ = query.visit(&mut relations);
    relations.0.retain(|path| !path.is_empty());
    relations.0
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
                    .map_or("anonymous", Name::as_str)
                    .to_owned()
            },
            ToString::to_string,
        )
    }
}
