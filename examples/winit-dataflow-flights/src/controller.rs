use crate::{
    config::Config,
    dataflow::{Engine, Metadata, Request},
    layout::{DashboardLayout, contains},
    scene::{self, Rendered},
    selection::{Focus, Selections},
    worker::{Worker, completion_key},
};
use anyhow::Result;
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneBuild, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    runtime::{
        DebounceConfig, DebouncedCommit, RuntimeHostCommand, RuntimeTooltipPresentation,
        RuntimeTooltipRow, RuntimeTooltipUpdate, RuntimeWakeKey,
    },
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, EventStreamContext, EventStreamFilter, UpdateStatus},
    window::{Key, MouseButton, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_panels::Rect;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text::TextEngine;
use avenger_widgets::{WidgetAction, WidgetRuntime};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct State {
    pub size: [f32; 2],
    pub engine: TextEngine,
    pub metadata: Metadata,
    pub config: Config,
    pub rendered: Arc<Rendered>,
    pub selections: Selections,
    pub bins: BTreeMap<String, i32>,
    pub widgets: WidgetRuntime,
    pub preview: Option<[f64; 4]>,
    pub error: Option<String>,
    pub pending: bool,
    layout: DashboardLayout,
    worker: Arc<Worker>,
    focus: Option<Focus>,
    generation: u64,
    drag: bool,
    commit: DebouncedCommit<[f64; 4]>,
}
impl State {
    pub async fn load(config: Config, engine: TextEngine) -> Result<(Self, Arc<Worker>)> {
        Self::load_at(config, engine, [1280., 940.]).await
    }
    pub async fn load_at(
        config: Config,
        engine: TextEngine,
        size: [f32; 2],
    ) -> Result<(Self, Arc<Worker>)> {
        let mut queries = Engine::load(config.clone()).await?;
        let metadata = queries.metadata.clone();
        let layout = DashboardLayout::solve(&metadata, size)?;
        let selections = Selections::new(
            &metadata.carriers,
            metadata.domains,
            [layout.scatter.width, layout.scatter.height],
            config.exact,
        )?;
        let bins = BTreeMap::new();
        let request = Request {
            selections: selections.clone(),
            scatter_size: [layout.scatter.width, layout.scatter.height],
            airline_size: [layout.airline.width, layout.airline.height],
            panel_size: layout.panel_size(),
            bins: bins.clone(),
            focus: None,
        };
        let evaluation = queries.job(&request, false).await?.unwrap().run().await?;
        if config.diagnostics {
            evaluation.print("Initial");
        }
        let rendered = Arc::new(Rendered::new(
            &evaluation,
            &metadata,
            layout.clone(),
            None,
            selections.clone(),
        )?);
        let worker = Worker::new(queries);
        Ok((
            Self {
                size,
                engine,
                metadata,
                config,
                rendered,
                selections,
                bins,
                widgets: WidgetRuntime::new(),
                preview: None,
                error: None,
                pending: false,
                layout,
                worker: worker.clone(),
                focus: None,
                generation: 0,
                drag: false,
                commit: DebouncedCommit::new(DebounceConfig {
                    wait: 35,
                    max_wait: Some(100),
                    leading: false,
                }),
            },
            worker,
        ))
    }
    fn request(&self) -> Request {
        Request {
            selections: self.selections.clone(),
            scatter_size: [self.layout.scatter.width, self.layout.scatter.height],
            airline_size: [self.layout.airline.width, self.layout.airline.height],
            panel_size: self.layout.panel_size(),
            bins: self.bins.clone(),
            focus: self.focus,
        }
    }
    fn submit(&mut self, label: &'static str) {
        self.generation += 1;
        self.pending = true;
        self.error = None;
        self.worker.submit(
            self.generation,
            self.request(),
            self.layout.clone(),
            self.rendered.clone(),
            label,
            false,
        );
    }
    fn activate(&mut self, focus: Option<Focus>) {
        if self.focus == focus {
            return;
        }
        self.focus = focus;
        self.worker.cancel_warmup();
        if focus.is_some() && self.config.preaggregate {
            self.worker.submit(
                self.generation,
                self.request(),
                self.layout.clone(),
                self.rendered.clone(),
                "Hover warm-up",
                true,
            );
        }
    }
    fn commit_brush(&mut self, brush: [f64; 4]) -> Result<()> {
        self.selections.brush(Some(brush))?;
        self.submit("Brush/drag");
        Ok(())
    }
    fn reset(&mut self) -> Result<()> {
        self.selections.brush(None)?;
        self.selections
            .select_carriers(self.metadata.carriers.iter().cloned().collect())?;
        self.bins.clear();
        self.preview = None;
        self.focus = None;
        self.worker.cancel_warmup();
        self.submit("Reset");
        Ok(())
    }
    pub fn brush_rect(&self, b: [f64; 4]) -> Rect {
        let r = self.rendered.layout.scatter;
        let domains = self.metadata.domains;
        let map = |v: f64, axis: usize| {
            ((v - domains[axis][0] as f64) / (domains[axis][1] - domains[axis][0]) as f64) as f32
        };
        let (mut x0, mut x1) = (map(b[0], 0) * r.width, map(b[1], 0) * r.width);
        let (mut y0, mut y1) = (
            (1. - map(b[3], 1)) * r.height,
            (1. - map(b[2], 1)) * r.height,
        );
        if !self.config.exact {
            let x = self
                .rendered
                .selections
                .scatter
                .pixel_grid(&avenger_selection::ProjectionId::new("x").unwrap())
                .unwrap();
            let y = self
                .rendered
                .selections
                .scatter
                .pixel_grid(&avenger_selection::ProjectionId::new("y").unwrap())
                .unwrap();
            let cell = |grid: &avenger_selection::PixelGrid, value: f64| {
                grid.cell(&value.into())
                    .expect("finite bound")
                    .expect("numeric bound") as f32
            };
            x0 = cell(x, b[0]) * 2.;
            x1 = cell(x, b[1]) * 2.;
            y0 = (cell(y, b[3]) + 1.) * 2.;
            y1 = (cell(y, b[2]) + 1.) * 2.;
        }
        x0 = x0.clamp(0., r.width);
        x1 = x1.clamp(x0, r.width);
        y0 = y0.clamp(0., r.height);
        y1 = y1.clamp(y0, r.height);
        Rect::new(r.x + x0, r.y + y0, x1 - x0, y1 - y0)
    }
    fn bounds(&self, start: [f32; 2], end: [f32; 2]) -> [f64; 4] {
        let r = self.rendered.layout.scatter;
        let inverse = |p: [f32; 2]| {
            let x = ((p[0] - r.x) / r.width).clamp(0., 1.);
            let y = (1. - (p[1] - r.y) / r.height).clamp(0., 1.);
            [
                self.metadata.domains[0][0] as f64
                    + x as f64 * (self.metadata.domains[0][1] - self.metadata.domains[0][0]) as f64,
                self.metadata.domains[1][0] as f64
                    + y as f64 * (self.metadata.domains[1][1] - self.metadata.domains[1][0]) as f64,
            ]
        };
        let a = inverse(start);
        let b = inverse(end);
        [
            a[0].min(b[0]),
            a[0].max(b[0]),
            a[1].min(b[1]),
            a[1].max(b[1]),
        ]
    }
    fn tooltip(&self, p: [f32; 2]) -> RuntimeHostCommand {
        let r = self.rendered.layout.scatter;
        let rows = if contains(r, p) {
            self.rendered.points.tooltip([p[0] - r.x, p[1] - r.y])
        } else {
            self.rendered
                .layout
                .panels
                .iter()
                .find(|(_, r)| contains(**r, p))
                .and_then(|(dest, r)| {
                    let bars = self.rendered.histograms.get(dest)?;
                    let index =
                        (((p[0] - r.x) / r.width * bars.len() as f32) as usize).min(bars.len() - 1);
                    Some(vec![
                        ("Destination".into(), dest.clone()),
                        ("Flights".into(), scene::comma(bars[index].count)),
                        ("Bin".into(), format!("{index}")),
                    ])
                })
        };
        RuntimeHostCommand::UpdateTooltip(match rows {
            Some(rows) => RuntimeTooltipUpdate::Show(RuntimeTooltipPresentation {
                owner: "flights-hover".into(),
                anchor: p,
                offset: [14., 14.],
                rows: rows
                    .into_iter()
                    .map(|(label, value)| RuntimeTooltipRow {
                        label: label.into(),
                        value,
                    })
                    .collect(),
                style: Default::default(),
            }),
            None => RuntimeTooltipUpdate::Clear,
        })
    }
}
fn commit_key() -> RuntimeWakeKey {
    RuntimeWakeKey::new("flights", 0, "brush-commit")
}
struct Builder;
#[async_trait]
impl SceneGraphBuilder<State> for Builder {
    async fn build(&self, state: &mut State) -> Result<SceneGraph, AvengerAppError> {
        self.build_with_effects(state).await.map(|b| b.scene_graph)
    }
    async fn build_with_effects(&self, state: &mut State) -> Result<SceneBuild, AvengerAppError> {
        scene::build(state).map_err(|e| AvengerAppError::InternalError(format!("{e:#}")))
    }
}
struct Input;
#[async_trait]
impl EventStreamHandler<State> for Input {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        match input(event, state, rtree) {
            Ok(status) => status,
            Err(e) => {
                state.error = Some(format!("{e:#}"));
                UpdateStatus {
                    rerender: true,
                    ..Default::default()
                }
            }
        }
    }
}
fn input(event: &SceneGraphEvent, s: &mut State, rtree: &SceneGraphRTree) -> Result<UpdateStatus> {
    let update = s.widgets.handle(event, rtree, Instant::now())?;
    let mut status = update.status;
    for event in update.events {
        if event.action != WidgetAction::Activated {
            continue;
        }
        status
            .commands
            .extend(s.commit.cancel(&commit_key()).commands);
        s.drag = false;
        s.preview = None;
        match event.id.as_str() {
            "reset" => s.reset()?,
            "all" => {
                s.selections
                    .select_carriers(s.metadata.carriers.iter().cloned().collect())?;
                s.activate(Some(Focus::Airline));
                s.submit("All airlines");
            }
            "none" => {
                s.selections.select_carriers(Default::default())?;
                s.activate(Some(Focus::Airline));
                s.submit("No airlines");
            }
            id if id.starts_with("bin-") => {
                let dest = &id[4..];
                let value = s.bins.entry(dest.into()).or_insert(30);
                *value = match *value {
                    15 => 30,
                    30 => 60,
                    _ => 15,
                };
                s.submit("Local bins");
            }
            _ => {}
        }
        status.rerender = true;
    }
    if status.consume {
        return Ok(status);
    }
    match event {
        SceneGraphEvent::RuntimeWake(wake) if wake.key == completion_key() => {
            if let Some((generation, result)) = s.worker.completed.lock().unwrap().take()
                && generation == s.generation
            {
                match result {
                    Ok(rendered) => {
                        s.rendered = rendered;
                        if !s.drag {
                            s.preview = None;
                        }
                        s.error = None;
                        status.rebuild_geometry = true;
                    }
                    Err(e) => s.error = Some(e),
                }
                s.pending = false;
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
        SceneGraphEvent::CursorMoved(e) if !s.drag => {
            let layout = &s.rendered.layout;
            let focus = if contains(layout.scatter, e.position) {
                Some(Focus::Scatter)
            } else if contains(layout.airline, e.position) {
                Some(Focus::Airline)
            } else {
                None
            };
            s.activate(focus);
            status.commands.push(s.tooltip(e.position));
        }
        SceneGraphEvent::MouseDown(e)
            if e.button == MouseButton::Left
                && contains(s.rendered.layout.scatter, e.position)
                && s.layout.scatter == s.rendered.layout.scatter =>
        {
            s.drag = true;
            s.preview = None;
            s.activate(Some(Focus::Scatter));
            status.suppress_click = true;
            status.commands.push(RuntimeHostCommand::UpdateTooltip(
                RuntimeTooltipUpdate::Clear,
            ));
        }
        SceneGraphEvent::Click(e)
            if e.button == MouseButton::Left && contains(s.rendered.layout.airline, e.position) =>
        {
            let r = s.rendered.layout.airline;
            let index = (((e.position[1] - r.y) / r.height * s.metadata.carriers.len() as f32)
                as usize)
                .min(s.metadata.carriers.len() - 1);
            s.activate(Some(Focus::Airline));
            s.selections.toggle(&s.metadata.carriers[index].clone())?;
            s.submit("Airline click");
            status.rerender = true;
        }
        SceneGraphEvent::KeyPress(e) if e.key == Key::Named(NamedKey::Escape) => {
            s.drag = false;
            s.preview = None;
            status
                .commands
                .extend(s.commit.cancel(&commit_key()).commands);
            s.selections.brush(None)?;
            s.submit("Clear brush");
            status.rerender = true;
        }
        SceneGraphEvent::WindowResize(e) => {
            resize(s, e.size, &mut status)?;
        }
        SceneGraphEvent::CanvasResize(e) => {
            resize(s, e.size, &mut status)?;
        }
        SceneGraphEvent::PointerCaptureLost | SceneGraphEvent::WindowFocused(false) => {
            s.drag = false;
            s.preview = None;
            status
                .commands
                .extend(s.commit.cancel(&commit_key()).commands);
            s.activate(None);
            status.rerender = true;
        }
        SceneGraphEvent::WindowCloseRequested => s.worker.shutdown(),
        _ => {}
    }
    Ok(status)
}
fn resize(s: &mut State, size: [f32; 2], status: &mut UpdateStatus) -> Result<()> {
    if size == s.size {
        return Ok(());
    }
    s.drag = false;
    s.preview = None;
    status
        .commands
        .extend(s.commit.cancel(&commit_key()).commands);
    s.size = size;
    s.layout = DashboardLayout::solve(&s.metadata, size)?;
    s.selections.regrid(
        s.metadata.domains,
        [s.layout.scatter.width, s.layout.scatter.height],
        s.config.exact,
    )?;
    s.activate(None);
    s.submit("Resize");
    status.rerender = true;
    Ok(())
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
        if !s.drag {
            return status;
        }
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
        let bounds = s.bounds(start, end);
        s.preview = Some(bounds);
        status.rerender = true;
        status.suppress_click = true;
        let mut update = s.commit.submit(bounds, Instant::now(), &commit_key());
        if matches!(event, SceneGraphEvent::MouseUp(_)) {
            status.commands.extend(update.commands);
            update = s.commit.flush(&commit_key());
            s.drag = false;
        }
        status.commands.extend(update.commands);
        if let Some(value) = update.commit
            && let Err(e) = s.commit_brush(value)
        {
            s.error = Some(format!("{e:#}"));
        }
        status
    }
}
pub async fn make_app(state: State) -> Result<AvengerApp<State>> {
    let engine = state.engine.clone();
    let start = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        mark_names: Some(vec!["scatter-hit".into()]),
        filter: Some(vec![EventStreamFilter::context(
            |e, _, _| matches!(e,SceneGraphEvent::MouseDown(e) if e.button==MouseButton::Left),
        )]),
        ..Default::default()
    };
    let end = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        ..Default::default()
    };
    let all = vec![
        SceneGraphEventType::MouseDown,
        SceneGraphEventType::MouseUp,
        SceneGraphEventType::Click,
        SceneGraphEventType::CursorMoved,
        SceneGraphEventType::KeyPress,
        SceneGraphEventType::KeyRelease,
        SceneGraphEventType::RuntimeWake,
        SceneGraphEventType::WindowResize,
        SceneGraphEventType::CanvasResize,
        SceneGraphEventType::PointerCaptureLost,
        SceneGraphEventType::WindowFocused,
        SceneGraphEventType::WindowCloseRequested,
        SceneGraphEventType::FocusEntered,
    ];
    Ok(AvengerApp::try_new_with_text_engine(
        state,
        Arc::new(Builder),
        vec![
            (
                EventStreamConfig {
                    types: all,
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
        engine,
    )
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_eventstream::window::{
        ElementState, WindowCursorMoved, WindowEvent, WindowMouseInput, WindowResizeEvent,
    };

    async fn app() -> Result<(AvengerApp<State>, Arc<Worker>)> {
        let (state, worker) = State::load(
            Config {
                preaggregate: false,
                ..Default::default()
            },
            avenger_text::default_text_engine(),
        )
        .await?;
        Ok((make_app(state).await?, worker))
    }
    async fn event(app: &mut AvengerApp<State>, event: WindowEvent) -> Result<UpdateStatus> {
        Ok(app.update_with_status(&event, Instant::now()).await?.status)
    }
    async fn move_to(app: &mut AvengerApp<State>, p: [f32; 2]) -> Result<UpdateStatus> {
        event(
            app,
            WindowEvent::CursorMoved(WindowCursorMoved { position: p }),
        )
        .await
    }
    async fn button(app: &mut AvengerApp<State>, pressed: bool) -> Result<UpdateStatus> {
        event(
            app,
            WindowEvent::MouseInput(WindowMouseInput {
                state: if pressed {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                },
                button: MouseButton::Left,
            }),
        )
        .await
    }
    async fn complete(app: &mut AvengerApp<State>, worker: &Worker) -> Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if worker.completed.lock().unwrap().is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
        .await?;
        let generation = app.app_state_mut().generation;
        event(
            app,
            WindowEvent::RuntimeWake(avenger_eventstream::runtime::RuntimeWakeEvent {
                key: completion_key(),
                generation,
            }),
        )
        .await?;
        assert!(!app.app_state_mut().pending);
        assert!(
            app.app_state_mut().error.is_none(),
            "{:?}",
            app.app_state_mut().error
        );
        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn outside_drag_flushes_and_widget_click_selects_none() -> Result<()> {
        let (mut app, worker) = app().await?;
        let r = app.app_state_mut().rendered.layout.scatter;
        move_to(&mut app, [r.x + 20., r.y + 20.]).await?;
        button(&mut app, true).await?;
        let preview = move_to(&mut app, [r.x + r.width + 80., r.y + r.height + 40.]).await?;
        assert!(preview.rerender);
        assert!(preview.suppress_click);
        assert!(app.app_state_mut().preview.is_some());
        let release = button(&mut app, false).await?;
        assert!(release.suppress_click);
        assert!(app.app_state_mut().selections.raw_brush.is_some());
        assert!(!app.app_state_mut().drag);
        assert!(app.app_state_mut().commit.pending_generation().is_none());
        complete(&mut app, &worker).await?;
        assert_eq!(app.app_state_mut().rendered.points.count, 327346);
        let control = app
            .app_state_mut()
            .widgets
            .semantics()
            .iter()
            .find(|s| s.target.widget.as_str() == "none")
            .unwrap()
            .bounds;
        move_to(
            &mut app,
            [
                control.x + control.width / 2.,
                control.y + control.height / 2.,
            ],
        )
        .await?;
        assert!(button(&mut app, true).await?.consume);
        button(&mut app, false).await?;
        assert!(app.app_state_mut().selections.carriers.is_empty());
        complete(&mut app, &worker).await?;
        assert_eq!(app.app_state_mut().rendered.points.count, 0);
        assert_eq!(app.app_state_mut().rendered.selected, 0);
        worker.shutdown();
        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hover_reuses_points_and_stale_completion_cannot_install() -> Result<()> {
        let (mut app, worker) = app().await?;
        let points = app.app_state_mut().rendered.points.clone();
        let r = app.app_state_mut().rendered.layout.scatter;
        let before = app.app_state_mut().generation;
        let update = move_to(&mut app, [r.x + 40., r.y + r.height - 40.]).await?;
        assert!(!update.rebuild_geometry);
        assert_eq!(app.app_state_mut().generation, before);
        assert!(Arc::ptr_eq(&points, &app.app_state_mut().rendered.points));
        app.app_state_mut().generation = 2;
        app.app_state_mut().pending = true;
        *worker.completed.lock().unwrap() = Some((1, Err("obsolete failure".into())));
        event(
            &mut app,
            WindowEvent::RuntimeWake(avenger_eventstream::runtime::RuntimeWakeEvent {
                key: completion_key(),
                generation: 1,
            }),
        )
        .await?;
        assert!(app.app_state_mut().pending);
        assert!(app.app_state_mut().error.is_none());
        worker.shutdown();
        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resize_cancels_preview_and_preserves_raw_committed_bounds() -> Result<()> {
        let (mut app, worker) = app().await?;
        let bounds = [10., 100., -10., 60.];
        app.app_state_mut().selections.brush(Some(bounds))?;
        let r = app.app_state_mut().rendered.layout.scatter;
        move_to(&mut app, [r.x + 20., r.y + 20.]).await?;
        button(&mut app, true).await?;
        move_to(&mut app, [r.x + 40., r.y + 40.]).await?;
        event(
            &mut app,
            WindowEvent::WindowResize(WindowResizeEvent {
                size: [1100., 1000.],
            }),
        )
        .await?;
        assert!(!app.app_state_mut().drag);
        assert!(app.app_state_mut().preview.is_none());
        assert_eq!(app.app_state_mut().selections.raw_brush, Some(bounds));
        assert!(app.app_state_mut().commit.pending_generation().is_none());
        complete(&mut app, &worker).await?;
        assert_eq!(app.app_state_mut().rendered.layout.size, [1100., 1000.]);
        worker.shutdown();
        Ok(())
    }
}
