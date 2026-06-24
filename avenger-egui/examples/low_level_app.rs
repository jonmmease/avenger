use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_egui::{AvengerCanvasHandle, Canvas};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::{Key, MouseButton, MouseScrollDelta, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::{
    marks::{mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark},
    scene_graph::SceneGraph,
};
use eframe::egui;

const INITIAL_WIDTH: f32 = 760.0;
const INITIAL_HEIGHT: f32 = 520.0;
const COLORS: [[f32; 4]; 4] = [
    [0.05, 0.42, 0.67, 1.0],
    [0.92, 0.56, 0.02, 1.0],
    [0.02, 0.62, 0.45, 1.0],
    [0.91, 0.20, 0.31, 1.0],
];

fn main() -> eframe::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let avenger_app = runtime.block_on(build_app());
    let scene = avenger_app.scene_graph_arc();
    let canvas = AvengerCanvasHandle::from_app(avenger_app);
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Avenger egui low-level app",
        native_options,
        Box::new(move |creation| {
            assert!(
                creation.wgpu_render_state.is_some(),
                "low_level_app requires eframe's WGPU renderer"
            );
            Ok(Box::new(LowLevelEguiApp {
                runtime,
                canvas,
                scene,
                scene_generation: 0,
                rendered_scene_generation: None,
                rendered_dimensions: None,
                rendered_render_invalidation_epoch: None,
                last_error: None,
            }))
        }),
    )
}

struct LowLevelEguiApp {
    runtime: tokio::runtime::Runtime,
    canvas: AvengerCanvasHandle<LowLevelState>,
    scene: Arc<SceneGraph>,
    scene_generation: u64,
    rendered_scene_generation: Option<u64>,
    rendered_dimensions: Option<CanvasDimensions>,
    rendered_render_invalidation_epoch: Option<u64>,
    last_error: Option<String>,
}

