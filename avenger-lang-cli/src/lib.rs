use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use avenger_chart::physical_cache::{
    CacheVersionProvider, EvaluationCache, EvaluationCacheConfig,
    install_shared_physical_cache_with_version_providers, physical_cache_disabled_by_env,
};
use avenger_chart_app::{
    ChartAppOptions, ChartAppState, ChartResizeBinding, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, canvas_frame_options_for_resize_policy_and_binding,
    chart_avenger_app_with_default_runtime_resources,
    chart_avenger_app_with_default_runtime_resources_and_snapshot,
    window_scene_sizing_for_resize_policy_and_binding,
};
use avenger_lang::{
    CompileAttempt, CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, CompileEnvironmentResourceVersion, CompileFailure,
    CompiledChartGeneration, CompiledDependency, Compiler, DiscoveredDependencySet, SourceOrigin,
};
use avenger_winit_wgpu::{
    HostUpdateInstallOutcome, HostUpdateSender, HostUpdateSubmitError, HostUpdateSubmitOutcome,
    PreparedHostUpdate,
};
use clap::{Args, Parser, Subcommand};
use datafusion::{
    arrow::datatypes::DataType,
    datasource::{physical_plan::FileScanConfig, source::DataSourceExec},
    execution::session_state::SessionStateBuilder,
    physical_plan::ExecutionPlan,
    prelude::SessionContext,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing_subscriber::EnvFilter;
use winit::window::WindowAttributes;

#[derive(Debug, Parser)]
#[command(name = "avenger", version, about = "Avenger chart language tools")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Display a chart in a native window and hot reload local changes.
    Watch(WatchArgs),
    /// Flatten source-module dependencies into one canonical Avenger module.
    Bundle(BundleArgs),
    /// Run the Avenger language server over standard input/output.
    Lsp(LspArgs),
}

#[derive(Clone, Debug, Args)]
pub struct BundleArgs {
    /// Avenger source module to bundle.
    #[arg(value_name = "MODULE")]
    module: PathBuf,

    /// Bundle only this named chart and its reachable closure.
    #[arg(long, value_name = "NAME")]
    chart: Option<String>,

    /// Write the bundle to a file instead of standard output.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Override the project/capability root (defaults to the module directory).
    #[arg(long, value_name = "DIR")]
    project_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct LspArgs {
    /// Quiet period before workspace-wide semantic analysis.
    #[arg(long, default_value_t = 120, value_name = "MILLIS")]
    debounce_ms: u64,

    /// Maximum size accepted for one open source document.
    #[arg(long, default_value_t = 8, value_name = "MB")]
    max_document_mb: usize,

    /// Maximum diagnostics published for one document in one batch.
    #[arg(long, default_value_t = 200, value_name = "COUNT")]
    max_diagnostics: usize,

    /// Maximum number of workspace roots retained by the server.
    #[arg(long, default_value_t = 32, value_name = "COUNT")]
    max_workspaces: usize,

    /// Maximum semantic tokens returned for one document.
    #[arg(long, default_value_t = 100_000, value_name = "COUNT")]
    max_semantic_tokens: usize,

    /// Maximum language requests evaluated concurrently.
    #[arg(long, default_value_t = 16, value_name = "COUNT")]
    max_concurrent_requests: usize,

    /// Maximum resolved projects and analyses retained per workspace.
    #[arg(long, default_value_t = 64, value_name = "COUNT")]
    max_analysis_cache_entries: usize,

    /// Maximum analyzed dataset stages retained per workspace.
    #[arg(long, default_value_t = 512, value_name = "COUNT")]
    max_dataset_cache_entries: usize,
}

#[derive(Clone, Debug, Args)]
pub struct WatchArgs {
    /// Avenger source module.
    #[arg(value_name = "MODULE")]
    module: PathBuf,

    /// Named chart entrypoint. Required when the module contains multiple charts.
    #[arg(long, value_name = "NAME")]
    chart: Option<String>,

    /// Override the project/capability root (defaults to the chart directory).
    #[arg(long, value_name = "DIR")]
    project_root: Option<PathBuf>,

    /// Filesystem-event quiet period before recompiling.
    #[arg(long, default_value_t = 100, value_name = "MILLIS")]
    debounce_ms: u64,

    /// Native render scale.
    #[arg(long, default_value_t = 4.0, value_name = "FACTOR")]
    scale: f32,

    /// Physical-plan cache memory budget.
    #[arg(long, default_value_t = 256, value_name = "MB")]
    cache_memory_mb: usize,

    /// Disable physical-plan caching for this watch process.
    #[arg(long)]
    no_cache: bool,

    /// Print process-wide physical-cache metric deltas after reload.
    #[arg(long)]
    log_cache: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    InvalidArguments(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("failed to construct language compiler: {0}")]
    Compiler(String),
    #[error("initial chart compilation failed")]
    InitialCompile,
    #[error("bundle compilation failed: {0}")]
    BundleCompile(CompileFailure),
    #[error("failed to prepare chart application: {0}")]
    App(String),
    #[error("failed to initialize filesystem watcher: {0}")]
    Watcher(String),
    #[error("failed to initialize process signal handling: {0}")]
    Signal(String),
    #[error("native event loop failed: {0}")]
    EventLoop(String),
}

#[derive(Clone)]
struct WatchEnvironmentFactory {
    cache: Option<Arc<EvaluationCache>>,
}

impl CompileEnvironmentFactory for WatchEnvironmentFactory {
    fn create(
        &self,
        request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        let context = if let Some(cache) = self.cache.as_ref() {
            let providers = if request.local_resource_versions.is_empty() {
                Vec::new()
            } else {
                vec![Arc::new(LocalResourceVersionProvider::new(
                    request.local_resource_versions.clone(),
                )) as Arc<dyn CacheVersionProvider>]
            };
            let builder = install_shared_physical_cache_with_version_providers(
                SessionStateBuilder::new().with_default_features(),
                cache.clone(),
                providers,
            );
            SessionContext::new_with_state(builder.build())
        } else {
            SessionContext::new()
        };
        Ok(CompileEnvironment::new(context))
    }
}

#[derive(Debug)]
struct LocalResourceVersionProvider {
    versions: Vec<CompileEnvironmentResourceVersion>,
}

impl LocalResourceVersionProvider {
    fn new(mut versions: Vec<CompileEnvironmentResourceVersion>) -> Self {
        versions.sort_by(|left, right| {
            right
                .path
                .components()
                .count()
                .cmp(&left.path.components().count())
                .then_with(|| left.path.cmp(&right.path))
        });
        Self { versions }
    }

