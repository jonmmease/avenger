use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use avenger_chart_lang_registry::{
    NativeRegistry, RegistryError, ResolvedDeclaration as NativeResolvedDeclaration, ResolvedPlot,
    ResolvedValue as NativeResolvedValue, builtins,
};
use avenger_lang_core::{
    DataCapabilities, DeclarationId, Diagnostic, EmptyEnvironmentProvider, EnvironmentProvider,
    ExpansionSourceMap, ImportCapabilities, ParsedProject, ProjectDependencyRole, ProjectFileId,
    ProjectFileKind, ProjectLoadAttempt, ProjectLoadRequest, ProjectLoader, ProjectRoot,
    ResolvedDeclaration, ResolvedProject, ResolvedTarget, ResolvedValue, SourceFile, SourceId,
    SourceLabel, SourceLoader, SourceLoaderError, SourceMap, SourceOrigin, SourceSpan,
    ast::{Decl, Value},
    expand_project,
    project::{normalize_path, resolve_import_origin},
    resolve_project as resolve_semantics, sort_diagnostics,
};
use datafusion::logical_expr::col;
use serde::{Deserialize, Serialize};

use crate::{
    AnalyzedDataset, ArtifactCacheKey, CatalogFactoryRegistry, CompileEnvironmentFactory,
    CompileEnvironmentRequest, CompiledChartArtifact, CompiledProject, CompilerOptions,
    DatasetLineage, DatasetProvenance, DatasetStageId, DefaultCompileEnvironmentFactory,
    DefaultSourceLoader, DependencyFingerprint, LanguageHost, ProjectAnalysis, ProjectChartId,
    ProjectDatasetId, ProjectDependencyFingerprints, ProjectFingerprint, TableFactoryRegistry,
    catalog::{CatalogAnalysis, CatalogOptions, register_and_analyze_catalog},
    lowering::{LoweredProject, analyze_chart_datasets, lower_project, lower_project_chart},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectCompilationMode {
    Sequential,
    #[default]
    Parallel,
}

/// Read-only cache telemetry intended for tests, hosts, and future inspector
/// tooling. It exposes immutable identities and counts, never cached sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompilerCacheSnapshot {
    pub resolved_projects: usize,
    pub project_analyses: usize,
    pub dataset_analyses: usize,
    pub chart_artifacts: usize,
    pub artifact_keys: Vec<ArtifactCacheKey>,
}

#[derive(Default)]
struct ArtifactCache {
    artifacts: BTreeMap<ArtifactCacheKey, CompiledChartArtifact>,
    chart_keys: BTreeMap<ProjectChartId, ArtifactCacheKey>,
}

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
    owners: BTreeMap<SourceId, BTreeSet<(SourceOrigin, DependencyRole)>>,
}

impl DiscoveredDependencySet {
    pub fn insert(&mut self, dependency: CompiledDependency) {
        self.dependencies.insert(
            (dependency.requested_origin.clone(), dependency.role),
            dependency,
        );
    }

    fn insert_owned(&mut self, owner: SourceId, dependency: CompiledDependency) {
        let key = (dependency.requested_origin.clone(), dependency.role);
        self.owners.entry(owner).or_default().insert(key.clone());
        self.dependencies.insert(key, dependency);
    }