impl eframe::App for LowLevelEguiApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_latest_scene(ctx);
        self.last_error = self.canvas.latest_scene_error();

        egui::SidePanel::left("controls").show(ctx, |ui| {
            let snapshot = self.canvas.app_state().snapshot();
            ui.heading("Low-level app");

            if ui.button("Reset").clicked() {
                self.canvas.app_state().reset();
                self.request_scene_rebuild(ctx);
            }

            if let Some(error) = &self.last_error {
                ui.colored_label(egui::Color32::RED, error);
            }

            ui.separator();
            ui.label(format!(
                "canvas: {:.0} x {:.0}",
                snapshot.width, snapshot.height
            ));
            ui.label(format!(
                "target: {:.1}, {:.1}",
                snapshot.target[0], snapshot.target[1]
            ));
            ui.label(format!("radius: {:.1}", snapshot.radius));
            ui.label(format!("dragging: {}", snapshot.dragging));
            ui.label(format!("hovering target: {}", snapshot.hovering_target));
            ui.label(format!("clicks: {}", snapshot.clicks));
            ui.label(format!("double-clicks: {}", snapshot.double_clicks));
            ui.label(format!("wheel events: {}", snapshot.wheel_events));
            ui.label(format!("key presses: {}", snapshot.key_presses));
            ui.label(format!("last event: {}", snapshot.last_event));
            if let Some(cursor) = snapshot.cursor {
                ui.label(format!("cursor: {:.1}, {:.1}", cursor[0], cursor[1]));
            } else {
                ui.label("cursor: none");
            }

            ui.separator();
            let status = self.canvas.frame_status();
            ui.label(format!(
                "texture generation: {}",
                status
                    .latest_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
            ui.label(format!(
                "scene generation: {} / requested {}",
                status
                    .latest_scene_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                status
                    .requested_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            ));
            ui.label(if status.render_pending {
                "render pending"
            } else {
                "render idle"
            });
            ui.separator();
            self.canvas.show_metrics(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let desired_size = ui.available_size_before_wrap();
            let dimensions = scene_dimensions(&self.scene, ctx);

            self.render_latest_scene_if_needed(ctx, frame, dimensions);

            let output = Canvas::new(&self.canvas)
                .desired_size(desired_size)
                .show(ui);
            if !output.events.is_empty() {
                self.canvas
                    .request_event_dispatch_with_repaint(self.runtime.handle(), ctx);
            }
            if output.response.dragged() || output.frame_status.render_pending {
                ctx.request_repaint();
            }
        });
    }
}

impl LowLevelEguiApp {
    fn request_scene_rebuild(&mut self, ctx: &egui::Context) {
        self.canvas
            .request_scene_rebuild_with_repaint(self.runtime.handle(), ctx, true);
        ctx.request_repaint();
    }

    fn poll_latest_scene(&mut self, ctx: &egui::Context) {
        if let Some(frame) = self.canvas.latest_scene_frame() {
            let generation = frame.generation.get();
            if generation != self.scene_generation {
                self.scene = frame.payload.clone();
                self.scene_generation = generation;
                ctx.request_repaint();
            }
        }
    }

    fn render_latest_scene_if_needed(
        &mut self,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
        dimensions: CanvasDimensions,
    ) {
        let dimensions_changed = self
            .rendered_dimensions
            .is_none_or(|rendered| !canvas_dimensions_eq(rendered, dimensions));
        let scene_changed = self.rendered_scene_generation != Some(self.scene_generation);
        let render_invalidation_epoch = self.canvas.render_invalidation_epoch();
        let render_invalidated =
            self.rendered_render_invalidation_epoch != Some(render_invalidation_epoch);
        if !dimensions_changed && !scene_changed && !render_invalidated {
            return;
        }

        if let Some(render_state) = frame.wgpu_render_state()
            && let Err(err) = self
                .canvas
                .request_background_scene_texture_with_repaint(
                    render_state,
                    ctx,
                    self.scene_generation,
                    self.scene.clone(),
                    dimensions,
                )
                .map(|status| {
                    if status.render_pending {
                        ctx.request_repaint();
                    }
                    if status.scene_generation == Some(self.scene_generation)
                        && status
                            .dimensions
                            .is_some_and(|rendered| canvas_dimensions_eq(rendered, dimensions))
                        && status.render_invalidation_epoch == Some(render_invalidation_epoch)
                    {
                        self.rendered_scene_generation = Some(self.scene_generation);
                        self.rendered_dimensions = Some(dimensions);
                        self.rendered_render_invalidation_epoch = Some(render_invalidation_epoch);
                    }
                })
        {
            self.last_error = Some(err.to_string());
        }
    }
}

#[derive(Clone)]
struct LowLevelState {
    inner: Arc<Mutex<LowLevelStateInner>>,
}

impl LowLevelState {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LowLevelStateInner::default())),
        }
    }

    fn snapshot(&self) -> LowLevelStateSnapshot {
        self.inner
            .lock()
            .expect("low-level app state lock poisoned")
            .snapshot()
    }

    fn reset(&self) {
        *self
            .inner
            .lock()
            .expect("low-level app state lock poisoned") = LowLevelStateInner::default();
    }

    fn handle_event(&self, event: &SceneGraphEvent) -> UpdateStatus {
        let mut state = self
            .inner
            .lock()
            .expect("low-level app state lock poisoned");
        let mut rerender = true;
        let mut rebuild_geometry = true;

        match event {
            SceneGraphEvent::CanvasResize(event) => {
                state.resize(event.size[0], event.size[1]);
                state.last_event = "canvas resize".to_string();
            }
            SceneGraphEvent::CursorMoved(event) => {
                state.cursor = Some(event.position);
                state.hovering_target = is_target_mark(event.mark_instance.as_ref());
                if state.dragging {
                    state.target = event.position;
                    state.clamp_target();
                }
                state.last_event = format!(
                    "cursor moved ({:.1}, {:.1})",
                    event.position[0], event.position[1]
                );
            }
            SceneGraphEvent::MouseEnter(event) => {
                state.hovering_target = event.mark_instance.name == "target";
                state.last_event = format!("mouse entered {}", event.mark_instance.name);
            }
            SceneGraphEvent::MouseLeave(event) => {
                if event.mark_instance.name == "target" {
                    state.hovering_target = false;
                }
                state.last_event = format!("mouse left {}", event.mark_instance.name);
            }
            SceneGraphEvent::MouseDown(event) => {
                if event.button == MouseButton::Left {
                    state.dragging = true;
                    state.target = event.position;
                    state.clamp_target();
                }
                state.last_event = format!("mouse down {:?}", event.button);
            }
            SceneGraphEvent::MouseUp(event) => {
                if event.button == MouseButton::Left {
                    state.dragging = false;
                }
                state.last_event = format!("mouse up {:?}", event.button);
            }
            SceneGraphEvent::Click(event) => {
                state.clicks += 1;
                state.color_index = (state.color_index + 1) % COLORS.len();
                state.last_event =
                    format!("click {:?}", event.mark_instance.as_ref().map(|m| &m.name));
            }
            SceneGraphEvent::DoubleClick(event) => {
                state.double_clicks += 1;
                state.target = event.position;
                state.radius = LowLevelStateInner::default().radius;
                state.color_index = 0;
                state.last_event = "double click reset".to_string();
            }
            SceneGraphEvent::MouseWheel(event) => {
                state.wheel_events += 1;
                state.radius = (state.radius + wheel_delta(event.delta)).clamp(8.0, 80.0);
                state.last_event = format!("mouse wheel radius {:.1}", state.radius);
            }
            SceneGraphEvent::KeyPress(event) => {
                state.key_presses += 1;
                let distance = if event.modifiers.shift { 20.0 } else { 5.0 };
                match event.key {
                    Key::Named(NamedKey::ArrowLeft) => state.target[0] -= distance,
                    Key::Named(NamedKey::ArrowRight) => state.target[0] += distance,
                    Key::Named(NamedKey::ArrowUp) => state.target[1] -= distance,
                    Key::Named(NamedKey::ArrowDown) => state.target[1] += distance,
                    Key::Named(NamedKey::Space) => {
                        state.target = [state.width * 0.5, state.height * 0.5];
                    }
                    _ => {
                        rerender = false;
                        rebuild_geometry = false;
                    }
                }
                state.clamp_target();
                state.last_event = format!("key press {:?}", event.key);
            }
            _ => {
                rerender = false;
                rebuild_geometry = false;
            }
        }

        UpdateStatus {
            rerender,
            rebuild_geometry,
            ..UpdateStatus::default()
        }
    }
}

