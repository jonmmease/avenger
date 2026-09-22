use crate::{
    config::Config,
    dataflow::{Engine, Evaluation, Warming},
    layout,
    replay::{self, Appended, Cursor, Replay},
    scene,
    selection::{PLOTS, Selections},
};
use anyhow::Result;
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    background::{BackgroundTask, BackgroundTasks},
    error::AvengerAppError,
};
use avenger_common::time::Instant;
use avenger_datafusion_dataflow::{TableSnapshot, TableStore};
use avenger_eventstream::{
    manager::EventStreamHandler,
    runtime::{DebounceConfig, DebouncedCommit, RuntimeWakeKey},
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, EventStreamContext, EventStreamFilter, UpdateStatus},
    window::{Key, MouseButton, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_panels::Rect;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text::TextEngine;
use std::{
    sync::Arc,
    time::{Duration, Instant as Clock},
};

#[derive(Clone, PartialEq)]
struct ContextKey {
    focus: usize,
    fixed: [Option<[f64; 2]>; 3],
}
impl ContextKey {
    fn new(focus: usize, selections: &Selections) -> Self {
        let mut fixed = selections.bounds;
        fixed[focus] = None;
        Self { focus, fixed }
    }
}

#[derive(Clone)]
pub struct State {
    pub text: TextEngine,
    pub plots: Arc<Vec<Rect>>,
    pub selections: Selections,
    pub result: Arc<Evaluation>,
    pub latest: TableSnapshot,
    pub preview: Option<(usize, [f64; 2])>,
    pub error: Option<String>,
    pub replay_error: Option<String>,
    pub warmup_message: String,
    pub period: String,
    pub playing: bool,
    pub ended: bool,
    pub pending: bool,
    pub interval_ms: u64,
    foreground: BackgroundTask<Option<Arc<Evaluation>>>,
    ingestion: BackgroundTask<Option<Appended>>,
    poll: BackgroundTask<()>,
    engine: Arc<Engine>,
    replay: Arc<Replay>,
    store: TableStore,
    cursor: Cursor,
    max_batches: Option<usize>,
    next_append: Clock,
    warming: Option<Arc<Warming>>,
    fallback: Option<TableSnapshot>,
    context: ContextKey,
    focus: usize,
    drag: Option<usize>,
    diagnostics: bool,
    commit: DebouncedCommit<(usize, [f64; 2])>,
}
impl State {
    pub async fn load(config: Config, text: TextEngine) -> Result<(Self, BackgroundTasks)> {
        let plots = Arc::new(layout::plots()?);
        let selections = Selections::new(plots[0].width)?;
        let replay = Replay::open(&config.data, config.batch_rows)?;
        let engine = Engine::new(replay::schema(), &selections, config.diagnostics).await?;
        let latest = TableSnapshot::empty(replay::schema());
        let tasks = BackgroundTasks::new();
        let context = ContextKey::new(0, &selections);
        let mut state = Self {
            text,
            plots,
            selections,
            result: Arc::new(Evaluation::empty(latest.clone())),
            store: TableStore::new(latest.clone()),
            latest,
            preview: None,
            error: None,
            replay_error: None,
            foreground: tasks.task(),
            ingestion: tasks.task(),
            poll: tasks.task(),
            engine,
            replay,
            cursor: Cursor::default(),
            max_batches: config.max_batches,
            next_append: Clock::now(),
            warming: None,
            fallback: None,
            context,
            focus: 0,
            drag: None,
            period: "Waiting for first batch".into(),
            playing: !config.paused,
            ended: false,
            pending: true,
            interval_ms: config.interval_ms,
            diagnostics: config.diagnostics,
            warmup_message: "Warming initial snapshot".into(),
            commit: DebouncedCommit::new(DebounceConfig {
                wait: 5,
                max_wait: Some(5),
                leading: false,
            }),
        };
        state.ensure_warming(false)?;
        state.schedule_poll()?;
        if state.playing {
            state.append()?;
        }
        Ok((state, tasks))
    }

    fn schedule_poll(&mut self) -> Result<()> {
        self.poll.submit(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(())
    }

    fn append(&mut self) -> Result<()> {
        if self.ended || self.ingestion.is_pending() {
            return Ok(());
        }
        if self
            .max_batches
            .is_some_and(|limit| self.cursor.batches >= limit)
        {
            self.ended = true;
            return Ok(());
        }
        self.next_append = Clock::now() + Duration::from_millis(self.interval_ms);
        // Pausing does not cancel an accepted append. Restart uses a new store,
        // so a late completion can only affect its discarded replay generation.
        self.ingestion
            .submit(self.replay.clone().append(self.cursor, self.store.clone()))?;
        Ok(())
    }

    fn ensure_warming(&mut self, retry: bool) -> Result<()> {
        if self.warming.is_some() {
            return Ok(());
        }
        if !retry
            && self
                .fallback
                .as_ref()
                .is_some_and(|s| s.id() == self.latest.id())
        {
            return Ok(());
        }
        self.warming = Some(Arc::new(self.engine.warm(
            &self.selections,
            self.focus,
            self.latest.clone(),
        )?));
        self.warmup_message = format!(
            "Warming {} rows with full recomputation",
            self.latest.num_rows()
        );
        Ok(())
    }

    fn observe(&mut self, snapshot: &TableSnapshot) {
        if self
            .fallback
            .as_ref()
            .is_none_or(|s| s.num_rows() <= snapshot.num_rows())
        {
            self.fallback = Some(snapshot.clone());
        }
        // A foreground hit can observe targets before the timer. Retire that
        // candidate now so eviction before the next probe cannot stall progress.
        if self
            .warming
            .as_ref()
            .is_some_and(|warming| warming.snapshot.id() == snapshot.id())
        {
            self.warming = None;
        }
    }

    async fn tick(&mut self) -> Result<bool> {
        self.schedule_poll()?;
        let was_ended = self.ended;
        if self.playing && Clock::now() >= self.next_append {
            self.append()?;
        }
        let Some(warming) = self.warming.clone() else {
            return Ok(was_ended != self.ended);
        };
        if let Some(rows) = warming.probe().await? {
            self.observe(&warming.snapshot);
            self.warmup_message = format!(
                "{} rows cached after {:.0} ms (queue + compute + observation)",
                warming.snapshot.num_rows(),
                warming.elapsed_ms()
            );
            if self.diagnostics {
                eprintln!(
                    "Warm targets observed: {} input rows, {rows} target rows, {:.2} ms",
                    warming.snapshot.num_rows(),
                    warming.elapsed_ms()
                );
            }
            self.ensure_warming(false)?;
            self.submit()?;
            Ok(true)
        } else {
            Ok(was_ended != self.ended)
        }
    }

    fn submit(&mut self) -> Result<()> {
        self.error = None;
        let context = ContextKey::new(self.focus, &self.selections);
        if context != self.context {
            // Construct the replacement before dropping the old handle so the
            // runtime can preserve a group's queue position when targets match.
            let warming = self
                .engine
                .warm(&self.selections, self.focus, self.latest.clone())?;
            self.warming = Some(Arc::new(warming));
            self.fallback = None;
            self.context = context;
            self.foreground.cancel();
        }
        self.ensure_warming(false)?;
        let engine = self.engine.clone();
        let selections = self.selections.clone();
        let latest = self.latest.clone();
        let focus = self.focus;
        let fallbacks = self
            .warming
            .iter()
            .map(|w| w.snapshot.clone())
            .chain(self.fallback.iter().cloned())
            .collect::<Vec<_>>();
        self.pending = true;
        self.foreground.submit(async move {
            Ok::<_, anyhow::Error>(
                engine
                    .read(&selections, focus, latest, &fallbacks)
                    .await?
                    .map(Arc::new),
            )
        })?;
        Ok(())
    }

    fn activate(&mut self, focus: Option<usize>) -> Result<()> {
        if let Some(focus) = focus
            && focus != self.focus
        {
            self.focus = focus;
            self.submit()?;
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<()> {
        self.ingestion.cancel();
        self.foreground.cancel();
        self.warming = None;
        self.fallback = None;
        self.engine.clear();
        self.latest = TableSnapshot::empty(replay::schema());
        self.store = TableStore::new(self.latest.clone());
        self.result = Arc::new(Evaluation::empty(self.latest.clone()));
        self.cursor = Cursor::default();
        self.period = "Restarted historical replay".into();
        self.error = None;
        self.replay_error = None;
        self.ended = false;
        self.pending = true;
        self.next_append = Clock::now();
        self.ensure_warming(false)?;
        if self.playing {
            self.append()?;
        }
        Ok(())
    }

    fn commit_brush(&mut self, (plot, bounds): (usize, [f64; 2])) -> Result<()> {
        self.selections.set(plot, Some(bounds))?;
        self.submit()
    }
    fn cancel_drag(&mut self, status: &mut UpdateStatus) {
        self.drag = None;
        self.preview = None;
        status
            .commands
            .extend(self.commit.cancel(&commit_key()).commands);
    }
    fn hit(&self, position: [f32; 2]) -> Option<usize> {
        self.plots.iter().position(|r| {
            position[0] >= r.x
                && position[0] <= r.x + r.width
                && position[1] >= r.y
                && position[1] <= r.y + r.height
        })
    }
}
fn commit_key() -> RuntimeWakeKey {
    RuntimeWakeKey::new("streaming-flights", 0, "brush")
}
struct Builder;
#[async_trait]
impl SceneGraphBuilder<State> for Builder {
    async fn build(&self, s: &mut State) -> Result<SceneGraph, AvengerAppError> {
        scene::build(s).map_err(|e| AvengerAppError::InternalError(format!("{e:#}")))
    }
}
struct Input;
#[async_trait]
impl EventStreamHandler<State> for Input {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        s: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        match input(event, s).await {
            Ok(status) => status,
            Err(e) => {
                s.error = Some(format!("{e:#}"));
                UpdateStatus {
                    rerender: true,
                    ..Default::default()
                }
            }
        }
    }
}
async fn input(event: &SceneGraphEvent, s: &mut State) -> Result<UpdateStatus> {
    let mut status = UpdateStatus::default();
    match event {
        SceneGraphEvent::RuntimeWake(wake) => {
            if let Some(result) = s.foreground.handle_wake(wake) {
                match result {
                    Ok(result) => match result.as_ref() {
                        Some(result) => {
                            s.observe(&result.snapshot);
                            s.ensure_warming(false)?;
                            s.result = result.clone();
                            s.pending = false;
                            s.error = None;
                        }
                        None => {
                            s.pending = true;
                            s.ensure_warming(true)?;
                        }
                    },
                    Err(error) => s.error = Some(error.to_string()),
                }
                status.rerender = true;
            }
            if let Some(result) = s.ingestion.handle_wake(wake) {
                match result {
                    Ok(result) => match result.as_ref() {
                        Some(appended) => {
                            s.replay_error = None;
                            s.latest = appended.snapshot.clone();
                            s.cursor = appended.next;
                            s.period = appended.period.clone();
                            s.ensure_warming(false)?;
                        }
                        None => s.ended = true,
                    },
                    Err(error) => {
                        s.replay_error = Some(error.to_string());
                        s.playing = false;
                    }
                }
                status.rerender = true;
            }
            if s.poll.handle_wake(wake).is_some() {
                status.rerender |= s.tick().await?;
            }
            let update = s.commit.handle_wakeup(wake, Instant::now());
            status.commands.extend(update.commands);
            if let Some(value) = update.commit {
                s.commit_brush(value)?;
                status.rerender = true;
            }
        }
        SceneGraphEvent::CursorMoved(e) if s.drag.is_none() => {
            let focus = s.hit(e.position);
            status.rerender = focus.is_some_and(|focus| focus != s.focus);
            s.activate(focus)?;
        }
        SceneGraphEvent::MouseDown(e) => {
            if let Some(i) = s.hit(e.position) {
                if e.button == MouseButton::Left {
                    s.drag = Some(i);
                    s.preview = None;
                    s.activate(Some(i))?;
                    status.suppress_click = true;
                } else if e.button == MouseButton::Right {
                    s.cancel_drag(&mut status);
                    s.selections.set(i, None)?;
                    s.submit()?;
                }
                status.rerender = true;
            }
        }
        SceneGraphEvent::KeyPress(e) if e.key == Key::Named(NamedKey::Space) => {
            s.playing = !s.playing;
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if matches!(e.key, Key::Character('n' | 'N')) => {
            s.playing = false;
            s.append()?;
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if matches!(e.key, Key::Character('r' | 'R')) => {
            s.cancel_drag(&mut status);
            s.restart()?;
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if matches!(e.key, Key::Character('w' | 'W')) => {
            let warming = s.engine.warm(&s.selections, s.focus, s.latest.clone())?;
            s.warming = Some(Arc::new(warming));
            s.error = None;
            s.warmup_message = "Retrying latest materializations".into();
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if matches!(e.key, Key::Character('+' | '=' | '-')) => {
            s.interval_ms = if e.key == Key::Character('-') {
                (s.interval_ms.saturating_mul(2)).min(60_000)
            } else {
                (s.interval_ms / 2).max(10)
            };
            s.next_append = Clock::now() + Duration::from_millis(s.interval_ms);
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if e.key == Key::Named(NamedKey::Escape) => {
            s.cancel_drag(&mut status);
            for i in 0..PLOTS.len() {
                s.selections.set(i, None)?;
            }
            s.submit()?;
            status.rerender = true;
        }
        SceneGraphEvent::PointerCaptureLost | SceneGraphEvent::WindowFocused(false) => {
            s.cancel_drag(&mut status);
            s.activate(None)?;
            status.rerender = true;
        }
        _ => {}
    }
    Ok(status)
}
struct Drag;
#[async_trait]
impl EventStreamHandler<State> for Drag {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        UpdateStatus::default()
    }
    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        s: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut status = UpdateStatus::default();
        let Some(i) = s.drag else {
            return status;
        };
        let Some(start) = context
            .start_event
            .as_ref()
            .and_then(|e| e.event.position())
        else {
            return status;
        };
        let Some(end) = event.position() else {
            return status;
        };
        let released = matches!(event, SceneGraphEvent::MouseUp(_));
        let update = (|| -> Result<()> {
            if released && (end[0] - start[0]).abs() < 2. {
                s.cancel_drag(&mut status);
                s.selections.set(i, None)?;
                s.submit()?;
            } else {
                let r = s.plots[i];
                let scale = PLOTS[i].scale(r.width);
                let a = scale.invert_scalar((start[0] - r.x).clamp(0., r.width))? as f64;
                let b = scale.invert_scalar((end[0] - r.x).clamp(0., r.width))? as f64;
                let value = (i, [a.min(b), a.max(b)]);
                s.preview = Some(value);
                let mut update = s.commit.submit(value, Instant::now(), &commit_key());
                if released {
                    status.commands.extend(update.commands);
                    update = s.commit.flush(&commit_key());
                }
                status.commands.extend(update.commands);
                if let Some(value) = update.commit {
                    s.commit_brush(value)?;
                }
                if released {
                    s.drag = None;
                    s.preview = None;
                }
            }
            Ok(())
        })();
        if let Err(e) = update {
            s.error = Some(format!("{e:#}"));
        }
        status.rerender = true;
        status.suppress_click = true;
        status
    }
}
pub async fn make_app(state: State) -> Result<AvengerApp<State>> {
    let text = state.text.clone();
    let start = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        mark_names: Some(PLOTS.iter().map(|p| p.name.into()).collect()),
        filter: Some(vec![EventStreamFilter::context(
            |e, _, _| matches!(e, SceneGraphEvent::MouseDown(e) if e.button == MouseButton::Left),
        )]),
        ..Default::default()
    };
    let end = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        ..Default::default()
    };
    Ok(AvengerApp::try_new_with_text_engine(
        state,
        Arc::new(Builder),
        vec![
            (
                EventStreamConfig {
                    types: vec![
                        SceneGraphEventType::MouseDown,
                        SceneGraphEventType::CursorMoved,
                        SceneGraphEventType::KeyPress,
                        SceneGraphEventType::RuntimeWake,
                        SceneGraphEventType::PointerCaptureLost,
                        SceneGraphEventType::WindowFocused,
                    ],
                    ..Default::default()
                },
                Arc::new(Input),
            ),
            (
                EventStreamConfig {
                    types: vec![
                        SceneGraphEventType::CursorMoved,
                        SceneGraphEventType::MouseUp,
                    ],
                    between: Some((Box::new(start), Box::new(end))),
                    emit_between_end_event: true,
                    ..Default::default()
                },
                Arc::new(Drag),
            ),
        ],
        text,
    )
    .await?)
}

#[cfg(test)]
fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine()
        .with_number_formatting(
            avenger_text::NumberFormatConfig::new("d3"),
            std::sync::Arc::new(registry),
        )
        .with_datetime_formatting(
            avenger_text::DateTimeFormatConfig::new("d3"),
            std::sync::Arc::new({
                let mut registry = avenger_text::DateTimeFormatRegistry::default();
                registry.register(
                    "d3",
                    std::sync::Arc::new(avenger_format_datetime_d3::D3DateTimeFormatProvider),
                );
                registry
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{batch, ready};
    use datafusion::parquet::arrow::ArrowWriter;

    async fn paused() -> Result<(State, BackgroundTasks, tempfile::TempDir)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("2016-01.parquet");
        let mut writer =
            ArrowWriter::try_new(std::fs::File::create(&path)?, replay::schema(), None)?;
        writer.write(&batch(0, 80))?;
        writer.close()?;
        let config = Config {
            data: path,
            batch_rows: 20,
            interval_ms: 1000,
            max_batches: None,
            paused: true,
            diagnostics: false,
            headless: false,
        };
        let (state, tasks) = State::load(config, d3_text_engine()).await?;
        Ok((state, tasks, dir))
    }

    #[tokio::test]
    async fn newer_ingestion_does_not_forget_warming_candidate_and_restart_clears_fallbacks()
    -> Result<()> {
        let (mut state, _tasks, _dir) = paused().await?;
        ready(state.warming.as_ref().unwrap()).await?;
        state.tick().await?;
        let first = state.store.append_batch(batch(0, 20))?;
        state.latest = first.clone();
        state.ensure_warming(false)?;
        let second = state.store.append_batch(batch(20, 20))?;
        state.latest = second.clone();
        state.ensure_warming(false)?;
        assert_eq!(state.warming.as_ref().unwrap().snapshot.id(), first.id());
        ready(state.warming.as_ref().unwrap()).await?;
        state.tick().await?;
        assert_eq!(state.fallback.as_ref().unwrap().id(), first.id());
        assert_eq!(state.warming.as_ref().unwrap().snapshot.id(), second.id());

        let context = state.context.clone();
        state.selections.set(0, Some([0., 90.]))?;
        assert!(ContextKey::new(0, &state.selections) == context);
        state.selections.set(1, Some([3., 12.]))?;
        state.submit()?;
        assert!(state.fallback.is_none());
        state.restart()?;
        assert_eq!(state.latest.num_rows(), 0);
        assert_eq!(state.result.snapshot.num_rows(), 0);
        assert!(state.fallback.is_none());
        assert_ne!(state.store.snapshot().id(), first.id());
        // Building the initial scene also checks the carrier layout and empty axes.
        let scene = scene::build(&state)?;
        assert_eq!([scene.width, scene.height], layout::SIZE);
        Ok(())
    }

    struct TestExecutor(
        tokio::sync::mpsc::UnboundedSender<avenger_eventstream::runtime::RuntimeWakeEvent>,
    );
    impl avenger_app::background::host::Executor for TestExecutor {
        fn spawn(
            &self,
            future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
        ) {
            tokio::spawn(future);
        }
        fn wake(&self, event: avenger_eventstream::runtime::RuntimeWakeEvent) {
            let _ = self.0.send(event);
        }
    }

    #[tokio::test]
    async fn cache_miss_keeps_the_displayed_chart_and_schedules_warming() -> Result<()> {
        let (mut state, tasks, _dir) = paused().await?;
        state.latest = state.store.append_batch(batch(0, 80))?;
        ready(
            &state
                .engine
                .warm(&state.selections, 0, state.latest.clone())?,
        )
        .await?;
        state.result = Arc::new(
            state
                .engine
                .read(&state.selections, 0, state.latest.clone(), &[])
                .await?
                .unwrap(),
        );
        state.fallback = Some(state.latest.clone());
        state.warming = None;
        state.poll.cancel();
        let displayed = state.result.clone();
        state.engine.clear();
        state.submit()?;
        assert!(state.warming.is_none());

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let host =
            avenger_app::background::host::Attachment::new(&tasks, Arc::new(TestExecutor(sender)))?;
        host.activate();
        let wake = tokio::time::timeout(Duration::from_secs(20), receiver.recv())
            .await?
            .unwrap();
        let status = input(&SceneGraphEvent::RuntimeWake(wake), &mut state).await?;
        assert!(status.rerender);
        assert!(Arc::ptr_eq(&displayed, &state.result));
        assert!(state.pending);
        assert_eq!(
            state.warming.as_ref().unwrap().snapshot.id(),
            state.latest.id()
        );
        Ok(())
    }
}
