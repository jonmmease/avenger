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
    AvailableNativeModule, BundleTarget, BundledSource, ChartEntrypointId, ChartSelector,
    DataCapabilities, DeclarationId, Diagnostic, EmptyEnvironmentProvider, EnvironmentProvider,
    ExpansionSourceMap, ImportCapabilities, ModuleDependencyRole, ModuleDependencyTarget,
    ModuleGraphLoadAttempt, ModuleGraphLoadRequest, ModuleGraphLoader, ModuleId, ModuleRoot,
    ParsedModuleGraph, ResolvedDeclaration, ResolvedKindBinding, ResolvedModuleGraph,
    ResolvedRelationTarget, ResolvedTarget, ResolvedValue, SourceId, SourceLabel, SourceLoader,
    SourceLoaderError, SourceMap, SourceModuleId, SourceOrigin, SourceSpan,
    ast::{Decl, Value},
    bundle_module_graph, expand_module_graph_with_limits,
    module_graph::{normalize_path, resolve_relative_origin},
    resolve_module_graph as resolve_semantics, sort_diagnostics,
};
use datafusion::logical_expr::col;
use serde::{Deserialize, Serialize};

use crate::{
    AnalyzedDataset, ArtifactCacheKey, CatalogFactoryRegistry, CompileEnvironmentFactory,
    CompileEnvironmentRequest, CompileEnvironmentResourceVersion, CompiledChartArtifact,
    CompiledModule, CompilerLimits, CompilerOptions, DatasetLineage, DatasetProvenance,
    DatasetStageId, DatasetStageKind, DefaultCompileEnvironmentFactory, DefaultSourceLoader,
    DependencyFingerprint, FunctionCategory, FunctionMetadata, LanguageHost, LocalResourceLimits,
    ModuleAnalysis, ModuleDatasetId, ModuleDependencyFingerprints, ModuleFingerprint,
    NativeRequirementSet, SourceLoaderLimits, TableFactoryRegistry,
    catalog::{CatalogAnalysis, CatalogOptions, register_and_analyze_catalog},
    lowering::{PreparedParams, analyze_chart_datasets, lower_module_chart, prepare_module_params},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModuleCompilationMode {
    Sequential,
    #[default]
    Parallel,
}

/// Read-only cache telemetry intended for tests, hosts, and future inspector
/// tooling. It exposes immutable identities and counts, never cached sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompilerCacheSnapshot {
    pub resolved_module_graphs: usize,
    pub module_analyses: usize,
    pub dataset_analyses: usize,
    pub chart_artifacts: usize,
    pub artifact_keys: Vec<ArtifactCacheKey>,
}

#[derive(Default)]
struct ArtifactCache {
    artifacts: BTreeMap<ArtifactCacheKey, CompiledChartArtifact>,
    chart_keys: BTreeMap<ChartEntrypointId, ArtifactCacheKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRole {
    RequestedModule,
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
    pub sources: SourceMap,
}

impl CompileFailure {
    fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            sources: SourceMap::default(),
        }
    }

    fn with_sources(mut self, sources: SourceMap) -> Self {
        self.sources = sources;
        self
    }

    /// Render one deterministic, source-aware diagnostic batch.
    pub fn render(&self) -> String {
        avenger_lang_core::render_diagnostics(&self.diagnostics, &self.sources)
    }
}

