//! Five million points drawn as a heatmap of their density. A background task counts the points
//! in each cell of a grid over the view that a pan or a zoom settles on, through DataFusion, and
//! the scene draws the latest heatmap through the current view meanwhile. See the README for the
//! controls.

mod data;
mod heat;

use std::sync::Arc;

use avenger_app::app::{AvengerApp, BuildContext, BuiltScene, Layer};
use avenger_app::error::AvengerAppError;
use avenger_app::task::Task;
use avenger_common::types::{FontWeight, TextBaseline};
use avenger_eventstream::manager::{EventStreamHandler, LayerTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::legend::{Colorbar, ColorbarConfig};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Edges, Layout, Leaf, Measure, Measurement, Rect, Side, Size, Slab, SolveOptions,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::mesh_image::{MeshImage, SceneMeshImageMark};
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::prelude::DataFrame;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::heat::{Heat, HeatKey};

/// The points that the example draws.
const POINTS: usize = 5_000_000;
/// The device pixels in a unit of the scene, at which the host draws.
const SCALE: f32 = 2.0;
/// The first view's domains.
const VIEW: ((f64, f64), (f64, f64)) = ((-4.5, 4.5), (-3.0, 3.0));

/// The colorbar's least width: its title's, and its labels' up to five million points, so the
/// plot stays put as the first heatmap arrives and as the counts change.
const COLORBAR: f32 = 88.0;

const INK: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

/// A drag's start: where it pressed, and the view's domains then.
#[derive(Clone)]
struct Drag {
    from: [f32; 2],
    x: (f64, f64),
    y: (f64, f64),
}

#[derive(Clone)]
pub struct AppState {
    /// The window's size, which the layout fills.
    canvas: [f32; 2],
    /// The plot area that the last scene laid out, which gestures map positions in.
    plot: Rect,
    /// The view's domains.
    x: (f64, f64),
    y: (f64, f64),
    drag: Option<Drag>,
    points: DataFrame,
    heat: Task<HeatKey, Heat>,
    engine: LabelEngine,
}

impl AppState {
    fn new(points: DataFrame) -> Self {
        Self {
            canvas: [800.0, 560.0],
            plot: Rect::new(0.0, 0.0, 1.0, 1.0),
            x: VIEW.0,
            y: VIEW.1,
            drag: None,
            points,
            heat: Task::new(),
            engine: avenger_typst_label::bundled_label_engine()
                .with_number_formatting(Arc::new(D3NumberFormatProvider::new())),
        }
    }

    /// Request the heatmap of the view.
    fn request_heat(&self) {
        let key = HeatKey {
            x: self.x,
            y: self.y,
        };
        let points = self.points.clone();
        self.heat
            .request(key.clone(), move |cancel| heat::heat(points, key, cancel));
    }

    /// Where `position`, in the window, lies in the plot as fractions of its width from the left
    /// and of its height from the bottom, or `None` outside the plot.
    fn in_plot(&self, position: [f32; 2]) -> Option<[f64; 2]> {
        let plot = self.plot;
        let fraction = [
            f64::from((position[0] - plot.x) / plot.width),
            f64::from(1.0 - (position[1] - plot.y) / plot.height),
        ];
        fraction
            .iter()
            .all(|fraction| (0.0..=1.0).contains(fraction))
            .then_some(fraction)
    }
}

/// Where `value` lies along a scene length `length` that spans `domain`.
fn place(domain: (f64, f64), length: f32, value: f64) -> f32 {
    ((value - domain.0) / (domain.1 - domain.0)) as f32 * length
}

/// The parts that the layout places: the plot, each axis part, the colorbar and the readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Plot,
    X(AxisPart),
    Y(AxisPart),
    Colorbar,
    Readout,
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The plot with its axes' parts below and to its left, the readout above, and the colorbar to
/// its right after a gap.
fn layout() -> Layout<Id> {
    let mut plot = Leaf::new().id(Id::Plot).min_size(Size::new(80.0, 80.0));
    for part in AXIS_PARTS {
        plot = plot
            .slab(Side::Left, Slab::new().id(Id::Y(part)))
            .slab(Side::Bottom, Slab::new().id(Id::X(part)));
    }
    plot.slab(Side::Top, Slab::new().id(Id::Readout).min_height(28.0))
        .slab(Side::Right, Slab::new().min_width(12.0))
        .slab(
            Side::Right,
            Slab::new().id(Id::Colorbar).min_width(COLORBAR),
        )
        .into()
}

struct Guides<'a> {
    x: &'a Axis,
    y: &'a Axis,
    colorbar: Option<&'a Colorbar>,
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<Id> for Guides<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        match id {
            Id::X(part) => Some(answer(self.x.extent(*part, width))),
            _ => None,
        }
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        match id {
            Id::Y(part) => Some(answer(self.y.extent(*part, height))),
            Id::Colorbar => self
                .colorbar
                .map(|colorbar| answer(colorbar.extent(height))),
            _ => None,
        }
    }
}