    fn owned_by(&self, owner: SourceId) -> impl Iterator<Item = &CompiledDependency> {
        self.owners
            .get(&owner)
            .into_iter()
            .flatten()
            .filter_map(|key| self.dependencies.get(key))
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

#[derive(Clone)]
pub struct CompiledChartGeneration {
    pub generation: u64,
    pub artifact: CompiledChartArtifact,
    pub environment: crate::CompileEnvironment,
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
    pub source_map: ExpansionSourceMap,
}

#[derive(Clone)]
pub struct Compiler {
    options: Arc<CompilerOptions>,
    host: LanguageHost,
    resolved_project_cache: Arc<Mutex<BTreeMap<String, ResolvedProject>>>,
    analysis_cache: Arc<Mutex<BTreeMap<String, ProjectAnalysis>>>,
    dataset_analysis_cache: Arc<Mutex<BTreeMap<String, AnalyzedDataset>>>,
    artifact_cache: Arc<Mutex<ArtifactCache>>,
    project_compilation_mode: ProjectCompilationMode,
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

    pub fn cache_snapshot(&self) -> CompilerCacheSnapshot {
        let artifact_cache = self
            .artifact_cache
            .lock()
            .expect("artifact cache lock poisoned");
        CompilerCacheSnapshot {
            resolved_projects: self
                .resolved_project_cache
                .lock()
                .expect("resolved-project cache lock poisoned")
                .len(),
            project_analyses: self
                .analysis_cache
                .lock()
                .expect("analysis cache lock poisoned")
                .len(),
            dataset_analyses: self
                .dataset_analysis_cache
                .lock()
                .expect("dataset-analysis cache lock poisoned")
                .len(),
            chart_artifacts: artifact_cache.artifacts.len(),
            artifact_keys: artifact_cache.artifacts.keys().cloned().collect(),
        }
    }

    pub async fn compile_file_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<CompiledChartArtifact> {
        let attempt = self.compile_file_generation_attempt(path, 0).await;
        CompileAttempt {
            result: attempt.result.map(|generation| generation.artifact),
            dependencies: attempt.dependencies,
        }
    }

    pub async fn compile_file_generation_attempt(
        &self,
        path: impl AsRef<Path>,
        generation: u64,
    ) -> CompileAttempt<CompiledChartGeneration> {
        let attempt = self.load_file_project_attempt(path).await;
        let dependencies = attempt.dependencies;
        let result = match attempt.result {
            Ok(parsed) => match self
                .resolve_parsed_project_attempt(CompileAttempt {
                    result: Ok(parsed.clone()),
                    dependencies: dependencies.clone(),
                })
                .result
            {
                Ok(project) => self
                    .lower_resolved_project(&project, generation)
                    .await
                    .and_then(|(mut lowered, environment, catalog)| {
                        if lowered.charts.len() != 1 {
                            return Err(CompileFailure {
                                diagnostics: vec![Diagnostic::error(
                                    "AVENGER-LOWER-003",
                                    "compile_file requires exactly one chart",
                                    SourceLabel::new(
                                        SourceSpan::empty(SourceId::new(0), 0),
                                        format!("resolved {} charts", lowered.charts.len()),
                                    ),
                                )],
                            });
                        }
                        let fingerprints = dependency_fingerprint_layers(
                            &self.options,
                            &parsed,
                            &project,
                            &dependencies,
                            &catalog,
                            &environment,
                        );
                        let mut artifact = lowered.charts.remove(0).artifact;
                        if let Some(fingerprint) = fingerprints.charts.get(&artifact.id) {
                            artifact.dependency_fingerprint = fingerprint.clone();
                        }
                        Ok(CompiledChartGeneration {
                            generation,
                            artifact,
                            environment,
                        })
                    }),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        CompileAttempt {
            result,
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
        let attempt = self.load_project_graph_attempt(root).await;
        let dependencies = attempt.dependencies;
        let result = match attempt.result {
            Ok(parsed) => {
                let resolved = self
                    .resolve_parsed_project_attempt(CompileAttempt {
                        result: Ok(parsed.clone()),
                        dependencies: dependencies.clone(),
                    })
                    .result;
                match resolved {
                    Ok(resolved) => {
                        self.compile_resolved_project(&parsed, &resolved, &dependencies, 0)
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        CompileAttempt {
            result,
            dependencies,
        }
    }

    pub async fn compile_project(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<CompiledProject, CompileFailure> {
        self.compile_project_attempt(root).await.result
    }

    async fn compile_resolved_project(
        &self,
        parsed: &ParsedProject,
        project: &ResolvedProject,
        dependencies: &DiscoveredDependencySet,
        generation: u64,
    ) -> Result<CompiledProject, CompileFailure> {
        let (environment, catalog) = self.analyze_resolved_project(project, generation).await?;
        let fingerprints = dependency_fingerprint_layers(
            &self.options,
            parsed,
            project,
            dependencies,
            &catalog,
            &environment,
        );
        let profile = self.options.native_registry.profile_id();
        let mut artifacts = BTreeMap::<ProjectChartId, CompiledChartArtifact>::new();
        let mut misses = Vec::<(DeclarationId, ProjectChartId, ArtifactCacheKey)>::new();
        {
            let cache = self
                .artifact_cache
                .lock()
                .expect("artifact cache lock poisoned");
            for chart in &project.charts {
                let public_id = ProjectChartId::new(chart.as_str());
                let fingerprint = fingerprints
                    .charts
                    .get(&public_id)
                    .cloned()
                    .unwrap_or_default();
                let key = ArtifactCacheKey::new(profile, fingerprint);
                if let Some(artifact) = cache.artifacts.get(&key).cloned() {
                    artifacts.insert(public_id, artifact);
                } else {
                    misses.push((chart.clone(), public_id, key));
                }
            }
        }

        let lower_one = |chart: DeclarationId| {
            let chart_environment = environment.fork();
            async move {
                lower_project_chart(
                    project,
                    &chart,
                    self.options.native_registry.as_ref(),
                    chart_environment.session_context(),
                    self.options.source_loader.as_ref(),
                    &self.options.import_capabilities,
                )
                .await
            }
        };
        let results = match self.project_compilation_mode {
            ProjectCompilationMode::Sequential => {
                let mut results = Vec::with_capacity(misses.len());
                for (chart, _, _) in &misses {
                    results.push(lower_one(chart.clone()).await);
                }
                results
            }
            ProjectCompilationMode::Parallel => {
                futures::future::join_all(
                    misses.iter().map(|(chart, _, _)| lower_one(chart.clone())),
                )
                .await
            }
        };

        let mut completed = Vec::new();
        let mut diagnostics = Vec::new();
        for ((_, public_id, key), result) in misses.into_iter().zip(results) {
            match result {
                Ok(mut chart) => {
                    chart.artifact.dependency_fingerprint = key.dependency_fingerprint.clone();
                    artifacts.insert(public_id.clone(), chart.artifact.clone());
                    completed.push((public_id, key, chart.artifact));
                }
                Err(mut errors) => diagnostics.append(&mut errors),
            }
        }
        if !diagnostics.is_empty() {
            sort_diagnostics(&mut diagnostics, &project.sources);
            return Err(CompileFailure { diagnostics });
        }

        // One synchronous publication point makes project compilation
        // cancellation-safe: dropping the future before this point cannot
        // expose a partially compiled chart set.
        {
            let mut cache = self
                .artifact_cache
                .lock()
                .expect("artifact cache lock poisoned");
            let current_ids = project
                .charts
                .iter()
                .map(|id| ProjectChartId::new(id.as_str()))
                .collect::<BTreeSet<_>>();
            let removed = cache
                .chart_keys
                .keys()
                .filter(|id| !current_ids.contains(*id))
                .cloned()
                .collect::<Vec<_>>();
            for id in removed {
                if let Some(key) = cache.chart_keys.remove(&id) {
                    cache.artifacts.remove(&key);
                }
            }
            for (id, key, artifact) in completed {
                if let Some(previous) = cache.chart_keys.insert(id, key.clone())
                    && previous != key
                {
                    cache.artifacts.remove(&previous);
                }
                cache.artifacts.insert(key, artifact);
            }
        }

        let charts = project
            .charts
            .iter()
            .filter_map(|id| {
                let id = ProjectChartId::new(id.as_str());
                artifacts.remove(&id).map(|artifact| (id, artifact))
            })
            .collect();
        Ok(CompiledProject {
            charts,
            sources: project.sources.clone(),
            native_registry_profile: profile.clone(),
            project_fingerprint: project_fingerprint_from_layers(&fingerprints),
            dependency_fingerprints: fingerprints,
        })
    }

    pub async fn analyze_project(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<ProjectAnalysis, CompileFailure> {
        let attempt = self.load_project_graph_attempt(root).await;
        let dependencies = attempt.dependencies;
        let parsed = attempt.result?;
        let project = self
            .resolve_parsed_project_attempt(CompileAttempt {
                result: Ok(parsed.clone()),
                dependencies: dependencies.clone(),
            })
            .result?;
        let (environment, catalog) = self.analyze_resolved_project(&project, 0).await?;
        let mut fingerprints = dependency_fingerprint_layers(
            &self.options,
            &parsed,
            &project,
            &dependencies,
            &catalog,
            &environment,
        );
        let project_fingerprint = project_fingerprint_from_layers(&fingerprints);
        let analysis_cache_key = format!(
            "{}\0{}",
            self.options.native_registry.profile_id().as_str(),
            project_fingerprint.as_str()
        );
        if let Some(cached) = self
            .analysis_cache
            .lock()
            .expect("analysis cache lock poisoned")
            .get(&analysis_cache_key)
            .cloned()
        {
            return Ok(cached);
        }
        let mut analysis = ProjectAnalysis::empty(
            project.sources.clone(),
            self.options.native_registry.profile_id().clone(),
            project_fingerprint,
        );
        analysis.lineage = catalog.lineage;
        let mut pending_datasets = BTreeMap::new();
        for (_, dataset) in catalog.datasets.iter() {
            let span = dataset.provenance.stage_span;
            let fingerprint = catalog
                .dataset_fingerprints
                .get(&dataset.stage)
                .cloned()
                .unwrap_or_else(|| analyzed_dataset_fingerprint(dataset));
            let dataset = self.cached_dataset(dataset.clone(), &fingerprint, &mut pending_datasets);
            analysis
                .datasets
                .insert(dataset)
                .map_err(|error| CompileFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-DATA-070",
                        "catalog dataset analysis indexing failed",
                        SourceLabel::new(span, error.to_string()),
                    )],
                })?;
        }
        let chart_datasets = analyze_chart_datasets(
            &project,
            self.options.native_registry.as_ref(),
            environment.session_context(),
            self.options.source_loader.as_ref(),
            &self.options.import_capabilities,
        )
        .await
        .map_err(|diagnostics| CompileFailure { diagnostics })?;
        let mut ordinals = BTreeMap::new();
        let mut previous = BTreeMap::new();
        for dataset in chart_datasets {
            let id = ProjectDatasetId::new(format!("chart:{}", dataset.dataset.as_str()));
            let ordinal = ordinals.entry(dataset.dataset.clone()).or_insert(0_u32);
            let stage = DatasetStageId::new(id.clone(), *ordinal);
            *ordinal += 1;
            let upstream_stages = previous
                .insert(dataset.dataset.clone(), stage.clone())
                .into_iter()
                .collect();
            let dataset = AnalyzedDataset {
                id,
                stage: stage.clone(),
                provenance: DatasetProvenance {
                    declaration_span: dataset.declaration_span,
                    stage_span: dataset.stage_span,
                    stage_kind: dataset.stage_kind,
                },
                qualified_name: Some(format!("chart:{}", dataset.dataset.as_str())),
                columns: dataset.columns,
                schema: dataset.schema,
                logical_plan_fingerprint: dataset.logical_plan_fingerprint,
            };
            let fingerprint = analyzed_dataset_fingerprint(&dataset);
            let stage_span = dataset.provenance.stage_span;
            fingerprints
                .datasets
                .insert(dataset.stage.clone(), fingerprint.clone());
            let dataset = self.cached_dataset(dataset, &fingerprint, &mut pending_datasets);
            analysis
                .datasets
                .insert(dataset)
                .map_err(|error| CompileFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-DATA-070",
                        "chart dataset analysis indexing failed",
                        SourceLabel::new(stage_span, error.to_string()),
                    )],
                })?;
            analysis
                .lineage
                .insert(
                    stage,
                    DatasetLineage {
                        upstream_stages,
                        columns: Vec::new(),
                    },
                )
                .map_err(|error| CompileFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-DATA-070",
                        "chart dataset lineage indexing failed",
                        SourceLabel::new(stage_span, error.to_string()),
                    )],
                })?;
        }
        analysis.dependency_fingerprints = fingerprints;
        self.dataset_analysis_cache
            .lock()
            .expect("dataset-analysis cache lock poisoned")
            .extend(pending_datasets);
        self.analysis_cache
            .lock()
            .expect("analysis cache lock poisoned")
            .insert(analysis_cache_key, analysis.clone());
        Ok(analysis)
    }

    fn cached_dataset(
        &self,
        dataset: AnalyzedDataset,
        fingerprint: &DependencyFingerprint,
        pending: &mut BTreeMap<String, AnalyzedDataset>,
    ) -> AnalyzedDataset {
        let key = format!(
            "{}\0{}",
            self.options.native_registry.profile_id().as_str(),
            fingerprint.as_str()
        );
        if let Some(cached) = self
            .dataset_analysis_cache
            .lock()
            .expect("dataset-analysis cache lock poisoned")
            .get(&key)
            .cloned()
        {
            cached
        } else {
            pending.insert(key, dataset.clone());
            dataset
        }
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
        let project = self.load_file_project_attempt(path).await.result?;
        let resolved = resolve_semantics(&project, self.host.authoring_schema())
            .result
            .map_err(|failure| CompileFailure {
                diagnostics: failure.diagnostics,
            })?;
        let expanded = expand_project(&project, &resolved).map_err(|failure| CompileFailure {
            diagnostics: failure.diagnostics,
        })?;
        let root = project.chart_roots.first().ok_or_else(|| CompileFailure {
            diagnostics: vec![Diagnostic::error(
                "AVENGER-EXPAND-003",
                "source expansion requires one chart root",
                SourceLabel::new(
                    SourceSpan::empty(SourceId::new(0), 0),
                    "the loaded project has no chart root",
                ),
            )],
        })?;
        let text = expanded
            .texts
            .get(root)
            .cloned()
            .ok_or_else(|| CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-EXPAND-004",
                    "expanded chart source is missing",
                    SourceLabel::new(
                        SourceSpan::empty(SourceId::new(0), 0),
                        format!("no expansion was emitted for `{}`", root.as_str()),
                    ),
                )],
            })?;
        Ok(ExpandedSource {
            text,
            sources: expanded.project.sources,
            source_map: expanded.source_map,
        })
    }

    /// Phase 4 frontend seam: load and semantically resolve one chart and its
    /// complete dependency closure without constructing native chart objects.
    pub async fn resolve_file_project_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<ResolvedProject> {
        let attempt = self.load_file_project_attempt(path).await;
        self.resolve_parsed_project_attempt(attempt)
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
        self.resolve_parsed_project_attempt(attempt)
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
                &self.options.data_capabilities,
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

    fn resolve_parsed_project_attempt(
        &self,
        attempt: CompileAttempt<ParsedProject>,
    ) -> CompileAttempt<ResolvedProject> {
        let dependencies = attempt.dependencies;
        let result = attempt.result.and_then(|project| {
            if let Some(cached) = self
                .resolved_project_cache
                .lock()
                .expect("resolved-project cache lock poisoned")
                .get(&project.fingerprint)
                .cloned()
            {
                return Ok(cached);
            }
            let fingerprint = project.fingerprint.clone();
            let resolved = resolve_parsed_project(project, self.host.authoring_schema())?;
            self.resolved_project_cache
                .lock()
                .expect("resolved-project cache lock poisoned")
                .insert(fingerprint, resolved.clone());
            Ok(resolved)
        });
        CompileAttempt {
            result,
            dependencies,
        }
    }

    /// Legacy programmatic Phase 0 harness retained as a small registry and
    /// artifact regression fixture. Production callers compile real DSL files.
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
            NativeResolvedDeclaration::new("symbol")
                .property("x", NativeResolvedValue::Expr(col("x")))
                .property("y", NativeResolvedValue::Expr(col("y")))
                .into(),
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

    async fn lower_resolved_project(
        &self,
        project: &ResolvedProject,
        generation: u64,
    ) -> Result<(LoweredProject, crate::CompileEnvironment, CatalogAnalysis), CompileFailure> {
        let (environment, catalog) = self.analyze_resolved_project(project, generation).await?;
        let mut lowered = lower_project(
            project,
            self.options.native_registry.as_ref(),
            environment.session_context(),
            self.options.source_loader.as_ref(),
            &self.options.import_capabilities,
        )
        .await
        .map_err(|diagnostics| CompileFailure { diagnostics })?;
        for chart in &mut lowered.charts {
            chart.artifact.dependency_fingerprint =
                DependencyFingerprint::new(catalog.dependency_fingerprint.clone());
        }
        Ok((lowered, environment, catalog))
    }

    async fn analyze_resolved_project(
        &self,
        project: &ResolvedProject,
        generation: u64,
    ) -> Result<(crate::CompileEnvironment, CatalogAnalysis), CompileFailure> {
        let request = CompileEnvironmentRequest {
            generation,
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
            .map_err(|error| CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-LOWER-005",
                    "compile environment creation failed",
                    SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), error.to_string()),
                )],
            })?;
        let catalog = register_and_analyze_catalog(
            project,
            &environment,
            CatalogOptions {
                project_root: &self.options.project_root,
                capabilities: &self.options.data_capabilities,
                environment_provider: self.options.environment.as_ref(),
                catalog_factories: &self.options.catalog_factories,
                table_factories: &self.options.table_factories,
            },
        )
        .await
        .map_err(|diagnostic| CompileFailure {
            diagnostics: vec![diagnostic],
        })?;
        Ok((environment, catalog))
    }
}

