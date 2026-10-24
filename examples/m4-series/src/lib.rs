//! Ten million samples of a random walk drawn as a line through at most four samples for each
//! pixel column. A background task decimates the view that a pan or a zoom settles on, through
//! DataFusion, and the scene draws the latest line through the current view meanwhile. See the
//! README for the controls.

mod data;
pub mod m4;

use std::sync::Arc;
use std::time::Duration;

use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app::task::Task;
use avenger_app_tools::navigation::PanZoom;
use avenger_color::ColorOrGradient;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Edges, Layout, Leaf, Measure, Measurement, Rect, Side, Size, Slab, SolveOptions,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::prelude::DataFrame;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::m4::{Line, LineKey};

/// The samples in the series.
const SAMPLES: usize = 10_000_000;
/// The device pixels in a unit of the scene, at which the host draws.
const SCALE: f32 = 2.0;
/// The most pixel columns of a line.
const MAX_COLUMNS: f32 = 4096.0;
/// The y domain until a line with samples arrives.
const NO_LINE: (f64, f64) = (-1.0, 1.0);

/// Room above the plot for the top y label's overhang, so the plot's top stays put as the
/// labels change.
const TOP: f32 = 12.0;
/// Room right of the plot for the last x label's overhang, which reaches 28 px at deep zooms.
const RIGHT: f32 = 32.0;
/// The y labels' least width, enough for labels as wide as "-12,000", so the plot's left edge
/// stays put as the y domain follows the line.
const Y_LABELS: f32 = 48.0;
const LINE: [f32; 4] = [0.27, 0.47, 0.71, 1.0];

#[derive(Clone)]
pub struct AppState {
    /// Plot bounds from the latest layout, used for navigation.
    plot: Rect,
    /// The horizontal scale whose domain navigation changes.
    x: ConfiguredScale,
    /// The y domain: the latest line's values, padded by a twentieth of their span, or the
    /// previous domain when no sample lies in the latest line's view.
    y: (f64, f64),
    series: DataFrame,
    line: Task<LineKey, Line>,
    engine: LabelEngine,
}

impl AppState {
    fn new(series: DataFrame) -> Self {
        Self {
            plot: Rect::new(0.0, 0.0, 1.0, 1.0),
            x: LinearScale::configured(whole(), (0.0, 1.0)),
            y: NO_LINE,
            series,
            line: Task::new(),
            engine: avenger_typst_label::bundled_label_engine()
                .with_number_formatting(Arc::new(D3NumberFormatProvider::new())),
        }
    }

    /// The view's x domain.
    fn x_domain(&self) -> (f64, f64) {
        (self.x.numeric_interval_domain()).expect("the view's x scale is linear")
    }

    /// Request the line of the view at the plot's width in device pixels.
    fn request_line(&self) {
        let key = LineKey {
            x: self.x_domain(),
            width: (self.plot.width * SCALE).round().clamp(1.0, MAX_COLUMNS) as u32,
        };
        let series = self.series.clone();
        self.line
            .request(key.clone(), move |cancel| m4::line(series, key, cancel));
    }
}

/// The x domain of the whole series.
fn whole() -> (f64, f64) {
    (0.0, (SAMPLES - 1) as f64)
}

/// Where `value` lies along a scene length `length` that spans `domain`.
fn place(domain: (f64, f64), length: f32, value: f64) -> f32 {
    ((value - domain.0) / (domain.1 - domain.0)) as f32 * length
}

/// The parts that the layout places: the plot and each axis part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Plot,
    X(AxisPart),
    Y(AxisPart),
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The plot with its axes' parts below and to its left, and fixed room above and to its right,
/// so new axis labels don't move the plot.
fn layout() -> Layout<Id> {
    let mut plot = Leaf::new().id(Id::Plot).min_size(Size::new(80.0, 80.0));
    for part in AXIS_PARTS {
        let y = match part {
            AxisPart::Labels => Slab::new().id(Id::Y(part)).min_width(Y_LABELS),
            _ => Slab::new().id(Id::Y(part)),
        };
        plot = plot
            .slab(Side::Left, y)
            .slab(Side::Bottom, Slab::new().id(Id::X(part)));
    }
    plot.slab(Side::Top, Slab::new().min_height(TOP))
        .slab(Side::Right, Slab::new().min_width(RIGHT))
        .into()
}

