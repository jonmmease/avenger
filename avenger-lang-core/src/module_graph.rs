//! Deterministic, I/O-agnostic Avenger source-module graph loading.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use avenger_chart_schema::NativeModuleId;

use crate::{
    Diagnostic, ExpansionOrImportFrame, ImportCapabilities, LoadedSource, SourceFile, SourceId,
    SourceLabel, SourceLoader, SourceLoaderError, SourceMap, SourceOrigin, SourceSpan,
    ast::{Decl, ImportClause},
    syntax::{ParsedFile, SyntaxLimits, parse_file_with_limits},
};

type ModuleGraphResult<T> = Result<T, Box<Diagnostic>>;
type ImportTrace = Vec<(SourceSpan, String)>;
type PendingImportData = (Option<String>, ImportTrace);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceModuleId(String);

impl SourceModuleId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum ModuleId {
    Source(SourceModuleId),
    Native(NativeModuleId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleDependencyRole {
    RequestedModule,
    Import,
    AmbientDataRoot,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ModuleDependencyTarget {
    Source(SourceOrigin),
    Native(NativeModuleId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDependency {
    /// The source origin or native module requested before resolution.
    pub requested: ModuleDependencyTarget,
    /// Loader-canonical origin, once known (for example after an HTTP redirect).
    pub canonical_origin: Option<SourceOrigin>,
    pub role: ModuleDependencyRole,
    pub content_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ModuleRoot {
    pub origin: SourceOrigin,
    pub role: ModuleDependencyRole,
}

impl ModuleRoot {
    pub fn requested(origin: SourceOrigin) -> Self {
        Self {
            origin,
            role: ModuleDependencyRole::RequestedModule,
        }
    }

    pub fn ambient_data(origin: SourceOrigin) -> Self {
        Self {
            origin,
            role: ModuleDependencyRole::AmbientDataRoot,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableNativeModule {
    pub schema_profile: String,
    pub implementation_profile: String,
}

#[derive(Clone, Debug)]
pub struct ModuleGraphLoadRequest {
    pub project_root: PathBuf,
    pub roots: Vec<ModuleRoot>,
    pub native_modules: BTreeMap<NativeModuleId, AvailableNativeModule>,
    pub capabilities: ImportCapabilities,
    pub schema_version: String,
    pub registry_version: String,
    pub limits: ModuleGraphLoadLimits,
}

/// Bounds applied while loading an untrusted module graph and its import closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleGraphLoadLimits {
    pub max_source_bytes: usize,
    pub max_total_source_bytes: usize,
    pub max_sources: usize,
    pub max_import_depth: usize,
    pub max_imports_per_source: usize,
    pub max_project_directory_depth: usize,
    pub max_project_directory_entries: usize,
    pub syntax: SyntaxLimits,
}

impl Default for ModuleGraphLoadLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 2 * 1024 * 1024,
            max_total_source_bytes: 64 * 1024 * 1024,
            max_sources: 1_024,
            max_import_depth: 64,
            max_imports_per_source: 256,
            max_project_directory_depth: 64,
            max_project_directory_entries: 100_000,
            syntax: SyntaxLimits::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModuleImportEdge {
    pub importer: SourceModuleId,
    pub imported: ModuleId,
    pub importer_source: SourceId,
    pub imported_source: Option<SourceId>,
    pub site: SourceSpan,
    pub specifier: String,
    pub clause: ImportClause,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ParsedModule {
    pub id: SourceModuleId,
    pub source: SourceId,
    pub origin: SourceOrigin,
    pub content_version: String,
    pub content_sha256: String,
    pub parsed: ParsedFile,
}

/// One top-level declaration in the deterministic merge of ambient data files.
#[derive(Clone, Debug)]
pub struct AmbientDataItem {
    pub source: SourceId,
    pub declaration: Decl,
}

#[derive(Clone, Debug)]
pub struct ParsedModuleGraph {
    pub sources: SourceMap,
    pub source_modules: BTreeMap<SourceModuleId, ParsedModule>,
    pub native_modules: BTreeMap<NativeModuleId, AvailableNativeModule>,
    pub imports: Vec<ModuleImportEdge>,
    pub requested_modules: Vec<SourceModuleId>,
    pub ambient_data_modules: Vec<SourceModuleId>,
    pub ambient_catalog: Vec<AmbientDataItem>,
    pub fingerprint: String,
}

impl ParsedModuleGraph {
    pub fn source_module(&self, id: &SourceModuleId) -> Option<&ParsedModule> {
        self.source_modules.get(id)
    }
}

#[derive(Clone, Debug)]
pub struct ModuleGraphLoadFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

#[derive(Clone, Debug)]
pub struct ModuleGraphLoadAttempt {
    pub result: Result<ParsedModuleGraph, ModuleGraphLoadFailure>,
    pub dependencies: Vec<ModuleDependency>,
}

pub struct ModuleGraphLoader<'a> {
    loader: &'a dyn SourceLoader,
}

impl<'a> ModuleGraphLoader<'a> {
    pub fn new(loader: &'a dyn SourceLoader) -> Self {
        Self { loader }
    }

    pub async fn load(&self, mut request: ModuleGraphLoadRequest) -> ModuleGraphLoadAttempt {
        request.roots.sort_by(|left, right| {
            left.origin
                .canonical_uri()
                .cmp(&right.origin.canonical_uri())
                .then(left.role.cmp(&right.role))
        });
        request
            .roots
            .dedup_by(|left, right| left.origin == right.origin && left.role == right.role);

        let mut state = LoadState::new(&request);
        for root in &request.roots {
            state.enqueue(root.origin.clone(), root.role, None, None);
        }

        while let Some(pending) = state.queue.pop_front() {
            if let Some(source) = state.origin_to_source.get(&pending.origin).copied() {
                state.record_existing(&pending, source);
                state.pending_edges.push(pending);
                continue;
            }
            if let Err(diagnostic) = self.load_one(&request, &mut state, pending).await {
                return state.failure(*diagnostic);
            }
        }

        if let Err(diagnostic) = state.finish_edges_and_validate() {
            return state.failure(*diagnostic);
        }
        ModuleGraphLoadAttempt {
            dependencies: state.dependencies(),
            result: Ok(state.finish()),
        }
    }

    async fn load_one(
        &self,
        request: &ModuleGraphLoadRequest,
        state: &mut LoadState<'_>,
        pending: PendingSource,
    ) -> ModuleGraphResult<()> {
        let dependency_index = state.record_candidate(&pending);
        if pending.trace.len() > request.limits.max_import_depth {
            return Err(module_diagnostic(
                "AVENGER-MODULE-021",
                "module import depth limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                format!(
                    "import depth is {}; limit is {}",
                    pending.trace.len(),
                    request.limits.max_import_depth
                ),
            ));
        }
        if matches!(pending.origin, SourceOrigin::Http(_)) && pending.sha256.is_none() {
            return Err(module_diagnostic(
                "AVENGER-MODULE-008",
                "HTTP imports require a SHA-256 pin",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                "add `sha256 '<64 lowercase hex characters>'` to this import",
            ));
        }
        let loaded = self
            .loader
            .load(&pending.origin, &request.capabilities)
            .await
            .map_err(|error| loader_diagnostic(&pending, state.fallback_source(), error))?;
        state.record_loaded(dependency_index, &loaded);

        if let Some(expected) = &pending.sha256 {
            validate_sha256(expected, &loaded, pending.site, state.fallback_source())?;
        }

        if state.origin_to_source.contains_key(&loaded.origin) {
            state.alias_requested_origin(pending.origin, loaded.origin.clone());
            state.pending_edges.push(PendingSource {
                origin: loaded.origin,
                ..pending
            });
            return Ok(());
        }

        if loaded.text.len() > request.limits.max_source_bytes {
            return Err(module_diagnostic(
                "AVENGER-MODULE-022",
                "module source size limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                format!(
                    "source is {} bytes; per-source limit is {} bytes",
                    loaded.text.len(),
                    request.limits.max_source_bytes
                ),
            ));
        }
        if state.loaded.len() >= request.limits.max_sources {
            return Err(module_diagnostic(
                "AVENGER-MODULE-023",
                "module graph source-count limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                format!("project source limit is {}", request.limits.max_sources),
            ));
        }
        let Some(total_source_bytes) = state.total_source_bytes.checked_add(loaded.text.len())
        else {
            return Err(module_diagnostic(
                "AVENGER-MODULE-024",
                "module graph total source-size limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                "project source byte count overflowed".to_string(),
            ));
        };
        if total_source_bytes > request.limits.max_total_source_bytes {
            return Err(module_diagnostic(
                "AVENGER-MODULE-024",
                "module graph total source-size limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                format!(
                    "project sources total {total_source_bytes} bytes; limit is {} bytes",
                    request.limits.max_total_source_bytes
                ),
            ));
        }
        state.total_source_bytes = total_source_bytes;

        let source_id = SourceId::new(state.next_source_id);
        state.next_source_id += 1;
        let source = SourceFile::new(source_id, loaded.origin.clone(), loaded.text.clone());
        state
            .sources
            .insert(source.clone())
            .expect("fresh source id");
        let parsed = parse_file_with_limits(&source, request.limits.syntax).map_err(|error| {
            let mut diagnostic = error.into_diagnostic();
            add_trace(&mut diagnostic, &pending.trace);
            Box::new(diagnostic)
        })?;
        validate_source_module_origin(&loaded.origin, source_id)?;
        let module_id = source_module_id(&request.project_root, &loaded.origin)?;
        let content_sha256 = sha256_hex(loaded.text.as_bytes());
        state
            .origin_to_source
            .insert(loaded.origin.clone(), source_id);
        state
            .origin_to_source
            .insert(pending.origin.clone(), source_id);
        state.source_to_module.insert(source_id, module_id.clone());
        state.loaded.insert(
            source_id,
            ParsedModule {
                id: module_id,
                source: source_id,
                origin: loaded.origin.clone(),
                content_version: loaded.version.as_str().to_owned(),
                content_sha256,
                parsed,
            },
        );
        state.pending_edges.push(PendingSource {
            origin: loaded.origin.clone(),
            ..pending.clone()
        });

        let import_spans = state.loaded[&source_id]
            .parsed
            .module_syntax
            .imports
            .iter()
            .map(|import| import.span)
            .collect::<Vec<_>>();
        let imports = state.loaded[&source_id]
            .parsed
            .ast
            .imports
            .clone()
            .into_iter()
            .zip(import_spans);
        let import_count = state.loaded[&source_id].parsed.ast.imports.len();
        if import_count > request.limits.max_imports_per_source {
            return Err(module_diagnostic(
                "AVENGER-MODULE-025",
                "per-source import-count limit exceeded",
                SourceSpan::empty(source_id, 0),
                format!(
                    "source has {import_count} imports; limit is {}",
                    request.limits.max_imports_per_source
                ),
            ));
        }
        for (import, import_site) in imports {
            let target =
                resolve_import_target(&loaded.origin, &import.source, &request.project_root)
                    .map_err(|message| {
                        module_diagnostic(
                            "AVENGER-MODULE-006",
                            "invalid import origin",
                            import_site,
                            message,
                        )
                    })?;
            let mut trace = pending.trace.clone();
            trace.push((import_site, import.source.clone()));
            let context = ImportContext {
                importer: source_id,
                site: import_site,
                specifier: import.source.clone(),
                clause: import.clause,
            };
            match target {
                ResolvedImportTarget::Source(target) => state.enqueue(
                    target,
                    ModuleDependencyRole::Import,
                    Some(context),
                    Some((import.sha256, trace)),
                ),
                ResolvedImportTarget::Native(module) => {
                    if import.sha256.is_some() {
                        return Err(module_diagnostic(
                            "AVENGER-MODULE-009",
                            "native module imports do not use source-byte hashes",
                            import_site,
                            "native compatibility is pinned by schema and implementation profiles",
                        ));
                    }
                    let Some(profiles) = request.native_modules.get(&module).cloned() else {
                        return Err(module_diagnostic(
                            "AVENGER-MODULE-016",
                            "native module is unavailable",
                            import_site,
                            format!("the host did not register `{module}`"),
                        ));
                    };
                    state.record_native_import(context, module, profiles);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ImportContext {
    importer: SourceId,
    site: SourceSpan,
    specifier: String,
    clause: ImportClause,
}

#[derive(Clone)]
struct PendingSource {
    origin: SourceOrigin,
    role: ModuleDependencyRole,
    import: Option<ImportContext>,
    sha256: Option<String>,
    site: Option<SourceSpan>,
    trace: ImportTrace,
}

struct LoadState<'a> {
    request: &'a ModuleGraphLoadRequest,
    queue: VecDeque<PendingSource>,
    pending_edges: Vec<PendingSource>,
    raw_dependencies: Vec<ModuleDependency>,
    sources: SourceMap,
    loaded: BTreeMap<SourceId, ParsedModule>,
    origin_to_source: BTreeMap<SourceOrigin, SourceId>,
    source_to_module: BTreeMap<SourceId, SourceModuleId>,
    native_imports: Vec<(ImportContext, NativeModuleId, AvailableNativeModule)>,
    finalized_edges: Vec<ModuleImportEdge>,
    referenced_native_modules: BTreeMap<NativeModuleId, AvailableNativeModule>,
    next_source_id: u32,
    total_source_bytes: usize,
}

impl<'a> LoadState<'a> {
    fn new(request: &'a ModuleGraphLoadRequest) -> Self {
        Self {
            request,
            queue: VecDeque::new(),
            pending_edges: Vec::new(),
            raw_dependencies: Vec::new(),
            sources: SourceMap::default(),
            loaded: BTreeMap::new(),
            origin_to_source: BTreeMap::new(),
            source_to_module: BTreeMap::new(),
            native_imports: Vec::new(),
            finalized_edges: Vec::new(),
            referenced_native_modules: BTreeMap::new(),
            next_source_id: 0,
            total_source_bytes: 0,
        }
    }

    fn enqueue(
        &mut self,
        origin: SourceOrigin,
        role: ModuleDependencyRole,
        import: Option<ImportContext>,
        import_data: Option<PendingImportData>,
    ) {
        let (sha256, trace) = import_data.unwrap_or_default();
        let site = import.as_ref().map(|context| context.site);
        self.queue.push_back(PendingSource {
            origin,
            role,
            import,
            sha256,
            site,
            trace,
        });
    }

    fn record_candidate(&mut self, pending: &PendingSource) -> usize {
        self.raw_dependencies.push(ModuleDependency {
            requested: ModuleDependencyTarget::Source(pending.origin.clone()),
            canonical_origin: None,
            role: pending.role,
            content_version: None,
        });
        self.raw_dependencies.len() - 1
    }

    fn record_loaded(&mut self, index: usize, loaded: &LoadedSource) {
        let dependency = &mut self.raw_dependencies[index];
        dependency.canonical_origin = Some(loaded.origin.clone());
        dependency.content_version = Some(loaded.version.as_str().to_owned());
    }

    fn record_existing(&mut self, pending: &PendingSource, source: SourceId) {
        let module = &self.loaded[&source];
        self.raw_dependencies.push(ModuleDependency {
            requested: ModuleDependencyTarget::Source(pending.origin.clone()),
            canonical_origin: Some(module.origin.clone()),
            role: pending.role,
            content_version: Some(module.content_version.clone()),
        });
    }

    fn record_native_import(
        &mut self,
        import: ImportContext,
        module: NativeModuleId,
        profiles: AvailableNativeModule,
    ) {
        self.raw_dependencies.push(ModuleDependency {
            requested: ModuleDependencyTarget::Native(module.clone()),
            canonical_origin: None,
            role: ModuleDependencyRole::Import,
            content_version: Some(format!(
                "{}:{}",
                profiles.schema_profile, profiles.implementation_profile
            )),
        });
        self.referenced_native_modules
            .insert(module.clone(), profiles.clone());
        self.native_imports.push((import, module, profiles));
    }

    fn alias_requested_origin(&mut self, requested: SourceOrigin, canonical: SourceOrigin) {
        if let Some(source) = self.origin_to_source.get(&canonical).copied() {
            self.origin_to_source.insert(requested, source);
        }
    }

    fn fallback_source(&self) -> SourceId {
        SourceId::new(self.next_source_id.saturating_sub(1))
    }

    fn dependencies(&self) -> Vec<ModuleDependency> {
        let mut dependencies = self.raw_dependencies.clone();
        dependencies.sort_by(|left, right| {
            left.requested
                .cmp(&right.requested)
                .then(left.role.cmp(&right.role))
        });
        dependencies.dedup_by(|left, right| {
            if left.requested == right.requested && left.role == right.role {
                if left.canonical_origin.is_none() {
                    left.canonical_origin = right.canonical_origin.clone();
                }
                if left.content_version.is_none() {
                    left.content_version = right.content_version.clone();
                }
                true
            } else {
                false
            }
        });
        dependencies
    }

    fn finish_edges_and_validate(&mut self) -> ModuleGraphResult<()> {
        let mut edges = Vec::new();
        for pending in &self.pending_edges {
            let Some(import) = &pending.import else {
                continue;
            };
            let Some(&imported_source) = self.origin_to_source.get(&pending.origin) else {
                continue;
            };
            edges.push(ModuleImportEdge {
                importer: self.source_to_module[&import.importer].clone(),
                imported: ModuleId::Source(self.source_to_module[&imported_source].clone()),
                importer_source: import.importer,
                imported_source: Some(imported_source),
                site: import.site,
                specifier: import.specifier.clone(),
                clause: import.clause.clone(),
                sha256: pending.sha256.clone(),
            });
        }
        for (import, module, _) in &self.native_imports {
            edges.push(ModuleImportEdge {
                importer: self.source_to_module[&import.importer].clone(),
                imported: ModuleId::Native(module.clone()),
                importer_source: import.importer,
                imported_source: None,
                site: import.site,
                specifier: import.specifier.clone(),
                clause: import.clause.clone(),
                sha256: None,
            });
        }
        edges.sort_by(|left, right| {
            left.importer
                .cmp(&right.importer)
                .then(left.imported.cmp(&right.imported))
                .then(left.site.cmp(&right.site))
        });
        detect_cycles(&edges, &self.loaded)?;
        validate_ambient_roots(&self.request.roots, &self.origin_to_source, &self.loaded)?;
        validate_ambient_catalogs(&self.request.roots, &self.origin_to_source, &self.loaded)?;
        self.finalized_edges = edges;
        Ok(())
    }

    fn finish(self) -> ParsedModuleGraph {
        let edges = self.finalized_edges.clone();
        let mut source_modules = BTreeMap::new();
        for module in self.loaded.values() {
            source_modules.insert(module.id.clone(), module.clone());
        }
        let requested_modules = root_ids(
            &self.request.roots,
            ModuleDependencyRole::RequestedModule,
            &self.origin_to_source,
            &self.source_to_module,
        );
        let ambient_data_modules = root_ids(
            &self.request.roots,
            ModuleDependencyRole::AmbientDataRoot,
            &self.origin_to_source,
            &self.source_to_module,
        );
        let ambient_catalog = merge_ambient_catalog(&ambient_data_modules, &source_modules);
        let fingerprint = module_graph_fingerprint(
            &source_modules,
            &self.referenced_native_modules,
            &edges,
            &self.request.schema_version,
            &self.request.registry_version,
        );
        ParsedModuleGraph {
            sources: self.sources,
            source_modules,
            native_modules: self.referenced_native_modules,
            imports: edges,
            requested_modules,
            ambient_data_modules,
            ambient_catalog,
            fingerprint,
        }
    }

    fn failure(&self, diagnostic: Diagnostic) -> ModuleGraphLoadAttempt {
        ModuleGraphLoadAttempt {
            result: Err(ModuleGraphLoadFailure {
                diagnostics: vec![diagnostic],
                sources: self.sources.clone(),
            }),
            dependencies: self.dependencies(),
        }
    }
}

fn validate_source_module_origin(origin: &SourceOrigin, source: SourceId) -> ModuleGraphResult<()> {
    let extension_required = matches!(origin, SourceOrigin::File(_) | SourceOrigin::Http(_));
    if extension_required && !origin_path(origin).ends_with(".avenger") {
        return Err(module_diagnostic(
            "AVENGER-MODULE-001",
            "unrecognized Avenger source-module extension",
            SourceSpan::empty(source, 0),
            "local and HTTP source modules must end in `.avenger`",
        ));
    }
    Ok(())
}

fn detect_cycles(
    edges: &[ModuleImportEdge],
    modules: &BTreeMap<SourceId, ParsedModule>,
) -> ModuleGraphResult<()> {
    let mut adjacency: BTreeMap<SourceId, Vec<(SourceId, SourceSpan)>> = BTreeMap::new();
    for edge in edges {
        if let Some(imported) = edge.imported_source {
            adjacency
                .entry(edge.importer_source)
                .or_default()
                .push((imported, edge.site));
        }
    }
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    for source in modules.keys().copied() {
        if let Some(cycle) = visit_cycle(source, &adjacency, &mut visiting, &mut visited) {
            let site = cycle
                .iter()
                .rev()
                .nth(1)
                .map(|(_, site)| *site)
                .unwrap_or_else(|| SourceSpan::empty(source, 0));
            let names = cycle
                .iter()
                .map(|(id, _)| modules[id].origin.display_name())
                .collect::<Vec<_>>()
                .join(" -> ");
            let mut diagnostic =
                module_diagnostic("AVENGER-MODULE-013", "import cycle detected", site, names);
            for edge in cycle.windows(2) {
                let (_, edge_site) = edge[0];
                let (target, _) = edge[1];
                diagnostic.trace.push(ExpansionOrImportFrame {
                    span: edge_site,
                    message: format!("imports {}", modules[&target].origin.display_name()),
                });
            }
            return Err(diagnostic);
        }
    }
    Ok(())
}

fn visit_cycle(
    source: SourceId,
    adjacency: &BTreeMap<SourceId, Vec<(SourceId, SourceSpan)>>,
    visiting: &mut Vec<(SourceId, SourceSpan)>,
    visited: &mut BTreeSet<SourceId>,
) -> Option<Vec<(SourceId, SourceSpan)>> {
    if let Some(position) = visiting
        .iter()
        .position(|(candidate, _)| *candidate == source)
    {
        let mut cycle = visiting[position..].to_vec();
        cycle.push((
            source,
            cycle.last().map_or(SourceSpan::empty(source, 0), |x| x.1),
        ));
        return Some(cycle);
    }
    if !visited.insert(source) {
        return None;
    }
    visiting.push((source, SourceSpan::empty(source, 0)));
    for &(target, site) in adjacency.get(&source).into_iter().flatten() {
        if let Some(last) = visiting.last_mut() {
            last.1 = site;
        }
        if let Some(cycle) = visit_cycle(target, adjacency, visiting, visited) {
            return Some(cycle);
        }
    }
    visiting.pop();
    None
}

fn validate_ambient_roots(
    roots: &[ModuleRoot],
    origins: &BTreeMap<SourceOrigin, SourceId>,
    modules: &BTreeMap<SourceId, ParsedModule>,
) -> ModuleGraphResult<()> {
    for root in roots
        .iter()
        .filter(|root| root.role == ModuleDependencyRole::AmbientDataRoot)
    {
        let Some(source) = origins.get(&root.origin).copied() else {
            continue;
        };
        let invalid = modules[&source].parsed.ast.items.iter().find(|item| {
            !matches!(
                item.declaration.keyword.as_str(),
                "table" | "schema" | "catalog"
            )
        });
        if let Some(item) = invalid {
            return Err(module_diagnostic(
                "AVENGER-MODULE-002",
                "ambient data root contains a non-data module item",
                SourceSpan::empty(source, 0),
                format!(
                    "`{}` items are not permitted in an ambient data root",
                    item.declaration.keyword
                ),
            ));
        }
    }
    Ok(())
}

fn validate_ambient_catalogs(
    roots: &[ModuleRoot],
    origins: &BTreeMap<SourceOrigin, SourceId>,
    modules: &BTreeMap<SourceId, ParsedModule>,
) -> ModuleGraphResult<()> {
    let mut paths: BTreeMap<Vec<String>, SourceId> = BTreeMap::new();
    for root in roots
        .iter()
        .filter(|root| root.role == ModuleDependencyRole::AmbientDataRoot)
    {
        let Some(source) = origins.get(&root.origin).copied() else {
            continue;
        };
        for declaration in modules[&source]
            .parsed
            .ast
            .items
            .iter()
            .map(|item| &item.declaration)
            .filter(|declaration| {
                matches!(declaration.keyword.as_str(), "table" | "schema" | "catalog")
            })
        {
            collect_catalog_paths(declaration, &[], source, &mut paths)?;
        }
    }
    Ok(())
}

fn collect_catalog_paths(
    declaration: &Decl,
    parent: &[String],
    source: SourceId,
    paths: &mut BTreeMap<Vec<String>, SourceId>,
) -> ModuleGraphResult<()> {
    if !matches!(declaration.keyword.as_str(), "catalog" | "schema" | "table") {
        return Ok(());
    }
    let Some(name) = &declaration.name else {
        return Ok(());
    };
    let mut path = parent.to_vec();
    path.push(name.to_string());
    if let Some(previous) = paths.insert(path.clone(), source) {
        return Err(module_diagnostic(
            "AVENGER-MODULE-014",
            "duplicate ambient catalog path",
            SourceSpan::empty(source, 0),
            format!(
                "`{}` was already declared in source {}",
                path.join("."),
                previous
            ),
        ));
    }
    for child in &declaration.children {
        collect_catalog_paths(child, &path, source, paths)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedImportTarget {
    Source(SourceOrigin),
    Native(NativeModuleId),
}

pub fn resolve_import_target(
    importer: &SourceOrigin,
    specifier: &str,
    project_root: &Path,
) -> Result<ResolvedImportTarget, String> {
    if specifier.starts_with("native:") {
        return NativeModuleId::new(specifier)
            .map(ResolvedImportTarget::Native)
            .map_err(|error| error.to_string());
    }
    if let Some(path) = specifier.strip_prefix("std:") {
        return Ok(ResolvedImportTarget::Source(SourceOrigin::Std(
            normalize_virtual_path(path)?,
        )));
    }
    if specifier.starts_with("http://") || specifier.starts_with("https://") {
        let url = Url::parse(specifier).map_err(|error| error.to_string())?;
        return Ok(ResolvedImportTarget::Source(SourceOrigin::Http(
            url.to_string(),
        )));
    }
    let source = match importer {
        SourceOrigin::File(path) => {
            let parent = path.parent().unwrap_or(project_root);
            let candidate = normalize_path(&parent.join(specifier));
            SourceOrigin::File(candidate)
        }
        SourceOrigin::Http(base) => {
            let url = Url::parse(base)
                .and_then(|base| base.join(specifier))
                .map_err(|error| error.to_string())?;
            SourceOrigin::Http(url.to_string())
        }
        SourceOrigin::Std(path) => {
            let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            SourceOrigin::Std(normalize_virtual_path(
                &parent.join(specifier).to_string_lossy(),
            )?)
        }
        SourceOrigin::Memory(path) => {
            let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            SourceOrigin::Memory(normalize_virtual_path(
                &parent.join(specifier).to_string_lossy(),
            )?)
        }
    };
    Ok(ResolvedImportTarget::Source(source))
}

/// Resolve a source-relative file, standard-library, or HTTP resource.
///
/// Unlike a module import, an external resource cannot name a `native:`
/// module because native modules have no source bytes to load.
pub fn resolve_relative_origin(
    declaring_origin: &SourceOrigin,
    specifier: &str,
    project_root: &Path,
) -> Result<SourceOrigin, String> {
    match resolve_import_target(declaring_origin, specifier, project_root)? {
        ResolvedImportTarget::Source(origin) => Ok(origin),
        ResolvedImportTarget::Native(module) => Err(format!(
            "native module `{module}` cannot be used as a source resource"
        )),
    }
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalize_virtual_path(value: &str) -> Result<String, String> {
    let mut segments = Vec::new();
    for segment in value.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(format!(
                        "virtual import path `{value}` escapes its origin root"
                    ));
                }
            }
            segment => segments.push(segment),
        }
    }
    Ok(segments.join("/"))
}

fn source_module_id(
    project_root: &Path,
    origin: &SourceOrigin,
) -> ModuleGraphResult<SourceModuleId> {
    let value = match origin {
        SourceOrigin::File(path) => normalize_path(path)
            .strip_prefix(normalize_path(project_root))
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .map_err(|_| {
                module_diagnostic(
                    "AVENGER-MODULE-007",
                    "source module is outside the project root",
                    SourceSpan::empty(SourceId::new(0), 0),
                    path.display().to_string(),
                )
            })?,
        _ => origin.canonical_uri(),
    };
    Ok(SourceModuleId::new(value))
}

fn origin_path(origin: &SourceOrigin) -> String {
    match origin {
        SourceOrigin::Memory(path) | SourceOrigin::Std(path) => path.clone(),
        SourceOrigin::File(path) => path.to_string_lossy().into_owned(),
        SourceOrigin::Http(url) => Url::parse(url)
            .ok()
            .map(|url| url.path().to_owned())
            .unwrap_or_else(|| url.clone()),
    }
}

fn validate_sha256(
    expected: &str,
    loaded: &LoadedSource,
    site: Option<SourceSpan>,
    fallback: SourceId,
) -> ModuleGraphResult<()> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(module_diagnostic(
            "AVENGER-MODULE-009",
            "invalid SHA-256 pin",
            site.unwrap_or_else(|| SourceSpan::empty(fallback, 0)),
            "expected exactly 64 lowercase hexadecimal characters",
        ));
    }
    let actual = sha256_hex(loaded.text.as_bytes());
    if actual != expected {
        return Err(module_diagnostic(
            "AVENGER-MODULE-009",
            "import SHA-256 mismatch",
            site.unwrap_or_else(|| SourceSpan::empty(fallback, 0)),
            format!("expected {expected}, received {actual}"),
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn module_graph_fingerprint(
    source_modules: &BTreeMap<SourceModuleId, ParsedModule>,
    native_modules: &BTreeMap<NativeModuleId, AvailableNativeModule>,
    edges: &[ModuleImportEdge],
    schema_version: &str,
    registry_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-module-graph-v1\0");
    hasher.update(schema_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(registry_version.as_bytes());
    for (id, module) in source_modules {
        hasher.update(b"\0source\0");
        hasher.update(id.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(module.origin.canonical_uri().as_bytes());
        hasher.update(b"\0");
        hasher.update(module.content_sha256.as_bytes());
    }
    for (id, profiles) in native_modules {
        hasher.update(b"\0native\0");
        hasher.update(id.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(profiles.schema_profile.as_bytes());
        hasher.update(b"\0");
        hasher.update(profiles.implementation_profile.as_bytes());
    }
    for edge in edges {
        hasher.update(b"\0edge\0");
        hasher.update(edge.importer.as_str().as_bytes());
        hasher.update(b"\0");
        match &edge.imported {
            ModuleId::Source(id) => {
                hasher.update(b"source\0");
                hasher.update(id.as_str().as_bytes());
            }
            ModuleId::Native(id) => {
                hasher.update(b"native\0");
                hasher.update(id.as_str().as_bytes());
            }
        }
        hasher.update(b"\0");
        hasher.update(edge.specifier.as_bytes());
        hasher.update(b"\0");
        hasher.update(
            serde_json::to_vec(&edge.clause)
                .expect("import clauses serialize")
                .as_slice(),
        );
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn root_ids(
    roots: &[ModuleRoot],
    role: ModuleDependencyRole,
    origins: &BTreeMap<SourceOrigin, SourceId>,
    ids: &BTreeMap<SourceId, SourceModuleId>,
) -> Vec<SourceModuleId> {
    let mut result = roots
        .iter()
        .filter(|root| root.role == role)
        .filter_map(|root| origins.get(&root.origin))
        .filter_map(|source| ids.get(source))
        .cloned()
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

fn merge_ambient_catalog(
    ambient_data: &[SourceModuleId],
    modules: &BTreeMap<SourceModuleId, ParsedModule>,
) -> Vec<AmbientDataItem> {
    let mut declarations = Vec::new();
    for id in ambient_data {
        let module = &modules[id];
        declarations.extend(
            module
                .parsed
                .ast
                .items
                .iter()
                .map(|item| &item.declaration)
                .filter(|declaration| {
                    matches!(declaration.keyword.as_str(), "table" | "schema" | "catalog")
                })
                .cloned()
                .map(|declaration| AmbientDataItem {
                    source: module.source,
                    declaration,
                }),
        );
    }
    declarations
}

fn loader_diagnostic(
    pending: &PendingSource,
    fallback: SourceId,
    error: SourceLoaderError,
) -> Box<Diagnostic> {
    let mut diagnostic = module_diagnostic(
        "AVENGER-MODULE-015",
        "source could not be loaded",
        pending
            .site
            .unwrap_or_else(|| SourceSpan::empty(fallback, 0)),
        error.to_string(),
    );
    add_trace(&mut diagnostic, &pending.trace);
    diagnostic
}

fn add_trace(diagnostic: &mut Diagnostic, trace: &[(SourceSpan, String)]) {
    diagnostic
        .trace
        .extend(trace.iter().map(|(span, source)| ExpansionOrImportFrame {
            span: *span,
            message: format!("imports {source}"),
        }));
}

fn module_diagnostic(
    code: &str,
    message: impl Into<String>,
    span: SourceSpan,
    label: impl Into<String>,
) -> Box<Diagnostic> {
    Box::new(Diagnostic::error(
        code,
        message,
        SourceLabel::new(span, label),
    ))
}