/// The chart's layer, with the streams that pan and zoom it.
struct ChartLayer(Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<AppState>>)>);

#[async_trait::async_trait]
impl Layer<AppState> for ChartLayer {
    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<AppState>>)> {
        self.0.clone()
    }

    async fn build(
        &self,
        state: &mut AppState,
        _: &BuildContext<'_>,
    ) -> Result<BuiltScene, AvengerAppError> {
        make_scene_graph(state)
            .map(Into::into)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))
    }
}

/// The scene: the axes of the view, the latest heatmap placed in it, its colorbar, and the
/// readout. Each scene places the latest heatmap through the current view, so it follows a pan
/// or a zoom until the view's own heatmap arrives.
fn make_scene_graph(state: &mut AppState) -> Result<SceneGraph, Box<dyn std::error::Error>> {
    let engine = &state.engine;
    let config = |side| AxisConfig {
        side,
        tick_format: ",f".into(),
        tick_label: None,
        style: Default::default(),
    };
    let x_axis = Axis::continuous(
        &LinearScale::configured(state.x, (0.0, 1.0)),
        "x",
        &config(GuideSide::Bottom),
        engine,
    )?;
    let y_axis = Axis::continuous(
        &LinearScale::configured(state.y, (1.0, 0.0)),
        "y",
        &config(GuideSide::Left),
        engine,
    )?;
    let latest = state.heat.latest();
    let colorbar = latest
        .as_ref()
        .map(|done| {
            Colorbar::new(
                &heat::color_scale(done.value.max),
                "Points per cell",
                &ColorbarConfig::default(),
                engine,
            )
        })
        .transpose()?;

    let [width, height] = state.canvas;
    let options = SolveOptions {
        padding: Edges::all(16.0),
        ..SolveOptions::sized(width, height)
    };
    let mut guides = Guides {
        x: &x_axis,
        y: &y_axis,
        colorbar: colorbar.as_ref(),
    };
    let solution = layout().measure(&options, &mut guides)?.into_solution();
    let region = |id: Id| solution.region(&id).map(|region| region.content);
    let plot = region(Id::Plot).ok_or("the layout places the plot")?;
    state.plot = plot;

    // The latest heatmap, over the view it was counted for, through the current view.
    let mut heatmap = Vec::new();
    if let Some(done) = &latest {
        let key = &done.key;
        let left = place(state.x, plot.width, key.x.0);
        let right = place(state.x, plot.width, key.x.1);
        let top = plot.height - place(state.y, plot.height, key.y.1);
        let bottom = plot.height - place(state.y, plot.height, key.y.0);
        heatmap.push(
            SceneMeshImageMark {
                instances: vec![MeshImage {
                    image: done.value.image.clone().into(),
                    positions: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
                    uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                    indices: vec![0, 1, 2, 0, 2, 3],
                }],
                ..Default::default()
            }
            .into(),
        );
    }
    let size = [plot.width, plot.height];
    let mut marks: Vec<SceneMark> = vec![SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            y_axis.grid(size).into(),
            x_axis.grid(size).into(),
            SceneGroup {
                marks: heatmap,
                clip: Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: plot.width,
                    height: plot.height,
                },
                ..Default::default()
            }
            .into(),
        ],
        ..Default::default()
    }
    .into()];

    for part in AXIS_PARTS {
        for (axis, id, length) in [
            (&x_axis, Id::X(part), plot.width),
            (&y_axis, Id::Y(part), plot.height),
        ] {
            if let (Some(slab), Some(mut group)) = (region(id), axis.part(part, length)) {
                group.origin = axis.origin([slab.x, slab.y, slab.width, slab.height]);
                marks.push(group.into());
            }
        }
    }
    if let (Some(colorbar), Some(slab)) = (&colorbar, region(Id::Colorbar)) {
        let mut group = colorbar.marks(slab.height).group;
        group.origin = colorbar.origin([slab.x, slab.y, slab.width, slab.height]);
        marks.push(group.into());
    }
    if let Some(slab) = region(Id::Readout) {
        let failure = state
            .heat
            .error()
            .map_or(String::new(), |err| format!(" · counting failed: {err}"));
        let readout = format!("{} points{failure}", thousands(POINTS));
        marks.push(text(&readout, slab.x, slab.y + slab.height - 10.0));
    }

    Ok(SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    })
}

