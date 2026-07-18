use std::{
    collections::BTreeSet,
    fs,
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
    EvaluationCache, EvaluationCacheConfig, install_shared_physical_cache,
    physical_cache_disabled_by_env,
};
use avenger_chart_app::{
    ChartAppOptions, ChartAppState, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app_with_default_runtime_resources,
};
use avenger_lang::{
    CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, CompileFailure, CompiledDependency, Compiler,
    DiscoveredDependencySet, SourceOrigin,
};
use avenger_winit_wgpu::{HostUpdateSender, PreparedHostUpdate};
use clap::{Args, Parser, Subcommand};
use datafusion::{execution::session_state::SessionStateBuilder, prelude::SessionContext};
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
        _request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        let context = if let Some(cache) = self.cache.as_ref() {
            let builder = install_shared_physical_cache(
                SessionStateBuilder::new().with_default_features(),
                cache.clone(),
            );
            SessionContext::new_with_state(builder.build())
        } else {
            SessionContext::new()
        };
        Ok(CompileEnvironment::new(context))
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
    let context = compiled.environment.session_context_arc();
    let bundle = worker_runtime
        .block_on(chart_avenger_app_with_default_runtime_resources(
            compiled.artifact.compiled_plot().clone(),
            context,
            ChartAppOptions::default(),
        ))
        .map_err(|error| CliError::App(error.to_string()))?;

    let title = normal_title(&chart);
    let window_options = bundle.configure_winit_options(
        WinitWgpuAvengerAppOptions::new(args.scale).window_attributes(
            WindowAttributes::default()
                .with_title(title.clone())
                .with_resizable(true),
        ),
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
    let dependency_count = dependency_targets(&chart, &initial_dependencies).len();
    println!(
        "ready {} ({} local dependencies, {:.1} ms)",
        chart.display(),
        dependency_count,
        initial_started.elapsed().as_secs_f64() * 1000.0
    );

    spawn_reload_worker(ReloadWorker {
        chart,
        compiler,
        runtime: worker_runtime,
        host_updates,
        initial_dependencies,
        debounce: Duration::from_millis(args.debounce_ms),
        cache,
        log_cache: args.log_cache,
        normal_title: title,
    })?;

    event_loop
        .run_app(&mut host)
        .map_err(|error| CliError::EventLoop(error.to_string()))
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
    compiler: Compiler,
    runtime: tokio::runtime::Runtime,
    host_updates: HostUpdateSender<ChartAppState>,
    initial_dependencies: DiscoveredDependencySet,
    debounce: Duration,
    cache: Option<Arc<EvaluationCache>>,
    log_cache: bool,
    normal_title: String,
}