#[derive(Clone)]
struct LowLevelStateSnapshot {
    width: f32,
    height: f32,
    target: [f32; 2],
    cursor: Option<[f32; 2]>,
    radius: f32,
    dragging: bool,
    hovering_target: bool,
    clicks: u64,
    double_clicks: u64,
    wheel_events: u64,
    key_presses: u64,
    last_event: String,
    color_index: usize,
}

struct LowLevelStateInner {
    width: f32,
    height: f32,
    target: [f32; 2],
    cursor: Option<[f32; 2]>,
    radius: f32,
    dragging: bool,
    hovering_target: bool,
    clicks: u64,
    double_clicks: u64,
    wheel_events: u64,
    key_presses: u64,
    last_event: String,
    color_index: usize,
}

impl Default for LowLevelStateInner {
    fn default() -> Self {
        Self {
            width: INITIAL_WIDTH,
            height: INITIAL_HEIGHT,
            target: [INITIAL_WIDTH * 0.5, INITIAL_HEIGHT * 0.5],
            cursor: None,
            radius: 34.0,
            dragging: false,
            hovering_target: false,
            clicks: 0,
            double_clicks: 0,
            wheel_events: 0,
            key_presses: 0,
            last_event: "initialized".to_string(),
            color_index: 0,
        }
    }
}

impl LowLevelStateInner {
    fn snapshot(&self) -> LowLevelStateSnapshot {
        LowLevelStateSnapshot {
            width: self.width,
            height: self.height,
            target: self.target,
            cursor: self.cursor,
            radius: self.radius,
            dragging: self.dragging,
            hovering_target: self.hovering_target,
            clicks: self.clicks,
            double_clicks: self.double_clicks,
            wheel_events: self.wheel_events,
            key_presses: self.key_presses,
            last_event: self.last_event.clone(),
            color_index: self.color_index,
        }
    }

    fn resize(&mut self, width: f32, height: f32) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        self.clamp_target();
    }

    fn clamp_target(&mut self) {
        self.target[0] = self.target[0].clamp(0.0, self.width);
        self.target[1] = self.target[1].clamp(0.0, self.height);
    }
}

struct LowLevelSceneBuilder;

#[async_trait]
impl SceneGraphBuilder<LowLevelState> for LowLevelSceneBuilder {
    async fn build(&self, state: &mut LowLevelState) -> Result<SceneGraph, AvengerAppError> {
        let snapshot = state.snapshot();
        Ok(scene_graph(snapshot))
    }
}

struct LowLevelEventHandler;

#[async_trait]
impl EventStreamHandler<LowLevelState> for LowLevelEventHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut LowLevelState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        state.handle_event(event)
    }
}

