use crate::{
    config::Config,
    dataflow::{Engine, Evaluation},
    layout, scene,
    selection::{PLOTS, Selections},
    worker::{Worker, completion_key},
};
use anyhow::Result;
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_common::time::Instant;
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
use std::sync::Arc;

#[derive(Clone)]
pub struct State {
    pub text: TextEngine,
    pub plots: Arc<Vec<Rect>>,
    pub selections: Selections,
    pub result: Arc<Evaluation>,
    pub rows: i64,
    pub preview: Option<(usize, [f64; 2])>,
    pub error: Option<String>,
    pub pending: bool,
    pub preaggregate: bool,
    pub warmup: String,
    worker: Arc<Worker>,
    focus: Option<usize>,
    generations: [u64; 2],
    drag: Option<usize>,
    commit: DebouncedCommit<(usize, [f64; 2])>,
}
impl State {
    pub async fn load(config: Config, text: TextEngine) -> Result<(Self, Arc<Worker>)> {
        let plots = Arc::new(layout::plots()?);
        let selections = Selections::new(plots[0].width)?;
        let engine = Engine::load(&config, &selections).await?;
        let result = Arc::new(engine.query(&selections, None, false).await?);
        let rows = result.bins[0].iter().map(|(_, n)| n).sum();
        let worker = Worker::new(engine);
        Ok((
            Self {
                text,
                plots,
                selections,
                result,
                rows,
                preaggregate: config.preaggregate,
                preview: None,
                error: None,
                pending: false,
                worker: worker.clone(),
                focus: None,
                generations: [0; 2],
                drag: None,
                warmup: if config.preaggregate {
                    "Hover a plot to warm its cross-filters"
                } else {
                    "Direct mode"
                }
                .into(),
                commit: DebouncedCommit::new(DebounceConfig {
                    wait: 5,
                    max_wait: Some(5),
                    leading: false,
                }),
            },
            worker,
        ))
    }
    fn submit(&mut self) {
        self.generations[0] += 1;
        self.pending = true;
        self.error = None;
        self.worker.submit(
            self.generations[0],
            self.selections.clone(),
            self.focus,
            false,
        );
    }
    fn activate(&mut self, focus: Option<usize>) {
        if focus == self.focus {
            return;
        }
        self.focus = focus;
        self.generations[1] += 1;
        self.worker.cancel(true);
        if !self.preaggregate {
            return;
        }
        self.warmup = "Hover a plot to warm its cross-filters".into();
        if let Some(i) = focus {
            self.warmup = format!("Warming {}…", PLOTS[i].title);
            self.worker
                .submit(self.generations[1], self.selections.clone(), focus, true);
        }
    }
    fn commit_brush(&mut self, (plot, bounds): (usize, [f64; 2])) -> Result<()> {
        self.selections.set(plot, Some(bounds))?;
        self.submit();
        Ok(())
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
    RuntimeWakeKey::new("mosaic-flights", 0, "brush")
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
        match input(event, s) {
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
fn input(event: &SceneGraphEvent, s: &mut State) -> Result<UpdateStatus> {
    let mut status = UpdateStatus::default();
    match event {
        SceneGraphEvent::RuntimeWake(wake)
            if wake.key == completion_key(false) || wake.key == completion_key(true) =>
        {
            let warm = wake.key == completion_key(true);
            if let Some((generation, result)) = s.worker.take(warm)
                && generation == s.generations[usize::from(warm)]
            {
                if warm {
                    s.warmup = match result {
                        Ok(r) => format!("Warm-up ready · {:.0} ms", r.elapsed_ms),
                        Err(e) => format!("Warm-up failed: {e}"),
                    };
                } else {
                    s.pending = false;
                    match result {
                        Ok(r) => {
                            s.result = Arc::new(r);
                            s.error = None;
                        }
                        Err(e) => s.error = Some(e),
                    }
                }
                status.rerender = true;
            }
        }
        SceneGraphEvent::RuntimeWake(wake) => {
            let update = s.commit.handle_wakeup(wake, Instant::now());
            status.commands.extend(update.commands);
            if let Some(value) = update.commit {
                s.commit_brush(value)?;
                status.rerender = true;
            }
        }
        SceneGraphEvent::CursorMoved(e) if s.drag.is_none() => {
            let focus = s.hit(e.position);
            status.rerender = focus != s.focus;
            s.activate(focus);
        }
        SceneGraphEvent::MouseDown(e) => {
            if let Some(i) = s.hit(e.position) {
                if e.button == MouseButton::Left {
                    s.drag = Some(i);
                    s.preview = None;
                    s.activate(Some(i));
                    status.suppress_click = true;
                } else if e.button == MouseButton::Right {
                    s.cancel_drag(&mut status);
                    s.selections.set(i, None)?;
                    s.submit();
                }
                status.rerender = true;
            }
        }
        SceneGraphEvent::KeyPress(e) if e.key == Key::Named(NamedKey::Escape) => {
            s.cancel_drag(&mut status);
            for i in 0..PLOTS.len() {
                s.selections.set(i, None)?;
            }
            s.submit();
            status.rerender = true;
        }
        SceneGraphEvent::PointerCaptureLost | SceneGraphEvent::WindowFocused(false) => {
            s.cancel_drag(&mut status);
            s.activate(None);
            status.rerender = true;
        }
        SceneGraphEvent::WindowCloseRequested => s.worker.shutdown(),
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
                s.submit();
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
                        SceneGraphEventType::WindowCloseRequested,
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
