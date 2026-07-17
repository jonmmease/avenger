use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use avenger_chart_lang_registry::{
    NativeRegistry, RegistryError, ResolvedDeclaration, ResolvedPlot, ResolvedValue, builtins,
};
use avenger_lang_core::{
    DataCapabilities, Diagnostic, EmptyEnvironmentProvider, EnvironmentProvider,
    ImportCapabilities, ParsedProject, ProjectDependencyRole, ProjectLoadAttempt,
    ProjectLoadRequest, ProjectLoader, ProjectRoot, ResolvedProject, SourceFile, SourceId,
    SourceLabel, SourceLoader, SourceLoaderError, SourceMap, SourceOrigin, SourceSpan,
    ast::{Decl, Value},
    project::{normalize_path, resolve_import_origin},
    resolve_project as resolve_semantics,
};
use datafusion::logical_expr::col;
use serde::{Deserialize, Serialize};

use crate::{
    CatalogFactoryRegistry, CompileEnvironmentFactory, CompileEnvironmentRequest,
    CompiledChartArtifact, CompiledProject, CompilerOptions, DefaultCompileEnvironmentFactory,
    DefaultSourceLoader, DependencyFingerprint, LanguageHost, ProjectAnalysis, ProjectChartId,
    ProjectFingerprint,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRole {
    RootChart,
    Import,
    DataConfiguration,
    LocalResource,
    RemoteResource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledDependency {
    pub role: DependencyRole,
    pub requested_origin: SourceOrigin,
    pub canonical_origin: SourceOrigin,
    pub content_version: Option<String>,
    pub nearest_existing_parent: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredDependencySet {
    dependencies: BTreeMap<(SourceOrigin, DependencyRole), CompiledDependency>,
}

impl DiscoveredDependencySet {
    pub fn insert(&mut self, dependency: CompiledDependency) {
        self.dependencies.insert(
            (dependency.requested_origin.clone(), dependency.role),
            dependency,
        );
    }

    pub fn iter(&self) -> impl Iterator<Item = &CompiledDependency> {
        self.dependencies.values()
    }

    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct CompileAttempt<T> {
    pub result: Result<T, CompileFailure>,
    pub dependencies: DiscoveredDependencySet,
}

#[derive(Clone, Debug)]
pub struct CompileFailure {
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Display for CompileFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.diagnostics.first() {
            Some(diagnostic) => write!(
                f,
                "{}[{}]: {}",
                match diagnostic.severity {
                    avenger_lang_core::DiagnosticSeverity::Error => "error",
                },
                diagnostic.code.as_str(),
                diagnostic.message
            ),
            None => f.write_str("language compilation failed without diagnostics"),
        }
    }
}

impl std::error::Error for CompileFailure {}

#[derive(Clone, Debug)]
pub struct ExpandedSource {
    pub text: String,
    pub sources: SourceMap,
}

#[derive(Clone)]
pub struct Compiler {
    options: Arc<CompilerOptions>,
    host: LanguageHost,
}

impl Compiler {
    pub fn builder() -> CompilerBuilder {
        CompilerBuilder::default()
    }

    pub fn options(&self) -> &CompilerOptions {
        &self.options
    }

    pub fn language_host(&self) -> &LanguageHost {
        &self.host
    }

    pub async fn compile_file_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<CompiledChartArtifact> {
        let attempt = self.resolve_file_project_attempt(path).await;
        map_resolved_project_to_compilation(attempt)
    }

    pub async fn compile_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<CompiledChartArtifact, CompileFailure> {
        self.compile_file_attempt(path).await.result
    }

    pub async fn compile_project_attempt(
        &self,
        root: impl AsRef<Path>,
    ) -> CompileAttempt<CompiledProject> {
        let attempt = self.resolve_project_graph_attempt(root).await;
        map_resolved_project_to_project_compilation(attempt)
    }

    pub async fn compile_project(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<CompiledProject, CompileFailure> {
        self.compile_project_attempt(root).await.result
    }

    pub async fn analyze_project(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<ProjectAnalysis, CompileFailure> {
        let project = self.resolve_project_graph_attempt(root).await.result?;
        Err(phase_five_frontend_failure(&project))
    }

    pub async fn check_project(&self, root: impl AsRef<Path>) -> Result<(), CompileFailure> {
        self.resolve_project_graph_attempt(root)
            .await
            .result
            .map(|_| ())
    }

    pub async fn expand_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ExpandedSource, CompileFailure> {
        Err(self
            .compile_file_attempt(path)
            .await
            .result
            .expect_err("Phase 5 native chart construction is unavailable"))
    }

    /// Phase 4 frontend seam: load and semantically resolve one chart and its
    /// complete dependency closure without constructing native chart objects.
    pub async fn resolve_file_project_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<ResolvedProject> {
        let attempt = self.load_file_project_attempt(path).await;
        map_parsed_project_to_resolution(attempt, self.host.authoring_schema())
    }

    /// Convenience wrapper for callers that do not need dependency metadata.
    pub async fn resolve_file_project(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ResolvedProject, CompileFailure> {
        self.resolve_file_project_attempt(path).await.result
    }

    /// Phase 4 frontend seam: discover and semantically resolve every chart
    /// root and ambient data file below a project directory.
    pub async fn resolve_project_graph_attempt(
        &self,
        root: impl AsRef<Path>,
    ) -> CompileAttempt<ResolvedProject> {
        let attempt = self.load_project_graph_attempt(root).await;
        map_parsed_project_to_resolution(attempt, self.host.authoring_schema())
    }

    /// Phase 3 frontend seam: load one chart and its complete import/data
    /// closure without performing semantic validation or lowering.
    pub async fn load_file_project_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<ParsedProject> {
        let chart = canonicalize_if_exists(&self.resolve_path(path.as_ref()));
        let ambient = if self.options.project_root.exists() {
            match discover_avenger_files(&self.options.project_root) {
                Ok(files) => files
                    .into_iter()
                    .filter(|path| is_data_path(path))
                    .collect::<Vec<_>>(),
                Err(error) => return discovery_failure(&self.options.project_root, error),
            }
        } else {
            Vec::new()
        };
        let mut roots = vec![ProjectRoot::chart(SourceOrigin::File(chart))];
        roots.extend(
            ambient
                .into_iter()
                .map(|path| ProjectRoot::data(SourceOrigin::File(path))),
        );
        self.load_roots(roots).await
    }

    /// Phase 3 frontend seam: discover all chart roots and ambient data files
    /// below a directory, then load their shared import closure once.
    pub async fn load_project_graph_attempt(
        &self,
        root: impl AsRef<Path>,
    ) -> CompileAttempt<ParsedProject> {
        let root = canonicalize_if_exists(&self.resolve_path(root.as_ref()));
        let files = match discover_avenger_files(&root) {
            Ok(files) => files,
            Err(error) => return discovery_failure(&root, error),
        };
        let roots = files
            .into_iter()
            .filter_map(|path| {
                if is_data_path(&path) {
                    Some(ProjectRoot::data(SourceOrigin::File(path)))
                } else if is_chart_path(&path) {
                    Some(ProjectRoot::chart(SourceOrigin::File(path)))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if roots
            .iter()
            .all(|root| root.role != ProjectDependencyRole::RootChart)
        {
            let source = SourceFile::new(SourceId::new(0), SourceOrigin::File(root.clone()), "");
            let mut sources = SourceMap::default();
            sources.insert(source).expect("fresh source id");
            return CompileAttempt {
                result: Err(CompileFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-PROJECT-016",
                        "project contains no chart roots",
                        SourceLabel::new(
                            SourceSpan::empty(SourceId::new(0), 0),
                            "add a .avenger chart file",
                        ),
                    )],
                }),
                dependencies: DiscoveredDependencySet::default(),
            };
        }
        self.load_roots(roots).await
    }

    async fn load_roots(&self, roots: Vec<ProjectRoot>) -> CompileAttempt<ParsedProject> {
        let request = ProjectLoadRequest {
            project_root: normalize_path(&self.options.project_root),
            roots,
            capabilities: self.options.import_capabilities.clone(),
            schema_version: "avenger-ast-core-1".to_owned(),
            registry_version: self
                .options
                .native_registry
                .profile_id()
                .as_str()
                .to_owned(),
        };
        let ProjectLoadAttempt {
            result,
            dependencies,
        } = ProjectLoader::new(self.options.source_loader.as_ref())
            .load(request)
            .await;
        let mut dependencies = compiler_dependencies(dependencies);
        let result = result.map_err(|failure| CompileFailure {
            diagnostics: failure.diagnostics,
        });
        let result = result.and_then(|mut project| {
            discover_local_resources(
                &project,
                &self.options.project_root,
                &self.options.import_capabilities,
                &mut dependencies,
            )?;
            project.fingerprint = augment_project_fingerprint(&project.fingerprint, &dependencies);
            Ok(project)
        });
        CompileAttempt {
            result,
            dependencies,
        }
    }

    /// Temporary Phase 0 vertical slice. Phase 5 replaces this with a real DSL
    /// fixture while retaining the same artifact wrapper.
    #[doc(hidden)]
    pub async fn compile_phase0_example(&self) -> Result<CompiledChartArtifact, CompileFailure> {
        let request = CompileEnvironmentRequest {
            generation: 0,
            native_registry_profile: self
                .options
                .native_registry
                .profile_id()
                .as_str()
                .to_string(),
        };
        let environment = self
            .options
            .environment_factory
            .create(&request)
            .map_err(|error| phase_zero_failure(error.to_string()))?;
        let context = environment.session_context();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.data = Some(
            context
                .sql("SELECT * FROM (VALUES (1.0, 2.5), (2.0, 3.2), (3.0, 4.8)) AS t(x, y)")
                .await
                .map_err(|error| phase_zero_failure(error.to_string()))?,
        );
        plot.marks.push(
            ResolvedDeclaration::new("symbol")
                .property("x", ResolvedValue::Expr(col("x")))
                .property("y", ResolvedValue::Expr(col("y"))),
        );
        let compiled = self
            .options
            .native_registry
            .compile_root(&plot, context)
            .await
            .map_err(|error| phase_zero_failure(error.to_string()))?;

        Ok(CompiledChartArtifact::new(
            ProjectChartId::new("phase0-example"),
            Some("Phase 0 programmatic example".to_string()),
            SourceId::new(0),
            Arc::new(compiled),
            self.options.native_registry.profile_id().clone(),
            DependencyFingerprint::new("phase0-programmatic"),
        ))
    }

    /// Temporary Phase 0 analysis artifact used to prove registry-profile
    /// propagation before project parsing and DataFusion planning land.
    #[doc(hidden)]
    pub fn analyze_phase0_empty(&self) -> ProjectAnalysis {
        ProjectAnalysis::empty(
            SourceMap::default(),
            self.options.native_registry.profile_id().clone(),
            ProjectFingerprint::new("phase0-empty-analysis"),
        )
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.options.project_root.join(path)
        }
    }
}

#[derive(Default)]
pub struct CompilerBuilder {
    project_root: Option<PathBuf>,
    import_capabilities: Option<ImportCapabilities>,
    data_capabilities: DataCapabilities,
    environment: Option<Arc<dyn EnvironmentProvider>>,
    native_registry: Option<Arc<NativeRegistry>>,
    source_loader: Option<Arc<dyn SourceLoader>>,
    catalog_factories: CatalogFactoryRegistry,
    environment_factory: Option<Arc<dyn CompileEnvironmentFactory>>,
}

impl CompilerBuilder {
    pub fn project_root(mut self, project_root: impl Into<PathBuf>) -> Self {
        self.project_root = Some(project_root.into());
        self
    }

    pub fn import_capabilities(mut self, capabilities: ImportCapabilities) -> Self {
        self.import_capabilities = Some(capabilities);
        self
    }

    pub fn data_capabilities(mut self, capabilities: DataCapabilities) -> Self {
        self.data_capabilities = capabilities;
        self
    }

    pub fn environment(mut self, environment: Arc<dyn EnvironmentProvider>) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn native_registry(mut self, native_registry: Arc<NativeRegistry>) -> Self {
        self.native_registry = Some(native_registry);
        self
    }

    pub fn source_loader(mut self, source_loader: Arc<dyn SourceLoader>) -> Self {
        self.source_loader = Some(source_loader);
        self
    }

    pub fn catalog_factories(mut self, catalog_factories: CatalogFactoryRegistry) -> Self {
        self.catalog_factories = catalog_factories;
        self
    }

    pub fn environment_factory(
        mut self,
        environment_factory: Arc<dyn CompileEnvironmentFactory>,
    ) -> Self {
        self.environment_factory = Some(environment_factory);
        self
    }

    pub fn build(self) -> Result<Compiler, CompilerBuildError> {
        let project_root = self
            .project_root
            .ok_or(CompilerBuildError::MissingProjectRoot)?;
        let project_root =
            std::fs::canonicalize(&project_root).unwrap_or_else(|_| normalize_path(&project_root));
        let registry = match self.native_registry {
            Some(registry) => registry,
            None => Arc::new(builtins::bootstrap_registry()?),
        };
        let source_loader = self.source_loader.map(Ok).unwrap_or_else(|| {
            DefaultSourceLoader::new(&project_root)
                .map(|loader| Arc::new(loader) as Arc<dyn SourceLoader>)
        })?;
        let options = CompilerOptions {
            import_capabilities: self
                .import_capabilities
                .unwrap_or_else(|| ImportCapabilities::project(&project_root)),
            project_root,
            data_capabilities: self.data_capabilities,
            environment: self
                .environment
                .unwrap_or_else(|| Arc::new(EmptyEnvironmentProvider)),
            native_registry: registry.clone(),
            source_loader,
            catalog_factories: self.catalog_factories,
            environment_factory: self
                .environment_factory
                .unwrap_or_else(|| Arc::new(DefaultCompileEnvironmentFactory)),
        };
        Ok(Compiler {
            options: Arc::new(options),
            host: LanguageHost::new(registry),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompilerBuildError {
    #[error("compiler project root is required")]
    MissingProjectRoot,
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    SourceLoader(#[from] SourceLoaderError),
}

fn phase_five_frontend_diagnostic(source: SourceId) -> Diagnostic {
    Diagnostic::error(
        "AV0005",
        "native chart lowering is not implemented yet",
        SourceLabel::new(
            SourceSpan::empty(source, 0),
            "Phase 4 resolved the project; Phase 5 constructs native charts",
        ),
    )
    .with_note("semantic validation and name resolution completed successfully")
}

fn phase_five_frontend_failure(project: &ResolvedProject) -> CompileFailure {
    let source = project
        .files
        .values()
        .find(|file| matches!(file.kind, avenger_lang_core::ProjectFileKind::Chart))
        .map_or(SourceId::new(0), |file| file.source);
    CompileFailure {
        diagnostics: vec![phase_five_frontend_diagnostic(source)],
    }
}

fn phase_zero_failure(message: String) -> CompileFailure {
    CompileFailure {
        diagnostics: vec![Diagnostic::error(
            "AV0004",
            "programmatic chart compilation failed",
            SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), message),
        )],
    }
}

fn nearest_existing_parent(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.exists())
        .map(Path::to_path_buf)
}

fn canonicalize_if_exists(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

fn compiler_dependencies(
    dependencies: Vec<avenger_lang_core::ProjectDependency>,
) -> DiscoveredDependencySet {
    let mut result = DiscoveredDependencySet::default();
    for dependency in dependencies {
        let requested_origin = dependency.requested_origin;
        let canonical_origin = dependency
            .canonical_origin
            .unwrap_or_else(|| requested_origin.clone());
        let nearest_existing_parent = match &requested_origin {
            SourceOrigin::File(path) => nearest_existing_parent(path),
            _ => None,
        };
        let role = match dependency.role {
            ProjectDependencyRole::RootChart => DependencyRole::RootChart,
            ProjectDependencyRole::Import => DependencyRole::Import,
            ProjectDependencyRole::DataConfiguration => DependencyRole::DataConfiguration,
        };
        result.insert(CompiledDependency {
            role,
            requested_origin,
            canonical_origin,
            content_version: dependency.content_version,
            nearest_existing_parent,
        });
    }
    result
}

fn map_parsed_project_to_resolution(
    attempt: CompileAttempt<ParsedProject>,
    schema: &avenger_chart_schema::NativeSchemaSnapshot,
) -> CompileAttempt<ResolvedProject> {
    let result = attempt.result.and_then(|project| {
        resolve_semantics(&project, schema)
            .result
            .map_err(|failure| CompileFailure {
                diagnostics: failure.diagnostics,
            })
    });
    CompileAttempt {
        result,
        dependencies: attempt.dependencies,
    }
}

fn map_resolved_project_to_compilation(
    attempt: CompileAttempt<ResolvedProject>,
) -> CompileAttempt<CompiledChartArtifact> {
    let result = attempt
        .result
        .and_then(|project| Err(phase_five_frontend_failure(&project)));
    CompileAttempt {
        result,
        dependencies: attempt.dependencies,
    }
}

fn map_resolved_project_to_project_compilation(
    attempt: CompileAttempt<ResolvedProject>,
) -> CompileAttempt<CompiledProject> {
    let result = attempt
        .result
        .and_then(|project| Err(phase_five_frontend_failure(&project)));
    CompileAttempt {
        result,
        dependencies: attempt.dependencies,
    }
}

fn discover_avenger_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    if !root.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} does not exist", root.display()),
        ));
    }
    if root.is_file() {
        return Ok(vec![normalize_path(root)]);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries.into_iter().rev() {
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && path.to_string_lossy().ends_with(".avenger") {
                files.push(normalize_path(&path));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_data_path(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".data.avenger")
}

fn is_definition_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.ends_with(".mark.avenger")
        || path.ends_with(".tool.avenger")
        || path.ends_with(".transform.avenger")
}

fn is_chart_path(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".avenger") && !is_data_path(path) && !is_definition_path(path)
}

fn discovery_failure<T>(root: &Path, error: std::io::Error) -> CompileAttempt<T> {
    CompileAttempt {
        result: Err(CompileFailure {
            diagnostics: vec![Diagnostic::error(
                "AVENGER-PROJECT-017",
                "project discovery failed",
                SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), error.to_string()),
            )],
        }),
        dependencies: {
            let mut dependencies = DiscoveredDependencySet::default();
            dependencies.insert(CompiledDependency {
                role: DependencyRole::LocalResource,
                requested_origin: SourceOrigin::File(root.to_path_buf()),
                canonical_origin: SourceOrigin::File(root.to_path_buf()),
                content_version: None,
                nearest_existing_parent: nearest_existing_parent(root),
            });
            dependencies
        },
    }
}

fn discover_local_resources(
    project: &ParsedProject,
    project_root: &Path,
    capabilities: &ImportCapabilities,
    dependencies: &mut DiscoveredDependencySet,
) -> Result<(), CompileFailure> {
    for file in project.files.values() {
        for declaration in file.parsed.ast.root.declarations() {
            discover_declaration_resources(
                declaration,
                file.source,
                &file.origin,
                project_root,
                capabilities,
                dependencies,
            )?;
        }
    }
    Ok(())
}

fn discover_declaration_resources(
    declaration: &Decl,
    source: SourceId,
    declaring_origin: &SourceOrigin,
    project_root: &Path,
    capabilities: &ImportCapabilities,
    dependencies: &mut DiscoveredDependencySet,
) -> Result<(), CompileFailure> {
    if declaration.keyword.as_str() == "table"
        && let Some(value) = declaration.props.get("path")
    {
        let mut paths = Vec::new();
        collect_string_values(value, &mut paths);
        for path in paths {
            discover_resource(
                path,
                source,
                declaring_origin,
                project_root,
                capabilities,
                dependencies,
            )?;
        }
    }
    for child in &declaration.children {
        discover_declaration_resources(
            child,
            source,
            declaring_origin,
            project_root,
            capabilities,
            dependencies,
        )?;
    }
    Ok(())
}

fn collect_string_values<'a>(value: &'a Value, paths: &mut Vec<&'a str>) {
    match value {
        Value::Str(path) => paths.push(path),
        Value::Array(values) => {
            for value in values {
                collect_string_values(value, paths);
            }
        }
        _ => {}
    }
}

fn discover_resource(
    path: &str,
    source: SourceId,
    declaring_origin: &SourceOrigin,
    project_root: &Path,
    capabilities: &ImportCapabilities,
    dependencies: &mut DiscoveredDependencySet,
) -> Result<(), CompileFailure> {
    let origin = if path.split_once("://").is_some() {
        SourceOrigin::Http(path.to_owned())
    } else {
        resolve_import_origin(declaring_origin, path, project_root).map_err(|message| {
            CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-PROJECT-018",
                    "invalid local data resource",
                    SourceLabel::new(SourceSpan::empty(source, 0), message),
                )],
            }
        })?
    };
    let role = match origin {
        SourceOrigin::Http(_) => DependencyRole::RemoteResource,
        _ => DependencyRole::LocalResource,
    };
    let mut dependency = CompiledDependency {
        role,
        requested_origin: origin.clone(),
        canonical_origin: origin.clone(),
        content_version: None,
        nearest_existing_parent: match &origin {
            SourceOrigin::File(path) => nearest_existing_parent(path),
            _ => None,
        },
    };
    dependencies.insert(dependency.clone());
    let SourceOrigin::File(candidate) = origin else {
        return Ok(());
    };
    let normalized_root = normalize_path(project_root);
    let normalized_candidate = normalize_path(&candidate);
    if !capabilities.allow_filesystem || !normalized_candidate.starts_with(&normalized_root) {
        return Err(resource_failure(
            source,
            "local data resource is outside the project capability root",
            normalized_candidate.display().to_string(),
        ));
    }
    if path.contains(['*', '?', '[']) {
        dependency.content_version = Some("glob".to_owned());
        dependencies.insert(dependency);
        return Ok(());
    }
    let canonical = std::fs::canonicalize(&normalized_candidate).map_err(|error| {
        resource_failure(
            source,
            "local data resource could not be loaded",
            format!("{}: {error}", normalized_candidate.display()),
        )
    })?;
    if !canonical.starts_with(&normalized_root) {
        return Err(resource_failure(
            source,
            "local data resource escapes the project through a symlink",
            canonical.display().to_string(),
        ));
    }
    dependency.canonical_origin = SourceOrigin::File(canonical.clone());
    dependency.nearest_existing_parent = canonical.parent().map(Path::to_path_buf);
    dependency.content_version = Some(resource_content_version(&canonical).map_err(|error| {
        resource_failure(
            source,
            "local data resource could not be fingerprinted",
            error.to_string(),
        )
    })?);
    dependencies.insert(dependency);
    Ok(())
}

fn resource_content_version(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    if path.is_dir() {
        return Ok("directory".to_owned());
    }
    let bytes = std::fs::read(path)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn resource_failure(source: SourceId, message: &str, label: String) -> CompileFailure {
    CompileFailure {
        diagnostics: vec![Diagnostic::error(
            "AVENGER-PROJECT-019",
            message,
            SourceLabel::new(SourceSpan::empty(source, 0), label),
        )],
    }
}

fn augment_project_fingerprint(
    source_fingerprint: &str,
    dependencies: &DiscoveredDependencySet,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-compiler-project-v1\0");
    hasher.update(source_fingerprint.as_bytes());
    for dependency in dependencies.iter() {
        hasher.update(b"\0dependency\0");
        hasher.update(dependency.canonical_origin.canonical_uri().as_bytes());
        hasher.update(b"\0");
        hasher.update(format!("{:?}", dependency.role).as_bytes());
        hasher.update(b"\0");
        if let Some(version) = &dependency.content_version {
            hasher.update(version.as_bytes());
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}