    fn version_for_location(&self, location: &str) -> Option<&CompileEnvironmentResourceVersion> {
        let relative = PathBuf::from(location);
        let absolute = Path::new("/").join(location);
        let canonical = fs::canonicalize(&absolute).ok();
        self.versions.iter().find(|version| {
            [
                Some(relative.as_path()),
                Some(absolute.as_path()),
                canonical.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|candidate| {
                candidate == version.path
                    || (version.recursive && candidate.starts_with(&version.path))
                    || version.path.ends_with(candidate)
            })
        })
    }
}

impl CacheVersionProvider for LocalResourceVersionProvider {
    fn source_version(
        &self,
        plan: &dyn ExecutionPlan,
    ) -> Option<avenger_chart::physical_cache::CacheVersion> {
        let scan = plan.downcast_ref::<DataSourceExec>()?;
        let files = scan.data_source().downcast_ref::<FileScanConfig>()?;
        let mut matched = Vec::new();
        for file in files.file_groups.iter().flat_map(|group| group.files()) {
            let location = file.object_meta.location.as_ref();
            let version = self.version_for_location(location)?;
            matched.push((location.to_owned(), version.content_version.as_str()));
        }
        if matched.is_empty() {
            return None;
        }
        matched.sort_unstable();
        matched.dedup();
        let mut lower = DefaultHasher::new();
        "avenger-local-resource-version-v1".hash(&mut lower);
        matched.hash(&mut lower);
        let mut upper = DefaultHasher::new();
        "avenger-local-resource-version-v1-upper".hash(&mut upper);
        matched.hash(&mut upper);
        Some(avenger_chart::physical_cache::CacheVersion(
            (u128::from(upper.finish()) << 64) | u128::from(lower.finish()),
        ))
    }
}

pub fn run() -> Result<(), CliError> {
    run_cli(Cli::parse())
}

pub fn run_cli(cli: Cli) -> Result<(), CliError> {
    init_tracing();
    match cli.command {
        Command::Watch(args) => run_watch(args),
        Command::Bundle(args) => run_bundle(args),
        Command::Lsp(args) => run_lsp(args),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn run_lsp(args: LspArgs) -> Result<(), CliError> {
    let max_document_bytes = args
        .max_document_mb
        .checked_mul(1024 * 1024)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| {
            CliError::InvalidArguments(
                "--max-document-mb must be greater than zero and fit in memory".to_owned(),
            )
        })?;
    if args.max_diagnostics == 0 {
        return Err(CliError::InvalidArguments(
            "--max-diagnostics must be greater than zero".to_owned(),
        ));
    }
    if args.max_workspaces == 0 {
        return Err(CliError::InvalidArguments(
            "--max-workspaces must be greater than zero".to_owned(),
        ));
    }
    if args.max_semantic_tokens == 0 {
        return Err(CliError::InvalidArguments(
            "--max-semantic-tokens must be greater than zero".to_owned(),
        ));
    }
    if args.max_concurrent_requests == 0 {
        return Err(CliError::InvalidArguments(
            "--max-concurrent-requests must be greater than zero".to_owned(),
        ));
    }
    if args.max_analysis_cache_entries == 0 || args.max_dataset_cache_entries == 0 {
        return Err(CliError::InvalidArguments(
            "LSP cache entry limits must be greater than zero".to_owned(),
        ));
    }
    let config = avenger_lsp::LspServerConfig {
        semantic_debounce: Duration::from_millis(args.debounce_ms),
        max_document_bytes,
        max_diagnostics_per_document: args.max_diagnostics,
        max_workspaces: args.max_workspaces,
        max_semantic_tokens_per_document: args.max_semantic_tokens,
        max_concurrent_requests: args.max_concurrent_requests,
        max_analysis_cache_entries: args.max_analysis_cache_entries,
        max_dataset_cache_entries: args.max_dataset_cache_entries,
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(avenger_lsp::run_stdio_with_config(config));
    Ok(())
}

fn run_bundle(args: BundleArgs) -> Result<(), CliError> {
    let module = canonical_module_path(&args.module)?;
    let project_root = canonical_project_root(args.project_root.as_deref(), &module)?;
    if !module.starts_with(&project_root) {
        return Err(CliError::InvalidArguments(format!(
            "module '{}' is outside project root '{}'",
            module.display(),
            project_root.display()
        )));
    }
    if let Some(output) = &args.output {
        let output = if output.is_absolute() {
            output.clone()
        } else {
            std::env::current_dir()?.join(output)
        };
        if output == module || fs::canonicalize(&output).ok().as_ref() == Some(&module) {
            return Err(CliError::InvalidArguments(
                "bundle output must not overwrite the input module".to_owned(),
            ));
        }
    }
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .map_err(|error| CliError::Compiler(error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let bundle = if let Some(chart) = args.chart.as_deref() {
        runtime.block_on(compiler.bundle_chart(&module, Some(chart)))
    } else {
        runtime.block_on(compiler.bundle_module(&module))
    }
    .map_err(CliError::BundleCompile)?;
    if let Some(output) = args.output {
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, bundle.text)?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(bundle.text.as_bytes())?;
        stdout.flush()?;
    }
    Ok(())
}

fn run_watch(args: WatchArgs) -> Result<(), CliError> {
    if args.scale <= 0.0 || !args.scale.is_finite() {
        return Err(CliError::InvalidArguments(
            "--scale must be a finite value greater than zero".to_string(),
        ));
    }
    let module = canonical_module_path(&args.module)?;
    let project_root = canonical_project_root(args.project_root.as_deref(), &module)?;
    if !module.starts_with(&project_root) {
        return Err(CliError::InvalidArguments(format!(
            "module '{}' is outside project root '{}'",
            module.display(),
            project_root.display()
        )));
    }

    let cache = make_cache(&args);
    let reporter = ProcessWatchReporter::new(project_root.clone());
    let environment_factory = Arc::new(WatchEnvironmentFactory {
        cache: cache.clone(),
    });
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .environment_factory(environment_factory)
        .build()
        .map_err(|error| CliError::Compiler(error.to_string()))?;

    let worker_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let initial_started = Instant::now();
    let attempt = worker_runtime.block_on(compiler.compile_chart_generation_attempt(
        &module,
        args.chart.as_deref(),
        1,
    ));
    let initial_dependencies = attempt.dependencies.clone();
    let compiled = match attempt.result {
        Ok(compiled) => compiled,
        Err(failure) => {
            reporter.stdout(compile_failure_batch(1, &failure));
            return Err(CliError::InitialCompile);
        }
    };
    let resize_policy = compiled.artifact.compiled_plot().resize_policy();
    let chart_app_options = chart_app_options_for_compiled(compiled.artifact.compiled_plot());
    let window_scene_sizing = window_scene_sizing_for_resize_policy_and_binding(
        resize_policy,
        &chart_app_options.resize_binding,
    );
    let canvas_frame = canvas_frame_options_for_resize_policy_and_binding(
        resize_policy,
        &chart_app_options.resize_binding,
    );
    let context = compiled.environment.session_context_arc();
    let mut bundle = worker_runtime
        .block_on(chart_avenger_app_with_default_runtime_resources(
            compiled.artifact.compiled_plot().clone(),
            context,
            chart_app_options,
        ))
        .map_err(|error| CliError::App(error.to_string()))?;
    let displayed_state = bundle.app.app_state_mut().clone();

    let title = normal_title(&module);
    let window_options = bundle.configure_winit_options(
        WinitWgpuAvengerAppOptions::new(args.scale)
            .window_attributes(
                WindowAttributes::default()
                    .with_title(title.clone())
                    .with_resizable(true),
            )
            .window_scene_sizing(window_scene_sizing)
            .canvas_frame(canvas_frame)
            .resize_settle_delay_ms(Some(120)),
    );
    let host_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (mut host, event_loop) = WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
        bundle.app,
        window_options,
        host_runtime,
    )
    .map_err(|error| CliError::EventLoop(error.to_string()))?;
    let host_updates = host.host_update_sender();
    let signal_host_updates = host_updates.clone();
    ctrlc::set_handler(move || {
        let _ = signal_host_updates.request_exit();
    })
    .map_err(|error| CliError::Signal(error.to_string()))?;
    let dependency_count = dependency_watch_set(&module, &initial_dependencies).len();
    println!(
        "ready {} ({} local dependencies, {:.1} ms)",
        project_relative_path(&project_root, &module),
        dependency_count,
        initial_started.elapsed().as_secs_f64() * 1000.0
    );

    let reload_worker = spawn_reload_worker(ReloadWorker {
        chart: module,
        chart_selector: args.chart,
        project_root,
        compiler,
        runtime: worker_runtime,
        host_updates,
        reporter,
        initial_dependencies,
        debounce: Duration::from_millis(args.debounce_ms),
        cache,
        log_cache: args.log_cache,
        normal_title: title,
        displayed_state,
    })?;

    let event_loop_result = event_loop
        .run_app(&mut host)
        .map_err(|error| CliError::EventLoop(error.to_string()));
    let host_result = host
        .take_fatal_error()
        .map_or(Ok(()), |error| Err(CliError::EventLoop(error)));
    let shutdown_result = reload_worker.shutdown();
    event_loop_result?;
    host_result?;
    shutdown_result
}

fn make_cache(args: &WatchArgs) -> Option<Arc<EvaluationCache>> {
    if args.no_cache || physical_cache_disabled_by_env() {
        return None;
    }
    let max_memory_bytes = args.cache_memory_mb.saturating_mul(1024 * 1024);
    Some(EvaluationCache::new(EvaluationCacheConfig {
        max_memory_bytes,
        min_seen_count: 1,
        ..EvaluationCacheConfig::default()
    }))
}

fn chart_app_options_for_compiled(compiled: &avenger_chart::plot::CompiledPlot) -> ChartAppOptions {
    let inferred = compiled.get_layout_spec().resize_params();
    let valid_float64_param = |name: Option<String>, axis: &str| {
        name.and_then(|name| {
            let valid = compiled
                .param_specs()
                .get(&name)
                .is_some_and(|spec| spec.data_type == DataType::Float64);
            if valid {
                Some(name)
            } else {
                tracing::warn!(
                    target: "avenger_lang_cli::resize",
                    axis,
                    param = name,
                    "direct canvas resize parameter is unavailable or is not float64; leaving the axis unbound"
                );
                None
            }
        })
    };
    ChartAppOptions {
        resize_binding: ChartResizeBinding {
            width_param: valid_float64_param(inferred.width_param, "width"),
            height_param: valid_float64_param(inferred.height_param, "height"),
        },
        ..ChartAppOptions::default()
    }
}

fn canonical_module_path(path: &Path) -> Result<PathBuf, CliError> {
    if path.extension().and_then(|value| value.to_str()) != Some("avenger") {
        return Err(CliError::InvalidArguments(format!(
            "module '{}' must end in .avenger",
            path.display()
        )));
    }
    let path = fs::canonicalize(path)?;
    if !path.is_file() {
        return Err(CliError::InvalidArguments(format!(
            "module '{}' is not a file",
            path.display()
        )));
    }
    Ok(path)
}

fn canonical_project_root(root: Option<&Path>, module: &Path) -> Result<PathBuf, CliError> {
    let root = match root {
        Some(root) => fs::canonicalize(root)?,
        None => module
            .parent()
            .ok_or_else(|| CliError::InvalidArguments("module has no parent directory".into()))?
            .to_path_buf(),
    };
    if !root.is_dir() {
        return Err(CliError::InvalidArguments(format!(
            "project root '{}' is not a directory",
            root.display()
        )));
    }
    Ok(root)
}

fn normal_title(chart: &Path) -> String {
    format!(
        "Avenger — {}",
        chart
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("chart.avenger")
    )
}

trait WatchCompiler: Send + Sync + 'static {
    fn compile_generation<'a>(
        &'a self,
        chart: &'a Path,
        selector: Option<&'a str>,
        generation: u64,
    ) -> Pin<Box<dyn Future<Output = CompileAttempt<CompiledChartGeneration>> + 'a>>;
}

impl WatchCompiler for Compiler {
    fn compile_generation<'a>(
        &'a self,
        chart: &'a Path,
        selector: Option<&'a str>,
        generation: u64,
    ) -> Pin<Box<dyn Future<Output = CompileAttempt<CompiledChartGeneration>> + 'a>> {
        Box::pin(self.compile_chart_generation_attempt(chart, selector, generation))
    }
}

trait ReloadHost: Clone + Send + Sync + 'static {
    fn submit_update(
        &self,
        update: PreparedHostUpdate<ChartAppState>,
    ) -> Result<HostUpdateSubmitOutcome, HostUpdateSubmitError>;

    fn mark_request_epoch(&self, epoch: u64);

    fn set_window_title(&self, title: String) -> Result<(), HostUpdateSubmitError>;
}

impl ReloadHost for HostUpdateSender<ChartAppState> {
    fn submit_update(
        &self,
        update: PreparedHostUpdate<ChartAppState>,
    ) -> Result<HostUpdateSubmitOutcome, HostUpdateSubmitError> {
        self.submit(update)
    }

    fn mark_request_epoch(&self, epoch: u64) {
        self.mark_request_epoch(epoch);
    }

    fn set_window_title(&self, title: String) -> Result<(), HostUpdateSubmitError> {
        self.set_window_title(title)
    }
}

trait WatchReporter: Clone + Send + Sync + 'static {
    fn stdout(&self, batch: String);
    fn stderr(&self, line: String);
}

#[derive(Clone, Debug)]
struct ProcessWatchReporter {
    project_root: PathBuf,
}

impl ProcessWatchReporter {
    fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn redact_project_root(&self, text: String) -> String {
        let root = self.project_root.to_string_lossy();
        if root.is_empty() || self.project_root.parent().is_none() {
            text
        } else {
            text.replace(root.as_ref(), ".")
        }
    }
}

impl WatchReporter for ProcessWatchReporter {
    fn stdout(&self, batch: String) {
        let batch = self.redact_project_root(batch);
        let stdout = io::stdout();
        let mut output = stdout.lock();
        let _ = output.write_all(batch.as_bytes());
        let _ = output.flush();
    }

    fn stderr(&self, line: String) {
        eprintln!("{}", self.redact_project_root(line));
    }
}

struct ReloadWorker<C = Compiler, H = HostUpdateSender<ChartAppState>, R = ProcessWatchReporter> {
    chart: PathBuf,
    chart_selector: Option<String>,
    project_root: PathBuf,
    compiler: C,
    runtime: tokio::runtime::Runtime,
    host_updates: H,
    reporter: R,
    initial_dependencies: DiscoveredDependencySet,
    debounce: Duration,
    cache: Option<Arc<EvaluationCache>>,
    log_cache: bool,
    normal_title: String,
    displayed_state: ChartAppState,
}

enum ReloadSignal {
    Changed(Vec<PathBuf>),
    Shutdown,
}