async fn build_app() -> AvengerApp<LowLevelState> {
    AvengerApp::try_new(
        LowLevelState::new(),
        Arc::new(LowLevelSceneBuilder),
        vec![(
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::CanvasResize,
                    SceneGraphEventType::CursorMoved,
                    SceneGraphEventType::MarkMouseEnter,
                    SceneGraphEventType::MarkMouseLeave,
                    SceneGraphEventType::MouseDown,
                    SceneGraphEventType::MouseUp,
                    SceneGraphEventType::Click,
                    SceneGraphEventType::DoubleClick,
                    SceneGraphEventType::MouseWheel,
                    SceneGraphEventType::KeyPress,
                ],
                ..EventStreamConfig::default()
            },
            Arc::new(LowLevelEventHandler),
        )],
    )
    .await
    .expect("build low-level avenger app")
}

fn scene_graph(snapshot: LowLevelStateSnapshot) -> SceneGraph {
    let mut marks = Vec::new();
    marks.push(background_mark(snapshot.width, snapshot.height));
    marks.push(target_mark(&snapshot));
    if let Some(cursor) = snapshot.cursor {
        marks.push(cursor_mark(cursor));
    }
    marks.push(border_mark(snapshot.width, snapshot.height));

    SceneGraph {
        marks,
        width: snapshot.width,
        height: snapshot.height,
        origin: [0.0, 0.0],
    }
}

fn background_mark(width: f32, height: f32) -> SceneMark {
    SceneRectMark {
        name: "background".to_string(),
        interactive: false,
        clip: false,
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: color([1.0, 1.0, 1.0, 1.0]).into(),
        ..Default::default()
    }
    .into()
}

fn target_mark(snapshot: &LowLevelStateSnapshot) -> SceneMark {
    let mut fill = COLORS[snapshot.color_index % COLORS.len()];
    if snapshot.dragging {
        fill[3] = 0.82;
    }
    let stroke = if snapshot.hovering_target || snapshot.dragging {
        [0.08, 0.10, 0.13, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };

    SceneSymbolMark {
        name: "target".to_string(),
        interactive: true,
        clip: true,
        x: snapshot.target[0].into(),
        y: snapshot.target[1].into(),
        size: (snapshot.radius * snapshot.radius).into(),
        fill: color(fill).into(),
        stroke: color(stroke).into(),
        stroke_width: Some(if snapshot.hovering_target { 3.0 } else { 1.5 }),
        zindex: Some(10),
        ..Default::default()
    }
    .into()
}

fn cursor_mark(cursor: [f32; 2]) -> SceneMark {
    SceneSymbolMark {
        name: "cursor".to_string(),
        interactive: false,
        clip: true,
        x: cursor[0].into(),
        y: cursor[1].into(),
        size: 64.0.into(),
        fill: color([0.08, 0.10, 0.13, 0.18]).into(),
        stroke: color([0.08, 0.10, 0.13, 0.42]).into(),
        stroke_width: Some(1.0),
        zindex: Some(20),
        ..Default::default()
    }
    .into()
}

fn border_mark(width: f32, height: f32) -> SceneMark {
    SceneRectMark {
        name: "border".to_string(),
        interactive: false,
        clip: false,
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: color([0.0, 0.0, 0.0, 0.0]).into(),
        stroke: color([0.08, 0.10, 0.13, 0.35]).into(),
        stroke_width: 1.0.into(),
        zindex: Some(30),
        ..Default::default()
    }
    .into()
}

fn color(rgba: [f32; 4]) -> ColorOrGradient {
    ColorOrGradient::Color(rgba)
}

fn wheel_delta(delta: MouseScrollDelta) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y * 3.0,
        MouseScrollDelta::PixelDelta(_, y) => y as f32 * 0.05,
    }
}

fn is_target_mark(mark: Option<&avenger_scenegraph::marks::mark::MarkInstance>) -> bool {
    mark.is_some_and(|mark| mark.name == "target")
}

fn canvas_dimensions_eq(left: CanvasDimensions, right: CanvasDimensions) -> bool {
    left.size == right.size && left.scale == right.scale
}

fn scene_dimensions(scene: &SceneGraph, ctx: &egui::Context) -> CanvasDimensions {
    CanvasDimensions {
        size: [scene.width.max(1.0), scene.height.max(1.0)],
        scale: ctx.pixels_per_point(),
    }
}