struct Guides<'a> {
    x: &'a Axis,
    y: &'a Axis,
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
            _ => None,
        }
    }
}

/// The chart's scene, which fills the window.
const CHART: &str = "chart";

/// Draws the chart and requests the view's line when the chart needs a tree rebuild.
/// This includes initialization, resize, and gesture settling.
struct Chart;

#[async_trait::async_trait]
impl AppBuild<AppState> for Chart {
    fn scenes(&self) -> Vec<Scene<AppState>> {
        vec![Scene::new(CHART, 0)]
    }

    async fn build(
        &self,
        state: &mut AppState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        let scene = make_scene_graph(state, scenes.size())
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        // A request for the line that the view has, or awaits, starts nothing, so the rebuild
        // that a finished line causes requests no other.
        if scenes.requested_tree(CHART) {
            state.request_line();
        }
        scenes.set(CHART, scene.into())
    }
}

/// The scene: the axes of the view and the latest line placed in it. Each scene
/// places the latest line through the current view, so it follows a pan or a zoom until the
/// view's own line arrives.
fn make_scene_graph(
    state: &mut AppState,
    size: [f32; 2],
) -> Result<SceneGraph, Box<dyn std::error::Error>> {
    let engine = &state.engine;
    let config = |side, format: &str| AxisConfig {
        side,
        tick_format: format.into(),
        tick_label: None,
        style: Default::default(),
    };
    let latest = state.line.latest();
    if let Some((low, high)) = latest.as_ref().and_then(|done| done.value.y) {
        let pad = ((high - low) / 20.0).max(0.5);
        state.y = (low - pad, high + pad);
    }
    let y = state.y;
    let x = state.x_domain();
    let x_axis = Axis::continuous(
        &state.x,
        "Sample",
        // SI prefixes keep the sample counts short.
        &config(GuideSide::Bottom, "~s"),
        engine,
    )?;
    let y_axis = Axis::continuous(
        &LinearScale::configured(y, (1.0, 0.0)),
        "Value",
        &config(GuideSide::Left, ",f"),
        engine,
    )?;

    let [width, height] = size;
    let options = SolveOptions {
        padding: Edges::all(16.0),
        ..SolveOptions::sized(width, height)
    };
    let mut guides = Guides {
        x: &x_axis,
        y: &y_axis,
    };
    let solution = layout().measure(&options, &mut guides)?.into_solution();
    let region = |id: Id| solution.region(&id).map(|region| region.content);
    let plot = region(Id::Plot).ok_or("the layout places the plot")?;
    state.plot = plot;

    // The latest line, through the current view.
    let mut line = Vec::new();
    if let Some(done) = &latest {
        let points = &done.value.points;
        line.push(
            SceneLineMark {
                len: points.len() as u32,
                x: points
                    .iter()
                    .map(|point| place(x, plot.width, point[0]))
                    .collect::<Vec<_>>()
                    .into(),
                y: points
                    .iter()
                    .map(|point| plot.height - place(y, plot.height, point[1]))
                    .collect::<Vec<_>>()
                    .into(),
                stroke: ColorOrGradient::Color(LINE),
                stroke_width: 1.0,
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
                marks: line,
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
    Ok(SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    })
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
    let series = data::walk(SAMPLES, partitions).expect("the series builds");
    let state = AppState::new(series);
    let line = state.line.clone();
    let engine = state.engine.clone();
    // The view comes to rest once it stays for 20 ms.
    let pan_zoom = PanZoom::x(CHART, |s: &AppState| s.plot, |s: &mut AppState| &mut s.x);
    let app = AvengerApp::builder(state, Chart, engine)
        .size([900.0, 480.0])
        .tool(pan_zoom.with_settle_delay(Duration::from_millis(20)))
        .build()
        .await
        .expect("Failed to create AvengerApp");

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let app = app.with_task(&line);
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(app, SCALE);
        } else {
            // The app's jobs and the host's event handling share one runtime.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .build()
                .expect("the runtime builds");
            let app = app
                .with_runtime(runtime.handle().clone())
                .expect("the runtime is multi-thread")
                .with_task(&line);
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop(app, SCALE, runtime);
        }
    }
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
