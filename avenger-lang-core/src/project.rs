//! Deterministic, I/O-agnostic project loading and import closure.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    Diagnostic, ExpansionOrImportFrame, ImportCapabilities, LoadedSource, SourceFile, SourceId,
    SourceLabel, SourceLoader, SourceLoaderError, SourceMap, SourceOrigin, SourceSpan,
    ast::{Decl, Name, Root, is_name},
    syntax::{ParsedFile, parse_file},
};

type ProjectResult<T> = Result<T, Box<Diagnostic>>;
type ImportTrace = Vec<(SourceSpan, String)>;
type PendingImportData = (Option<String>, ImportTrace);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectFileId(String);

impl ProjectFileId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Mark,
    Tool,
    Transform,
}

impl DefinitionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Tool => "tool",
            Self::Transform => "transform",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "definition_kind")]
pub enum ProjectFileKind {
    Chart,
    Definition(DefinitionKind),
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectDependencyRole {
    RootChart,
    Import,
    DataConfiguration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDependency {
    /// The origin attempted before a potentially fallible loader operation.
    pub requested_origin: SourceOrigin,
    /// Loader-canonical origin, once known (for example after an HTTP redirect).
    pub canonical_origin: Option<SourceOrigin>,
    pub role: ProjectDependencyRole,
    pub content_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectRoot {
    pub origin: SourceOrigin,
    pub role: ProjectDependencyRole,
}

impl ProjectRoot {
    pub fn chart(origin: SourceOrigin) -> Self {
        Self {
            origin,
            role: ProjectDependencyRole::RootChart,
        }
    }

    pub fn data(origin: SourceOrigin) -> Self {
        Self {
            origin,
            role: ProjectDependencyRole::DataConfiguration,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProjectLoadRequest {
    pub project_root: PathBuf,
    pub roots: Vec<ProjectRoot>,
    pub capabilities: ImportCapabilities,
    pub schema_version: String,
    pub registry_version: String,
    pub limits: ProjectLoadLimits,
}

/// Bounds applied while loading an untrusted project and its import closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectLoadLimits {
    pub max_source_bytes: usize,
    pub max_total_source_bytes: usize,
    pub max_sources: usize,
    pub max_import_depth: usize,
    pub max_imports_per_source: usize,
    pub max_project_directory_depth: usize,
    pub max_project_directory_entries: usize,
}

impl Default for ProjectLoadLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 2 * 1024 * 1024,
            max_total_source_bytes: 64 * 1024 * 1024,
            max_sources: 1_024,
            max_import_depth: 64,
            max_imports_per_source: 256,
            max_project_directory_depth: 64,
            max_project_directory_entries: 100_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportEdge {
    pub importer: SourceId,
    pub imported: SourceId,
    pub site: SourceSpan,
    pub binding: String,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectFile {
    pub id: ProjectFileId,
    pub source: SourceId,
    pub origin: SourceOrigin,
    pub kind: ProjectFileKind,
    pub content_version: String,
    pub content_sha256: String,
    pub parsed: ParsedFile,
}

/// One top-level declaration in the deterministic merge of ambient data files.
#[derive(Clone, Debug)]
pub struct AmbientDataDeclaration {
    pub source: SourceId,
    pub declaration: Decl,
}

#[derive(Clone, Debug)]
pub struct ParsedProject {
    pub sources: SourceMap,
    pub files: BTreeMap<ProjectFileId, ProjectFile>,
    pub imports: Vec<ImportEdge>,
    pub chart_roots: Vec<ProjectFileId>,
    pub ambient_data: Vec<ProjectFileId>,
    pub ambient_catalog: Vec<AmbientDataDeclaration>,
    pub fingerprint: String,
}

impl ParsedProject {
    pub fn file(&self, id: &ProjectFileId) -> Option<&ProjectFile> {
        self.files.get(id)
    }
}

#[derive(Clone, Debug)]
pub struct ProjectLoadFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

#[derive(Clone, Debug)]
pub struct ProjectLoadAttempt {
    pub result: Result<ParsedProject, ProjectLoadFailure>,
    pub dependencies: Vec<ProjectDependency>,
}

pub struct ProjectLoader<'a> {
    loader: &'a dyn SourceLoader,
}

impl<'a> ProjectLoader<'a> {
    pub fn new(loader: &'a dyn SourceLoader) -> Self {
        Self { loader }
    }

    pub async fn load(&self, mut request: ProjectLoadRequest) -> ProjectLoadAttempt {
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
        ProjectLoadAttempt {
            dependencies: state.dependencies(),
            result: Ok(state.finish()),
        }
    }

    async fn load_one(
        &self,
        request: &ProjectLoadRequest,
        state: &mut LoadState<'_>,
        pending: PendingSource,
    ) -> ProjectResult<()> {
        let dependency_index = state.record_candidate(&pending);
        if pending.trace.len() > request.limits.max_import_depth {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-021",
                "project import depth limit exceeded",
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
            return Err(project_diagnostic(
                "AVENGER-PROJECT-008",
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
            return Err(project_diagnostic(
                "AVENGER-PROJECT-022",
                "project source size limit exceeded",
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
            return Err(project_diagnostic(
                "AVENGER-PROJECT-023",
                "project source-count limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                format!("project source limit is {}", request.limits.max_sources),
            ));
        }
        let Some(total_source_bytes) = state.total_source_bytes.checked_add(loaded.text.len())
        else {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-024",
                "project total source-size limit exceeded",
                pending
                    .site
                    .unwrap_or_else(|| SourceSpan::empty(state.fallback_source(), 0)),
                "project source byte count overflowed".to_string(),
            ));
        };
        if total_source_bytes > request.limits.max_total_source_bytes {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-024",
                "project total source-size limit exceeded",
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
        let mut parsed = parse_file(&source).map_err(|error| {
            let mut diagnostic = error.into_diagnostic();
            add_trace(&mut diagnostic, &pending.trace);
            Box::new(diagnostic)
        })?;
        let kind = classify_and_validate(&loaded.origin, &mut parsed, source_id)?;
        let file_id = project_file_id(&request.project_root, &loaded.origin)?;
        let content_sha256 = sha256_hex(loaded.text.as_bytes());
        state
            .origin_to_source
            .insert(loaded.origin.clone(), source_id);
        state
            .origin_to_source
            .insert(pending.origin.clone(), source_id);
        state.source_to_file.insert(source_id, file_id.clone());
        state.loaded.insert(
            source_id,
            ProjectFile {
                id: file_id,
                source: source_id,
                origin: loaded.origin.clone(),
                kind,
                content_version: loaded.version.as_str().to_owned(),
                content_sha256,
                parsed,
            },
        );
        state.pending_edges.push(PendingSource {
            origin: loaded.origin.clone(),
            ..pending.clone()
        });

        let imports = state.loaded[&source_id]
            .parsed
            .ast
            .imports
            .clone()
            .into_iter()
            .zip(state.loaded[&source_id].parsed.import_spans.clone());
        let import_count = state.loaded[&source_id].parsed.ast.imports.len();
        if import_count > request.limits.max_imports_per_source {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-025",
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
                resolve_import_origin(&loaded.origin, &import.source, &request.project_root)
                    .map_err(|message| {
                        project_diagnostic(
                            "AVENGER-PROJECT-006",
                            "invalid import origin",
                            import_site,
                            message,
                        )
                    })?;
            let explicit_alias = import.alias.is_some();
            let binding = import
                .alias
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| inferred_import_name(&target));
            let mut trace = pending.trace.clone();
            trace.push((import_site, import.source.clone()));
            state.enqueue(
                target,
                ProjectDependencyRole::Import,
                Some(ImportContext {
                    importer: source_id,
                    site: import_site,
                    binding,
                    explicit_alias,
                }),
                Some((import.sha256, trace)),
            );
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ImportContext {
    importer: SourceId,
    site: SourceSpan,
    binding: Option<String>,
    explicit_alias: bool,
}

#[derive(Clone)]
struct PendingSource {
    origin: SourceOrigin,
    role: ProjectDependencyRole,
    import: Option<ImportContext>,
    sha256: Option<String>,
    site: Option<SourceSpan>,
    trace: ImportTrace,
}

struct LoadState<'a> {
    request: &'a ProjectLoadRequest,
    queue: VecDeque<PendingSource>,
    pending_edges: Vec<PendingSource>,
    raw_dependencies: Vec<ProjectDependency>,
    sources: SourceMap,
    loaded: BTreeMap<SourceId, ProjectFile>,
    origin_to_source: BTreeMap<SourceOrigin, SourceId>,
    source_to_file: BTreeMap<SourceId, ProjectFileId>,
    finalized_edges: Vec<ImportEdge>,
    next_source_id: u32,
    total_source_bytes: usize,
}

impl<'a> LoadState<'a> {
    fn new(request: &'a ProjectLoadRequest) -> Self {
        Self {
            request,
            queue: VecDeque::new(),
            pending_edges: Vec::new(),
            raw_dependencies: Vec::new(),
            sources: SourceMap::default(),
            loaded: BTreeMap::new(),
            origin_to_source: BTreeMap::new(),
            source_to_file: BTreeMap::new(),
            finalized_edges: Vec::new(),
            next_source_id: 0,
            total_source_bytes: 0,
        }
    }

    fn enqueue(
        &mut self,
        origin: SourceOrigin,
        role: ProjectDependencyRole,
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
        self.raw_dependencies.push(ProjectDependency {
            requested_origin: pending.origin.clone(),
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
        let file = &self.loaded[&source];
        self.raw_dependencies.push(ProjectDependency {
            requested_origin: pending.origin.clone(),
            canonical_origin: Some(file.origin.clone()),
            role: pending.role,
            content_version: Some(file.content_version.clone()),
        });
    }

    fn alias_requested_origin(&mut self, requested: SourceOrigin, canonical: SourceOrigin) {
        if let Some(source) = self.origin_to_source.get(&canonical).copied() {
            self.origin_to_source.insert(requested, source);
        }
    }

    fn fallback_source(&self) -> SourceId {
        SourceId::new(self.next_source_id.saturating_sub(1))
    }

    fn dependencies(&self) -> Vec<ProjectDependency> {
        let mut dependencies = self.raw_dependencies.clone();
        dependencies.sort_by(|left, right| {
            left.requested_origin
                .canonical_uri()
                .cmp(&right.requested_origin.canonical_uri())
                .then(left.role.cmp(&right.role))
        });
        dependencies.dedup_by(|left, right| {
            if left.requested_origin == right.requested_origin && left.role == right.role {
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

    fn finish_edges_and_validate(&mut self) -> ProjectResult<()> {
        let mut edges = Vec::new();
        let mut bindings: BTreeMap<SourceId, BTreeSet<String>> = BTreeMap::new();
        for pending in &self.pending_edges {
            let Some(import) = &pending.import else {
                continue;
            };
            let Some(&imported) = self.origin_to_source.get(&pending.origin) else {
                continue;
            };
            validate_import_matrix(
                self.loaded[&import.importer].kind,
                self.loaded[&imported].kind,
                import.site,
            )?;
            let imported_file = &self.loaded[&imported];
            let binding = if imported_file.kind == ProjectFileKind::Data {
                let root_binding = imported_data_binding(imported_file, import.site)?;
                if import.explicit_alias {
                    import.binding.clone().ok_or_else(|| {
                        project_diagnostic(
                            "AVENGER-PROJECT-010",
                            "import requires an alias",
                            import.site,
                            "expected an explicit alias",
                        )
                    })?
                } else {
                    root_binding
                }
            } else {
                import.binding.clone().ok_or_else(|| {
                    project_diagnostic(
                        "AVENGER-PROJECT-010",
                        "import requires an alias",
                        import.site,
                        "the imported file name is not a valid Avenger name; add `as <name>`",
                    )
                })?
            };
            if !bindings
                .entry(import.importer)
                .or_default()
                .insert(binding.clone())
            {
                return Err(project_diagnostic(
                    "AVENGER-PROJECT-011",
                    "duplicate import binding",
                    import.site,
                    format!("`{binding}` is already bound by another import in this file"),
                ));
            }
            edges.push(ImportEdge {
                importer: import.importer,
                imported,
                site: import.site,
                binding,
                sha256: pending.sha256.clone(),
            });
        }
        edges.sort_by(|left, right| {
            left.importer
                .cmp(&right.importer)
                .then(left.imported.cmp(&right.imported))
                .then(left.binding.cmp(&right.binding))
        });
        detect_cycles(&edges, &self.loaded)?;
        validate_root_roles(&self.request.roots, &self.origin_to_source, &self.loaded)?;
        validate_ambient_catalogs(&self.request.roots, &self.origin_to_source, &self.loaded)?;
        validate_imported_data_collisions(
            &edges,
            &self.request.roots,
            &self.origin_to_source,
            &self.loaded,
        )?;
        self.finalized_edges = edges;
        Ok(())
    }

    fn finish(self) -> ParsedProject {
        let edges = self.finalized_edges.clone();
        let mut files = BTreeMap::new();
        for file in self.loaded.values() {
            files.insert(file.id.clone(), file.clone());
        }
        let chart_roots = root_ids(
            &self.request.roots,
            ProjectDependencyRole::RootChart,
            &self.origin_to_source,
            &self.source_to_file,
        );
        let ambient_data = root_ids(
            &self.request.roots,
            ProjectDependencyRole::DataConfiguration,
            &self.origin_to_source,
            &self.source_to_file,
        );
        let ambient_catalog = merge_ambient_catalog(&ambient_data, &files);
        let fingerprint = project_fingerprint(
            &files,
            &edges,
            &self.request.schema_version,
            &self.request.registry_version,
        );
        ParsedProject {
            sources: self.sources,
            files,
            imports: edges,
            chart_roots,
            ambient_data,
            ambient_catalog,
            fingerprint,
        }
    }

    fn failure(&self, diagnostic: Diagnostic) -> ProjectLoadAttempt {
        ProjectLoadAttempt {
            result: Err(ProjectLoadFailure {
                diagnostics: vec![diagnostic],
                sources: self.sources.clone(),
            }),
            dependencies: self.dependencies(),
        }
    }
}

fn imported_data_binding(file: &ProjectFile, site: SourceSpan) -> ProjectResult<String> {
    let Root::Data(declarations) = &file.parsed.ast.root else {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-002",
            "data file kind does not contain a data root",
            site,
            file.origin.display_name(),
        ));
    };
    let [declaration] = declarations.as_slice() else {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-020",
            "imported data pack must contain exactly one root",
            site,
            "wrap the pack in one named schema or catalog",
        ));
    };
    if !matches!(declaration.keyword.as_str(), "schema" | "catalog") {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-020",
            "imported data pack root must be a schema or catalog",
            site,
            "top-level table collections must be wrapped before import",
        ));
    }
    declaration
        .name
        .as_ref()
        .map(ToString::to_string)
        .ok_or_else(|| {
            project_diagnostic(
                "AVENGER-PROJECT-020",
                "imported data pack root must be named",
                site,
                "add `as <name>` to the schema or catalog declaration",
            )
        })
}

fn classify_and_validate(
    origin: &SourceOrigin,
    parsed: &mut ParsedFile,
    source: SourceId,
) -> ProjectResult<ProjectFileKind> {
    let path = origin_path(origin);
    let kind = if path.ends_with(".data.avenger") {
        ProjectFileKind::Data
    } else if path.ends_with(".mark.avenger") {
        ProjectFileKind::Definition(DefinitionKind::Mark)
    } else if path.ends_with(".tool.avenger") {
        ProjectFileKind::Definition(DefinitionKind::Tool)
    } else if path.ends_with(".transform.avenger") {
        ProjectFileKind::Definition(DefinitionKind::Transform)
    } else if path.ends_with(".avenger") {
        ProjectFileKind::Chart
    } else {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-001",
            "unrecognized Avenger file extension",
            SourceSpan::empty(source, 0),
            "expected .avenger, .mark.avenger, .tool.avenger, .transform.avenger, or .data.avenger",
        ));
    };
    let root_matches = matches!(
        (kind, &parsed.ast.root),
        (ProjectFileKind::Chart, Root::Chart(_))
            | (ProjectFileKind::Data, Root::Data(_))
            | (ProjectFileKind::Definition(_), Root::Define(_))
    );
    if !root_matches {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-002",
            "file extension does not match its root declaration",
            SourceSpan::empty(source, 0),
            format!("{} requires a matching root kind", origin.display_name()),
        ));
    }
    if let (ProjectFileKind::Definition(expected), Root::Define(decl)) = (kind, &parsed.ast.root)
        && decl.kind.as_ref().map(Name::as_str) != Some(expected.as_str())
    {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-003",
            "definition extension does not match definition kind",
            SourceSpan::empty(source, 0),
            format!("expected `define {}`", expected.as_str()),
        ));
    }
    if kind != ProjectFileKind::Data {
        let name = canonical_file_name(&path).ok_or_else(|| {
            project_diagnostic(
                "AVENGER-PROJECT-004",
                "file name is not a valid Avenger name",
                SourceSpan::empty(source, 0),
                "rename the file to a bare identifier or use an import alias",
            )
        })?;
        let declaration_name = match &parsed.ast.root {
            Root::Chart(decl) | Root::Define(decl) => decl.name.as_ref(),
            Root::Data(_) => None,
        };
        if declaration_name.is_some_and(|declared| declared.as_str() != name) {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-005",
                "declaration name does not match file name",
                SourceSpan::empty(source, 0),
                format!("expected `{name}`"),
            ));
        }
        parsed.ast.name = Some(Name::new(name).expect("validated file name"));
    }
    Ok(kind)
}

fn validate_import_matrix(
    importer: ProjectFileKind,
    imported: ProjectFileKind,
    site: SourceSpan,
) -> ProjectResult<()> {
    let allowed = match importer {
        ProjectFileKind::Chart => {
            matches!(
                imported,
                ProjectFileKind::Definition(_) | ProjectFileKind::Data
            )
        }
        ProjectFileKind::Definition(_) => matches!(imported, ProjectFileKind::Definition(_)),
        ProjectFileKind::Data => matches!(imported, ProjectFileKind::Data),
    };
    if allowed {
        Ok(())
    } else {
        Err(project_diagnostic(
            "AVENGER-PROJECT-012",
            "invalid import for this file kind",
            site,
            format!("a {importer:?} file cannot import a {imported:?} file"),
        ))
    }
}

fn detect_cycles(
    edges: &[ImportEdge],
    files: &BTreeMap<SourceId, ProjectFile>,
) -> ProjectResult<()> {
    let mut adjacency: BTreeMap<SourceId, Vec<(SourceId, SourceSpan)>> = BTreeMap::new();
    for edge in edges {
        adjacency
            .entry(edge.importer)
            .or_default()
            .push((edge.imported, edge.site));
    }
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    for source in files.keys().copied() {
        if let Some(cycle) = visit_cycle(source, &adjacency, &mut visiting, &mut visited) {
            let site = cycle
                .iter()
                .rev()
                .nth(1)
                .map(|(_, site)| *site)
                .unwrap_or_else(|| SourceSpan::empty(source, 0));
            let names = cycle
                .iter()
                .map(|(id, _)| files[id].origin.display_name())
                .collect::<Vec<_>>()
                .join(" -> ");
            let mut diagnostic =
                project_diagnostic("AVENGER-PROJECT-013", "import cycle detected", site, names);
            for edge in cycle.windows(2) {
                let (_, edge_site) = edge[0];
                let (target, _) = edge[1];
                diagnostic.trace.push(ExpansionOrImportFrame {
                    span: edge_site,
                    message: format!("imports {}", files[&target].origin.display_name()),
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

fn validate_root_roles(
    roots: &[ProjectRoot],
    origins: &BTreeMap<SourceOrigin, SourceId>,
    files: &BTreeMap<SourceId, ProjectFile>,
) -> ProjectResult<()> {
    for root in roots {
        let Some(source) = origins.get(&root.origin).copied() else {
            continue;
        };
        let actual = files[&source].kind;
        let valid = match root.role {
            ProjectDependencyRole::RootChart => actual == ProjectFileKind::Chart,
            ProjectDependencyRole::DataConfiguration => actual == ProjectFileKind::Data,
            ProjectDependencyRole::Import => true,
        };
        if !valid {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-002",
                "project root role does not match its file kind",
                SourceSpan::empty(source, 0),
                format!("expected {:?}, found {actual:?}", root.role),
            ));
        }
    }
    Ok(())
}

fn validate_ambient_catalogs(
    roots: &[ProjectRoot],
    origins: &BTreeMap<SourceOrigin, SourceId>,
    files: &BTreeMap<SourceId, ProjectFile>,
) -> ProjectResult<()> {
    let mut paths: BTreeMap<Vec<String>, SourceId> = BTreeMap::new();
    for root in roots
        .iter()
        .filter(|root| root.role == ProjectDependencyRole::DataConfiguration)
    {
        let Some(source) = origins.get(&root.origin).copied() else {
            continue;
        };
        let Root::Data(declarations) = &files[&source].parsed.ast.root else {
            continue;
        };
        for declaration in declarations {
            collect_catalog_paths(declaration, &[], source, &mut paths)?;
        }
    }
    Ok(())
}

fn validate_imported_data_collisions(
    edges: &[ImportEdge],
    roots: &[ProjectRoot],
    origins: &BTreeMap<SourceOrigin, SourceId>,
    files: &BTreeMap<SourceId, ProjectFile>,
) -> ProjectResult<()> {
    let ambient_names = roots
        .iter()
        .filter(|root| root.role == ProjectDependencyRole::DataConfiguration)
        .filter_map(|root| origins.get(&root.origin))
        .flat_map(|source| match &files[source].parsed.ast.root {
            Root::Data(declarations) => declarations.as_slice(),
            _ => &[],
        })
        .filter_map(|declaration| declaration.name.as_ref())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    for edge in edges {
        if files[&edge.imported].kind == ProjectFileKind::Data
            && ambient_names.contains(&edge.binding)
        {
            return Err(project_diagnostic(
                "AVENGER-PROJECT-014",
                "imported data pack collides with the ambient catalog",
                edge.site,
                format!("`{}` is already configured by ambient data", edge.binding),
            ));
        }
    }
    Ok(())
}

fn collect_catalog_paths(
    declaration: &Decl,
    parent: &[String],
    source: SourceId,
    paths: &mut BTreeMap<Vec<String>, SourceId>,
) -> ProjectResult<()> {
    if !matches!(declaration.keyword.as_str(), "catalog" | "schema" | "table") {
        return Ok(());
    }
    let Some(name) = &declaration.name else {
        return Ok(());
    };
    let mut path = parent.to_vec();
    path.push(name.to_string());
    if let Some(previous) = paths.insert(path.clone(), source) {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-014",
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

pub fn resolve_import_origin(
    importer: &SourceOrigin,
    specifier: &str,
    project_root: &Path,
) -> Result<SourceOrigin, String> {
    if let Some(path) = specifier.strip_prefix("std:") {
        return Ok(SourceOrigin::Std(normalize_virtual_path(path)?));
    }
    if specifier.starts_with("http://") || specifier.starts_with("https://") {
        let url = Url::parse(specifier).map_err(|error| error.to_string())?;
        return Ok(SourceOrigin::Http(url.to_string()));
    }
    match importer {
        SourceOrigin::File(path) => {
            let parent = path.parent().unwrap_or(project_root);
            let candidate = normalize_path(&parent.join(specifier));
            Ok(SourceOrigin::File(candidate))
        }
        SourceOrigin::Http(base) => {
            let url = Url::parse(base)
                .and_then(|base| base.join(specifier))
                .map_err(|error| error.to_string())?;
            Ok(SourceOrigin::Http(url.to_string()))
        }
        SourceOrigin::Std(path) => {
            let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            Ok(SourceOrigin::Std(normalize_virtual_path(
                &parent.join(specifier).to_string_lossy(),
            )?))
        }
        SourceOrigin::Memory(path) => {
            let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            Ok(SourceOrigin::Memory(normalize_virtual_path(
                &parent.join(specifier).to_string_lossy(),
            )?))
        }
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

fn project_file_id(project_root: &Path, origin: &SourceOrigin) -> ProjectResult<ProjectFileId> {
    let value = match origin {
        SourceOrigin::File(path) => normalize_path(path)
            .strip_prefix(normalize_path(project_root))
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .map_err(|_| {
                project_diagnostic(
                    "AVENGER-PROJECT-007",
                    "file source is outside the project root",
                    SourceSpan::empty(SourceId::new(0), 0),
                    path.display().to_string(),
                )
            })?,
        _ => origin.canonical_uri(),
    };
    Ok(ProjectFileId::new(value))
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

fn canonical_file_name(path: &str) -> Option<&str> {
    let file_name = Path::new(path).file_name()?.to_str()?;
    let stem = [
        ".mark.avenger",
        ".tool.avenger",
        ".transform.avenger",
        ".avenger",
    ]
    .iter()
    .find_map(|suffix| file_name.strip_suffix(suffix))?;
    let unversioned = stem.split_once('@').map_or(stem, |(name, _)| name);
    is_name(unversioned).then_some(unversioned)
}

fn inferred_import_name(origin: &SourceOrigin) -> Option<String> {
    canonical_file_name(&origin_path(origin))
        .map(ToOwned::to_owned)
        .or_else(|| match origin {
            SourceOrigin::Std(path) => Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| is_name(name))
                .map(ToOwned::to_owned),
            _ => None,
        })
}

fn validate_sha256(
    expected: &str,
    loaded: &LoadedSource,
    site: Option<SourceSpan>,
    fallback: SourceId,
) -> ProjectResult<()> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-009",
            "invalid SHA-256 pin",
            site.unwrap_or_else(|| SourceSpan::empty(fallback, 0)),
            "expected exactly 64 lowercase hexadecimal characters",
        ));
    }
    let actual = sha256_hex(loaded.text.as_bytes());
    if actual != expected {
        return Err(project_diagnostic(
            "AVENGER-PROJECT-009",
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

fn project_fingerprint(
    files: &BTreeMap<ProjectFileId, ProjectFile>,
    edges: &[ImportEdge],
    schema_version: &str,
    registry_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-project-v1\0");
    hasher.update(schema_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(registry_version.as_bytes());
    for (id, file) in files {
        hasher.update(b"\0source\0");
        hasher.update(id.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(file.origin.canonical_uri().as_bytes());
        hasher.update(b"\0");
        hasher.update(file.content_sha256.as_bytes());
    }
    for edge in edges {
        hasher.update(b"\0edge\0");
        hasher.update(edge.importer.get().to_le_bytes());
        hasher.update(edge.imported.get().to_le_bytes());
        hasher.update(edge.binding.as_bytes());
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn root_ids(
    roots: &[ProjectRoot],
    role: ProjectDependencyRole,
    origins: &BTreeMap<SourceOrigin, SourceId>,
    ids: &BTreeMap<SourceId, ProjectFileId>,
) -> Vec<ProjectFileId> {
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
    ambient_data: &[ProjectFileId],
    files: &BTreeMap<ProjectFileId, ProjectFile>,
) -> Vec<AmbientDataDeclaration> {
    let mut declarations = Vec::new();
    for id in ambient_data {
        let file = &files[id];
        if let Root::Data(roots) = &file.parsed.ast.root {
            declarations.extend(
                roots
                    .iter()
                    .cloned()
                    .map(|declaration| AmbientDataDeclaration {
                        source: file.source,
                        declaration,
                    }),
            );
        }
    }
    declarations
}

fn loader_diagnostic(
    pending: &PendingSource,
    fallback: SourceId,
    error: SourceLoaderError,
) -> Box<Diagnostic> {
    let mut diagnostic = project_diagnostic(
        "AVENGER-PROJECT-015",
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

fn project_diagnostic(
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