const EVENT_QUEUE_CAPACITY: usize = 64;
const EVENT_PATH_LIMIT: usize = 1_024;
const WATCHER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_JOIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn next_reload_burst(
    signal_rx: &mpsc::Receiver<ReloadSignal>,
    debounce: Duration,
) -> Option<Vec<PathBuf>> {
    let mut affected = match signal_rx.recv() {
        Ok(ReloadSignal::Changed(paths)) => paths,
        Ok(ReloadSignal::Shutdown) | Err(_) => return None,
    };
    loop {
        match signal_rx.recv_timeout(debounce) {
            Ok(ReloadSignal::Changed(paths)) => {
                let remaining = EVENT_PATH_LIMIT.saturating_sub(affected.len());
                affected.extend(paths.into_iter().take(remaining));
            }
            Ok(ReloadSignal::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => break,
        }
    }
    affected.sort();
    affected.dedup();
    Some(affected)
}

struct ReloadWorkerHandle {
    signals: mpsc::SyncSender<ReloadSignal>,
    #[cfg(test)]
    request_reload: Arc<dyn Fn(Vec<PathBuf>) + Send + Sync>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl ReloadWorkerHandle {
    #[cfg(test)]
    fn request_reload(&self, paths: Vec<PathBuf>) {
        (self.request_reload)(paths);
    }

    fn shutdown(mut self) -> Result<(), CliError> {
        self.stopping.store(true, Ordering::Release);
        let _ = self.signals.try_send(ReloadSignal::Shutdown);
        if let Some(join) = self.join.take() {
            join_worker_with_timeout(join, WORKER_SHUTDOWN_TIMEOUT)?;
        }
        Ok(())
    }
}

fn join_worker_with_timeout(
    join: thread::JoinHandle<()>,
    timeout: Duration,
) -> Result<(), CliError> {
    let started = Instant::now();
    while !join.is_finished() {
        if started.elapsed() >= timeout {
            return Err(CliError::Watcher(format!(
                "reload worker did not stop within {:.1} seconds",
                timeout.as_secs_f64()
            )));
        }
        thread::sleep(WORKER_JOIN_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
    }
    join.join()
        .map_err(|_| CliError::Watcher("reload worker panicked during shutdown".into()))
}

fn spawn_reload_worker<C, H, R>(
    worker: ReloadWorker<C, H, R>,
) -> Result<ReloadWorkerHandle, CliError>
where
    C: WatchCompiler,
    H: ReloadHost,
    R: WatchReporter,
{
    let (signal_tx, signal_rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let targets = Arc::new(Mutex::new(DependencyWatchSet::new()));
    let change_epoch = Arc::new(AtomicU64::new(0));
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_targets = targets.clone();
    let callback_stopping = stopping.clone();
    let worker_stopping = stopping.clone();
    let request_tx = signal_tx.clone();
    let request_epoch = change_epoch.clone();
    let request_stopping = stopping.clone();
    let request_host_updates = worker.host_updates.clone();
    let request_reload: Arc<dyn Fn(Vec<PathBuf>) + Send + Sync> = Arc::new(move |mut paths| {
        if request_stopping.load(Ordering::Acquire) {
            return;
        }
        let epoch = request_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        request_host_updates.mark_request_epoch(epoch);
        paths.truncate(EVENT_PATH_LIMIT);
        tracing::trace!(
            target: "avenger_lang_cli::watch",
            epoch,
            path_count = paths.len(),
            "queued filesystem change"
        );
        let _ = request_tx.try_send(ReloadSignal::Changed(paths));
    });
    let callback_reload = request_reload.clone();
    let callback_reporter = worker.reporter.clone();
    let join = thread::Builder::new()
        .name("avenger-watch-reload".to_string())
        .spawn(move || {
            let watcher = RecommendedWatcher::new(
                move |result: Result<Event, notify::Error>| match result {
                    Ok(event)
                        if !callback_stopping.load(Ordering::Acquire) && relevant_event(&event) =>
                    {
                        let relevant = callback_targets
                            .lock()
                            .is_ok_and(|targets| event_matches_targets(&event, &targets));
                        if relevant {
                            callback_reload(event.paths);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => callback_reporter.stderr(format!(
                        "avenger watch: filesystem watcher error: {error}"
                    )),
                },
                Config::default(),
            );
            let mut watcher = match watcher {
                Ok(watcher) => watcher,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
            };
            let mut watched_anchors = BTreeMap::new();
            let mut last_good =
                dependency_watch_set(&worker.chart, &worker.initial_dependencies);
            let mut displayed_state = worker.displayed_state;
            let mut generation = 1_u64;
            if let Err(error) =
                update_watch_set(&mut watcher, &mut watched_anchors, &targets, &last_good)
            {
                let _ = ready_tx.send(Err(error.to_string()));
                return;
            }
            let _ = ready_tx.send(Ok(()));

            'worker: loop {
                let Some(affected) = next_reload_burst(&signal_rx, worker.debounce) else {
                    break 'worker;
                };
                if worker_stopping.load(Ordering::Acquire) {
                    break;
                }
                generation = generation.saturating_add(1);
                let compile_epoch = change_epoch.load(Ordering::Acquire);
                let _reload_span = tracing::debug_span!(
                    target: "avenger_lang_cli::watch",
                    "reload_generation",
                    generation,
                    request_epoch = compile_epoch,
                    affected_path_count = affected.len()
                )
                .entered();
                let before = worker.cache.as_ref().map(|cache| cache.metrics());
                let started = Instant::now();
                let Some(attempt) = block_on_until_stopped(
                    &worker.runtime,
                    worker_stopping.as_ref(),
                    worker
                        .compiler
                        .compile_generation(
                            &worker.chart,
                            worker.chart_selector.as_deref(),
                            generation,
                        ),
                ) else {
                    break;
                };
                if worker_stopping.load(Ordering::Acquire) {
                    break;
                }
                if compile_epoch != change_epoch.load(Ordering::Acquire) {
                    continue;
                }
                let attempt_targets = dependency_watch_set(&worker.chart, &attempt.dependencies);
                match attempt.result {
                    Ok(compiled) => {
                        let resize_policy = compiled.artifact.compiled_plot().resize_policy();
                        let chart_app_options =
                            chart_app_options_for_compiled(compiled.artifact.compiled_plot());
                        let window_scene_sizing =
                            window_scene_sizing_for_resize_policy_and_binding(
                                resize_policy,
                                &chart_app_options.resize_binding,
                            );
                        let canvas_frame = canvas_frame_options_for_resize_policy_and_binding(
                            resize_policy,
                            &chart_app_options.resize_binding,
                        );
                        let context = compiled.environment.session_context_arc();
                        let bundle = block_on_until_stopped(
                            &worker.runtime,
                            worker_stopping.as_ref(),
                            async {
                                let snapshot = displayed_state.snapshot_state().await;
                                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                                compiled.artifact.compiled_plot().clone(),
                                context,
                                chart_app_options,
                                &snapshot,
                                )
                                .await
                            },
                        );
                        let Some(bundle) = bundle else {
                            break;
                        };
                        if compile_epoch != change_epoch.load(Ordering::Acquire) {
                            continue;
                        }
                        match bundle {
                            Ok((mut bundle, migration)) => {
                                let replacement_state = bundle.app.app_state_mut().clone();
                                let effective = last_good
                                    .union(&attempt_targets)
                                    .cloned()
                                    .collect::<DependencyWatchSet>();
                                if let Err(error) = update_watch_set(
                                    &mut watcher,
                                    &mut watched_anchors,
                                    &targets,
                                    &effective,
                                ) {
                                    worker.reporter.stderr(format!(
                                        "avenger watch: failed to update watch set: {error}"
                                    ));
                                }
                                let hub = bundle.runtime_resources.render_invalidation_hub.clone();
                                let (completion_tx, completion_rx) = mpsc::sync_channel(1);
                                let submit = worker.host_updates.submit_update(PreparedHostUpdate {
                                        generation,
                                        request_epoch: compile_epoch,
                                        app: bundle.app,
                                        render_invalidation_hub: Some(hub),
                                        window_title: Some(worker.normal_title.clone()),
                                        window_scene_sizing,
                                        canvas_frame,
                                        completion: Some(completion_tx),
                                    });
                                match submit {
                                    Ok(HostUpdateSubmitOutcome::Superseded) => continue,
                                    Err(_) => return,
                                    Ok(HostUpdateSubmitOutcome::Queued) => {}
                                }
                                let install = loop {
                                    match completion_rx.recv_timeout(Duration::from_millis(25)) {
                                        Ok(outcome) => break Some(outcome),
                                        Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                                        Err(mpsc::RecvTimeoutError::Timeout)
                                            if worker_stopping.load(Ordering::Acquire) =>
                                        {
                                            break None;
                                        }
                                        Err(mpsc::RecvTimeoutError::Timeout)
                                            if compile_epoch
                                                < change_epoch.load(Ordering::Acquire) =>
                                        {
                                            break Some(HostUpdateInstallOutcome::Superseded);
                                        }
                                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                                    }
                                };
                                match install {
                                    Some(HostUpdateInstallOutcome::Installed) => {
                                        last_good = attempt_targets;
                                        if let Err(error) = update_watch_set(
                                            &mut watcher,
                                            &mut watched_anchors,
                                            &targets,
                                            &last_good,
                                        ) {
                                            worker.reporter.stderr(format!(
                                                "avenger watch: failed to update watch set: {error}"
                                            ));
                                        }
                                        displayed_state = replacement_state;
                                        worker.reporter.stdout(reload_success_batch(
                                            ReloadSuccessSummary {
                                                generation,
                                                elapsed: started.elapsed(),
                                                before: before.as_ref(),
                                                cache: worker.cache.as_ref(),
                                                log_cache: worker.log_cache,
                                                migration,
                                                project_root: &worker.project_root,
                                                affected: &affected,
                                            },
                                        ));
                                    }
                                    Some(HostUpdateInstallOutcome::Superseded) => continue,
                                    Some(HostUpdateInstallOutcome::Failed(error)) => {
                                        let _ = worker.host_updates.set_window_title(format!(
                                            "{} [runtime error]",
                                            worker.normal_title
                                        ));
                                        worker.reporter.stderr(format!(
                                            "avenger watch: generation {generation} installation failed: {error}"
                                        ));
                                    }
                                    None => return,
                                }
                            }
                            Err(error) => {
                                let effective = last_good
                                    .union(&attempt_targets)
                                    .cloned()
                                    .collect::<BTreeSet<_>>();
                                if let Err(watch_error) = update_watch_set(
                                    &mut watcher,
                                    &mut watched_anchors,
                                    &targets,
                                    &effective,
                                ) {
                                    worker.reporter.stderr(format!(
                                        "avenger watch: failed to update watch set: {watch_error}"
                                    ));
                                }
                                let _ = worker.host_updates.set_window_title(format!(
                                    "{} [runtime error]",
                                    worker.normal_title
                                ));
                                worker.reporter.stderr(format!(
                                    "avenger watch: generation {generation}: {error}"
                                ));
                            }
                        }
                    }
                    Err(failure) => {
                        let effective = last_good
                            .union(&attempt_targets)
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        if let Err(error) = update_watch_set(
                            &mut watcher,
                            &mut watched_anchors,
                            &targets,
                            &effective,
                        ) {
                            worker.reporter.stderr(format!(
                                "avenger watch: failed to update watch set: {error}"
                            ));
                        }
                        let _ = worker
                            .host_updates
                            .set_window_title(format!("{} [compile error]", worker.normal_title));
                        worker
                            .reporter
                            .stdout(compile_failure_batch(generation, &failure));
                    }
                }
            }
        })
        .map_err(|error| CliError::Watcher(error.to_string()))?;

    match ready_rx.recv_timeout(WATCHER_STARTUP_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            stopping.store(true, Ordering::Release);
            let _ = signal_tx.try_send(ReloadSignal::Shutdown);
            let _ = join_worker_with_timeout(join, WORKER_SHUTDOWN_TIMEOUT);
            return Err(CliError::Watcher(error));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            stopping.store(true, Ordering::Release);
            let _ = signal_tx.try_send(ReloadSignal::Shutdown);
            let _ = join_worker_with_timeout(join, WORKER_SHUTDOWN_TIMEOUT);
            return Err(CliError::Watcher(
                "reload worker exited before filesystem watcher initialization completed".into(),
            ));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            stopping.store(true, Ordering::Release);
            let _ = signal_tx.try_send(ReloadSignal::Shutdown);
            if join.is_finished() {
                let _ = join.join();
            }
            return Err(CliError::Watcher(format!(
                "filesystem watcher initialization did not complete within {:.1} seconds",
                WATCHER_STARTUP_TIMEOUT.as_secs_f64()
            )));
        }
    }
    Ok(ReloadWorkerHandle {
        signals: signal_tx,
        #[cfg(test)]
        request_reload,
        stopping,
        join: Some(join),
    })
}

fn block_on_until_stopped<F, T>(
    runtime: &tokio::runtime::Runtime,
    stopping: &std::sync::atomic::AtomicBool,
    future: F,
) -> Option<T>
where
    F: Future<Output = T>,
{
    runtime.block_on(async {
        tokio::select! {
            output = future => Some(output),
            _ = async {
                loop {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => None,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum WatchTargetKind {
    Exact,
    Tree,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct WatchTarget {
    path: PathBuf,
    kind: WatchTargetKind,
}

type DependencyWatchSet = BTreeSet<WatchTarget>;

fn dependency_watch_set(root: &Path, dependencies: &DiscoveredDependencySet) -> DependencyWatchSet {
    let mut targets = DependencyWatchSet::from([WatchTarget {
        path: root.to_path_buf(),
        kind: WatchTargetKind::Exact,
    }]);
    for dependency in dependencies.iter() {
        add_dependency_paths(&mut targets, dependency);
    }
    targets
}

fn add_dependency_paths(targets: &mut DependencyWatchSet, dependency: &CompiledDependency) {
    for origin in [&dependency.requested_origin, &dependency.canonical_origin] {
        if let SourceOrigin::File(path) = origin {
            targets.insert(watch_target(path));
        }
    }
}

fn watch_target(path: &Path) -> WatchTarget {
    if let Some(root) = glob_watch_root(path) {
        WatchTarget {
            path: root,
            kind: WatchTargetKind::Tree,
        }
    } else {
        WatchTarget {
            path: path.to_path_buf(),
            kind: if path.is_dir() {
                WatchTargetKind::Tree
            } else {
                WatchTargetKind::Exact
            },
        }
    }
}

fn glob_watch_root(path: &Path) -> Option<PathBuf> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchAnchorMode {
    NonRecursive,
    Recursive,
}

impl WatchAnchorMode {
    fn notify_mode(self) -> RecursiveMode {
        match self {
            Self::NonRecursive => RecursiveMode::NonRecursive,
            Self::Recursive => RecursiveMode::Recursive,
        }
    }
}

trait DependencyWatcher {
    fn watch_path(&mut self, path: &Path, mode: WatchAnchorMode) -> notify::Result<()>;
    fn unwatch_path(&mut self, path: &Path) -> notify::Result<()>;
}

impl DependencyWatcher for RecommendedWatcher {
    fn watch_path(&mut self, path: &Path, mode: WatchAnchorMode) -> notify::Result<()> {
        self.watch(path, mode.notify_mode())
    }

    fn unwatch_path(&mut self, path: &Path) -> notify::Result<()> {
        self.unwatch(path)
    }
}

fn update_watch_set<W: DependencyWatcher>(
    watcher: &mut W,
    watched_anchors: &mut BTreeMap<PathBuf, WatchAnchorMode>,
    shared_targets: &Arc<Mutex<DependencyWatchSet>>,
    targets: &DependencyWatchSet,
) -> notify::Result<()> {
    let mut desired_anchors = BTreeMap::new();
    for target in targets {
        let Some(anchor) = target.path.ancestors().find(|ancestor| ancestor.is_dir()) else {
            continue;
        };
        let mode = if target.kind == WatchTargetKind::Tree && anchor == target.path {
            WatchAnchorMode::Recursive
        } else {
            WatchAnchorMode::NonRecursive
        };
        desired_anchors
            .entry(anchor.to_path_buf())
            .and_modify(|current| {
                if mode == WatchAnchorMode::Recursive {
                    *current = mode;
                }
            })
            .or_insert(mode);
    }

    for (anchor, old_mode) in watched_anchors.iter() {
        if desired_anchors.get(anchor) != Some(old_mode) {
            watcher.unwatch_path(anchor)?;
        }
    }
    for (anchor, new_mode) in &desired_anchors {
        if watched_anchors.get(anchor) != Some(new_mode) {
            watcher.watch_path(anchor, *new_mode)?;
        }
    }
    *watched_anchors = desired_anchors;
    if let Ok(mut current) = shared_targets.lock() {
        *current = targets.clone();
    }
    Ok(())
}

fn relevant_event(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn event_matches_targets(event: &Event, targets: &DependencyWatchSet) -> bool {
    event.paths.iter().any(|event_path| {
        targets.iter().any(|target| {
            let lexical_match = match target.kind {
                WatchTargetKind::Exact => {
                    event_path.as_path() == target.path || target.path.starts_with(event_path)
                }
                WatchTargetKind::Tree => {
                    event_path.starts_with(&target.path) || target.path.starts_with(event_path)
                }
            };
            lexical_match
                || fs::canonicalize(event_path).ok().is_some_and(|canonical| {
                    canonical == target.path
                        || (target.kind == WatchTargetKind::Tree
                            && canonical.starts_with(&target.path))
                })
        })
    })
}

fn compile_failure_batch(generation: u64, failure: &CompileFailure) -> String {
    format!(
        "generation {generation} compilation failed:\n{}",
        failure.render()
    )
}

struct ReloadSuccessSummary<'a> {
    generation: u64,
    elapsed: Duration,
    before: Option<&'a avenger_chart::physical_cache::CacheMetricsSnapshot>,
    cache: Option<&'a Arc<EvaluationCache>>,
    log_cache: bool,
    migration: avenger_chart::plot::StateMigrationReport,
    project_root: &'a Path,
    affected: &'a [PathBuf],
}

fn reload_success_batch(summary: ReloadSuccessSummary<'_>) -> String {
    use std::fmt::Write as _;

    let mut output = format!(
        "reloaded generation {} after {} in {:.1} ms",
        summary.generation,
        display_affected_paths(summary.project_root, summary.affected),
        summary.elapsed.as_secs_f64() * 1000.0
    );
    if summary.log_cache {
        if let (Some(before), Some(cache)) = (summary.before, summary.cache) {
            let after = cache.metrics();
            let _ = write!(
                output,
                " (cache: +{} hits, +{} misses, {} entries, {} bytes)",
                after.hits.saturating_sub(before.hits),
                after.misses.saturating_sub(before.misses),
                after.entries,
                after.bytes
            );
        } else {
            output.push_str(" (cache disabled)");
        }
    }
    let _ = writeln!(
        output,
        " (state: {} migrated, {} reset)",
        summary.migration.migrated(),
        summary.migration.reset()
    );
    output
}

fn display_affected_paths(project_root: &Path, affected: &[PathBuf]) -> String {
    let mut paths = affected
        .iter()
        .map(|path| {
            path.strip_prefix(project_root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        "filesystem change".to_string()
    } else {
        paths.join(", ")
    }
}

fn project_relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_common::time::Instant as ChartInstant;
    use avenger_eventstream::window::{
        ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
    };
    use avenger_geometry::rtree::SceneGraphRTree;
    use clap::Parser;
    use filetime::{FileTime, set_file_mtime};
    use rstar::RTreeObject;
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::sync::Condvar;

    fn copy_directory(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("create copied fixture directory");
        for entry in fs::read_dir(source).expect("read fixture directory") {
            let entry = entry.expect("read fixture entry");
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            if source_path.is_dir() {
                copy_directory(&source_path, &destination_path);
            } else {
                fs::copy(&source_path, &destination_path).unwrap_or_else(|error| {
                    panic!(
                        "copy {} to {}: {error}",
                        source_path.display(),
                        destination_path.display()
                    )
                });
            }
        }
    }

    fn watch_fixture_copy() -> tempfile::TempDir {
        let temporary = tempfile::tempdir().expect("create temporary watch project");
        copy_directory(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/watch_project"),
            temporary.path(),
        );
        temporary
    }

    fn runtime_param(state: &ChartAppState, name: &str) -> datafusion::scalar::ScalarValue {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build state inspection runtime")
            .block_on(state.params())[name]
            .clone()
    }

    fn compile_and_prepare_scene(
        runtime: &tokio::runtime::Runtime,
        compiler: &Compiler,
        chart: &Path,
        generation: u64,
    ) -> (
        Value,
        ChartAppState,
        DiscoveredDependencySet,
        ChartResizeBinding,
    ) {
        let attempt =
            runtime.block_on(compiler.compile_chart_generation_attempt(chart, None, generation));
        let dependencies = attempt.dependencies;
        let compiled = attempt
            .result
            .unwrap_or_else(|failure| panic!("compile {}: {}", chart.display(), failure.render()));
        let app_options = chart_app_options_for_compiled(compiled.artifact.compiled_plot());
        let resize_binding = app_options.resize_binding.clone();
        let mut bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                compiled.artifact.compiled_plot().clone(),
                compiled.environment.session_context_arc(),
                app_options,
            ))
            .unwrap_or_else(|error| panic!("prepare {}: {error}", chart.display()));
        let scene = serde_json::to_value(bundle.app.scene_graph()).expect("serialize scene graph");
        let state = bundle.app.app_state_mut().clone();
        (scene, state, dependencies, resize_binding)
    }

    struct FakeCompileStep {
        generation: u64,
        delay: Duration,
        attempt: CompileAttempt<CompiledChartGeneration>,
    }

    struct FakeWatchCompiler {
        steps: Mutex<VecDeque<FakeCompileStep>>,
    }

    impl FakeWatchCompiler {
        fn new(steps: Vec<FakeCompileStep>) -> Self {
            Self {
                steps: Mutex::new(steps.into()),
            }
        }
    }

    impl WatchCompiler for FakeWatchCompiler {
        fn compile_generation<'a>(
            &'a self,
            _chart: &'a Path,
            _selector: Option<&'a str>,
            generation: u64,
        ) -> Pin<Box<dyn Future<Output = CompileAttempt<CompiledChartGeneration>> + 'a>> {
            let step = self
                .steps
                .lock()
                .expect("fake compiler steps")
                .pop_front()
                .expect("fake compiler has a queued step");
            assert_eq!(step.generation, generation);
            Box::pin(async move {
                tokio::time::sleep(step.delay).await;
                step.attempt
            })
        }
    }

    #[derive(Clone)]
    struct InstalledFakeGeneration {
        generation: u64,
        state: ChartAppState,
        scene: Value,
    }

    #[derive(Default)]
    struct FakeHostState {
        latest_request_epoch: u64,
        installed: Vec<InstalledFakeGeneration>,
        titles: Vec<String>,
        next_outcomes: VecDeque<HostUpdateInstallOutcome>,
    }

    #[derive(Clone, Default)]
    struct FakeReloadHost {
        state: Arc<(Mutex<FakeHostState>, Condvar)>,
    }

    impl FakeReloadHost {
        fn wait_for_installs(&self, count: usize) -> Vec<InstalledFakeGeneration> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (lock, changed) = self.state.as_ref();
            let mut state = lock.lock().expect("fake host state");
            while state.installed.len() < count {
                let now = Instant::now();
                assert!(
                    now < deadline,
                    "timed out waiting for {count} host installs"
                );
                let (next, _) = changed
                    .wait_timeout(state, deadline.saturating_duration_since(now))
                    .expect("wait for fake host install");
                state = next;
            }
            state.installed.clone()
        }

        fn titles(&self) -> Vec<String> {
            self.state.0.lock().expect("fake host state").titles.clone()
        }
    }

    impl ReloadHost for FakeReloadHost {
        fn submit_update(
            &self,
            mut update: PreparedHostUpdate<ChartAppState>,
        ) -> Result<HostUpdateSubmitOutcome, HostUpdateSubmitError> {
            let (lock, changed) = self.state.as_ref();
            let mut state = lock
                .lock()
                .map_err(|_| HostUpdateSubmitError::QueuePoisoned)?;
            if update.request_epoch < state.latest_request_epoch {
                if let Some(completion) = update.completion.take() {
                    let _ = completion.send(HostUpdateInstallOutcome::Superseded);
                }
                return Ok(HostUpdateSubmitOutcome::Superseded);
            }
            let outcome = state
                .next_outcomes
                .pop_front()
                .unwrap_or(HostUpdateInstallOutcome::Installed);
            if outcome == HostUpdateInstallOutcome::Installed {
                let app_state = update.app.app_state_mut().clone();
                let scene = serde_json::to_value(update.app.scene_graph())
                    .expect("serialize fake-host scene");
                state.installed.push(InstalledFakeGeneration {
                    generation: update.generation,
                    state: app_state,
                    scene,
                });
            }
            if let Some(title) = update.window_title.take() {
                state.titles.push(title);
            }
            if let Some(completion) = update.completion.take() {
                let _ = completion.send(outcome);
            }
            changed.notify_all();
            Ok(HostUpdateSubmitOutcome::Queued)
        }

        fn mark_request_epoch(&self, epoch: u64) {
            let mut state = self.state.0.lock().expect("fake host state");
            state.latest_request_epoch = state.latest_request_epoch.max(epoch);
        }

        fn set_window_title(&self, title: String) -> Result<(), HostUpdateSubmitError> {
            let (lock, changed) = self.state.as_ref();
            lock.lock()
                .map_err(|_| HostUpdateSubmitError::QueuePoisoned)?
                .titles
                .push(title);
            changed.notify_all();
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeWatchReporter {
        stdout: Arc<(Mutex<Vec<String>>, Condvar)>,
        stderr: Arc<Mutex<Vec<String>>>,
    }

    impl FakeWatchReporter {
        fn wait_for_stdout(&self, count: usize) -> Vec<String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (lock, changed) = self.stdout.as_ref();
            let mut batches = lock.lock().expect("fake stdout");
            while batches.len() < count {
                let now = Instant::now();
                assert!(
                    now < deadline,
                    "timed out waiting for {count} stdout batches"
                );
                let (next, _) = changed
                    .wait_timeout(batches, deadline.saturating_duration_since(now))
                    .expect("wait for stdout batch");
                batches = next;
            }
            batches.clone()
        }
    }

    impl WatchReporter for FakeWatchReporter {
        fn stdout(&self, batch: String) {
            let (lock, changed) = self.stdout.as_ref();
            lock.lock().expect("fake stdout").push(batch);
            changed.notify_all();
        }

        fn stderr(&self, line: String) {
            self.stderr.lock().expect("fake stderr").push(line);
        }
    }

    #[test]
    fn watch_command_parses_the_initial_options() {
        let cli = Cli::try_parse_from([
            "avenger",
            "watch",
            "chart.avenger",
            "--debounce-ms",
            "25",
            "--scale",
            "2",
            "--no-cache",
        ])
        .unwrap();
        let Command::Watch(args) = cli.command else {
            panic!("expected watch command");
        };
        assert_eq!(args.module, PathBuf::from("chart.avenger"));
        assert_eq!(args.chart, None);
        assert_eq!(args.debounce_ms, 25);
        assert_eq!(args.scale, 2.0);
        assert!(args.no_cache);
    }

    #[test]
    fn watch_command_defaults_to_four_x_native_render_scale() {
        let cli = Cli::try_parse_from(["avenger", "watch", "chart.avenger"]).unwrap();
        let Command::Watch(args) = cli.command else {
            panic!("expected watch command");
        };
        assert_eq!(args.scale, 4.0);
    }

    #[test]
    fn bundle_command_parses_module_chart_and_output() {
        let cli = Cli::try_parse_from([
            "avenger",
            "bundle",
            "charts.avenger",
            "--chart",
            "summary",
            "-o",
            "dist/summary.avenger",
        ])
        .unwrap();
        let Command::Bundle(args) = cli.command else {
            panic!("expected bundle command");
        };
        assert_eq!(args.module, PathBuf::from("charts.avenger"));
        assert_eq!(args.chart.as_deref(), Some("summary"));
        assert_eq!(args.output, Some(PathBuf::from("dist/summary.avenger")));
    }

    #[test]
    fn bundle_writes_standalone_source_and_refuses_input_overwrite() {
        let project = tempfile::tempdir().unwrap();
        let module = project.path().join("chart.avenger");
        fs::write(
            &module,
            "avenger 1; chart cartesian as chart { data: { values: [{ x: 1; y: 2; }]; } mark symbol { x: encoded \"x\"; y: encoded \"y\"; } }",
        )
        .unwrap();
        let output = project.path().join("dist/bundle.avenger");
        run_bundle(BundleArgs {
            module: module.clone(),
            chart: None,
            output: Some(output.clone()),
            project_root: Some(project.path().to_path_buf()),
        })
        .unwrap();
        let bundled = fs::read_to_string(output).unwrap();
        assert!(bundled.contains("chart cartesian as chart"));

        let error = run_bundle(BundleArgs {
            module: module.clone(),
            chart: None,
            output: Some(module),
            project_root: Some(project.path().to_path_buf()),
        })
        .unwrap_err();
        assert!(error.to_string().contains("must not overwrite"));
    }

    #[test]
    fn lsp_command_parses_resource_controls() {
        let cli = Cli::try_parse_from([
            "avenger",
            "lsp",
            "--debounce-ms",
            "25",
            "--max-document-mb",
            "4",
            "--max-diagnostics",
            "50",
            "--max-workspaces",
            "3",
        ])
        .unwrap();
        let Command::Lsp(args) = cli.command else {
            panic!("expected lsp command");
        };
        assert_eq!(args.debounce_ms, 25);
        assert_eq!(args.max_document_mb, 4);
        assert_eq!(args.max_diagnostics, 50);
        assert_eq!(args.max_workspaces, 3);
        assert_eq!(args.max_semantic_tokens, 100_000);
        assert_eq!(args.max_concurrent_requests, 16);
        assert_eq!(args.max_analysis_cache_entries, 64);
        assert_eq!(args.max_dataset_cache_entries, 512);
    }

    #[test]
    fn filesystem_event_filter_matches_only_dependency_paths() {
        let target = PathBuf::from("/tmp/project/chart.avenger");
        let targets = DependencyWatchSet::from([WatchTarget {
            path: target.clone(),
            kind: WatchTargetKind::Exact,
        }]);
        let matching = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![target],
            attrs: Default::default(),
        };
        let unrelated = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/tmp/project/notes.txt")],
            attrs: Default::default(),
        };
        assert!(event_matches_targets(&matching, &targets));
        assert!(!event_matches_targets(&unrelated, &targets));
    }

    #[test]
    fn filesystem_event_filter_covers_atomic_rename_remove_and_recreate() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

        let target = PathBuf::from("/tmp/project/chart.avenger");
        let targets = DependencyWatchSet::from([WatchTarget {
            path: target.clone(),
            kind: WatchTargetKind::Exact,
        }]);
        let events = [
            Event {
                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                paths: vec![
                    PathBuf::from("/tmp/project/.chart.avenger.tmp"),
                    target.clone(),
                ],
                attrs: Default::default(),
            },
            Event {
                kind: EventKind::Remove(RemoveKind::File),
                paths: vec![target.clone()],
                attrs: Default::default(),
            },
            Event {
                kind: EventKind::Create(CreateKind::File),
                paths: vec![target],
                attrs: Default::default(),
            },
        ];
        for event in events {
            assert!(relevant_event(&event));
            assert!(event_matches_targets(&event, &targets));
        }
    }

    #[test]
    fn newly_created_missing_parent_wakes_the_target() {
        let targets = DependencyWatchSet::from([WatchTarget {
            path: PathBuf::from("/tmp/project/missing/chart.avenger"),
            kind: WatchTargetKind::Exact,
        }]);
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::Folder),
            paths: vec![PathBuf::from("/tmp/project/missing")],
            attrs: Default::default(),
        };
        assert!(event_matches_targets(&event, &targets));
    }

    #[test]
    fn directory_and_glob_resources_match_descendant_changes() {
        let directory = WatchTarget {
            path: PathBuf::from("/tmp/project/data/parts"),
            kind: WatchTargetKind::Tree,
        };
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/project/data/parts/new.csv")],
            attrs: Default::default(),
        };
        assert!(event_matches_targets(
            &event,
            &DependencyWatchSet::from([directory])
        ));
        assert_eq!(
            glob_watch_root(Path::new("/tmp/project/data/parts/*.csv")),
            Some(PathBuf::from("/tmp/project/data/parts"))
        );
    }

    #[test]
    fn debounce_coalesces_and_bounds_one_reload_burst() {
        let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let first = PathBuf::from("/tmp/project/chart.avenger");
        let second = PathBuf::from("/tmp/project/data.csv");
        tx.send(ReloadSignal::Changed(vec![first.clone(), second.clone()]))
            .expect("send first change");
        tx.send(ReloadSignal::Changed(vec![first.clone()]))
            .expect("send duplicate change");
        tx.send(ReloadSignal::Changed(
            (0..EVENT_PATH_LIMIT + 50)
                .map(|index| PathBuf::from(format!("/tmp/project/generated/{index}")))
                .collect(),
        ))
        .expect("send oversized change set");

        let burst =
            next_reload_burst(&rx, Duration::from_millis(1)).expect("receive one debounced burst");
        assert!(burst.contains(&first));
        assert!(burst.contains(&second));
        assert!(burst.len() <= EVENT_PATH_LIMIT);
        assert_eq!(burst.iter().filter(|path| *path == &first).count(), 1);
    }

    #[test]
    fn shutdown_during_debounce_discards_the_partial_burst() {
        let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        tx.send(ReloadSignal::Changed(vec![PathBuf::from(
            "/tmp/project/chart.avenger",
        )]))
        .expect("send change");
        tx.send(ReloadSignal::Shutdown).expect("send shutdown");
        assert_eq!(next_reload_burst(&rx, Duration::from_secs(1)), None);
    }

    #[test]
    fn worker_join_is_bounded() {
        join_worker_with_timeout(thread::spawn(|| {}), Duration::from_secs(1))
            .expect("join completed worker");

        let timeout_error = join_worker_with_timeout(
            thread::spawn(|| thread::sleep(Duration::from_millis(30))),
            Duration::from_millis(1),
        )
        .expect_err("slow worker must not block shutdown indefinitely");
        assert!(timeout_error.to_string().contains("did not stop within"));
    }

    #[test]
    fn process_output_redacts_the_canonical_project_root() {
        let reporter = ProcessWatchReporter::new(PathBuf::from("/private/work/chart-project"));
        assert_eq!(
            reporter.redact_project_root(
                "error at /private/work/chart-project/defs/point.avenger".to_string()
            ),
            "error at ./defs/point.avenger"
        );
    }

    #[derive(Default)]
    struct FakeDependencyWatcher {
        watched: BTreeMap<PathBuf, WatchAnchorMode>,
        unwatched: Vec<PathBuf>,
    }

    impl DependencyWatcher for FakeDependencyWatcher {
        fn watch_path(&mut self, path: &Path, mode: WatchAnchorMode) -> notify::Result<()> {
            self.watched.insert(path.to_path_buf(), mode);
            Ok(())
        }

        fn unwatch_path(&mut self, path: &Path) -> notify::Result<()> {
            self.watched.remove(path);
            self.unwatched.push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn watch_set_replaces_exact_and_recursive_anchors_without_backend_restart() {
        let temporary = tempfile::tempdir().expect("create temporary watch root");
        let data = temporary.path().join("data");
        fs::create_dir(&data).expect("create data directory");
        let shared = Arc::new(Mutex::new(DependencyWatchSet::new()));
        let mut backend = FakeDependencyWatcher::default();
        let mut anchors = BTreeMap::new();
        let first = DependencyWatchSet::from([
            WatchTarget {
                path: temporary.path().join("chart.avenger"),
                kind: WatchTargetKind::Exact,
            },
            WatchTarget {
                path: data.clone(),
                kind: WatchTargetKind::Tree,
            },
        ]);
        update_watch_set(&mut backend, &mut anchors, &shared, &first).expect("install watch set");
        assert_eq!(
            backend.watched.get(&data),
            Some(&WatchAnchorMode::Recursive)
        );

        let second = DependencyWatchSet::from([WatchTarget {
            path: temporary.path().join("chart.avenger"),
            kind: WatchTargetKind::Exact,
        }]);
        update_watch_set(&mut backend, &mut anchors, &shared, &second).expect("replace watch set");
        assert!(backend.unwatched.contains(&data));
        assert_eq!(*shared.lock().expect("shared targets"), second);
    }

    #[test]
    fn environment_factory_isolates_contexts_over_one_cache() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let factory = WatchEnvironmentFactory {
            cache: Some(cache.clone()),
        };
        let request = CompileEnvironmentRequest {
            generation: 1,
            native_registry_profile: "test".to_string(),
            local_resource_versions: Vec::new(),
        };
        let first = factory.create(&request).unwrap();
        let second = factory
            .create(&CompileEnvironmentRequest {
                generation: 2,
                ..request
            })
            .unwrap();
        assert!(!std::ptr::eq(
            first.session_context(),
            second.session_context()
        ));
        let first_cache =
            avenger_chart::physical_cache::physical_cache_from_ctx(first.session_context())
                .unwrap();
        let second_cache =
            avenger_chart::physical_cache::physical_cache_from_ctx(second.session_context())
                .unwrap();
        assert!(Arc::ptr_eq(&cache, &first_cache));
        assert!(Arc::ptr_eq(&cache, &second_cache));
    }

    #[test]
    fn local_resource_provider_maps_datafusion_locations_to_snapshot_versions() {
        let provider = LocalResourceVersionProvider::new(vec![
            CompileEnvironmentResourceVersion {
                path: PathBuf::from("/tmp/project/data/rows.csv"),
                recursive: false,
                content_version: "sha256:rows".to_string(),
            },
            CompileEnvironmentResourceVersion {
                path: PathBuf::from("/tmp/project/data/parts"),
                recursive: true,
                content_version: "directory-sha256:parts".to_string(),
            },
        ]);
        assert_eq!(
            provider
                .version_for_location("tmp/project/data/rows.csv")
                .map(|version| version.content_version.as_str()),
            Some("sha256:rows")
        );
        assert_eq!(
            provider
                .version_for_location("tmp/project/data/parts/one.csv")
                .map(|version| version.content_version.as_str()),
            Some("directory-sha256:parts")
        );
        assert!(
            provider
                .version_for_location("tmp/project/data/untracked.csv")
                .is_none()
        );
    }

    #[test]
    fn cancellable_runtime_wait_stops_an_in_flight_job() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build cancellation runtime");
        let stopping = std::sync::atomic::AtomicBool::new(true);
        let output =
            block_on_until_stopped(&runtime, &stopping, std::future::pending::<&'static str>());
        assert_eq!(output, None);
    }

    #[test]
    fn stock_visual_fixture_roots_prepare_with_the_cli_host() {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects");
        let manifest_path = fixtures
            .parent()
            .expect("fixture projects directory has a parent")
            .join("visual_cases.json");
        let manifest: Value = serde_json::from_slice(
            &fs::read(&manifest_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display())),
        )
        .expect("fixture manifest is valid JSON");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build fixture runtime");
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_memory_bytes: 16 * 1024 * 1024,
            min_seen_count: 1,
            ..EvaluationCacheConfig::default()
        });
        let cases = manifest["cases"]
            .as_array()
            .expect("fixture manifest cases is an array");
        let expected = cases
            .iter()
            .filter(|case| {
                case["expectation"] == "visual"
                    && case
                        .get("host")
                        .is_none_or(|host| host.as_str() == Some("stock"))
            })
            .count();
        let mut prepared = 0_usize;

        for case in cases {
            if case["expectation"] != "visual"
                || case
                    .get("host")
                    .is_some_and(|host| host.as_str() != Some("stock"))
            {
                continue;
            }
            let root = case["root"].as_str().expect("fixture root is a string");
            let selector = case.get("selector").and_then(Value::as_str);
            let project_name = Path::new(root)
                .components()
                .next()
                .expect("fixture root has a project component");
            let project_root = fixtures.join(project_name.as_os_str());
            let chart = fixtures.join(root);
            let compiler = Compiler::builder()
                .project_root(&project_root)
                .environment_factory(Arc::new(WatchEnvironmentFactory {
                    cache: Some(cache.clone()),
                }))
                .build()
                .unwrap_or_else(|error| panic!("build compiler for {root}: {error}"));
            let generation = runtime
                .block_on(compiler.compile_chart_generation_attempt(&chart, selector, 1))
                .result
                .unwrap_or_else(|failure| panic!("compile {root}: {:#?}", failure.diagnostics));
            let context = generation.environment.session_context_arc();
            runtime
                .block_on(chart_avenger_app_with_default_runtime_resources(
                    generation.artifact.compiled_plot().clone(),
                    context,
                    ChartAppOptions::default(),
                ))
                .unwrap_or_else(|error| panic!("prepare CLI chart app for {root}: {error}"));
            prepared += 1;
        }

        assert_eq!(prepared, expected);
    }

    #[test]
    fn watch_project_exercises_import_catalog_data_and_typed_state() {
        let project = watch_fixture_copy();
        let chart = project.path().join("chart.avenger");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build watch fixture runtime");
        let compiler = Compiler::builder()
            .project_root(project.path())
            .build()
            .expect("build watch fixture compiler");
        let (scene, state, dependencies, resize_binding) =
            compile_and_prepare_scene(&runtime, &compiler, &chart, 1);

        assert!(!scene["marks"].as_array().expect("scene marks").is_empty());
        assert_eq!(
            runtime.block_on(state.params())["point_size"],
            datafusion::scalar::ScalarValue::Float64(Some(180.0))
        );
        assert_eq!(
            resize_binding,
            ChartResizeBinding::width_height("canvas_width", "canvas_height")
        );
        let dependency_paths = dependencies
            .iter()
            .flat_map(|dependency| [&dependency.requested_origin, &dependency.canonical_origin])
            .filter_map(|origin| match origin {
                SourceOrigin::File(path) => Some(path.as_path()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        for expected in [
            "chart.avenger",
            "marks/badge.avenger",
            "data.avenger",
            "data/rows.csv",
        ] {
            let expected = fs::canonicalize(project.path().join(expected))
                .expect("canonicalize expected dependency");
            assert!(
                dependency_paths.contains(expected.as_path()),
                "compiler dependency closure omitted {}: {dependency_paths:#?}",
                expected.display()
            );
        }
    }

    #[test]
    fn chart_source_default_edits_are_visible_across_reload_migration() {
        let project = tempfile::tempdir().expect("create temporary multi-chart project");
        copy_directory(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../avenger-lang-compiler/tests/fixtures/modules/multi_chart"),
            project.path(),
        );
        let chart = project.path().join("charts.avenger");
        let original = fs::read_to_string(&chart).expect("read multi-chart source");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build default-edit runtime");
        let compiler = Compiler::builder()
            .project_root(project.path())
            .build()
            .expect("build multi-chart compiler");

        let first = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, Some("cartesian"), 1))
            .result
            .expect("compile first cartesian generation");
        let mut first_bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                first.artifact.compiled_plot().clone(),
                first.environment.session_context_arc(),
                ChartAppOptions::default(),
            ))
            .expect("prepare first cartesian generation");
        let first_scene =
            serde_json::to_value(first_bundle.app.scene_graph()).expect("serialize first scene");
        let first_state = first_bundle.app.app_state_mut().clone();
        let untouched_snapshot = runtime.block_on(first_state.snapshot_state());

        fs::write(
            &chart,
            original.replace("CAST(64.0 AS DOUBLE)", "CAST(256.0 AS DOUBLE)"),
        )
        .expect("edit point-size default");
        let second = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, Some("cartesian"), 2))
            .result
            .expect("compile edited cartesian generation");
        let (mut second_bundle, report) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    second.artifact.compiled_plot().clone(),
                    second.environment.session_context_arc(),
                    ChartAppOptions::default(),
                    &untouched_snapshot,
                ),
            )
            .expect("prepare edited cartesian generation");
        let second_scene =
            serde_json::to_value(second_bundle.app.scene_graph()).expect("serialize second scene");
        let second_state = second_bundle.app.app_state_mut().clone();

        assert_eq!(report.params_migrated, 1);
        assert_eq!(
            runtime.block_on(second_state.params())["point_size"],
            datafusion::scalar::ScalarValue::Float64(Some(256.0))
        );
        assert_ne!(
            second_scene, first_scene,
            "the reloaded scene must reflect the newly authored param initializer"
        );

        second_state
            .set_param(
                "point_size",
                datafusion::scalar::ScalarValue::Float64(Some(321.0)),
            )
            .expect("modify runtime point size");
        let modified_snapshot = runtime.block_on(second_state.snapshot_state());
        fs::write(
            &chart,
            original.replace("CAST(64.0 AS DOUBLE)", "CAST(512.0 AS DOUBLE)"),
        )
        .expect("edit point-size default again");
        let third = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, Some("cartesian"), 3))
            .result
            .expect("compile third cartesian generation");
        let (mut third_bundle, _) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    third.artifact.compiled_plot().clone(),
                    third.environment.session_context_arc(),
                    ChartAppOptions::default(),
                    &modified_snapshot,
                ),
            )
            .expect("prepare third cartesian generation");
        let third_state = third_bundle.app.app_state_mut().clone();
        assert_eq!(
            runtime.block_on(third_state.params())["point_size"],
            datafusion::scalar::ScalarValue::Float64(Some(321.0)),
            "runtime-modified state must continue to win across reloads"
        );

        let third_snapshot = runtime.block_on(third_state.snapshot_state());
        fs::write(
            &chart,
            original.replace(
                "param CAST(64.0 AS DOUBLE) as point_size;",
                "param 64 as point_size;",
            ),
        )
        .expect("change the inferred point-size type");
        let fourth = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, Some("cartesian"), 4))
            .result
            .expect("compile type-changing cartesian generation");
        let (mut fourth_bundle, report) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    fourth.artifact.compiled_plot().clone(),
                    fourth.environment.session_context_arc(),
                    ChartAppOptions::default(),
                    &third_snapshot,
                ),
            )
            .expect("prepare type-changing cartesian generation");
        assert_eq!(report.params_migrated, 0);
        assert_eq!(report.params_reset, 1);
        assert_eq!(
            runtime.block_on(fourth_bundle.app.app_state_mut().params())["point_size"],
            datafusion::scalar::ScalarValue::Int64(Some(64))
        );
    }

    #[test]
    fn interactive_acceptance_fixture_prepares_visible_state_contracts() {
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/interactive_state");
        let chart = project_root.join("chart.avenger");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build interactive acceptance runtime");
        let compiler = Compiler::builder()
            .project_root(&project_root)
            .build()
            .expect("build interactive acceptance compiler");
        let first = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 1))
            .result
            .expect("compile interactive acceptance fixture");
        let plot = first.artifact.compiled_plot();

        assert!(plot.store_specs().get("dragged").is_some());
        assert!(plot.selection_specs().get("picked").is_some());
        assert_eq!(
            chart_app_options_for_compiled(plot).resize_binding,
            ChartResizeBinding::width_height("canvas_width", "canvas_height")
        );

        let mut first_bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                plot.clone(),
                first.environment.session_context_arc(),
                chart_app_options_for_compiled(plot),
            ))
            .expect("prepare interactive acceptance fixture");
        let initial_scene = serde_json::to_value(first_bundle.app.scene_graph())
            .expect("serialize initial acceptance scene");
        let point_center = {
            let rtree = SceneGraphRTree::from_scene_graph(first_bundle.app.scene_graph());
            let point = rtree
                .iter()
                .filter(|geometry| geometry.mark_instance.name == "points")
                .min_by_key(|geometry| geometry.mark_instance.instance_index)
                .expect("interactive source point geometry");
            let envelope = point.envelope();
            let lower = envelope.lower();
            let upper = envelope.upper();
            (1..10)
                .flat_map(|x_step| (1..10).map(move |y_step| (x_step, y_step)))
                .map(|(x_step, y_step)| {
                    [
                        lower[0] + (upper[0] - lower[0]) * x_step as f32 / 10.0,
                        lower[1] + (upper[1] - lower[1]) * y_step as f32 / 10.0,
                    ]
                })
                .find(|position| {
                    rtree
                        .pick_top_mark_at_point(position)
                        .is_some_and(|instance| instance.name == "points")
                })
                .expect("clickable source point area away from guide rules")
        };

        for event in [
            WindowEvent::CursorMoved(WindowCursorMoved {
                position: point_center,
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
            }),
        ] {
            runtime
                .block_on(first_bundle.app.update(&event, ChartInstant::now()))
                .expect("dispatch selection click");
        }
        let first_metrics = runtime.block_on(first_bundle.app.app_state_mut().event_metrics());
        assert_eq!(
            runtime.block_on(first_bundle.app.app_state_mut().params())["selected_size"],
            datafusion::scalar::ScalarValue::Float64(Some(520.0)),
            "selection click metrics: {first_metrics:?}"
        );

        for event in [
            WindowEvent::CursorMoved(WindowCursorMoved {
                position: [80.0, 80.0],
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            WindowEvent::CursorMoved(WindowCursorMoved {
                position: [160.0, 80.0],
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
            }),
        ] {
            runtime
                .block_on(first_bundle.app.update(&event, ChartInstant::now()))
                .expect("dispatch store drag");
        }
        let interacted_scene = serde_json::to_value(first_bundle.app.scene_graph())
            .expect("serialize interacted acceptance scene");
        assert_ne!(interacted_scene, initial_scene);
        assert_eq!(
            runtime.block_on(first_bundle.app.app_state_mut().params())["selected_size"],
            datafusion::scalar::ScalarValue::Float64(Some(520.0)),
            "a drag on an unrelated mark must not retrigger the point click binding"
        );
        let snapshot = runtime.block_on(first_bundle.app.app_state_mut().snapshot_state());

        let second = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 2))
            .result
            .expect("compile replacement acceptance fixture");
        let (mut second_bundle, migration) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    second.artifact.compiled_plot().clone(),
                    second.environment.session_context_arc(),
                    chart_app_options_for_compiled(second.artifact.compiled_plot()),
                    &snapshot,
                ),
            )
            .expect("prepare migrated acceptance fixture");
        assert_eq!(migration.params_migrated, 3);
        assert_eq!(migration.stores_migrated, 1);
        assert_eq!(migration.selections_migrated, 1);
        assert_eq!(migration.reset(), 0);
        assert_eq!(
            serde_json::to_value(second_bundle.app.scene_graph())
                .expect("serialize migrated acceptance scene"),
            interacted_scene
        );

        let migrated_point_center = {
            let rtree = SceneGraphRTree::from_scene_graph(second_bundle.app.scene_graph());
            let point = rtree
                .iter()
                .filter(|geometry| geometry.mark_instance.name == "points")
                .min_by_key(|geometry| geometry.mark_instance.instance_index)
                .expect("migrated source point geometry");
            let envelope = point.envelope();
            let lower = envelope.lower();
            let upper = envelope.upper();
            (1..10)
                .flat_map(|x_step| (1..10).map(move |y_step| (x_step, y_step)))
                .map(|(x_step, y_step)| {
                    [
                        lower[0] + (upper[0] - lower[0]) * x_step as f32 / 10.0,
                        lower[1] + (upper[1] - lower[1]) * y_step as f32 / 10.0,
                    ]
                })
                .find(|position| {
                    rtree
                        .pick_top_mark_at_point(position)
                        .is_some_and(|instance| instance.name == "points")
                })
                .expect("clickable migrated source point area away from guide rules")
        };
        for event in [
            WindowEvent::CursorMoved(WindowCursorMoved {
                position: migrated_point_center,
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
            }),
        ] {
            runtime
                .block_on(second_bundle.app.update(&event, ChartInstant::now()))
                .expect("dispatch migrated selection toggle");
        }
        assert_eq!(
            runtime.block_on(second_bundle.app.app_state_mut().params())["selected_size"],
            datafusion::scalar::ScalarValue::Float64(Some(80.0)),
            "the same point click must toggle the migrated selection off"
        );
    }

    #[test]
    fn shared_cache_reuses_style_reload_and_invalidates_same_metadata_data_change() {
        let project = watch_fixture_copy();
        let chart = project.path().join("chart.avenger");
        let definition = project.path().join("marks/badge.avenger");
        let data = project.path().join("data/rows.csv");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build cache integration runtime");
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_memory_bytes: 16 * 1024 * 1024,
            min_seen_count: 1,
            ..EvaluationCacheConfig::default()
        });
        let cached_compiler = Compiler::builder()
            .project_root(project.path())
            .environment_factory(Arc::new(WatchEnvironmentFactory {
                cache: Some(cache.clone()),
            }))
            .build()
            .expect("build cached watch compiler");
        let uncached_compiler = Compiler::builder()
            .project_root(project.path())
            .build()
            .expect("build uncached watch compiler");

        let (cached_initial, _, _, _) =
            compile_and_prepare_scene(&runtime, &cached_compiler, &chart, 1);
        let (uncached_initial, _, _, _) =
            compile_and_prepare_scene(&runtime, &uncached_compiler, &chart, 1);
        assert_eq!(cached_initial, uncached_initial);

        let definition_source = fs::read_to_string(&definition).expect("read mark definition");
        fs::write(
            &definition,
            definition_source.replacen("size: direct", "size: encoded", 1),
        )
        .expect("edit channel mode");
        let before_mode = cache.metrics();
        let (mode_changed, _, _, _) =
            compile_and_prepare_scene(&runtime, &cached_compiler, &chart, 2);
        let after_mode = cache.metrics();
        assert_ne!(
            mode_changed, cached_initial,
            "mode-only edit must install the newly compiled scene"
        );
        assert!(
            after_mode.hits > before_mode.hits,
            "an identical SQL expression plan should hit across a mode-only reload: before={before_mode:?}, after={after_mode:?}"
        );

        let definition_source =
            fs::read_to_string(&definition).expect("read mode-edited definition");
        fs::write(&definition, definition_source.replace("#7c3aed", "#dc2626"))
            .expect("edit mark style");
        let before_style = cache.metrics();
        let (styled, _, _, _) = compile_and_prepare_scene(&runtime, &cached_compiler, &chart, 3);
        let after_style = cache.metrics();
        assert_ne!(styled, mode_changed, "style edit must change the scene");
        assert!(
            after_style.hits > before_style.hits,
            "unchanged data plan should hit across a style-only reload: before={before_style:?}, after={after_style:?}"
        );

        let metadata = fs::metadata(&data).expect("read data metadata");
        let original_mtime = FileTime::from_last_modification_time(&metadata);
        let original_len = metadata.len();
        fs::write(&data, "x,y\n1,4\n3,2\n").expect("replace CSV with same-length content");
        assert_eq!(
            fs::metadata(&data).expect("new data metadata").len(),
            original_len
        );
        set_file_mtime(&data, original_mtime).expect("restore CSV modification time");
        let before_data = cache.metrics();
        let (changed_data, _, _, _) =
            compile_and_prepare_scene(&runtime, &cached_compiler, &chart, 4);
        let after_data = cache.metrics();
        assert_ne!(
            changed_data, styled,
            "changed CSV content must change the scene"
        );
        assert!(
            after_data.misses > before_data.misses,
            "same-size, same-mtime data replacement must miss stale physical results: before={before_data:?}, after={after_data:?}"
        );

        let current_uncached = Compiler::builder()
            .project_root(project.path())
            .build()
            .expect("build current uncached compiler");
        let (uncached_changed_data, _, _, _) =
            compile_and_prepare_scene(&runtime, &current_uncached, &chart, 4);
        assert_eq!(changed_data, uncached_changed_data);
    }

    #[test]
    fn headless_reload_coordinator_recovers_migrates_and_installs_only_latest() {
        let project = watch_fixture_copy();
        let chart = project.path().join("chart.avenger");
        let definition = project.path().join("marks/badge.avenger");
        let original_chart = fs::read_to_string(&chart).expect("read watch chart");
        let original_definition =
            fs::read_to_string(&definition).expect("read watch mark definition");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build reload coordinator runtime");
        let compiler = Compiler::builder()
            .project_root(project.path())
            .build()
            .expect("build reload fixture compiler");

        let initial_attempt =
            runtime.block_on(compiler.compile_chart_generation_attempt(&chart, None, 1));
        let initial_dependencies = initial_attempt.dependencies.clone();
        let initial = initial_attempt.result.expect("compile initial chart");
        let mut initial_bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                initial.artifact.compiled_plot().clone(),
                initial.environment.session_context_arc(),
                ChartAppOptions::default(),
            ))
            .expect("prepare initial chart");
        let displayed_state = initial_bundle.app.app_state_mut().clone();
        displayed_state
            .set_param(
                "point_size",
                datafusion::scalar::ScalarValue::Float64(Some(321.0)),
            )
            .expect("set displayed state before reload");

        fs::write(
            &definition,
            original_definition.replace("size: direct 180.0", "size: encoded 180.0"),
        )
        .expect("prepare mode-only generation");
        let success = runtime.block_on(compiler.compile_chart_generation_attempt(&chart, None, 2));

        fs::write(
            &chart,
            original_chart.replace("CAST(180.0 AS DOUBLE)", "CAST('not-a-number' AS DOUBLE)"),
        )
        .expect("prepare generation with an invalid constant cast");
        let failure = runtime.block_on(compiler.compile_chart_generation_attempt(&chart, None, 3));
        assert!(failure.result.is_err());

        fs::write(&chart, &original_chart).expect("restore valid generation");
        let repaired = runtime.block_on(compiler.compile_chart_generation_attempt(&chart, None, 5));
        assert!(repaired.result.is_ok());
        let newest = repaired.clone();

        let fake_compiler = FakeWatchCompiler::new(vec![
            FakeCompileStep {
                generation: 2,
                delay: Duration::ZERO,
                attempt: success,
            },
            FakeCompileStep {
                generation: 3,
                delay: Duration::ZERO,
                attempt: failure.clone(),
            },
            FakeCompileStep {
                generation: 4,
                delay: Duration::ZERO,
                attempt: failure,
            },
            FakeCompileStep {
                generation: 5,
                delay: Duration::ZERO,
                attempt: repaired,
            },
            FakeCompileStep {
                generation: 6,
                delay: Duration::from_millis(250),
                attempt: newest.clone(),
            },
            FakeCompileStep {
                generation: 7,
                delay: Duration::ZERO,
                attempt: newest,
            },
        ]);
        let host = FakeReloadHost::default();
        let reporter = FakeWatchReporter::default();
        let handle = spawn_reload_worker(ReloadWorker {
            chart: chart.clone(),
            chart_selector: None,
            project_root: project.path().to_path_buf(),
            compiler: fake_compiler,
            runtime,
            host_updates: host.clone(),
            reporter: reporter.clone(),
            initial_dependencies,
            debounce: Duration::from_millis(1),
            cache: None,
            log_cache: false,
            normal_title: "Avenger — chart.avenger".to_string(),
            displayed_state,
        })
        .expect("start headless reload worker");

        handle.request_reload(vec![definition.clone()]);
        let first_install = host.wait_for_installs(1);
        assert_eq!(first_install[0].generation, 2);
        assert_eq!(
            runtime_param(&first_install[0].state, "point_size"),
            datafusion::scalar::ScalarValue::Float64(Some(321.0))
        );

        handle.request_reload(vec![chart.clone()]);
        let after_first_failure = reporter.wait_for_stdout(2);
        assert_eq!(
            after_first_failure
                .iter()
                .filter(|batch| batch.contains("compilation failed"))
                .count(),
            1
        );
        assert!(
            after_first_failure
                .iter()
                .any(|batch| batch.contains("cannot be cast")),
            "{after_first_failure:#?}"
        );
        assert_eq!(host.wait_for_installs(1).len(), 1);

        handle.request_reload(vec![chart.clone()]);
        let after_second_failure = reporter.wait_for_stdout(3);
        assert_eq!(
            after_second_failure
                .iter()
                .filter(|batch| batch.contains("compilation failed"))
                .count(),
            2,
            "the same diagnostic must print once for each distinct generation"
        );

        handle.request_reload(vec![chart.clone()]);
        let repaired_installs = host.wait_for_installs(2);
        assert_eq!(repaired_installs[1].generation, 5);
        assert_eq!(
            runtime_param(&repaired_installs[1].state, "point_size"),
            datafusion::scalar::ScalarValue::Float64(Some(321.0))
        );
        assert_eq!(repaired_installs[0].scene, repaired_installs[1].scene);

        handle.request_reload(vec![chart.clone()]);
        thread::sleep(Duration::from_millis(25));
        handle.request_reload(vec![chart.clone()]);
        let latest_installs = host.wait_for_installs(3);
        assert_eq!(
            latest_installs
                .iter()
                .map(|installed| installed.generation)
                .collect::<Vec<_>>(),
            vec![2, 5, 7]
        );
        let final_stdout = reporter.wait_for_stdout(5);
        assert!(
            !final_stdout
                .iter()
                .any(|batch| batch.contains("generation 6"))
        );
        let titles = host.titles();
        assert!(
            titles
                .iter()
                .any(|title| title.ends_with("[compile error]"))
        );
        assert_eq!(
            titles.last().map(String::as_str),
            Some("Avenger — chart.avenger")
        );

        handle.shutdown().expect("stop headless reload worker");
    }

    #[test]
    fn successive_dsl_generations_migrate_compatible_state() {
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/interactive_state");
        let chart = project_root.join("chart.avenger");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build migration runtime");
        let compiler = Compiler::builder()
            .project_root(&project_root)
            .build()
            .expect("build fixture compiler");

        let first = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 1))
            .result
            .expect("compile first generation");
        let mut first_bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                first.artifact.compiled_plot().clone(),
                first.environment.session_context_arc(),
                ChartAppOptions::default(),
            ))
            .expect("prepare first generation");
        let first_state = first_bundle.app.app_state_mut().clone();
        first_state
            .set_param(
                "selected_size",
                datafusion::scalar::ScalarValue::Float64(Some(320.0)),
            )
            .expect("set first-generation state");
        assert_eq!(
            runtime.block_on(first_state.params())["selected_size"],
            datafusion::scalar::ScalarValue::Float64(Some(320.0))
        );
        let snapshot = runtime.block_on(first_state.snapshot_state());

        let second = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 2))
            .result
            .expect("compile second generation");
        let (mut second_bundle, report) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    second.artifact.compiled_plot().clone(),
                    second.environment.session_context_arc(),
                    ChartAppOptions::default(),
                    &snapshot,
                ),
            )
            .expect("prepare migrated generation");
        let second_state = second_bundle.app.app_state_mut().clone();
        assert_eq!(report.params_migrated, 3);
        assert_eq!(report.stores_migrated, 1);
        assert_eq!(report.selections_migrated, 1);
        assert_eq!(report.reset(), 0);
        assert_eq!(
            runtime.block_on(second_state.params())["selected_size"],
            datafusion::scalar::ScalarValue::Float64(Some(320.0))
        );
    }

    #[test]
    fn built_in_widget_document_state_migrates_across_generations() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects/11_widget_surface");
        let chart = project_root.join("text_input.avenger");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build widget migration runtime");
        let compiler = Compiler::builder()
            .project_root(&project_root)
            .build()
            .expect("build widget fixture compiler");

        let first = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 1))
            .result
            .expect("compile first widget generation");
        let mut first_bundle = runtime
            .block_on(chart_avenger_app_with_default_runtime_resources(
                first.artifact.compiled_plot().clone(),
                first.environment.session_context_arc(),
                ChartAppOptions::default(),
            ))
            .expect("prepare first widget generation");
        let first_state = first_bundle.app.app_state_mut().clone();
        first_state
            .set_param(
                "query_state",
                datafusion::scalar::ScalarValue::Utf8(Some("edited".to_string())),
            )
            .expect("set widget value param");
        first_state
            .set_param(
                "query__cursor",
                datafusion::scalar::ScalarValue::UInt64(Some(3)),
            )
            .expect("set widget cursor param");
        first_state
            .set_param(
                "query__selected_text",
                datafusion::scalar::ScalarValue::Utf8(Some("dit".to_string())),
            )
            .expect("set widget selection param");
        let snapshot = runtime.block_on(first_state.snapshot_state());

        let second = runtime
            .block_on(compiler.compile_chart_generation_attempt(&chart, None, 2))
            .result
            .expect("compile second widget generation");
        let (mut second_bundle, report) = runtime
            .block_on(
                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                    second.artifact.compiled_plot().clone(),
                    second.environment.session_context_arc(),
                    ChartAppOptions::default(),
                    &snapshot,
                ),
            )
            .expect("prepare migrated widget generation");
        let second_state = second_bundle.app.app_state_mut().clone();
        let params = runtime.block_on(second_state.params());
        assert_eq!(report.params_migrated, 3);
        assert_eq!(report.params_reset, 0);
        assert_eq!(
            params["query_state"],
            datafusion::scalar::ScalarValue::Utf8(Some("edited".to_string()))
        );
        assert_eq!(
            params["query__cursor"],
            datafusion::scalar::ScalarValue::UInt64(Some(3))
        );
        assert_eq!(
            params["query__selected_text"],
            datafusion::scalar::ScalarValue::Utf8(Some("dit".to_string()))
        );
    }
}