fn text(source: &str, x: f32, y: f32) -> SceneMark {
    SceneTextMark {
        text: source.to_string().into(),
        x: x.into(),
        y: y.into(),
        baseline: TextBaseline::Alphabetic.into(),
        font: "Lato".to_string().into(),
        font_size: 14.0.into(),
        font_weight: FontWeight(400).into(),
        color: INK.into(),
        ..Default::default()
    }
    .into()
}

/// `count` with commas between its thousands.
fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

const UNCHANGED: UpdateStatus = UpdateStatus {
    rerender: false,
    rebuild_geometry: false,
};

/// A change to draw. Gestures read the plot from the state, so none needs the hit-testing tree.
const REDRAW: UpdateStatus = UpdateStatus {
    rerender: true,
    rebuild_geometry: false,
};

/// A press in the plot starts a drag there.
struct DragStart;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for DragStart {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseDown(event) = event else {
            return UNCHANGED;
        };
        if event.button == MouseButton::Left && state.in_plot(event.position).is_some() {
            state.drag = Some(Drag {
                from: event.position,
                x: state.x,
                y: state.y,
            });
        }
        UNCHANGED
    }
}

/// A moving cursor pans the view that a drag started from, so the point under the press stays
/// under the cursor.
struct DragMove;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for DragMove {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let (Some(drag), Some(position)) = (&state.drag, event.position()) else {
            return UNCHANGED;
        };
        let dx = f64::from((position[0] - drag.from[0]) / state.plot.width);
        let dy = f64::from((position[1] - drag.from[1]) / state.plot.height);
        let shift = |(start, end): (f64, f64), by: f64| {
            let delta = (end - start) * by;
            (start + delta, end + delta)
        };
        (state.x, state.y) = (shift(drag.x, -dx), shift(drag.y, dy));
        REDRAW
    }
}

/// End the drag on release or cancellation.
struct DragEnd;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for DragEnd {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        state.drag = None;
        UNCHANGED
    }
}

