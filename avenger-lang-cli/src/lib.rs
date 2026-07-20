use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
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
    ChartAppOptions, ChartAppState, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    canvas_frame_options_for_resize_policy, chart_avenger_app_with_default_runtime_resources,
    chart_avenger_app_with_default_runtime_resources_and_snapshot,
    window_scene_sizing_for_resize_policy,
};
use avenger_lang::{
    CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, CompileEnvironmentResourceVersion, CompileFailure,
    CompiledDependency, Compiler, DiscoveredDependencySet, SourceOrigin,
};
use avenger_winit_wgpu::{
    HostUpdateInstallOutcome, HostUpdateSender, HostUpdateSubmitOutcome, PreparedHostUpdate,
};
use clap::{Args, Parser, Subcommand};
use datafusion::{
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
}

#[derive(Clone, Debug, Args)]
pub struct WatchArgs {
    /// Avenger chart source file.
    #[arg(value_name = "CHART")]
    chart: PathBuf,

    /// Override the project/capability root (defaults to the chart directory).
    #[arg(long, value_name = "DIR")]
    project_root: Option<PathBuf>,

    /// Filesystem-event quiet period before recompiling.
    #[arg(long, default_value_t = 100, value_name = "MILLIS")]
    debounce_ms: u64,

    /// Native render scale.
    #[arg(long, default_value_t = 1.0, value_name = "FACTOR")]
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
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn run_watch(args: WatchArgs) -> Result<(), CliError> {
    if args.scale <= 0.0 || !args.scale.is_finite() {
        return Err(CliError::InvalidArguments(
            "--scale must be a finite value greater than zero".to_string(),
        ));
    }
    let chart = canonical_chart_path(&args.chart)?;
    let project_root = canonical_project_root(args.project_root.as_deref(), &chart)?;
    if !chart.starts_with(&project_root) {
        return Err(CliError::InvalidArguments(format!(
            "chart '{}' is outside project root '{}'",
            chart.display(),
            project_root.display()
        )));
    }

    let cache = make_cache(&args);
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
    let attempt = worker_runtime.block_on(compiler.compile_file_generation_attempt(&chart, 1));
    let initial_dependencies = attempt.dependencies.clone();
    let compiled = match attempt.result {
        Ok(compiled) => compiled,
        Err(failure) => {
            print_compile_failure(1, &failure);
            return Err(CliError::InitialCompile);
        }
    };
    let resize_policy = compiled.artifact.compiled_plot().resize_policy();
    let context = compiled.environment.session_context_arc();
    let mut bundle = worker_runtime
        .block_on(chart_avenger_app_with_default_runtime_resources(
            compiled.artifact.compiled_plot().clone(),
            context,
            ChartAppOptions::default(),
        ))
        .map_err(|error| CliError::App(error.to_string()))?;
    let displayed_state = bundle.app.app_state_mut().clone();

    let title = normal_title(&chart);
    let window_options = bundle.configure_winit_options(
        WinitWgpuAvengerAppOptions::new(args.scale)
            .window_attributes(
                WindowAttributes::default()
                    .with_title(title.clone())
                    .with_resizable(true),
            )
            .window_scene_sizing(window_scene_sizing_for_resize_policy(resize_policy))
            .canvas_frame(canvas_frame_options_for_resize_policy(resize_policy))
            .resize_settle_delay_ms(Some(120)),
    );
    let host_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (mut host, event_loop) = WinitWgpuAvengerApp::new_and_event_loop_with_options(
        bundle.app,
        window_options,
        host_runtime,
    );
    let host_updates = host.host_update_sender();
    let signal_host_updates = host_updates.clone();
    ctrlc::set_handler(move || {
        let _ = signal_host_updates.request_exit();
    })
    .map_err(|error| CliError::Signal(error.to_string()))?;
    let dependency_count = dependency_watch_set(&chart, &initial_dependencies).len();
    println!(
        "ready {} ({} local dependencies, {:.1} ms)",
        chart.display(),
        dependency_count,
        initial_started.elapsed().as_secs_f64() * 1000.0
    );

    let reload_worker = spawn_reload_worker(ReloadWorker {
        chart,
        project_root,
        compiler,
        runtime: worker_runtime,
        host_updates,
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
    let shutdown_result = reload_worker.shutdown();
    event_loop_result?;
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

fn canonical_chart_path(path: &Path) -> Result<PathBuf, CliError> {
    if path.extension().and_then(|value| value.to_str()) != Some("avenger") {
        return Err(CliError::InvalidArguments(format!(
            "chart '{}' must end in .avenger",
            path.display()
        )));
    }
    let path = fs::canonicalize(path)?;
    if !path.is_file() {
        return Err(CliError::InvalidArguments(format!(
            "chart '{}' is not a file",
            path.display()
        )));
    }
    Ok(path)
}

fn canonical_project_root(root: Option<&Path>, chart: &Path) -> Result<PathBuf, CliError> {
    let root = match root {
        Some(root) => fs::canonicalize(root)?,
        None => chart
            .parent()
            .ok_or_else(|| CliError::InvalidArguments("chart has no parent directory".into()))?
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

struct ReloadWorker {
    chart: PathBuf,
    project_root: PathBuf,
    compiler: Compiler,
    runtime: tokio::runtime::Runtime,
    host_updates: HostUpdateSender<ChartAppState>,
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

struct ReloadWorkerHandle {
    signals: mpsc::SyncSender<ReloadSignal>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl ReloadWorkerHandle {
    fn shutdown(mut self) -> Result<(), CliError> {
        self.stopping.store(true, Ordering::Release);
        let _ = self.signals.try_send(ReloadSignal::Shutdown);
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| CliError::Watcher("reload worker panicked during shutdown".into()))?;
        }
        Ok(())
    }
}

fn spawn_reload_worker(worker: ReloadWorker) -> Result<ReloadWorkerHandle, CliError> {
    const EVENT_QUEUE_CAPACITY: usize = 64;
    const EVENT_PATH_LIMIT: usize = 1_024;
    let (signal_tx, signal_rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let targets = Arc::new(Mutex::new(DependencyWatchSet::new()));
    let change_epoch = Arc::new(AtomicU64::new(0));
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_targets = targets.clone();
    let callback_epoch = change_epoch.clone();
    let callback_stopping = stopping.clone();
    let worker_stopping = stopping.clone();
    let callback_tx = signal_tx.clone();
    let callback_host_updates = worker.host_updates.clone();
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
                            let epoch = callback_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                            callback_host_updates.mark_request_epoch(epoch);
                            let mut paths = event.paths;
                            paths.truncate(EVENT_PATH_LIMIT);
                            let _ = callback_tx.try_send(ReloadSignal::Changed(paths));
                        }
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("avenger watch: filesystem watcher error: {error}"),
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
                let mut affected = match signal_rx.recv() {
                    Ok(ReloadSignal::Changed(paths)) => paths,
                    Ok(ReloadSignal::Shutdown) | Err(_) => break,
                };
                loop {
                    match signal_rx.recv_timeout(worker.debounce) {
                        Ok(ReloadSignal::Changed(paths)) => {
                            let remaining = EVENT_PATH_LIMIT.saturating_sub(affected.len());
                            affected.extend(paths.into_iter().take(remaining));
                        }
                        Ok(ReloadSignal::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break 'worker;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                if worker_stopping.load(Ordering::Acquire) {
                    break;
                }
                affected.sort();
                affected.dedup();
                generation = generation.saturating_add(1);
                let compile_epoch = change_epoch.load(Ordering::Acquire);
                let before = worker.cache.as_ref().map(|cache| cache.metrics());
                let started = Instant::now();
                let Some(attempt) = block_on_until_stopped(
                    &worker.runtime,
                    worker_stopping.as_ref(),
                    worker
                        .compiler
                        .compile_file_generation_attempt(&worker.chart, generation),
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
                        let context = compiled.environment.session_context_arc();
                        let bundle = block_on_until_stopped(
                            &worker.runtime,
                            worker_stopping.as_ref(),
                            async {
                                let snapshot = displayed_state.snapshot_state().await;
                                chart_avenger_app_with_default_runtime_resources_and_snapshot(
                                compiled.artifact.compiled_plot().clone(),
                                context,
                                ChartAppOptions::default(),
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
                                    eprintln!("avenger watch: failed to update watch set: {error}");
                                }
                                let hub = bundle.runtime_resources.render_invalidation_hub.clone();
                                let (completion_tx, completion_rx) = mpsc::sync_channel(1);
                                let submit = worker.host_updates.submit(PreparedHostUpdate {
                                        generation,
                                        request_epoch: compile_epoch,
                                        app: bundle.app,
                                        render_invalidation_hub: Some(hub),
                                        window_title: Some(worker.normal_title.clone()),
                                        window_scene_sizing:
                                            window_scene_sizing_for_resize_policy(resize_policy),
                                        canvas_frame: canvas_frame_options_for_resize_policy(
                                            resize_policy,
                                        ),
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
                                            eprintln!(
                                                "avenger watch: failed to update watch set: {error}"
                                            );
                                        }
                                        displayed_state = replacement_state;
                                        print_reload_success(
                                            generation,
                                            started.elapsed(),
                                            before.as_ref(),
                                            worker.cache.as_ref(),
                                            worker.log_cache,
                                            migration,
                                            &worker.project_root,
                                            &affected,
                                        );
                                    }
                                    Some(HostUpdateInstallOutcome::Superseded) => continue,
                                    Some(HostUpdateInstallOutcome::Failed(error)) => {
                                        let _ = worker.host_updates.set_window_title(format!(
                                            "{} [runtime error]",
                                            worker.normal_title
                                        ));
                                        eprintln!(
                                            "avenger watch: generation {generation} installation failed: {error}"
                                        );
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
                                    eprintln!(
                                        "avenger watch: failed to update watch set: {watch_error}"
                                    );
                                }
                                let _ = worker.host_updates.set_window_title(format!(
                                    "{} [runtime error]",
                                    worker.normal_title
                                ));
                                eprintln!("avenger watch: generation {generation}: {error}");
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
                            eprintln!("avenger watch: failed to update watch set: {error}");
                        }
                        let _ = worker
                            .host_updates
                            .set_window_title(format!("{} [compile error]", worker.normal_title));
                        print_compile_failure(generation, &failure);
                    }
                }
            }
        })
        .map_err(|error| CliError::Watcher(error.to_string()))?;

    ready_rx
        .recv()
        .map_err(|error| CliError::Watcher(error.to_string()))?
        .map_err(CliError::Watcher)?;
    Ok(ReloadWorkerHandle {
        signals: signal_tx,
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

fn print_compile_failure(generation: u64, failure: &CompileFailure) {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let _ = writeln!(output, "generation {generation} compilation failed:");
    let _ = write!(output, "{}", failure.render());
}

fn print_reload_success(
    generation: u64,
    elapsed: Duration,
    before: Option<&avenger_chart::physical_cache::CacheMetricsSnapshot>,
    cache: Option<&Arc<EvaluationCache>>,
    log_cache: bool,
    migration: avenger_chart::plot::StateMigrationReport,
    project_root: &Path,
    affected: &[PathBuf],
) {
    print!(
        "reloaded generation {generation} after {} in {:.1} ms",
        display_affected_paths(project_root, affected),
        elapsed.as_secs_f64() * 1000.0
    );
    if log_cache {
        if let (Some(before), Some(cache)) = (before, cache) {
            let after = cache.metrics();
            print!(
                " (cache: +{} hits, +{} misses, {} entries, {} bytes)",
                after.hits.saturating_sub(before.hits),
                after.misses.saturating_sub(before.misses),
                after.entries,
                after.bytes
            );
        } else {
            print!(" (cache disabled)");
        }
    }
    print!(
        " (state: {} migrated, {} reset)",
        migration.migrated(),
        migration.reset()
    );
    println!();
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::Value;

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
        let Command::Watch(args) = cli.command;
        assert_eq!(args.chart, PathBuf::from("chart.avenger"));
        assert_eq!(args.debounce_ms, 25);
        assert_eq!(args.scale, 2.0);
        assert!(args.no_cache);
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
        let mut prepared = 0_usize;

        for case in manifest["cases"]
            .as_array()
            .expect("fixture manifest cases is an array")
        {
            if case["expectation"] != "visual"
                || case
                    .get("host")
                    .is_some_and(|host| host.as_str() != Some("stock"))
            {
                continue;
            }
            let root = case["root"].as_str().expect("fixture root is a string");
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
                .block_on(compiler.compile_file_generation_attempt(&chart, 1))
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

        assert_eq!(prepared, 37, "stock visual fixture census changed");
    }

    #[test]
    fn successive_dsl_generations_migrate_compatible_state() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-lang-compiler/tests/fixtures/projects/03_interactive_brush");
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
            .block_on(compiler.compile_file_generation_attempt(&chart, 1))
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
                "hover_count",
                datafusion::scalar::ScalarValue::Int64(Some(9)),
            )
            .expect("set first-generation state");
        assert_eq!(
            runtime.block_on(first_state.params())["hover_count"],
            datafusion::scalar::ScalarValue::Int64(Some(9))
        );
        let snapshot = runtime.block_on(first_state.snapshot_state());

        let second = runtime
            .block_on(compiler.compile_file_generation_attempt(&chart, 2))
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
        assert_eq!(report.params_migrated, 2);
        assert_eq!(report.stores_migrated, 1);
        assert_eq!(report.selections_migrated, 1);
        assert_eq!(report.reset(), 0);
        assert_eq!(
            runtime.block_on(second_state.params())["hover_count"],
            datafusion::scalar::ScalarValue::Int64(Some(9))
        );
    }
}
