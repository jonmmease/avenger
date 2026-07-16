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
    ImportCapabilities, InMemorySourceLoader, SourceFile, SourceId, SourceLabel, SourceLoader,
    SourceMap, SourceOrigin, SourceSpan,
};
use datafusion::logical_expr::col;
use serde::{Deserialize, Serialize};

use crate::{
    CatalogFactoryRegistry, CompileEnvironmentFactory, CompileEnvironmentRequest,
    CompiledChartArtifact, CompiledProject, CompilerOptions, DefaultCompileEnvironmentFactory,
    DependencyFingerprint, LanguageHost, ProjectAnalysis, ProjectChartId, ProjectFingerprint,
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
            (dependency.canonical_origin.clone(), dependency.role),
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
        let path = self.resolve_path(path.as_ref());
        let origin = SourceOrigin::File(path.clone());
        let mut dependencies = DiscoveredDependencySet::default();
        dependencies.insert(CompiledDependency {
            role: DependencyRole::RootChart,
            canonical_origin: origin.clone(),
            content_version: None,
            nearest_existing_parent: nearest_existing_parent(&path),
        });

        let loaded = self
            .options
            .source_loader
            .load(&origin, &self.options.import_capabilities)
            .await;
        let (source, diagnostic) = match loaded {
            Ok(loaded) => {
                dependencies.insert(CompiledDependency {
                    role: DependencyRole::RootChart,
                    canonical_origin: loaded.origin.clone(),
                    content_version: Some(loaded.version.as_str().to_string()),
                    nearest_existing_parent: nearest_existing_parent(&path),
                });
                let source = SourceFile::new(SourceId::new(0), loaded.origin, loaded.text);
                let diagnostic = phase_zero_frontend_diagnostic(&source);
                (source, diagnostic)
            }
            Err(error) => {
                let source = SourceFile::new(SourceId::new(0), origin, "");
                let diagnostic = Diagnostic::error(
                    "AV0003",
                    "source could not be loaded",
                    SourceLabel::new(SourceSpan::empty(source.id, 0), error.to_string()),
                );
                (source, diagnostic)
            }
        };
        let mut sources = SourceMap::default();
        sources.insert(source).expect("fresh source id");
        let _ = sources;

        CompileAttempt {
            result: Err(CompileFailure {
                diagnostics: vec![diagnostic],
            }),
            dependencies,
        }
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
        let attempt = self.compile_file_attempt(root.as_ref()).await;
        CompileAttempt {
            result: Err(attempt
                .result
                .expect_err("phase-zero frontend is unavailable")),
            dependencies: attempt.dependencies,
        }
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
        Err(self
            .compile_file_attempt(root)
            .await
            .result
            .expect_err("phase-zero frontend is unavailable"))
    }

    pub async fn check_project(&self, root: impl AsRef<Path>) -> Result<(), CompileFailure> {
        self.analyze_project(root).await.map(|_| ())
    }

    pub async fn expand_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ExpandedSource, CompileFailure> {
        Err(self
            .compile_file_attempt(path)
            .await
            .result
            .expect_err("phase-zero frontend is unavailable"))
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
        let registry = match self.native_registry {
            Some(registry) => registry,
            None => Arc::new(builtins::bootstrap_registry()?),
        };
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
            source_loader: self
                .source_loader
                .unwrap_or_else(|| Arc::new(InMemorySourceLoader::default())),
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
}

fn phase_zero_frontend_diagnostic(source: &SourceFile) -> Diagnostic {
    Diagnostic::error(
        "AV0000",
        "the strict DSL frontend is not implemented yet",
        SourceLabel::new(
            SourceSpan::empty(source.id, 0),
            "Phase 0 accepts only the temporary programmatic compiler harness",
        ),
    )
    .with_note("source parsing begins in Phase 2")
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