#[derive(Default)]
pub struct CompilerBuilder {
    project_root: Option<PathBuf>,
    import_capabilities: Option<ImportCapabilities>,
    data_capabilities: Option<DataCapabilities>,
    environment: Option<Arc<dyn EnvironmentProvider>>,
    native_registry: Option<Arc<NativeRegistry>>,
    source_loader: Option<Arc<dyn SourceLoader>>,
    catalog_factories: CatalogFactoryRegistry,
    table_factories: TableFactoryRegistry,
    environment_factory: Option<Arc<dyn CompileEnvironmentFactory>>,
    project_compilation_mode: ProjectCompilationMode,
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
        self.data_capabilities = Some(capabilities);
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

    pub fn table_factories(mut self, table_factories: TableFactoryRegistry) -> Self {
        self.table_factories = table_factories;
        self
    }

    pub fn environment_factory(
        mut self,
        environment_factory: Arc<dyn CompileEnvironmentFactory>,
    ) -> Self {
        self.environment_factory = Some(environment_factory);
        self
    }

    pub fn project_compilation_mode(mut self, mode: ProjectCompilationMode) -> Self {
        self.project_compilation_mode = mode;
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
            None => Arc::new(builtins::stock_registry()?),
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
            data_capabilities: self
                .data_capabilities
                .unwrap_or_else(DataCapabilities::project),
            environment: self
                .environment
                .unwrap_or_else(|| Arc::new(EmptyEnvironmentProvider)),
            native_registry: registry.clone(),
            source_loader,
            catalog_factories: self.catalog_factories,
            table_factories: self.table_factories,
            environment_factory: self
                .environment_factory
                .unwrap_or_else(|| Arc::new(DefaultCompileEnvironmentFactory)),
        };
        Ok(Compiler {
            options: Arc::new(options),
            host: LanguageHost::new(registry),
            resolved_project_cache: Arc::new(Mutex::new(BTreeMap::new())),
            analysis_cache: Arc::new(Mutex::new(BTreeMap::new())),
            dataset_analysis_cache: Arc::new(Mutex::new(BTreeMap::new())),
            artifact_cache: Arc::new(Mutex::new(ArtifactCache::default())),
            project_compilation_mode: self.project_compilation_mode,
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

fn resolve_parsed_project(
    project: ParsedProject,
    schema: &avenger_chart_schema::NativeSchemaSnapshot,
) -> Result<ResolvedProject, CompileFailure> {
    let resolved = resolve_semantics(&project, schema)
        .result
        .map_err(|failure| CompileFailure {
            diagnostics: failure.diagnostics,
        })?;
    if resolved.definitions.is_empty() {
        return Ok(resolved);
    }
    let expanded = expand_project(&project, &resolved).map_err(|failure| CompileFailure {
        diagnostics: failure.diagnostics,
    })?;
    match resolve_semantics(&expanded.project, schema).result {
        Ok(mut resolved) => {
            resolved.expansion_source_map = expanded.source_map;
            Ok(resolved)
        }
        Err(mut failure) => {
            expanded
                .source_map
                .remap_diagnostics(&mut failure.diagnostics);
            Err(CompileFailure {
                diagnostics: failure.diagnostics,
            })
        }
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
    import_capabilities: &ImportCapabilities,
    data_capabilities: &DataCapabilities,
    dependencies: &mut DiscoveredDependencySet,
) -> Result<(), CompileFailure> {
    for file in project.files.values() {
        for declaration in file.parsed.ast.root.declarations() {
            discover_declaration_resources(
                declaration,
                file.source,
                &file.origin,
                project_root,
                import_capabilities,
                data_capabilities,
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
    import_capabilities: &ImportCapabilities,
    data_capabilities: &DataCapabilities,
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
                data_capabilities.allow_filesystem,
                dependencies,
            )?;
        }
    }
    if declaration.keyword.as_str() == "theme"
        && let Some(Value::Str(path)) = declaration.props.get("from")
    {
        discover_resource(
            path,
            source,
            declaring_origin,
            project_root,
            import_capabilities.allow_filesystem,
            dependencies,
        )?;
    }
    for child in &declaration.children {
        discover_declaration_resources(
            child,
            source,
            declaring_origin,
            project_root,
            import_capabilities,
            data_capabilities,
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
    allow_filesystem: bool,
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
    dependencies.insert_owned(source, dependency.clone());
    let SourceOrigin::File(candidate) = origin else {
        return Ok(());
    };
    let normalized_root = normalize_path(project_root);
    let normalized_candidate = normalize_path(&candidate);
    if !allow_filesystem || !normalized_candidate.starts_with(&normalized_root) {
        return Err(resource_failure(
            source,
            "local data resource is outside the project capability root",
            normalized_candidate.display().to_string(),
        ));
    }
    if path.contains(['*', '?', '[']) {
        dependency.content_version = Some(glob_content_version(&normalized_candidate).map_err(
            |error| resource_failure(source, "local data glob could not be fingerprinted", error),
        )?);
        dependencies.insert_owned(source, dependency);
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
    dependencies.insert_owned(source, dependency);
    Ok(())
}

pub(crate) fn resource_content_version(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    if path.is_dir() {
        let mut files = Vec::new();
        collect_directory_files(path, path, &mut files)?;
        let mut hasher = Sha256::new();
        hasher.update(b"avenger-directory-v1\0");
        for file in files {
            hasher.update(
                file.strip_prefix(path)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .as_bytes(),
            );
            hasher.update(b"\0");
            hasher.update(std::fs::read(file)?);
            hasher.update(b"\0");
        }
        return Ok(format!("directory-sha256:{:x}", hasher.finalize()));
    }
    let bytes = std::fs::read(path)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn collect_directory_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    let mut entries = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            let canonical = std::fs::canonicalize(&path)?;
            if !canonical.starts_with(root) {
                return Err(std::io::Error::other(format!(
                    "directory member `{}` escapes through a symlink",
                    path.display()
                )));
            }
        }
        if metadata.is_dir() {
            collect_directory_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn glob_content_version(pattern: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let pattern = pattern.to_string_lossy();
    let mut matches = glob::glob(&pattern)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    matches.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-glob-v1\0");
    for path in matches {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        let version = resource_content_version(&path).map_err(|error| error.to_string())?;
        hasher.update(version.as_bytes());
        hasher.update(b"\0");
    }
    Ok(format!("glob-sha256:{:x}", hasher.finalize()))
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

fn source_dependency_fingerprints(
    project: &ParsedProject,
    dependencies: &DiscoveredDependencySet,
) -> BTreeMap<ProjectFileId, DependencyFingerprint> {
    project
        .files
        .iter()
        .map(|(id, file)| {
            let mut parts = vec![
                id.as_str().to_owned(),
                file.origin.canonical_uri(),
                file.content_sha256.clone(),
                file.content_version.clone(),
            ];
            for dependency in dependencies.owned_by(file.source) {
                parts.push(dependency.canonical_origin.canonical_uri());
                parts.push(dependency.content_version.clone().unwrap_or_default());
            }
            (
                id.clone(),
                DependencyFingerprint::new(stable_hash(
                    "avenger-source-dependency-v1",
                    parts.iter().map(String::as_str),
                )),
            )
        })
        .collect()
}

fn compiler_options_fingerprint(options: &CompilerOptions) -> String {
    let import_capabilities =
        serde_json::to_string(&options.import_capabilities).unwrap_or_default();
    let data_capabilities = serde_json::to_string(&options.data_capabilities).unwrap_or_default();
    let catalog_factories = options
        .catalog_factories
        .kinds()
        .collect::<Vec<_>>()
        .join("\0");
    let table_factories = options
        .table_factories
        .kinds()
        .collect::<Vec<_>>()
        .join("\0");
    stable_hash(
        "avenger-compiler-options-v1",
        [
            options.native_registry.profile_id().as_str(),
            import_capabilities.as_str(),
            data_capabilities.as_str(),
            catalog_factories.as_str(),
            table_factories.as_str(),
        ],
    )
}

fn chart_file<'a>(
    project: &'a ResolvedProject,
    chart: &DeclarationId,
) -> Option<&'a ProjectFileId> {
    project.files.iter().find_map(|(file_id, file)| {
        file.roots
            .iter()
            .any(|root| declaration_contains(root, chart))
            .then_some(file_id)
    })
}

fn declaration_contains(declaration: &ResolvedDeclaration, target: &DeclarationId) -> bool {
    &declaration.id == target
        || declaration
            .children
            .iter()
            .any(|child| declaration_contains(child, target))
}

fn definition_closure_fingerprint(
    project: &ParsedProject,
    root: &ProjectFileId,
    sources: &BTreeMap<ProjectFileId, DependencyFingerprint>,
) -> DependencyFingerprint {
    let source_to_file = project
        .files
        .iter()
        .map(|(id, file)| (file.source, id))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = BTreeMap::<ProjectFileId, BTreeSet<ProjectFileId>>::new();
    for edge in &project.imports {
        let (Some(importer), Some(imported)) = (
            source_to_file.get(&edge.importer),
            source_to_file.get(&edge.imported),
        ) else {
            continue;
        };
        adjacency
            .entry((*importer).clone())
            .or_default()
            .insert((*imported).clone());
    }
    let mut pending = vec![root.clone()];
    let mut included = BTreeSet::new();
    while let Some(file) = pending.pop() {
        if !included.insert(file.clone()) {
            continue;
        }
        for imported in adjacency.get(&file).into_iter().flatten() {
            let include = project
                .files
                .get(imported)
                .is_some_and(|file| matches!(file.kind, ProjectFileKind::Definition(_)));
            if include {
                pending.push(imported.clone());
            }
        }
    }
    let mut parts = Vec::new();
    for file in included {
        parts.push(file.as_str().to_owned());
        if let Some(fingerprint) = sources.get(&file) {
            parts.push(fingerprint.as_str().to_owned());
        }
    }
    DependencyFingerprint::new(stable_hash(
        "avenger-definition-closure-v1",
        parts.iter().map(String::as_str),
    ))
}

fn collect_chart_catalog_dependencies(
    project: &ResolvedProject,
    chart: &ResolvedDeclaration,
) -> (BTreeSet<DeclarationId>, BTreeSet<String>) {
    fn visit_target(target: &ResolvedTarget, tables: &mut BTreeSet<DeclarationId>) {
        if let ResolvedTarget::Declaration(id) = target {
            tables.insert(id.clone());
        }
    }
    fn visit_value(
        project: &ResolvedProject,
        property: Option<&str>,
        value: &ResolvedValue,
        tables: &mut BTreeSet<DeclarationId>,
        names: &mut BTreeSet<String>,
    ) {
        match value {
            ResolvedValue::String(name) if property == Some("table") => {
                names.insert(name.clone());
                if let Some(table) = crate::catalog::resolved_table_for_name(project, name) {
                    tables.insert(table.id.clone());
                }
            }
            ResolvedValue::Binding(binding) => visit_target(&binding.target, tables),
            ResolvedValue::Reference(reference) => visit_target(&reference.target, tables),
            ResolvedValue::Expression(expression) => {
                for reference in &expression.references {
                    names.insert(reference.authored_path.join("."));
                    visit_target(&reference.target, tables);
                }
            }
            ResolvedValue::Query(query) => {
                for reference in &query.references {
                    names.insert(reference.authored_path.join("."));
                    visit_target(&reference.target, tables);
                }
            }
            ResolvedValue::Visual(value) | ResolvedValue::Pattern(value) => {
                visit_value(project, property, value, tables, names);
            }
            ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
                for value in values {
                    visit_value(project, None, value, tables, names);
                }
            }
            ResolvedValue::Object {
                head,
                properties,
                children,
                ..
            } => {
                if let Some(head) = head {
                    visit_value(project, None, head, tables, names);
                }
                for (name, value) in properties {
                    visit_value(project, Some(name), value, tables, names);
                }
                for child in children {
                    visit_declaration(project, child, tables, names);
                }
            }
            ResolvedValue::DefinitionArgument(target) => visit_target(target, tables),
            _ => {}
        }
    }
    fn visit_declaration(
        project: &ResolvedProject,
        declaration: &ResolvedDeclaration,
        tables: &mut BTreeSet<DeclarationId>,
        names: &mut BTreeSet<String>,
    ) {
        for (name, value) in &declaration.properties {
            visit_value(project, Some(name), value, tables, names);
        }
        for child in &declaration.children {
            visit_declaration(project, child, tables, names);
        }
    }

    let mut tables = BTreeSet::new();
    let mut names = BTreeSet::new();
    visit_declaration(project, chart, &mut tables, &mut names);
    let mut pending = tables.iter().cloned().collect::<Vec<_>>();
    let by_id = project
        .catalog_tables
        .values()
        .map(|table| (table.id.clone(), table))
        .collect::<BTreeMap<_, _>>();
    while let Some(id) = pending.pop() {
        if let Some(table) = by_id.get(&id) {
            for dependency in &table.dependencies {
                if tables.insert(dependency.clone()) {
                    pending.push(dependency.clone());
                }
            }
        }
    }
    (tables, names)
}

fn dependency_fingerprint_layers(
    options: &CompilerOptions,
    parsed: &ParsedProject,
    resolved: &ResolvedProject,
    dependencies: &DiscoveredDependencySet,
    catalog: &CatalogAnalysis,
    environment: &crate::CompileEnvironment,
) -> ProjectDependencyFingerprints {
    let sources = source_dependency_fingerprints(parsed, dependencies);
    let mut result = ProjectDependencyFingerprints {
        sources,
        datasets: catalog.dataset_fingerprints.clone(),
        data_catalog: DependencyFingerprint::new(catalog.dependency_fingerprint.clone()),
        compile_environment: DependencyFingerprint::new(
            environment.dependency_fingerprint().to_owned(),
        ),
        ..ProjectDependencyFingerprints::default()
    };
    let compiler_options = compiler_options_fingerprint(options);
    let declarations = resolved
        .files
        .values()
        .flat_map(|file| file.roots.iter())
        .flat_map(declarations_depth_first)
        .map(|declaration| (declaration.id.clone(), declaration))
        .collect::<BTreeMap<_, _>>();
    for chart_id in &resolved.charts {
        let public_id = ProjectChartId::new(chart_id.as_str());
        let definition = chart_file(resolved, chart_id)
            .map(|file| definition_closure_fingerprint(parsed, file, &result.sources))
            .unwrap_or_default();
        result
            .definition_closures
            .insert(public_id.clone(), definition.clone());
        let mut parts = vec![
            public_id.as_str().to_owned(),
            definition.as_str().to_owned(),
            compiler_options.clone(),
            environment.dependency_fingerprint().to_owned(),
        ];
        if let Some(chart) = declarations.get(chart_id) {
            parts.push(serde_json::to_string(chart).unwrap_or_default());
            let (tables, names) = collect_chart_catalog_dependencies(resolved, chart);
            for table in tables {
                if let Some(fingerprint) = catalog.table_fingerprints.get(&table) {
                    parts.push(fingerprint.as_str().to_owned());
                }
            }
            for name in names {
                for (stage, fingerprint) in &catalog.dataset_fingerprints {
                    let matches_name = catalog.datasets.iter().any(|(_, dataset)| {
                        &dataset.stage == stage
                            && dataset.qualified_name.as_deref() == Some(name.as_str())
                    });
                    if matches_name {
                        parts.push(fingerprint.as_str().to_owned());
                    }
                }
            }
        }
        result.charts.insert(
            public_id,
            DependencyFingerprint::new(stable_hash(
                "avenger-chart-artifact-v1",
                parts.iter().map(String::as_str),
            )),
        );
    }
    result
}

fn declarations_depth_first(
    declaration: &ResolvedDeclaration,
) -> Box<dyn Iterator<Item = &ResolvedDeclaration> + '_> {
    Box::new(
        std::iter::once(declaration).chain(
            declaration
                .children
                .iter()
                .flat_map(declarations_depth_first),
        ),
    )
}

fn project_fingerprint_from_layers(
    fingerprints: &ProjectDependencyFingerprints,
) -> ProjectFingerprint {
    let mut parts = Vec::new();
    for (source, fingerprint) in &fingerprints.sources {
        parts.push(source.as_str().to_owned());
        parts.push(fingerprint.as_str().to_owned());
    }
    parts.push(fingerprints.data_catalog.as_str().to_owned());
    parts.push(fingerprints.compile_environment.as_str().to_owned());
    for (chart, fingerprint) in &fingerprints.charts {
        parts.push(chart.as_str().to_owned());
        parts.push(fingerprint.as_str().to_owned());
    }
    ProjectFingerprint::new(stable_hash(
        "avenger-compiled-project-v1",
        parts.iter().map(String::as_str),
    ))
}

fn analyzed_dataset_fingerprint(dataset: &AnalyzedDataset) -> DependencyFingerprint {
    let mut parts = vec![
        dataset.id.as_str().to_owned(),
        dataset.stage.ordinal.to_string(),
        dataset.provenance.stage_span.source.get().to_string(),
        dataset.provenance.stage_span.range.start.to_string(),
        dataset.provenance.stage_span.range.end.to_string(),
        format!("{:?}", dataset.provenance.stage_kind),
        dataset.qualified_name.clone().unwrap_or_default(),
        dataset.logical_plan_fingerprint.clone().unwrap_or_default(),
    ];
    for column in &dataset.columns {
        parts.push(column.qualifier.clone().unwrap_or_default());
        parts.push(column.name.clone());
        parts.push(column.data_type.to_string());
        parts.push(column.nullable.to_string());
    }
    DependencyFingerprint::new(stable_hash(
        "avenger-dataset-analysis-v1",
        parts.iter().map(String::as_str),
    ))
}

fn stable_hash<'a>(domain: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
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