/// The wheel zooms the view about the cursor, 20% a step.
struct WheelZoom;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for WheelZoom {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseWheel(event) = event else {
            return UNCHANGED;
        };
        let Some(anchor) = state.in_plot(event.position) else {
            return UNCHANGED;
        };
        // A desktop wheel reports a line a notch, a browser about 100 pixels a notch, and a
        // trackpad a few pixels an event. An event zooms at most two steps.
        let steps = match event.delta {
            MouseScrollDelta::LineDelta(x, y) => f64::from(x + y),
            MouseScrollDelta::PixelDelta(x, y) => (x + y) / 100.0,
        };
        let factor = 1.2_f64.powf(-steps.clamp(-2.0, 2.0));
        let zoom = |(start, end): (f64, f64), anchor: f64| {
            let at = start + anchor * (end - start);
            (at + (start - at) * factor, at + (end - at) * factor)
        };
        (state.x, state.y) = (zoom(state.x, anchor[0]), zoom(state.y, anchor[1]));
        REDRAW
    }
}

/// A double click restores the first view.
struct Reset;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for Reset {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        (state.x, state.y) = VIEW;
        REDRAW
    }
}

/// The layout fills the window at its new size.
struct WindowResized;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for WindowResized {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let SceneGraphEvent::WindowResize(resize) = event else {
            return UNCHANGED;
        };
        // A minimized window has no size to lay the plot out in.
        if resize.size.iter().any(|&length| length < 1.0) {
            return UNCHANGED;
        }
        state.canvas = resize.size;
        REDRAW
    }
}

/// Once the view settles, request its heatmap.
struct RequestHeat;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for RequestHeat {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        state.request_heat();
        UNCHANGED
    }
}

fn on(types: SceneGraphEventType) -> EventStreamConfig {
    EventStreamConfig {
        types: vec![types],
        ..Default::default()
    }
}

/// Run the app in a window.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
            // A browser plan yields to the page between its tasks' polls, and a poll runs until
            // its partition ends or it spends Tokio's budget of 128 batches, so the data splits
            // into small partitions.
            let partitions = 16;
        } else {
            env_logger::init();
            let partitions = std::thread::available_parallelism().map_or(4, |count| count.get());
        }
    }
    let points = data::points(POINTS, partitions).expect("the points build");
    let state = AppState::new(points);
    let heat = state.heat.clone();
    let engine = state.engine.clone();
    let throttled = |types| EventStreamConfig {
        // At most one update a frame.
        throttle: Some(8),
        ..on(types)
    };
    let app = AvengerApp::try_new(
        state,
        vec![Arc::new(ChartLayer(vec![
            (on(SceneGraphEventType::MouseDown), Arc::new(DragStart)),
            (
                throttled(SceneGraphEventType::CursorMoved),
                Arc::new(DragMove),
            ),
            (on(SceneGraphEventType::MouseUp), Arc::new(DragEnd)),
            (on(SceneGraphEventType::MouseCancel), Arc::new(DragEnd)),
            (on(SceneGraphEventType::MouseWheel), Arc::new(WheelZoom)),
            (on(SceneGraphEventType::DoubleClick), Arc::new(Reset)),
            (
                on(SceneGraphEventType::WindowResize),
                Arc::new(WindowResized),
            ),
            (
                EventStreamConfig {
                    types: vec![
                        SceneGraphEventType::CursorMoved,
                        SceneGraphEventType::MouseWheel,
                        SceneGraphEventType::DoubleClick,
                    ],
                    // The view settles once it stays for 20 ms.
                    debounce: Some(DebounceConfig::new(20)),
                    ..Default::default()
                },
                Arc::new(RequestHeat),
            ),
        ]))],
        engine,
    )
    .await
    .expect("Failed to create AvengerApp");

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let mut app = app.with_task(&heat);
            app.app_state_mut().request_heat();
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(app, SCALE);
        } else {
            // The app's jobs and the host's event handling share one runtime.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .build()
                .expect("the runtime builds");
            let mut app = app
                .with_runtime(runtime.handle().clone())
                .expect("the runtime is multi-thread")
                .with_task(&heat);
            app.app_state_mut().request_heat();
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop(app, SCALE, runtime);
        }
    }
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