fn spawn_reload_worker(worker: ReloadWorker) -> Result<(), CliError> {
    let (change_tx, change_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let targets = Arc::new(Mutex::new(BTreeSet::new()));
    let change_epoch = Arc::new(AtomicU64::new(0));
    let callback_targets = targets.clone();
    let callback_epoch = change_epoch.clone();
    thread::Builder::new()
        .name("avenger-watch-reload".to_string())
        .spawn(move || {
            let callback_tx = change_tx;
            let watcher = RecommendedWatcher::new(
                move |result: Result<Event, notify::Error>| match result {
                    Ok(event) if relevant_event(&event) => {
                        let relevant = callback_targets
                            .lock()
                            .is_ok_and(|targets| event_matches_targets(&event, &targets));
                        if relevant {
                            callback_epoch.fetch_add(1, Ordering::AcqRel);
                            let _ = callback_tx.send(());
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
            let mut watched_anchors = BTreeSet::new();
            let mut last_good = dependency_targets(&worker.chart, &worker.initial_dependencies);
            let mut generation = 1_u64;
            if let Err(error) =
                update_watch_set(&mut watcher, &mut watched_anchors, &targets, &last_good)
            {
                let _ = ready_tx.send(Err(error.to_string()));
                return;
            }
            let _ = ready_tx.send(Ok(()));

            while change_rx.recv().is_ok() {
                while change_rx.recv_timeout(worker.debounce).is_ok() {}
                generation = generation.saturating_add(1);
                let compile_epoch = change_epoch.load(Ordering::Acquire);
                let before = worker.cache.as_ref().map(|cache| cache.metrics());
                let started = Instant::now();
                let attempt = worker.runtime.block_on(
                    worker
                        .compiler
                        .compile_file_generation_attempt(&worker.chart, generation),
                );
                if compile_epoch != change_epoch.load(Ordering::Acquire) {
                    continue;
                }
                let attempt_targets = dependency_targets(&worker.chart, &attempt.dependencies);
                match attempt.result {
                    Ok(compiled) => {
                        let context = compiled.environment.session_context_arc();
                        let bundle = worker.runtime.block_on(
                            chart_avenger_app_with_default_runtime_resources(
                                compiled.artifact.compiled_plot().clone(),
                                context,
                                ChartAppOptions::default(),
                            ),
                        );
                        if compile_epoch != change_epoch.load(Ordering::Acquire) {
                            continue;
                        }
                        match bundle {
                            Ok(bundle) => {
                                last_good = attempt_targets;
                                if let Err(error) = update_watch_set(
                                    &mut watcher,
                                    &mut watched_anchors,
                                    &targets,
                                    &last_good,
                                ) {
                                    eprintln!("avenger watch: failed to update watch set: {error}");
                                }
                                let hub = bundle.runtime_resources.render_invalidation_hub.clone();
                                if worker
                                    .host_updates
                                    .submit(PreparedHostUpdate {
                                        generation,
                                        app: bundle.app,
                                        render_invalidation_hub: Some(hub),
                                        window_title: Some(worker.normal_title.clone()),
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                                print_reload_success(
                                    generation,
                                    started.elapsed(),
                                    before.as_ref(),
                                    worker.cache.as_ref(),
                                    worker.log_cache,
                                );
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
        .map_err(CliError::Watcher)
}

fn dependency_targets(root: &Path, dependencies: &DiscoveredDependencySet) -> BTreeSet<PathBuf> {
    let mut targets = BTreeSet::from([root.to_path_buf()]);
    for dependency in dependencies.iter() {
        add_dependency_paths(&mut targets, dependency);
    }
    targets
}

fn add_dependency_paths(targets: &mut BTreeSet<PathBuf>, dependency: &CompiledDependency) {
    for origin in [&dependency.requested_origin, &dependency.canonical_origin] {
        if let SourceOrigin::File(path) = origin {
            targets.insert(path.clone());
        }
    }
}

fn update_watch_set(
    watcher: &mut RecommendedWatcher,
    watched_anchors: &mut BTreeSet<PathBuf>,
    shared_targets: &Arc<Mutex<BTreeSet<PathBuf>>>,
    targets: &BTreeSet<PathBuf>,
) -> notify::Result<()> {
    let desired_anchors = targets
        .iter()
        .filter_map(|path| {
            path.ancestors()
                .skip(1)
                .find(|ancestor| ancestor.is_dir())
                .map(Path::to_path_buf)
        })
        .collect::<BTreeSet<_>>();

    for anchor in desired_anchors.difference(watched_anchors) {
        watcher.watch(anchor, RecursiveMode::NonRecursive)?;
    }
    for anchor in watched_anchors.difference(&desired_anchors) {
        watcher.unwatch(anchor)?;
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

fn event_matches_targets(event: &Event, targets: &BTreeSet<PathBuf>) -> bool {
    event.paths.iter().any(|event_path| {
        targets.iter().any(|target| {
            event_path == target
                || target.starts_with(event_path)
                || fs::canonicalize(event_path)
                    .ok()
                    .is_some_and(|canonical| canonical == *target)
        })
    })
}

fn print_compile_failure(generation: u64, failure: &CompileFailure) {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let _ = writeln!(output, "generation {generation} compilation failed:");
    for diagnostic in &failure.diagnostics {
        let _ = writeln!(
            output,
            "error[{}]: {}\n --> {}:{}..{}: {}",
            diagnostic.code.as_str(),
            diagnostic.message,
            diagnostic.primary.span.source,
            diagnostic.primary.span.range.start,
            diagnostic.primary.span.range.end,
            diagnostic.primary.message,
        );
        for note in &diagnostic.notes {
            let _ = writeln!(output, "  = note: {note}");
        }
    }
}

fn print_reload_success(
    generation: u64,
    elapsed: Duration,
    before: Option<&avenger_chart::physical_cache::CacheMetricsSnapshot>,
    cache: Option<&Arc<EvaluationCache>>,
    log_cache: bool,
) {
    print!(
        "reloaded generation {generation} in {:.1} ms",
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
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
        let targets = BTreeSet::from([target.clone()]);
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
        let targets = BTreeSet::from([PathBuf::from("/tmp/project/missing/chart.avenger")]);
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::Folder),
            paths: vec![PathBuf::from("/tmp/project/missing")],
            attrs: Default::default(),
        };
        assert!(event_matches_targets(&event, &targets));
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
}
