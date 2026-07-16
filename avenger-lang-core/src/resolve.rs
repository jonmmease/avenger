//! Registry-directed semantic resolution for parsed Avenger projects.
//!
//! This phase deliberately stops before DataFusion planning and native chart
//! construction. Its output contains no unresolved authored names: SQL
//! placeholders, typed references, state l-values, structural paths, and
//! definition imports are all bound to opaque semantic identities.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    ValueShape,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Diagnostic, LANGUAGE_MAJOR, PhysicalField, PhysicalType, SourceId, SourceLabel, SourceMap,
    SourceSpan,
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
    pub migration_key: StateMigrationKey,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
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
    pub migration_key: StateMigrationKey,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSelection {
    pub id: SelectionId,
    pub declaration: DeclarationId,
    pub source_name: String,
    pub migration_key: StateMigrationKey,
    pub lexical_scope: String,
    pub owner_ancestry: Vec<DeclarationId>,
    pub generated_by: Option<GeneratedStateOrigin>,
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
    pub slots: BTreeMap<String, DefinitionSlot>,
    pub channels: BTreeMap<String, DefinitionChannel>,
    pub outputs: BTreeMap<String, Option<ResolvedValue>>,
    pub exports: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionSlot {
    pub shape: String,
    pub required: bool,
    pub default: Option<ResolvedValue>,
    pub enum_values: Vec<String>,
    pub reference_kind: Option<String>,
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
    pub properties: BTreeMap<String, ResolvedValue>,
    pub children: Vec<ResolvedDeclaration>,
    pub runtime_target: Option<ResolvedTarget>,
    pub public_path: Option<String>,
    pub parts: BTreeMap<String, ResolvedPart>,
    pub exports: BTreeMap<String, ResolvedTarget>,
    pub transform_outputs: BTreeMap<String, ResolvedOutputHandle>,
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
    Part {
        declaration: DeclarationId,
        alias: String,
    },
    Output(ResolvedOutputHandle),
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
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExpression {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedQuery {
    pub sql: String,
    pub bindings: Vec<ResolvedBinding>,
    pub helpers: Vec<ResolvedHelper>,
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
    label: String,
    values: BTreeMap<String, ValueSymbol>,
    selections: BTreeMap<String, SelectionId>,
    structural: BTreeMap<String, DeclarationId>,
    events: BTreeMap<String, EventId>,
}

#[derive(Clone, Debug)]
enum ValueSymbol {
    Param(ParamId, Option<PhysicalType>),
    Store(StoreId),
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
    stores: BTreeMap<StoreId, ResolvedStore>,
    selections: BTreeMap<SelectionId, ResolvedSelection>,
    param_dependencies: BTreeMap<ParamId, BTreeSet<ParamId>>,
    table_dependencies: BTreeMap<DeclarationId, BTreeSet<DeclarationId>>,
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
            stores: BTreeMap::new(),
            selections: BTreeMap::new(),
            param_dependencies: BTreeMap::new(),
            table_dependencies: BTreeMap::new(),
        }
    }

    fn run(&mut self) -> ResolveAttempt {
        self.check_versions();
        self.build_import_bindings();
        self.extract_definition_schemas();
        self.predeclare_project();
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
                    let resolved =
                        self.resolve_declaration(file, declaration, &path, None, None, false);
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
        sort_diagnostics(&mut self.diagnostics, &self.project.sources);
        if !self.diagnostics.is_empty() {
            return ResolveAttempt {
                result: Err(ResolveFailure {
                    diagnostics: std::mem::take(&mut self.diagnostics),
                    sources: self.project.sources.clone(),
                }),
            };
        }

        let mut public_targets = BTreeMap::new();
        for file in files.values() {
            for root in &file.roots {
                collect_public_targets(root, &mut public_targets);
            }
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
            let id = declaration_id(file_id, &[0], declaration.keyword.as_str());
            let mut slots = BTreeMap::new();
            let mut channels = BTreeMap::new();
            let mut outputs = BTreeMap::new();
            let mut exports = BTreeMap::new();
            for child in &declaration.children {
                match child.keyword.as_str() {
                    "slot" => {
                        let Some(name) = child.name.as_ref() else {
                            continue;
                        };
                        let shape = child.kind.as_ref().map_or("", Name::as_str).to_owned();
                        let default = child.props.get("default").map(unresolved_value);
                        let enum_values = value_names(child.props.get("values"));
                        let reference_kind = child
                            .props
                            .get("kind")
                            .and_then(value_atom)
                            .map(str::to_owned);
                        slots.insert(
                            name.to_string(),
                            DefinitionSlot {
                                shape,
                                required: default.is_none(),
                                default,
                                enum_values,
                                reference_kind,
                            },
                        );
                    }
                    "channel" => {
                        if let Some(name) = child.name.as_ref() {
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
                            outputs.insert(
                                name.to_string(),
                                child.props.get("value").map(unresolved_value),
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
                            if let Some(alias) = alias
                                && exports.insert(alias.clone(), path).is_some()
                            {
                                self.error(
                                    "AVENGER-RESOLVE-003",
                                    "duplicate definition export",
                                    root_span(file),
                                    format!("export alias `{alias}` is declared more than once"),
                                );
                            }
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
                        .kind
                        .as_ref()
                        .map_or_else(|| file_id.as_str().to_owned(), ToString::to_string),
                    slots,
                    channels,
                    outputs,
                    exports,
                },
            );
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

    fn predeclare_declaration(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        path: Vec<usize>,
        containing_scope: ScopeId,
        ancestry: Vec<DeclarationId>,
    ) {
        let id = declaration_id(&file.id, &path, declaration.keyword.as_str());
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
            self.predeclare_name(
                file,
                declaration,
                containing_scope,
                name.as_str(),
                &id,
                runtime_target.clone(),
                &ancestry,
                span,
            );
        }
        if is_structural(declaration) {
            self.instances
                .entry(id.clone())
                .or_insert_with(|| InstanceInterface {
                    child_scope,
                    ..InstanceInterface::default()
                });
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
                    ValueSymbol::Param(param_id, data_type),
                    span,
                );
            }
            "store" => {
                let store_id = StoreId(semantic_hash(&[
                    "store-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
                self.insert_value_symbol(scope, name, ValueSymbol::Store(store_id), span);
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
                    .insert(name.to_owned(), selection_id)
                    .is_some()
                {
                    self.error(
                        "AVENGER-RESOLVE-011",
                        "duplicate selection name",
                        span,
                        format!("`{name}` is already declared in this scope"),
                    );
                }
            }
            "on" => {
                let event_id = EventId(semantic_hash(&[
                    "event-runtime",
                    file.id.as_str(),
                    id.as_str(),
                    &ancestry_text(ancestry),
                ]));
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
                if let Some(target) = runtime_target {
                    self.instances.entry(id.clone()).or_default();
                    if let Some(interface) = self.instances.get_mut(id) {
                        interface.exports.insert("self".to_owned(), target);
                    }
                }
            }
            _ => {}
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
            if let Some(schema) = self.native_schema(file, declaration, None) {
                self.install_native_interface(file, declaration, &info, &schema);
            }
            if let Some(interface) = self.instances.get_mut(&info.id) {
                for child in &declaration.children {
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
                            interface.pending_exports.insert(alias, source);
                        }
                    }
                }
            }
        }
        self.resolve_pending_exports();
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
            .map(|export| (export.alias.clone(), export.value_kind.clone()))
            .collect::<Vec<_>>();
        let _ = interface;
        for (alias, value_kind) in export_entries {
            if let Some(existing) = existing_export_binding(declaration, &alias) {
                if let Some(target) = self.resolve_binding_path(
                    info.containing_scope,
                    &existing,
                    None,
                    info.span,
                    false,
                ) {
                    self.instances
                        .get_mut(&info.id)
                        .expect("instance exists")
                        .exports
                        .insert(alias, target);
                }
                continue;
            }
            let target = self.generated_state_target(file, declaration, info, &alias, &value_kind);
            if let Some(target) = target {
                self.instances
                    .get_mut(&info.id)
                    .expect("instance exists")
                    .exports
                    .insert(alias, target);
            }
        }
    }

    fn generated_state_target(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        info: &DeclInfo,
        alias: &str,
        value_kind: &str,
    ) -> Option<ResolvedTarget> {
        let origin = GeneratedStateOrigin {
            declaration: info.id.clone(),
            export_role: alias.to_owned(),
        };
        let migration_key = StateMigrationKey(semantic_hash(&[
            "generated-migration",
            file.id.as_str(),
            info.id.as_str(),
            alias,
            &ancestry_text(&info.ancestry),
        ]));
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
            let default = declaration
                .props
                .get("default")
                .map(unresolved_value)
                .unwrap_or(ResolvedValue::Null);
            self.params.insert(
                id.clone(),
                ResolvedParam {
                    id: id.clone(),
                    declaration: info.id.clone(),
                    source_name: format!("{}.{}", declaration.name_string(), alias),
                    data_type,
                    default,
                    sharing: StateSharing::Shared,
                    migration_key,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
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
                    migration_key,
                    lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                    owner_ancestry: info.ancestry.clone(),
                    generated_by: Some(origin),
                },
            );
            return Some(ResolvedTarget::Selection(id));
        }
        None
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
                if let Some(target) = self.resolve_any_path(
                    scope,
                    &path,
                    SourceSpan::empty(SourceId::new(0), 0),
                    false,
                ) {
                    self.instances
                        .get_mut(&id)
                        .expect("instance exists")
                        .exports
                        .insert(alias, target);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_declaration(
        &mut self,
        file: &ProjectFile,
        declaration: &Decl,
        path: &[usize],
        inherited_coordinate: Option<&str>,
        parent_public_path: Option<&str>,
        in_event: bool,
    ) -> ResolvedDeclaration {
        let info = self
            .declarations
            .get(&(file.id.clone(), path.to_vec()))
            .cloned()
            .unwrap_or_else(|| DeclInfo {
                id: declaration_id(&file.id, path, declaration.keyword.as_str()),
                span: root_span(file),
                containing_scope: ScopeId(0),
                child_scope: None,
                ancestry: Vec::new(),
                runtime_target: runtime_target(
                    declaration,
                    &declaration_id(&file.id, path, declaration.keyword.as_str()),
                ),
            });
        let coordinate = declaration_coordinate(declaration, inherited_coordinate);
        let parent = parent_declaration(file, path);
        self.validate_placement(declaration, parent, info.span);
        self.validate_visibility(declaration, parent, info.span);

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
            self.validate_native_declaration(declaration, schema, info.span);
        }
        if let Some(schema) = definition_schema.as_ref() {
            self.validate_definition_instance(declaration, schema, info.span);
        }
        self.validate_core_declaration(declaration, info.span, in_event);

        let value_scope = info.child_scope.unwrap_or(info.containing_scope);
        let event_context = in_event || declaration.keyword.as_str() == "on";
        let mut properties = BTreeMap::new();
        for (name, value) in declaration.props.iter() {
            let resolved =
                self.resolve_value(value_scope, value, info.span, event_context, declaration);
            if let Some(shape) = native_schema
                .as_ref()
                .and_then(|schema| schema_property(schema, name.as_str()))
            {
                self.validate_value_shape(&resolved, shape, name.as_str(), info.span);
            } else if let Some(slot) = definition_schema
                .as_ref()
                .and_then(|schema| schema.slots.get(name.as_str()))
            {
                self.validate_definition_value(&resolved, slot, name.as_str(), info.span);
            }
            properties.insert(name.to_string(), resolved);
        }
        if let Some(schema) = native_schema.as_ref() {
            for (name, property) in &schema.properties {
                if !properties.contains_key(name)
                    && let Some(default) = &property.default
                {
                    properties.insert(name.clone(), resolved_json(default));
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
        }

        let public_path = declaration_public_path(declaration, parent_public_path);
        self.resolve_state_declaration(file, declaration, &info, &properties);

        let mut children = Vec::new();
        let mut transform_outputs = BTreeMap::new();
        for (index, child) in declaration.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            let resolved = self.resolve_declaration(
                file,
                child,
                &child_path,
                coordinate.as_deref(),
                public_path.as_deref().or(parent_public_path),
                event_context,
            );
            if child.keyword.as_str() == "transform" {
                self.install_sequential_transform(value_scope, child, &resolved, info.span);
                for (name, output) in &resolved.transform_outputs {
                    transform_outputs.insert(name.clone(), output.clone());
                }
            }
            children.push(resolved);
        }
        if declaration.keyword.as_str() == "transform" {
            transform_outputs = self.transform_outputs(declaration, &info, &children);
        }
        self.validate_event_actions(declaration, &children, info.span);

        let interface = self.instances.get(&info.id).cloned().unwrap_or_default();
        ResolvedDeclaration {
            id: info.id,
            source: file.source,
            span: info.span,
            keyword: declaration.keyword.to_string(),
            kind: declaration.kind.as_ref().map(ToString::to_string),
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate,
            properties,
            children,
            runtime_target: info.runtime_target,
            public_path,
            parts: interface.parts,
            exports: interface.exports,
            transform_outputs,
        }
    }

    fn native_schema(
        &self,
        _file: &ProjectFile,
        declaration: &Decl,
        coordinate: Option<&str>,
    ) -> Option<KindSchema> {
        let kind = declaration.kind.as_ref()?.as_str();
        let key = match declaration.keyword.as_str() {
            "chart" | "plot" | "view" => NativeKindKey::new(NativeKindNamespace::Coordinate, kind),
            "mark" => NativeKindKey::mark(coordinate?, kind),
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

    fn imported_definition(&self, file: &ProjectFile, kind: &str) -> Option<&DefinitionSchema> {
        let imported = self.imports.get(&file.id)?.get(kind)?;
        self.definitions.get(imported)
    }

    fn validate_native_declaration(
        &mut self,
        declaration: &Decl,
        schema: &KindSchema,
        span: SourceSpan,
    ) {
        let accepted = schema
            .properties
            .keys()
            .chain(schema.channels.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for (name, _) in declaration.props.iter() {
            if !accepted.contains(name.as_str()) && !core_property(declaration, name.as_str()) {
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
            if channel.required && declaration.props.get(name).is_none() {
                self.error(
                    "AVENGER-RESOLVE-022",
                    "missing required channel",
                    span,
                    format!("`{}` requires channel `{name}:`", schema.key.kind),
                );
            }
        }
        if !schema.compatible_coordinates.is_empty() {
            let coordinate = nearest_coordinate_kind(declaration);
            if let Some(coordinate) = coordinate
                && !schema.compatible_coordinates.contains(coordinate)
            {
                self.error(
                    "AVENGER-RESOLVE-023",
                    "declaration is incompatible with its coordinate system",
                    span,
                    format!("`{}` does not support `{coordinate}`", schema.key.kind),
                );
            }
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
            _ => {}
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

    fn validate_visibility(&mut self, declaration: &Decl, parent: Option<&Decl>, span: SourceSpan) {
        if declaration.visibility == Visibility::Default {
            return;
        }
        if declaration.name.is_none() || !is_structural(declaration) {
            self.error(
                "AVENGER-RESOLVE-041",
                "visibility requires a named structural declaration",
                span,
                "only named structural identities can be public or private",
            );
        }
        if declaration.visibility == Visibility::Public
            && !parent.is_some_and(|parent| parent.visibility == Visibility::Private)
        {
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
        let valid = match slot.shape.as_str() {
            "expr" => is_expression_value(value),
            "expr_list" => {
                matches!(value, ResolvedValue::Array(values) if values.iter().all(is_expression_value))
            }
            "literal" => is_literal_value(value),
            "number" => matches!(value, ResolvedValue::Number(_)),
            "string" => matches!(value, ResolvedValue::String(_)),
            "boolean" => matches!(value, ResolvedValue::Boolean(_)),
            "enum" => {
                matches!(value, ResolvedValue::Atom(atom) if slot.enum_values.iter().any(|candidate| candidate == atom))
            }
            "function" => matches!(value, ResolvedValue::Call { .. }),
            "ref" => matches!(value, ResolvedValue::Reference(_)),
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
                ResolvedValue::Expression(ResolvedExpression {
                    helpers: helpers_in_sql(&sql),
                    sql,
                    bindings,
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
                ResolvedValue::Query(ResolvedQuery {
                    helpers: helpers_in_sql(&sql),
                    sql,
                    bindings,
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
                        self.resolve_inline_declaration(scope, child, span, index, in_event)
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
    ) -> ResolvedDeclaration {
        self.validate_core_declaration(declaration, span, in_event);
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
        ResolvedDeclaration {
            id,
            source: span.source,
            span,
            keyword: declaration.keyword.to_string(),
            kind: declaration.kind.as_ref().map(ToString::to_string),
            name: declaration.name.as_ref().map(ToString::to_string),
            visibility: declaration.visibility,
            coordinate: None,
            properties,
            children: declaration
                .children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    self.resolve_inline_declaration(scope, child, span, index, in_event)
                })
                .collect(),
            runtime_target: None,
            public_path: None,
            parts: BTreeMap::new(),
            exports: BTreeMap::new(),
            transform_outputs: BTreeMap::new(),
        }
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
        owner: &Decl,
    ) -> Option<ResolvedBinding> {
        let path = path.iter().map(ToString::to_string).collect::<Vec<_>>();
        let target = self.resolve_binding_path(scope, &path, Some(kind), span, true)?;
        if time != BindingTime::Current && !in_event {
            self.error(
                "AVENGER-RESOLVE-060",
                "temporal state read is outside an event binding",
                span,
                "`@start` and `@previous` are defined only for event invocations",
            );
        }
        if time == BindingTime::Start && owner.props.get("between").is_none() {
            self.error(
                "AVENGER-RESOLVE-061",
                "`@start` requires a between interaction",
                span,
                "the containing event must declare `between:`",
            );
        }
        if time != BindingTime::Current && matches!(target, ResolvedTarget::Store(_)) {
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
            ResolvedTarget::Param(_) => BindingKind::Param,
            ResolvedTarget::Store(_) => BindingKind::Store,
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
        let target = self.resolve_any_path(scope, &authored_path, span, true)?;
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
        } else {
            let mut cursor = Some(scope);
            let mut declaration = None;
            while let Some(id) = cursor {
                if let Some(found) = self.scopes[id.0].structural.get(first) {
                    declaration = Some(found.clone());
                    break;
                }
                cursor = self.scopes[id.0].parent;
            }
            if let Some(declaration) = declaration {
                let mut current = declaration;
                for (index, segment) in path.iter().enumerate().skip(1) {
                    let Some(interface) = self.instances.get(&current) else {
                        break;
                    };
                    if index == path.len() - 1 {
                        if let Some(target) = interface.exports.get(segment) {
                            return Some(target.clone());
                        }
                        if interface.parts.contains_key(segment) {
                            return Some(ResolvedTarget::Part {
                                declaration: current,
                                alias: segment.clone(),
                            });
                        }
                    }
                    if let Some(ResolvedTarget::Declaration(next)) = interface.exports.get(segment)
                    {
                        current = next.clone();
                        continue;
                    }
                    break;
                }
            }
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
                let migration_key = StateMigrationKey(semantic_hash(&[
                    "param-migration",
                    file.id.as_str(),
                    info.id.as_str(),
                    &ancestry_text(&info.ancestry),
                ]));
                let dependencies = resolved_param_dependencies(&default);
                self.param_dependencies.insert(id.clone(), dependencies);
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
                        lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                        owner_ancestry: info.ancestry.clone(),
                        generated_by: None,
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
                self.selections.insert(
                    id.clone(),
                    ResolvedSelection {
                        id,
                        declaration: info.id.clone(),
                        source_name: name,
                        migration_key: StateMigrationKey(semantic_hash(&[
                            "selection-migration",
                            file.id.as_str(),
                            info.id.as_str(),
                            &ancestry_text(&info.ancestry),
                        ])),
                        lexical_scope: self.scopes[info.containing_scope.0].label.clone(),
                        owner_ancestry: info.ancestry.clone(),
                        generated_by: None,
                    },
                );
            }
            _ => {}
        }
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
        for row in declaration
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "row")
        {
            let values = row
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
                    None => {}
                }
            }
            rows.push(values);
        }
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
                migration_key: StateMigrationKey(semantic_hash(&[
                    "store-migration",
                    file.id.as_str(),
                    info.id.as_str(),
                    &ancestry_text(&info.ancestry),
                ])),
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
            && !matches!(source, Value::Expr(_) | Value::Binding { .. })
        {
            self.error(
                "AVENGER-RESOLVE-076",
                "typed value boundary mismatch",
                span,
                format!("{boundary}: {error}"),
            );
        }
        if let ResolvedValue::Binding(binding) = resolved
            && let ResolvedTarget::Param(id) = &binding.target
            && let Some(param) = self.params.get(id)
            && &param.data_type != data_type
        {
            self.error(
                "AVENGER-RESOLVE-077",
                "typed value boundary requires exact Arrow type equality",
                span,
                format!(
                    "{boundary} expects `{data_type}`, but referenced param has `{}`; author an explicit CAST",
                    param.data_type
                ),
            );
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
        output_names.sort();
        output_names.dedup();
        output_names
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
        children: &[ResolvedDeclaration],
        span: SourceSpan,
    ) {
        if declaration.keyword.as_str() != "on" {
            return;
        }
        let mut seen_action = false;
        for child in children {
            if child.keyword == "set" {
                seen_action = true;
                self.validate_action(child, declaration, span);
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

    fn validate_action(&mut self, action: &ResolvedDeclaration, event: &Decl, span: SourceSpan) {
        let kind = action.kind.as_deref().unwrap_or("");
        if kind == "cursor" {
            if let Some(value) = action.properties.get("value")
                && !matches!(
                    value,
                    ResolvedValue::String(_)
                        | ResolvedValue::Expression(_)
                        | ResolvedValue::Binding(_)
                )
            {
                self.error(
                    "AVENGER-RESOLVE-091",
                    "cursor action requires a UTF-8 scalar expression",
                    span,
                    "cursor is a write-only peer of params and stores",
                );
            }
            return;
        }
        let path = action
            .properties
            .get("target")
            .and_then(resolved_path)
            .unwrap_or_default();
        let scope = self
            .declarations
            .values()
            .find(|info| info.id == action.id)
            .map_or(ScopeId(0), |info| info.containing_scope);
        let expected = match kind {
            "param" => Some(BindingKind::Param),
            "store" => Some(BindingKind::Store),
            "selection" => None,
            _ => return,
        };
        let target = if kind == "selection" {
            self.resolve_any_path(scope, &path, span, true)
        } else {
            self.resolve_binding_path(scope, &path, expected, span, true)
        };
        if kind == "selection" && !matches!(target, Some(ResolvedTarget::Selection(_))) {
            self.error(
                "AVENGER-RESOLVE-092",
                "selection action target has the wrong kind",
                span,
                format!("`{}` is not a selection", path.join(".")),
            );
        }
        if matches!(action.properties.get("at"), Some(ResolvedValue::Atom(at)) if at == "start")
            && event.props.get("between").is_none()
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
                            .map(ParamId::as_str)
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
                            .map(DeclarationId::as_str)
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

    fn error(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        span: SourceSpan,
        label: impl Into<String>,
    ) {
        self.diagnostics.push(Diagnostic::error(
            code,
            message,
            SourceLabel::new(span, label),
        ));
    }

    // The remaining resolution operations are implemented below in focused
    // helpers so validation can accumulate independent diagnostics.
}

fn collect_public_targets(
    declaration: &ResolvedDeclaration,
    output: &mut BTreeMap<String, ResolvedTarget>,
) {
    if let (Some(path), Some(target)) = (&declaration.public_path, &declaration.runtime_target) {
        output.insert(path.clone(), target.clone());
    }
    for (alias, target) in &declaration.exports {
        if let Some(path) = &declaration.public_path {
            output.insert(format!("{path}.{alias}"), target.clone());
        }
    }
    for child in &declaration.children {
        collect_public_targets(child, output);
    }
}

fn declaration_coordinate(declaration: &Decl, inherited: Option<&str>) -> Option<String> {
    match declaration.keyword.as_str() {
        "chart" | "plot" | "view" => declaration.kind.as_ref().map(ToString::to_string),
        _ => inherited.map(str::to_owned),
    }
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
        "chart" => matches!(property, "data" | "title" | "subtitle" | "layout" | "theme"),
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

fn nearest_coordinate_kind(_declaration: &Decl) -> Option<&str> {
    // Coordinate compatibility is also encoded by coordinate-specific keys.
    // The inherited coordinate is retained on `ResolvedDeclaration` and is
    // checked by `native_schema`; this helper remains conservative for the
    // coordinate-independent tool-key form.
    None
}

fn declaration_public_path(declaration: &Decl, parent: Option<&str>) -> Option<String> {
    let name = declaration.name.as_ref()?.as_str();
    if declaration.visibility == Visibility::Private || declaration.keyword.as_str() == "view" {
        return None;
    }
    Some(match parent {
        Some(parent) if declaration.visibility != Visibility::Public => format!("{parent}.{name}"),
        _ => name.to_owned(),
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
                    target: ResolvedTarget::Param(_),
                    ..
                })
            )
        }
        ValueShape::TableBinding => {
            matches!(
                value,
                ResolvedValue::Binding(ResolvedBinding {
                    target: ResolvedTarget::Store(_),
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

fn namespace_matches_target(namespace: NativeKindNamespace, target: &ResolvedTarget) -> bool {
    matches!(
        (namespace, target),
        (NativeKindNamespace::Mark, ResolvedTarget::Mark(_))
            | (NativeKindNamespace::Tool, ResolvedTarget::Tool(_))
            | (NativeKindNamespace::Widget, ResolvedTarget::Widget(_))
    )
}

fn reference_kind_matches(target: &ResolvedTarget, kind: RefKind) -> bool {
    matches!(
        (kind, target),
        (
            RefKind::Mark,
            ResolvedTarget::Mark(_) | ResolvedTarget::Part { .. }
        ) | (RefKind::Group, ResolvedTarget::Declaration(_))
            | (RefKind::Selection, ResolvedTarget::Selection(_))
            | (RefKind::Tool, ResolvedTarget::Tool(_))
            | (RefKind::Widget, ResolvedTarget::Widget(_))
            | (RefKind::Resource, ResolvedTarget::Declaration(_))
    )
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
        }),
        Value::Query(value) => ResolvedValue::Query(ResolvedQuery {
            sql: value.canonical_sql(),
            bindings: Vec::new(),
            helpers: helpers_in_sql(&value.canonical_sql()),
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
        "chart" | "define" | "group" | "view" | "cell" | "plot" | "overlay"
    ) || (matches!(declaration.keyword.as_str(), "tool" | "mark")
        && !declaration.children.is_empty())
}

fn is_instance_boundary(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "chart" | "group" | "tool" | "widget" | "mark" | "view" | "cell" | "plot"
    )
}

fn declaration_id(file: &ProjectFileId, path: &[usize], keyword: &str) -> DeclarationId {
    DeclarationId(semantic_hash(&[
        "declaration",
        file.as_str(),
        &path_text(path),
        keyword,
    ]))
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

fn existing_export_binding(declaration: &Decl, alias: &str) -> Option<Vec<String>> {
    let property = format!("{alias}_param");
    let Value::Binding { path, .. } = declaration.props.get(&property)? else {
        return None;
    };
    Some(path.iter().map(ToString::to_string).collect())
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
        })
        .collect()
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