impl std::fmt::Display for CompileFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.diagnostics.first() {
            Some(diagnostic) => write!(
                f,
                "{}[{}]: {}",
                match diagnostic.severity {
                    avenger_lang_core::DiagnosticSeverity::Error => "error",
                    avenger_lang_core::DiagnosticSeverity::Warning => "warning",
                    avenger_lang_core::DiagnosticSeverity::Information => "information",
                    avenger_lang_core::DiagnosticSeverity::Hint => "hint",
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
    resolved_module_graph_cache: Arc<Mutex<BTreeMap<String, ResolvedModuleGraph>>>,
    analysis_cache: Arc<Mutex<BTreeMap<String, ModuleAnalysis>>>,
    dataset_analysis_cache: Arc<Mutex<BTreeMap<String, AnalyzedDataset>>>,
    artifact_cache: Arc<Mutex<ArtifactCache>>,
    module_compilation_mode: ModuleCompilationMode,
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
            resolved_module_graphs: self
                .resolved_module_graph_cache
                .lock()
                .expect("resolved-module-graph cache lock poisoned")
                .len(),
            module_analyses: self
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

    /// Bound the immutable frontend caches retained by long-lived editor hosts.
    ///
    /// Eviction affects performance only: every entry is content-addressed and
    /// can be reconstructed from the next immutable module-graph snapshot.
    pub fn trim_editor_caches(&self, max_module_entries: usize, max_dataset_entries: usize) {
        let max_module_entries = max_module_entries.max(1);
        let max_dataset_entries = max_dataset_entries.max(1);
        let mut resolved = self
            .resolved_module_graph_cache
            .lock()
            .expect("resolved-module-graph cache lock poisoned");
        while resolved.len() > max_module_entries {
            resolved.pop_first();
        }
        drop(resolved);
        let mut analyses = self
            .analysis_cache
            .lock()
            .expect("analysis cache lock poisoned");
        while analyses.len() > max_module_entries {
            analyses.pop_first();
        }
        drop(analyses);
        let mut datasets = self
            .dataset_analysis_cache
            .lock()
            .expect("dataset-analysis cache lock poisoned");
        while datasets.len() > max_dataset_entries {
            datasets.pop_first();
        }
    }

    /// Fork compiler inputs for one immutable editor snapshot while sharing
    /// only content-addressed compiler caches with the parent.
    pub fn fork_with_source_loader(&self, source_loader: Arc<dyn SourceLoader>) -> Self {
        let mut options = (*self.options).clone();
        options.source_loader = source_loader;
        let mut fork = self.clone();
        fork.options = Arc::new(options);
        fork
    }

    pub async fn compile_chart_attempt(
        &self,
        path: impl AsRef<Path>,
        selector: Option<&str>,
    ) -> CompileAttempt<CompiledChartArtifact> {
        let attempt = self
            .compile_chart_generation_attempt(path, selector, 0)
            .await;
        CompileAttempt {
            result: attempt.result.map(|generation| generation.artifact),
            dependencies: attempt.dependencies,
        }
    }

    pub async fn compile_chart_generation_attempt(
        &self,
        path: impl AsRef<Path>,
        selector: Option<&str>,
        generation: u64,
    ) -> CompileAttempt<CompiledChartGeneration> {
        let attempt = self.load_module_graph_attempt(path).await;
        let dependencies = attempt.dependencies;
        let result = match attempt.result {
            Ok(parsed) => match self
                .resolve_parsed_module_graph_attempt(CompileAttempt {
                    result: Ok(parsed.clone()),
                    dependencies: dependencies.clone(),
                })
                .result
            {
                Ok(project) => match select_chart_entrypoint(&parsed, &project, selector) {
                    Ok(entrypoint) => {
                        async {
                            let (environment, catalog, prepared_params) = self
                                .analyze_resolved_module_graph(
                                    &project,
                                    generation,
                                    &dependencies,
                                    Some(&project.entrypoints[&entrypoint].reachable_items),
                                )
                                .await?;
                            let fingerprints = dependency_fingerprint_layers(
                                &self.options,
                                &parsed,
                                &project,
                                &dependencies,
                                &catalog,
                                &environment,
                            );
                            let lowered = lower_module_chart(
                                &project,
                                &entrypoint,
                                self.options.native_registry.as_ref(),
                                environment.session_context(),
                                self.options.source_loader.as_ref(),
                                &self.options.import_capabilities,
                                &prepared_params,
                            )
                            .await
                            .map_err(|diagnostics| CompileFailure {
                                diagnostics,
                                sources: project.sources.clone(),
                            })?;
                            let mut artifact = lowered.artifact;
                            artifact.native_requirements = native_requirements_for_entrypoint(
                                &project,
                                &entrypoint,
                                self.options.native_registry.as_ref(),
                            );
                            if let Some(fingerprint) = fingerprints.charts.get(&artifact.id) {
                                artifact.dependency_fingerprint = fingerprint.clone();
                            }
                            Ok(CompiledChartGeneration {
                                generation,
                                artifact,
                                environment,
                            })
                        }
                        .await
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        CompileAttempt {
            result,
            dependencies,
        }
    }

    pub async fn compile_chart(
        &self,
        path: impl AsRef<Path>,
        selector: Option<&str>,
    ) -> Result<CompiledChartArtifact, CompileFailure> {
        self.compile_chart_attempt(path, selector).await.result
    }

    pub async fn compile_module_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<CompiledModule> {
        let attempt = self.load_module_graph_attempt(path).await;
        let dependencies = attempt.dependencies;
        let result = match attempt.result {
            Ok(parsed) => {
                let requested = parsed.requested_modules.first().cloned();
                let resolved = self
                    .resolve_parsed_module_graph_attempt(CompileAttempt {
                        result: Ok(parsed.clone()),
                        dependencies: dependencies.clone(),
                    })
                    .result;
                match resolved {
                    Ok(resolved) => match requested {
                        Some(requested) => {
                            self.compile_resolved_module(
                                &parsed,
                                &resolved,
                                &requested,
                                &dependencies,
                                0,
                            )
                            .await
                        }
                        None => Err(module_root_failure(
                            &resolved.sources,
                            "the loaded graph has no requested module",
                        )),
                    },
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

    pub async fn compile_module(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<CompiledModule, CompileFailure> {
        self.compile_module_attempt(path).await.result
    }

    /// Flatten one selected chart and its reachable source-item closure into a
    /// canonical single source module.
    pub async fn bundle_chart(
        &self,
        path: impl AsRef<Path>,
        selector: Option<&str>,
    ) -> Result<BundledSource, CompileFailure> {
        let attempt = self.load_module_graph_attempt(path).await;
        let parsed = attempt.result?;
        let resolved = resolve_semantics(&parsed, self.host.authoring_schema())
            .result
            .map_err(|failure| CompileFailure {
                diagnostics: failure.diagnostics,
                sources: parsed.sources.clone(),
            })?;
        let entrypoint = select_chart_entrypoint(&parsed, &resolved, selector)?;
        bundle_module_graph(&parsed, &resolved, BundleTarget::Chart(entrypoint)).map_err(
            |failure| CompileFailure {
                diagnostics: failure.diagnostics,
                sources: failure.sources,
            },
        )
    }

    /// Flatten the complete public/private interface of the requested module
    /// and every source item it reaches.
    pub async fn bundle_module(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<BundledSource, CompileFailure> {
        let attempt = self.load_module_graph_attempt(path).await;
        let parsed = attempt.result?;
        let requested = parsed.requested_modules.first().cloned().ok_or_else(|| {
            module_root_failure(&parsed.sources, "the loaded graph has no requested module")
        })?;
        let resolved = resolve_semantics(&parsed, self.host.authoring_schema())
            .result
            .map_err(|failure| CompileFailure {
                diagnostics: failure.diagnostics,
                sources: parsed.sources.clone(),
            })?;
        bundle_module_graph(&parsed, &resolved, BundleTarget::Module(requested)).map_err(
            |failure| CompileFailure {
                diagnostics: failure.diagnostics,
                sources: failure.sources,
            },
        )
    }

    async fn compile_resolved_module(
        &self,
        parsed: &ParsedModuleGraph,
        project: &ResolvedModuleGraph,
        requested: &SourceModuleId,
        dependencies: &DiscoveredDependencySet,
        generation: u64,
    ) -> Result<CompiledModule, CompileFailure> {
        let exports = project
            .source_modules
            .get(requested)
            .map(|module| module.exports.clone())
            .ok_or_else(|| {
                module_root_failure(
                    &project.sources,
                    format!(
                        "requested source module `{}` is missing",
                        requested.as_str()
                    ),
                )
            })?;
        let reachable_items = project
            .entrypoints
            .iter()
            .filter(|(entrypoint, _)| &entrypoint.module == requested)
            .flat_map(|(_, entrypoint)| entrypoint.reachable_items.iter().cloned())
            .collect::<BTreeSet<_>>();
        let (environment, catalog, prepared_params) = self
            .analyze_resolved_module_graph(
                project,
                generation,
                dependencies,
                Some(&reachable_items),
            )
            .await?;
        let analysis = self
            .finish_module_analysis(
                parsed,
                project,
                dependencies,
                &environment,
                &catalog,
                &prepared_params,
            )
            .await?;
        let fingerprints = analysis.dependency_fingerprints.clone();
        let module_fingerprint = analysis.module_fingerprint;
        let mut artifacts = BTreeMap::<ChartEntrypointId, CompiledChartArtifact>::new();
        let mut misses = Vec::<(ChartEntrypointId, NativeRequirementSet, ArtifactCacheKey)>::new();
        {
            let cache = self
                .artifact_cache
                .lock()
                .expect("artifact cache lock poisoned");
            for public_id in project
                .entrypoints
                .keys()
                .filter(|entrypoint| &entrypoint.module == requested)
            {
                let fingerprint = fingerprints
                    .charts
                    .get(public_id)
                    .cloned()
                    .unwrap_or_default();
                let fingerprint = chart_artifact_cache_fingerprint(
                    &fingerprint,
                    project,
                    public_id,
                    &prepared_params,
                );
                let requirements = native_requirements_for_entrypoint(
                    project,
                    public_id,
                    self.options.native_registry.as_ref(),
                );
                let key = ArtifactCacheKey::new(&requirements, fingerprint);
                if let Some(artifact) = cache.artifacts.get(&key).cloned() {
                    artifacts.insert(public_id.clone(), artifact);
                } else {
                    misses.push((public_id.clone(), requirements, key));
                }
            }
        }

        let prepared_params = &prepared_params;
        let lower_one = |entrypoint: ChartEntrypointId| {
            let chart_environment = environment.fork();
            async move {
                lower_module_chart(
                    project,
                    &entrypoint,
                    self.options.native_registry.as_ref(),
                    chart_environment.session_context(),
                    self.options.source_loader.as_ref(),
                    &self.options.import_capabilities,
                    prepared_params,
                )
                .await
            }
        };
        let results = match self.module_compilation_mode {
            ModuleCompilationMode::Sequential => {
                let mut results = Vec::with_capacity(misses.len());
                for (entrypoint, _, _) in &misses {
                    results.push(lower_one(entrypoint.clone()).await);
                }
                results
            }
            ModuleCompilationMode::Parallel => {
                futures::future::join_all(
                    misses
                        .iter()
                        .map(|(entrypoint, _, _)| lower_one(entrypoint.clone())),
                )
                .await
            }
        };

        let mut completed = Vec::new();
        let mut diagnostics = Vec::new();
        for ((public_id, requirements, key), result) in misses.into_iter().zip(results) {
            match result {
                Ok(mut chart) => {
                    chart.artifact.native_requirements = requirements;
                    chart.artifact.dependency_fingerprint = fingerprints
                        .charts
                        .get(&public_id)
                        .cloned()
                        .unwrap_or_default();
                    artifacts.insert(public_id.clone(), chart.artifact.clone());
                    completed.push((public_id, key, chart.artifact));
                }
                Err(mut errors) => diagnostics.append(&mut errors),
            }
        }
        if !diagnostics.is_empty() {
            sort_diagnostics(&mut diagnostics, &project.sources);
            return Err(CompileFailure {
                diagnostics,
                sources: project.sources.clone(),
            });
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
                .entrypoints
                .keys()
                .filter(|entrypoint| &entrypoint.module == requested)
                .cloned()
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
            .entrypoints
            .keys()
            .filter(|entrypoint| &entrypoint.module == requested)
            .filter_map(|id| artifacts.remove(id).map(|artifact| (id.clone(), artifact)))
            .collect::<indexmap::IndexMap<_, _>>();
        let native_requirements = NativeRequirementSet::union(
            charts
                .values()
                .map(|artifact| &artifact.native_requirements),
        )
        .unwrap_or_else(|| {
            NativeRequirementSet::builtin_only(self.options.native_registry.as_ref())
        });
        Ok(CompiledModule {
            charts,
            exports,
            sources: project.sources.clone(),
            native_requirements,
            module_fingerprint,
            dependency_fingerprints: fingerprints,
        })
    }

    pub async fn analyze_module(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ModuleAnalysis, CompileFailure> {
        let attempt = self.load_module_graph_attempt(path).await;
        let dependencies = attempt.dependencies;
        let parsed = attempt.result?;
        let project = self
            .resolve_parsed_module_graph_attempt(CompileAttempt {
                result: Ok(parsed.clone()),
                dependencies: dependencies.clone(),
            })
            .result?;
        let (environment, catalog, prepared_params) = self
            .analyze_resolved_module_graph(&project, 0, &dependencies, None)
            .await?;
        self.finish_module_analysis(
            &parsed,
            &project,
            &dependencies,
            &environment,
            &catalog,
            &prepared_params,
        )
        .await
    }

    /// Analyze an explicit immutable root inventory without filesystem
    /// discovery. Editor hosts use this with a snapshot source-loader overlay.
    pub async fn analyze_module_roots(
        &self,
        roots: Vec<ModuleRoot>,
        generation: u64,
    ) -> Result<ModuleAnalysis, CompileFailure> {
        let attempt = self.load_module_roots_attempt(roots).await;
        let dependencies = attempt.dependencies;
        let parsed = attempt.result?;
        let project = self
            .resolve_parsed_module_graph_attempt(CompileAttempt {
                result: Ok(parsed.clone()),
                dependencies: dependencies.clone(),
            })
            .result?;
        let (environment, catalog, prepared_params) = self
            .analyze_resolved_module_graph(&project, generation, &dependencies, None)
            .await?;
        self.finish_module_analysis(
            &parsed,
            &project,
            &dependencies,
            &environment,
            &catalog,
            &prepared_params,
        )
        .await
    }

    async fn finish_module_analysis(
        &self,
        parsed: &ParsedModuleGraph,
        project: &ResolvedModuleGraph,
        dependencies: &DiscoveredDependencySet,
        environment: &crate::CompileEnvironment,
        catalog: &CatalogAnalysis,
        prepared_params: &PreparedParams,
    ) -> Result<ModuleAnalysis, CompileFailure> {
        let mut fingerprints = dependency_fingerprint_layers(
            &self.options,
            parsed,
            project,
            dependencies,
            catalog,
            environment,
        );
        let module_fingerprint = project_fingerprint_from_layers(&fingerprints);
        let analysis_cache_key = format!(
            "{}\0{}",
            self.options.native_registry.profile_id().as_str(),
            module_fingerprint.as_str()
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
        let mut analysis = ModuleAnalysis::empty(
            project.sources.clone(),
            self.options.native_registry.profile_id().clone(),
            module_fingerprint,
        );
        analysis.param_types.clone_from(&prepared_params.types);
        analysis.resolved_module_graph = Some(Arc::new(project.clone()));
        let state = environment.session_context().state();
        macro_rules! collect_udfs {
            ($functions:expr, $category:expr) => {
                for (name, function) in $functions {
                    let signature = function.signature();
                    let documentation = function.documentation();
                    let parameter_names = signature.parameter_names.clone().unwrap_or_else(|| {
                        documentation
                            .and_then(|docs| docs.arguments.as_ref())
                            .map(|arguments| {
                                arguments.iter().map(|(name, _)| name.clone()).collect()
                            })
                            .unwrap_or_default()
                    });
                    analysis.functions.functions.push(FunctionMetadata {
                        name: name.clone(),
                        category: $category,
                        signature: Some(format!("{:?}", signature.type_signature)),
                        parameter_names,
                        return_type: None,
                        volatility: Some(format!("{:?}", signature.volatility)),
                        description: documentation.map(|docs| docs.description.clone()),
                        syntax_example: documentation.map(|docs| docs.syntax_example.clone()),
                        arguments: documentation
                            .and_then(|docs| docs.arguments.clone())
                            .unwrap_or_default(),
                    });
                }
            };
        }
        collect_udfs!(state.scalar_functions(), FunctionCategory::Scalar);
        collect_udfs!(state.aggregate_functions(), FunctionCategory::Aggregate);
        collect_udfs!(state.window_functions(), FunctionCategory::Window);
        for name in state.table_functions().keys() {
            analysis.functions.functions.push(FunctionMetadata {
                name: name.clone(),
                category: FunctionCategory::Table,
                signature: None,
                parameter_names: Vec::new(),
                return_type: None,
                volatility: None,
                description: None,
                syntax_example: None,
                arguments: Vec::new(),
            });
        }
        analysis.functions.functions.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then(left.name.cmp(&right.name))
        });
        analysis.lineage = catalog.lineage.clone();
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
                    sources: project.sources.clone(),
                })?;
        }
        let chart_datasets = analyze_chart_datasets(
            project,
            self.options.native_registry.as_ref(),
            environment.session_context(),
            self.options.source_loader.as_ref(),
            &self.options.import_capabilities,
            prepared_params,
        )
        .await
        .map_err(|mut diagnostics| {
            sort_diagnostics(&mut diagnostics, &project.sources);
            CompileFailure {
                diagnostics,
                sources: project.sources.clone(),
            }
        })?;
        let mut ordinals = BTreeMap::new();
        let mut previous = BTreeMap::new();
        for dataset in chart_datasets {
            if dataset.stage_kind == DatasetStageKind::MarkInput {
                analysis
                    .mark_channels
                    .insert(dataset.dataset.clone(), dataset.mark_channels.clone());
            }
            let id = ModuleDatasetId::new(format!("chart:{}", dataset.dataset.as_str()));
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
                qualified_path: None,
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
                    sources: project.sources.clone(),
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
                    sources: project.sources.clone(),
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

    pub async fn check_module(&self, path: impl AsRef<Path>) -> Result<(), CompileFailure> {
        self.analyze_module(path).await.map(|_| ())
    }

    pub async fn expand_module(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ExpandedSource, CompileFailure> {
        let project = self.load_module_graph_attempt(path).await.result?;
        let resolved = resolve_semantics(&project, self.host.authoring_schema())
            .result
            .map_err(|failure| CompileFailure {
                diagnostics: failure.diagnostics,
                sources: project.sources.clone(),
            })?;
        let expanded =
            expand_module_graph_with_limits(&project, &resolved, self.options.limits.expansion)
                .map_err(|failure| CompileFailure {
                    diagnostics: failure.diagnostics,
                    sources: project.sources.clone(),
                })?;
        let root = project
            .requested_modules
            .first()
            .ok_or_else(|| CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-EXPAND-003",
                    "source expansion requires one requested module",
                    SourceLabel::new(
                        SourceSpan::empty(SourceId::new(0), 0),
                        "the loaded graph has no requested module",
                    ),
                )],
                sources: project.sources.clone(),
            })?;
        let text = expanded
            .texts
            .get(root)
            .cloned()
            .ok_or_else(|| CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-EXPAND-004",
                    "expanded module source is missing",
                    SourceLabel::new(
                        SourceSpan::empty(SourceId::new(0), 0),
                        format!("no expansion was emitted for `{}`", root.as_str()),
                    ),
                )],
                sources: project.sources.clone(),
            })?;
        Ok(ExpandedSource {
            text,
            sources: expanded.module_graph.sources,
            source_map: expanded.source_map,
        })
    }

    /// Load and semantically resolve one requested module and its complete
    /// dependency closure without constructing native chart objects.
    pub async fn resolve_module_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<ResolvedModuleGraph> {
        let attempt = self.load_module_graph_attempt(path).await;
        self.resolve_parsed_module_graph_attempt(attempt)
    }

    /// Convenience wrapper for callers that do not need dependency metadata.
    pub async fn resolve_module(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ResolvedModuleGraph, CompileFailure> {
        self.resolve_module_attempt(path).await.result
    }

    /// Load one requested module and its complete import/data closure without
    /// performing semantic validation or lowering.
    pub async fn load_module_graph_attempt(
        &self,
        path: impl AsRef<Path>,
    ) -> CompileAttempt<ParsedModuleGraph> {
        let module = canonicalize_if_exists(&self.resolve_path(path.as_ref()));
        self.load_module_roots_attempt(vec![ModuleRoot::requested(SourceOrigin::File(module))])
            .await
    }

    /// Load an explicit module-root inventory and its import closure without
    /// scanning a directory or inferring module roles from filenames.
    pub async fn load_module_roots_attempt(
        &self,
        roots: Vec<ModuleRoot>,
    ) -> CompileAttempt<ParsedModuleGraph> {
        let request = ModuleGraphLoadRequest {
            project_root: normalize_path(&self.options.project_root),
            roots,
            native_modules: self
                .host
                .authoring_schema()
                .modules
                .keys()
                .filter_map(|id| {
                    self.options
                        .native_registry
                        .native_module(id)
                        .map(|(_, registered)| {
                            (
                                id.clone(),
                                AvailableNativeModule {
                                    schema_profile: registered.schema_profile.as_str().to_owned(),
                                    implementation_profile: registered
                                        .implementation_profile
                                        .as_str()
                                        .to_owned(),
                                },
                            )
                        })
                })
                .collect(),
            capabilities: self.options.import_capabilities.clone(),
            schema_version: "avenger-ast-core-1".to_owned(),
            registry_version: self
                .options
                .native_registry
                .profile_id()
                .as_str()
                .to_owned(),
            limits: self.options.limits.module_graph,
        };
        let ModuleGraphLoadAttempt {
            result,
            dependencies,
        } = ModuleGraphLoader::new(self.options.source_loader.as_ref())
            .load(request)
            .await;
        let mut dependencies = compiler_dependencies(dependencies);
        let result = result.map_err(|failure| CompileFailure {
            diagnostics: failure.diagnostics,
            sources: failure.sources,
        });
        let result = result.and_then(|mut project| {
            discover_local_resources(
                &project,
                &self.options.project_root,
                &self.options.import_capabilities,
                &self.options.data_capabilities,
                self.options.limits.resources,
                &mut dependencies,
            )
            .map_err(|failure| failure.with_sources(project.sources.clone()))?;
            project.fingerprint = augment_project_fingerprint(&project.fingerprint, &dependencies);
            Ok(project)
        });
        CompileAttempt {
            result,
            dependencies,
        }
    }

    fn resolve_parsed_module_graph_attempt(
        &self,
        attempt: CompileAttempt<ParsedModuleGraph>,
    ) -> CompileAttempt<ResolvedModuleGraph> {
        let dependencies = attempt.dependencies;
        let result = attempt.result.and_then(|project| {
            if let Some(cached) = self
                .resolved_module_graph_cache
                .lock()
                .expect("resolved-module-graph cache lock poisoned")
                .get(&project.fingerprint)
                .cloned()
            {
                return Ok(cached);
            }
            let fingerprint = project.fingerprint.clone();
            let resolved = resolve_parsed_module_graph(
                project,
                self.host.authoring_schema(),
                self.options.limits.expansion,
            )?;
            self.resolved_module_graph_cache
                .lock()
                .expect("resolved-module-graph cache lock poisoned")
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
            local_resource_versions: Vec::new(),
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
            ChartEntrypointId {
                module: SourceModuleId::new("programmatic:phase0-example"),
                selector: ChartSelector::Named("phase0-example".to_owned()),
            },
            Some("Phase 0 programmatic example".to_string()),
            SourceId::new(0),
            Arc::new(compiled),
            NativeRequirementSet::builtin_only(self.options.native_registry.as_ref()),
            DependencyFingerprint::new("phase0-programmatic"),
        ))
    }

    /// Temporary Phase 0 analysis artifact used to prove registry-profile
    /// propagation before project parsing and DataFusion planning land.
    #[doc(hidden)]
    pub fn analyze_phase0_empty(&self) -> ModuleAnalysis {
        ModuleAnalysis::empty(
            SourceMap::default(),
            self.options.native_registry.profile_id().clone(),
            ModuleFingerprint::new("phase0-empty-analysis"),
        )
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.options.project_root.join(path)
        };
        std::fs::canonicalize(&resolved).unwrap_or_else(|_| normalize_path(&resolved))
    }

    async fn analyze_resolved_module_graph(
        &self,
        project: &ResolvedModuleGraph,
        generation: u64,
        dependencies: &DiscoveredDependencySet,
        reachable_items: Option<&BTreeSet<avenger_lang_core::ModuleItemId>>,
    ) -> Result<(crate::CompileEnvironment, CatalogAnalysis, PreparedParams), CompileFailure> {
        let request = CompileEnvironmentRequest {
            generation,
            native_registry_profile: self
                .options
                .native_registry
                .profile_id()
                .as_str()
                .to_string(),
            local_resource_versions: compile_environment_resource_versions(dependencies),
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
                sources: project.sources.clone(),
            })?;
        let prepared_params = prepare_module_params(
            project,
            self.options.native_registry.as_ref(),
            environment.session_context(),
            self.options.source_loader.as_ref(),
            &self.options.import_capabilities,
        )
        .map_err(|diagnostics| CompileFailure {
            diagnostics,
            sources: project.sources.clone(),
        })?;
        let catalog = register_and_analyze_catalog(
            project,
            &environment,
            &prepared_params.types,
            CatalogOptions {
                project_root: &self.options.project_root,
                capabilities: &self.options.data_capabilities,
                environment_provider: self.options.environment.as_ref(),
                catalog_factories: &self.options.catalog_factories,
                table_factories: &self.options.table_factories,
                reachable_items,
            },
        )
        .await
        .map_err(|diagnostic| CompileFailure {
            diagnostics: vec![diagnostic],
            sources: project.sources.clone(),
        })?;
        Ok((environment, catalog, prepared_params))
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
    module_compilation_mode: ModuleCompilationMode,
    limits: CompilerLimits,
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

    pub fn module_compilation_mode(mut self, mode: ModuleCompilationMode) -> Self {
        self.module_compilation_mode = mode;
        self
    }

    pub fn limits(mut self, limits: CompilerLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn build(self) -> Result<Compiler, CompilerBuildError> {
        let project_root = self
            .project_root
            .ok_or(CompilerBuildError::MissingModuleRoot)?;
        let project_root =
            std::fs::canonicalize(&project_root).unwrap_or_else(|_| normalize_path(&project_root));
        let registry = match self.native_registry {
            Some(registry) => registry,
            None => Arc::new(builtins::stock_registry()?),
        };
        let limits = self.limits;
        let source_loader = self.source_loader.map(Ok).unwrap_or_else(|| {
            DefaultSourceLoader::with_limits(
                &project_root,
                SourceLoaderLimits {
                    max_source_bytes: limits.module_graph.max_source_bytes,
                    max_redirects: 5,
                },
            )
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
            limits,
        };
        Ok(Compiler {
            options: Arc::new(options),
            host: LanguageHost::new(registry),
            resolved_module_graph_cache: Arc::new(Mutex::new(BTreeMap::new())),
            analysis_cache: Arc::new(Mutex::new(BTreeMap::new())),
            dataset_analysis_cache: Arc::new(Mutex::new(BTreeMap::new())),
            artifact_cache: Arc::new(Mutex::new(ArtifactCache::default())),
            module_compilation_mode: self.module_compilation_mode,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompilerBuildError {
    #[error("compiler project root is required")]
    MissingModuleRoot,
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    SourceLoader(#[from] SourceLoaderError),
}

fn phase_zero_failure(message: String) -> CompileFailure {
    CompileFailure::new(vec![Diagnostic::error(
        "AV0004",
        "programmatic chart compilation failed",
        SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), message),
    )])
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
    dependencies: Vec<avenger_lang_core::ModuleDependency>,
) -> DiscoveredDependencySet {
    let mut result = DiscoveredDependencySet::default();
    for dependency in dependencies {
        let requested_origin = match dependency.requested {
            ModuleDependencyTarget::Source(origin) => origin,
            ModuleDependencyTarget::Native(_) => continue,
        };
        let canonical_origin = dependency
            .canonical_origin
            .unwrap_or_else(|| requested_origin.clone());
        let nearest_existing_parent = match &requested_origin {
            SourceOrigin::File(path) => nearest_existing_parent(path),
            _ => None,
        };
        let role = match dependency.role {
            ModuleDependencyRole::RequestedModule => DependencyRole::RequestedModule,
            ModuleDependencyRole::Import => DependencyRole::Import,
            ModuleDependencyRole::AmbientDataRoot => DependencyRole::DataConfiguration,
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

fn compile_environment_resource_versions(
    dependencies: &DiscoveredDependencySet,
) -> Vec<CompileEnvironmentResourceVersion> {
    let mut versions = BTreeMap::new();
    for dependency in dependencies
        .iter()
        .filter(|dependency| dependency.role == DependencyRole::LocalResource)
    {
        let Some(content_version) = dependency.content_version.as_ref() else {
            continue;
        };
        for origin in [&dependency.requested_origin, &dependency.canonical_origin] {
            let SourceOrigin::File(path) = origin else {
                continue;
            };
            let (path, recursive) = if let Some(root) = static_glob_root(path) {
                (root, true)
            } else {
                (path.clone(), path.is_dir())
            };
            versions.insert((path, recursive), content_version.clone());
        }
    }
    versions
        .into_iter()
        .map(
            |((path, recursive), content_version)| CompileEnvironmentResourceVersion {
                path,
                recursive,
                content_version,
            },
        )
        .collect()
}

fn static_glob_root(path: &Path) -> Option<PathBuf> {
    let mut root = PathBuf::new();
    let mut found_pattern = false;
    for component in path.components() {
        if component
            .as_os_str()
            .to_string_lossy()
            .contains(['*', '?', '['])
        {
            found_pattern = true;
            break;
        }
        root.push(component.as_os_str());
    }
    found_pattern.then_some(root)
}

fn resolve_parsed_module_graph(
    project: ParsedModuleGraph,
    schema: &avenger_chart_schema::NativeSchemaSnapshot,
    expansion_limits: avenger_lang_core::ExpansionLimits,
) -> Result<ResolvedModuleGraph, CompileFailure> {
    let sources = project.sources.clone();
    let resolved = resolve_semantics(&project, schema)
        .result
        .map_err(|failure| CompileFailure {
            diagnostics: failure.diagnostics,
            sources: sources.clone(),
        })?;
    if resolved.definitions.is_empty() {
        return Ok(resolved);
    }
    let authoring_items = resolved.authoring_items.clone();
    let expanded = expand_module_graph_with_limits(&project, &resolved, expansion_limits).map_err(
        |failure| CompileFailure {
            diagnostics: failure.diagnostics,
            sources: sources.clone(),
        },
    )?;
    match resolve_semantics(&expanded.module_graph, schema).result {
        Ok(mut resolved) => {
            resolved.expansion_source_map = expanded.source_map;
            resolved.authoring_items = authoring_items;
            Ok(resolved)
        }
        Err(mut failure) => {
            expanded
                .source_map
                .remap_diagnostics(&mut failure.diagnostics);
            Err(CompileFailure {
                diagnostics: failure.diagnostics,
                sources: expanded.module_graph.sources,
            })
        }
    }
}

fn discover_local_resources(
    project: &ParsedModuleGraph,
    project_root: &Path,
    import_capabilities: &ImportCapabilities,
    data_capabilities: &DataCapabilities,
    limits: LocalResourceLimits,
    dependencies: &mut DiscoveredDependencySet,
) -> Result<(), CompileFailure> {
    let mut context = ResourceDiscoveryContext {
        project_root,
        import_capabilities,
        data_capabilities,
        limits,
        budget: ResourceFingerprintBudget::default(),
        dependencies,
    };
    for file in project.source_modules.values() {
        for item in &file.parsed.ast.items {
            discover_declaration_resources(
                &item.declaration,
                file.source,
                &file.origin,
                &mut context,
            )?;
        }
    }
    Ok(())
}

struct ResourceDiscoveryContext<'a> {
    project_root: &'a Path,
    import_capabilities: &'a ImportCapabilities,
    data_capabilities: &'a DataCapabilities,
    limits: LocalResourceLimits,
    budget: ResourceFingerprintBudget,
    dependencies: &'a mut DiscoveredDependencySet,
}

fn discover_declaration_resources(
    declaration: &Decl,
    source: SourceId,
    declaring_origin: &SourceOrigin,
    context: &mut ResourceDiscoveryContext<'_>,
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
                context.data_capabilities.allow_filesystem,
                context,
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
            context.import_capabilities.allow_filesystem,
            context,
        )?;
    }
    for child in &declaration.children {
        discover_declaration_resources(child, source, declaring_origin, context)?;
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
    allow_filesystem: bool,
    context: &mut ResourceDiscoveryContext<'_>,
) -> Result<(), CompileFailure> {
    let origin = if path.split_once("://").is_some() {
        SourceOrigin::Http(path.to_owned())
    } else {
        resolve_relative_origin(declaring_origin, path, context.project_root).map_err(
            |message| CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-PROJECT-018",
                    "invalid local data resource",
                    SourceLabel::new(SourceSpan::empty(source, 0), message),
                )],
                sources: SourceMap::default(),
            },
        )?
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
    context
        .dependencies
        .insert_owned(source, dependency.clone());
    let SourceOrigin::File(candidate) = origin else {
        return Ok(());
    };
    let normalized_root = normalize_path(context.project_root);
    let normalized_candidate = normalize_path(&candidate);
    if !allow_filesystem || !normalized_candidate.starts_with(&normalized_root) {
        return Err(resource_failure(
            source,
            "local data resource is outside the project capability root",
            normalized_candidate.display().to_string(),
        ));
    }
    if path.contains(['*', '?', '[']) {
        dependency.content_version = Some(
            glob_content_version_with_budget(
                &normalized_candidate,
                context.limits,
                &mut context.budget,
            )
            .map_err(|error| {
                resource_failure(source, "local data glob could not be fingerprinted", error)
            })?,
        );
        context.dependencies.insert_owned(source, dependency);
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
    dependency.content_version = Some(
        resource_content_version_with_budget(&canonical, context.limits, &mut context.budget)
            .map_err(|error| {
                resource_failure(
                    source,
                    "local data resource could not be fingerprinted",
                    error.to_string(),
                )
            })?,
    );
    context.dependencies.insert_owned(source, dependency);
    Ok(())
}

pub(crate) fn resource_content_version(path: &Path) -> Result<String, std::io::Error> {
    resource_content_version_with_limits(path, LocalResourceLimits::default())
}

fn resource_content_version_with_limits(
    path: &Path,
    limits: LocalResourceLimits,
) -> Result<String, std::io::Error> {
    let mut budget = ResourceFingerprintBudget::default();
    resource_content_version_with_budget(path, limits, &mut budget)
}

#[derive(Default)]
struct ResourceFingerprintBudget {
    files: usize,
    bytes: u64,
    directory_entries: usize,
}

fn resource_content_version_with_budget(
    path: &Path,
    limits: LocalResourceLimits,
    budget: &mut ResourceFingerprintBudget,
) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    if path.is_dir() {
        let mut files = Vec::new();
        let max_new_files = limits.max_files.saturating_sub(budget.files);
        collect_directory_files(path, path, 0, limits, max_new_files, budget, &mut files)?;
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
            hash_resource_file(&file, limits, budget, &mut hasher)?;
            hasher.update(b"\0");
        }
        return Ok(format!("directory-sha256:{:x}", hasher.finalize()));
    }
    let mut hasher = Sha256::new();
    hash_resource_file(path, limits, budget, &mut hasher)?;
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_resource_file(
    path: &Path,
    limits: LocalResourceLimits,
    budget: &mut ResourceFingerprintBudget,
    hasher: &mut sha2::Sha256,
) -> Result<(), std::io::Error> {
    use sha2::Digest;
    use std::io::Read;

    let metadata = std::fs::metadata(path)?;
    if metadata.len() > limits.max_file_bytes {
        return Err(resource_limit_error(format!(
            "resource `{}` is {} bytes; per-file limit is {} bytes",
            path.display(),
            metadata.len(),
            limits.max_file_bytes
        )));
    }
    if budget.files >= limits.max_files {
        return Err(resource_limit_error(format!(
            "resource tree exceeds {} files",
            limits.max_files
        )));
    }
    let projected_total_bytes = budget
        .bytes
        .checked_add(metadata.len())
        .ok_or_else(|| resource_limit_error("resource byte count overflowed".to_string()))?;
    if projected_total_bytes > limits.max_total_bytes {
        return Err(resource_limit_error(format!(
            "resource tree totals {projected_total_bytes} bytes; limit is {} bytes",
            limits.max_total_bytes
        )));
    }

    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut file_bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file_bytes = file_bytes
            .checked_add(read as u64)
            .ok_or_else(|| resource_limit_error("resource byte count overflowed".to_string()))?;
        if file_bytes > limits.max_file_bytes {
            return Err(resource_limit_error(format!(
                "resource `{}` grew beyond the {} byte per-file limit while reading",
                path.display(),
                limits.max_file_bytes
            )));
        }
        let actual_total_bytes = budget
            .bytes
            .checked_add(file_bytes)
            .ok_or_else(|| resource_limit_error("resource byte count overflowed".to_string()))?;
        if actual_total_bytes > limits.max_total_bytes {
            return Err(resource_limit_error(format!(
                "resource tree grew beyond the {} byte total limit while reading",
                limits.max_total_bytes
            )));
        }
        hasher.update(&buffer[..read]);
    }
    budget.files += 1;
    budget.bytes += file_bytes;
    Ok(())
}

fn collect_directory_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    limits: LocalResourceLimits,
    max_new_files: usize,
    budget: &mut ResourceFingerprintBudget,
    files: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    if depth > limits.max_directory_depth {
        return Err(resource_limit_error(format!(
            "resource directory depth exceeds {} at `{}`",
            limits.max_directory_depth,
            directory.display()
        )));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        budget.directory_entries = budget.directory_entries.checked_add(1).ok_or_else(|| {
            resource_limit_error("resource directory entry count overflowed".to_string())
        })?;
        if budget.directory_entries > limits.max_directory_entries {
            return Err(resource_limit_error(format!(
                "resource discovery exceeds {} directory entries",
                limits.max_directory_entries
            )));
        }
        entries.push(entry?);
    }
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
            collect_directory_files(root, &path, depth + 1, limits, max_new_files, budget, files)?;
        } else if metadata.is_file() {
            if files.len() >= max_new_files {
                return Err(resource_limit_error(format!(
                    "resource tree exceeds {} files",
                    limits.max_files
                )));
            }
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn glob_content_version(pattern: &Path) -> Result<String, String> {
    glob_content_version_with_limits(pattern, LocalResourceLimits::default())
}

fn glob_content_version_with_limits(
    pattern: &Path,
    limits: LocalResourceLimits,
) -> Result<String, String> {
    let mut budget = ResourceFingerprintBudget::default();
    glob_content_version_with_budget(pattern, limits, &mut budget)
}

fn glob_content_version_with_budget(
    pattern: &Path,
    limits: LocalResourceLimits,
    budget: &mut ResourceFingerprintBudget,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let pattern = pattern.to_string_lossy();
    let mut matches = Vec::new();
    for matched in glob::glob(&pattern).map_err(|error| error.to_string())? {
        if matches.len() >= limits.max_files.saturating_sub(budget.files) {
            return Err(format!(
                "resource glob exceeds {} top-level matches",
                limits.max_files
            ));
        }
        matches.push(matched.map_err(|error| error.to_string())?);
    }
    matches.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-glob-v1\0");
    for path in matches {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        let version = resource_content_version_with_budget(&path, limits, budget)
            .map_err(|error| error.to_string())?;
        hasher.update(version.as_bytes());
        hasher.update(b"\0");
    }
    Ok(format!("glob-sha256:{:x}", hasher.finalize()))
}

fn resource_limit_error(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn resource_failure(source: SourceId, message: &str, label: String) -> CompileFailure {
    CompileFailure::new(vec![Diagnostic::error(
        "AVENGER-PROJECT-019",
        message,
        SourceLabel::new(SourceSpan::empty(source, 0), label),
    )])
}

fn source_dependency_fingerprints(
    project: &ParsedModuleGraph,
    dependencies: &DiscoveredDependencySet,
) -> BTreeMap<SourceModuleId, DependencyFingerprint> {
    project
        .source_modules
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
            "avenger-language-profile-explicit-channel-modes-v1",
            options.native_registry.profile_id().as_str(),
            import_capabilities.as_str(),
            data_capabilities.as_str(),
            catalog_factories.as_str(),
            table_factories.as_str(),
        ],
    )
}

fn select_chart_entrypoint(
    parsed: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    selector: Option<&str>,
) -> Result<ChartEntrypointId, CompileFailure> {
    let Some(requested) = parsed.requested_modules.first() else {
        return Err(module_root_failure(
            &resolved.sources,
            "the loaded graph has no requested module",
        ));
    };
    let candidates = resolved
        .entrypoints
        .keys()
        .filter(|entrypoint| &entrypoint.module == requested)
        .cloned()
        .collect::<Vec<_>>();
    let source = resolved
        .source_modules
        .get(requested)
        .map(|module| module.source)
        .unwrap_or_else(|| SourceId::new(0));
    let span = SourceSpan::empty(source, 0);
    if let Some(selector) = selector {
        if let Some(entrypoint) = candidates.iter().find(|entrypoint| {
            matches!(&entrypoint.selector, ChartSelector::Named(name) if name == selector)
        }) {
            return Ok(entrypoint.clone());
        }
        let available = candidates
            .iter()
            .filter_map(|entrypoint| match &entrypoint.selector {
                ChartSelector::Named(name) => Some(name.as_str()),
                ChartSelector::Anonymous => None,
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CompileFailure {
            diagnostics: vec![Diagnostic::error(
                "AVENGER-RESOLVE-241",
                "chart selector does not match an entrypoint",
                SourceLabel::new(
                    span,
                    if available.is_empty() {
                        format!("the requested module has no named chart `{selector}`")
                    } else {
                        format!("no chart is named `{selector}`; available charts: {available}")
                    },
                ),
            )],
            sources: resolved.sources.clone(),
        });
    }
    match candidates.as_slice() {
        [entrypoint] => Ok(entrypoint.clone()),
        [] => Err(CompileFailure {
            diagnostics: vec![Diagnostic::error(
                "AVENGER-RESOLVE-240",
                "requested module has no chart entrypoint",
                SourceLabel::new(span, "add a chart or compile this module as a library"),
            )],
            sources: resolved.sources.clone(),
        }),
        _ => {
            let available = candidates
                .iter()
                .filter_map(|entrypoint| match &entrypoint.selector {
                    ChartSelector::Named(name) => Some(name.as_str()),
                    ChartSelector::Anonymous => None,
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(CompileFailure {
                diagnostics: vec![Diagnostic::error(
                    "AVENGER-RESOLVE-242",
                    "chart selector is required",
                    SourceLabel::new(
                        span,
                        format!("choose one of the module's charts: {available}"),
                    ),
                )],
                sources: resolved.sources.clone(),
            })
        }
    }
}

fn native_requirements_for_entrypoint(
    resolved: &ResolvedModuleGraph,
    entrypoint: &ChartEntrypointId,
    registry: &NativeRegistry,
) -> NativeRequirementSet {
    fn visit(
        declaration: &ResolvedDeclaration,
        modules: &mut BTreeSet<avenger_chart_schema::NativeModuleId>,
    ) {
        if let Some(ResolvedKindBinding::Native { export, .. }) = &declaration.kind_binding
            && let ModuleId::Native(module) = &export.module
        {
            modules.insert(module.clone());
        }
        for child in &declaration.children {
            visit(child, modules);
        }
        for value in declaration.properties.values() {
            visit_value(value, modules);
        }
    }

    fn visit_value(
        value: &ResolvedValue,
        modules: &mut BTreeSet<avenger_chart_schema::NativeModuleId>,
    ) {
        match value {
            ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
                for value in values {
                    visit_value(value, modules);
                }
            }
            ResolvedValue::Channel {
                expression: value, ..
            }
            | ResolvedValue::Pattern(value) => {
                visit_value(value, modules);
            }
            ResolvedValue::ChannelValue(channel) => {
                visit_value(&channel.head.expression, modules);
                if let Some(otherwise) = &channel.otherwise {
                    visit_value(&otherwise.expression, modules);
                }
                for condition in &channel.conditions {
                    visit_value(&condition.predicate, modules);
                    visit_value(&condition.branch.expression, modules);
                }
                for value in channel.configuration.values() {
                    visit_value(value, modules);
                }
            }
            ResolvedValue::Object {
                head,
                properties,
                children,
                ..
            } => {
                if let Some(head) = head {
                    visit_value(head, modules);
                }
                for value in properties.values() {
                    visit_value(value, modules);
                }
                for child in children {
                    visit(child, modules);
                }
            }
            _ => {}
        }
    }

    let mut modules = BTreeSet::new();
    if let Some(entrypoint) = resolved.entrypoints.get(entrypoint)
        && let Some(chart) = find_resolved_declaration(resolved, &entrypoint.declaration)
    {
        visit(chart, &mut modules);
    }
    NativeRequirementSet::from_modules(registry, modules)
        .expect("resolved native module requirements are installed in the compiler host")
}

fn find_resolved_declaration<'a>(
    resolved: &'a ResolvedModuleGraph,
    id: &DeclarationId,
) -> Option<&'a ResolvedDeclaration> {
    fn find<'a>(
        declaration: &'a ResolvedDeclaration,
        id: &DeclarationId,
    ) -> Option<&'a ResolvedDeclaration> {
        if &declaration.id == id {
            return Some(declaration);
        }
        declaration
            .children
            .iter()
            .find_map(|child| find(child, id))
    }
    resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
        .find_map(|root| find(root, id))
}

fn module_root_failure(sources: &SourceMap, message: impl Into<String>) -> CompileFailure {
    CompileFailure {
        diagnostics: vec![Diagnostic::error(
            "AVENGER-MODULE-049",
            "requested module is unavailable",
            SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), message),
        )],
        sources: sources.clone(),
    }
}

fn item_closure_fingerprint(
    parsed: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    entrypoint: &ChartEntrypointId,
) -> DependencyFingerprint {
    let mut parts = Vec::new();
    let items = resolved
        .authoring_items
        .chart_closures
        .get(entrypoint)
        .into_iter()
        .flatten();
    for item in items {
        let Some(item_order) = resolved.authoring_items.item_order.get(&item.module) else {
            continue;
        };
        let Some(index) = item_order.iter().position(|candidate| candidate == item) else {
            continue;
        };
        let Some(authored) = parsed
            .source_modules
            .get(&item.module)
            .and_then(|module| module.parsed.ast.items.get(index))
        else {
            continue;
        };
        parts.push(item.module.as_str().to_owned());
        parts.push(item.declaration.as_str().to_owned());
        parts.push(serde_json::to_string(authored).unwrap_or_default());
    }
    DependencyFingerprint::new(stable_hash(
        "avenger-item-closure-v1",
        parts.iter().map(String::as_str),
    ))
}

fn collect_chart_catalog_dependencies(
    project: &ResolvedModuleGraph,
    chart: &ResolvedDeclaration,
) -> (BTreeSet<DeclarationId>, BTreeSet<String>) {
    fn visit_target(target: &ResolvedTarget, tables: &mut BTreeSet<DeclarationId>) {
        if let ResolvedTarget::Declaration(id) = target {
            tables.insert(id.clone());
        }
    }
    fn visit_value(
        project: &ResolvedModuleGraph,
        property: Option<&str>,
        value: &ResolvedValue,
        tables: &mut BTreeSet<DeclarationId>,
        names: &mut BTreeSet<String>,
    ) {
        match value {
            ResolvedValue::Relation(reference) => {
                names.insert(reference.authored_path.join("."));
                if let ResolvedRelationTarget::Relation(relation) = &reference.target
                    && let Some(table) = project.catalog_tables.get(relation)
                {
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
                for reference in &query.relations {
                    names.insert(reference.authored_path.join("."));
                    if let ResolvedRelationTarget::Relation(relation) = &reference.target
                        && let Some(table) = project.catalog_tables.get(relation)
                    {
                        tables.insert(table.id.clone());
                    }
                }
            }
            ResolvedValue::Channel {
                expression: value, ..
            }
            | ResolvedValue::Pattern(value) => {
                visit_value(project, property, value, tables, names);
            }
            ResolvedValue::ChannelValue(channel) => {
                visit_value(project, property, &channel.head.expression, tables, names);
                if let Some(otherwise) = &channel.otherwise {
                    visit_value(project, property, &otherwise.expression, tables, names);
                }
                for condition in &channel.conditions {
                    visit_value(project, property, &condition.predicate, tables, names);
                    visit_value(
                        project,
                        property,
                        &condition.branch.expression,
                        tables,
                        names,
                    );
                }
                for value in channel.configuration.values() {
                    visit_value(project, property, value, tables, names);
                }
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
        project: &ResolvedModuleGraph,
        declaration: &ResolvedDeclaration,
        tables: &mut BTreeSet<DeclarationId>,
        names: &mut BTreeSet<String>,
    ) {
        for reference in &declaration.relation_references {
            names.insert(reference.authored_path.join("."));
            if let ResolvedRelationTarget::Relation(relation) = &reference.target
                && let Some(table) = project.catalog_tables.get(relation)
            {
                tables.insert(table.id.clone());
            }
        }
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
    parsed: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    dependencies: &DiscoveredDependencySet,
    catalog: &CatalogAnalysis,
    environment: &crate::CompileEnvironment,
) -> ModuleDependencyFingerprints {
    let sources = source_dependency_fingerprints(parsed, dependencies);
    let mut result = ModuleDependencyFingerprints {
        sources,
        datasets: catalog.dataset_fingerprints.clone(),
        data_catalog: DependencyFingerprint::new(catalog.dependency_fingerprint.clone()),
        compile_environment: DependencyFingerprint::new(
            environment.dependency_fingerprint().to_owned(),
        ),
        ..ModuleDependencyFingerprints::default()
    };
    let compiler_options = compiler_options_fingerprint(options);
    let declarations = resolved
        .source_modules
        .values()
        .flat_map(|file| file.roots.iter())
        .flat_map(declarations_depth_first)
        .map(|declaration| (declaration.id.clone(), declaration))
        .collect::<BTreeMap<_, _>>();
    for (entrypoint_id, entrypoint) in &resolved.entrypoints {
        let item_closure = item_closure_fingerprint(parsed, resolved, entrypoint_id);
        result
            .item_closures
            .insert(entrypoint_id.clone(), item_closure.clone());
        let mut parts = vec![
            serde_json::to_string(entrypoint_id).unwrap_or_default(),
            item_closure.as_str().to_owned(),
            compiler_options.clone(),
            environment.dependency_fingerprint().to_owned(),
        ];
        if let Some(chart) = declarations.get(&entrypoint.declaration) {
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
            entrypoint_id.clone(),
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
    fingerprints: &ModuleDependencyFingerprints,
) -> ModuleFingerprint {
    let mut parts = Vec::new();
    for (source, fingerprint) in &fingerprints.sources {
        parts.push(source.as_str().to_owned());
        parts.push(fingerprint.as_str().to_owned());
    }
    parts.push(fingerprints.data_catalog.as_str().to_owned());
    parts.push(fingerprints.compile_environment.as_str().to_owned());
    for (chart, fingerprint) in &fingerprints.charts {
        parts.push(serde_json::to_string(chart).unwrap_or_default());
        parts.push(fingerprint.as_str().to_owned());
    }
    ModuleFingerprint::new(stable_hash(
        "avenger-compiled-module-v2",
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

fn chart_artifact_cache_fingerprint(
    base: &DependencyFingerprint,
    project: &ResolvedModuleGraph,
    entrypoint: &ChartEntrypointId,
    prepared_params: &PreparedParams,
) -> DependencyFingerprint {
    use avenger_chart_core::SerializableScalar;
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"avenger-chart-artifact-param-values-v1\0");
    hasher.update(base.as_str().as_bytes());
    if let Some(entrypoint) = project.entrypoints.get(entrypoint) {
        for id in entrypoint.params.keys() {
            let Some(param) = prepared_params.values.get(id) else {
                continue;
            };
            hasher.update(b"\0param\0");
            hasher.update(id.as_str().as_bytes());
            hasher.update(b"\0");
            let bytes = serde_json::to_vec(&SerializableScalar::new(param.default.clone()))
                .expect("prepared param values were validated as serializable");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
    }
    DependencyFingerprint::new(format!("sha256:{:x}", hasher.finalize()))
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
