//! Thirty random walks drawn as lines, with a tooltip that lists the eight series nearest the
//! pointer at the step under it. The lines' vertices are the hit-testing tree's entries, and
//! each series' rows are its scene data. See the README.

use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, Float64Array, Int64Array, RecordBatch};
use arrow::datatypes::{Float64Type, Int64Type};
use avenger_app::app::{AvengerApp, BuiltScene, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{Edges, Layout, Leaf, Measure, Measurement, Side, Size, Slab, SolveOptions};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::line::{LineHit, SceneLineMark};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_tooltip::{FocusStrategy, TooltipContent, TooltipField, TooltipTarget, Tooltips};
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The series, and the samples in each.
const SERIES: usize = 30;
const SAMPLES: usize = 500;
/// The most series that the tooltip lists.
const LISTED: usize = 8;
/// The device pixels in a unit of the scene, at which the host draws.
const SCALE: f32 = 2.0;
/// Tableau's ten categorical colors, which the series take in turn.
const PALETTE: [[f32; 4]; 10] = [
    [0.306, 0.475, 0.655, 1.0],
    [0.949, 0.557, 0.169, 1.0],
    [0.882, 0.341, 0.349, 1.0],
    [0.463, 0.718, 0.698, 1.0],
    [0.349, 0.631, 0.310, 1.0],
    [0.929, 0.788, 0.282, 1.0],
    [0.690, 0.478, 0.631, 1.0],
    [1.000, 0.616, 0.655, 1.0],
    [0.612, 0.459, 0.373, 1.0],
    [0.729, 0.690, 0.675, 1.0],
];

/// A series: its name and the rows of its samples, a step and a value each.
#[derive(Clone)]
struct Series {
    name: String,
    rows: Arc<RecordBatch>,
}

impl Series {
    fn steps(&self) -> &Int64Array {
        self.rows.column(0).as_primitive::<Int64Type>()
    }

    fn values(&self) -> &Float64Array {
        self.rows.column(1).as_primitive::<Float64Type>()
    }
}

/// Seeded random walks, the same at every run: each step adds a uniform draw from -1 to 1.
fn walks() -> Vec<Series> {
    // A xorshift generator, so the walks need no random number crate.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut uniform = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    (0..SERIES)
        .map(|index| {
            let mut value = 0.0;
            let values: Vec<f64> = (0..SAMPLES)
                .map(|_| {
                    value += uniform();
                    value
                })
                .collect();
            let steps: ArrayRef = Arc::new(Int64Array::from_iter_values(0..SAMPLES as i64));
            let values: ArrayRef = Arc::new(Float64Array::from(values));
            let rows = RecordBatch::try_from_iter([("step", steps), ("value", values)]).unwrap();
            Series {
                name: format!("Series {}", index + 1),
                rows: Arc::new(rows),
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct AppState {
    /// The window's size, which the layout fills.
    canvas: [f32; 2],
    series: Vec<Series>,
    /// The value domain, which spans every series.
    values: (f64, f64),
    engine: LabelEngine,
}

impl AppState {
    fn new() -> Self {
        let series = walks();
        let (low, high) = series
            .iter()
            .flat_map(|series| series.values().values().iter().copied())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                (low.min(value), high.max(value))
            });
        Self {
            canvas: [900.0, 520.0],
            series,
            values: (low, high),
            engine: avenger_typst_label::bundled_label_engine()
                .with_number_formatting(Arc::new(D3NumberFormatProvider::new())),
        }
    }
}

/// The parts that the layout places: the plot and each axis part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Plot,
    X(AxisPart),
    Y(AxisPart),
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The plot with its axes' parts below and to its left.
fn layout() -> Layout<Id> {
    let mut plot = Leaf::new().id(Id::Plot).min_size(Size::new(80.0, 80.0));
    for part in AXIS_PARTS {
        plot = plot
            .slab(Side::Left, Slab::new().id(Id::Y(part)))
            .slab(Side::Bottom, Slab::new().id(Id::X(part)));
    }
    plot.into()
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

struct Builder;

#[async_trait::async_trait]
impl SceneGraphBuilder<AppState> for Builder {
    async fn build(&self, state: &mut AppState) -> Result<BuiltScene, AvengerAppError> {
        make_scene(state).map_err(|err| AvengerAppError::InternalError(err.to_string()))
    }
}

/// The scene: the axes, and a line for each series whose vertices hits find. Each line's rows
/// are its scene data, one for each vertex.
fn make_scene(state: &mut AppState) -> Result<BuiltScene, Box<dyn std::error::Error>> {
    let engine = &state.engine;
    let config = |side, format: &str| AxisConfig {
        side,
        tick_format: format.into(),
        tick_label: None,
        style: Default::default(),
    };
    let steps = (0.0, (SAMPLES - 1) as f64);
    let x_axis = Axis::continuous(
        &LinearScale::configured(steps, (0.0, 1.0)),
        "Step",
        &config(GuideSide::Bottom, ",d"),
        engine,
    )?;
    let y_axis = Axis::continuous(
        &LinearScale::configured(state.values, (1.0, 0.0)),
        "Value",
        &config(GuideSide::Left, ",f"),
        engine,
    )?;

    let [width, height] = state.canvas;
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

    let x_scale = LinearScale::configured(steps, (0.0, f64::from(plot.width)));
    let y_scale = LinearScale::configured(state.values, (f64::from(plot.height), 0.0));
    let mut data = SceneData::new();
    let mut lines: Vec<SceneMark> = Vec::new();
    for (index, series) in state.series.iter().enumerate() {
        let steps: ArrayRef = Arc::new(series.steps().clone());
        let values: ArrayRef = Arc::new(series.values().clone());
        lines.push(
            SceneLineMark {
                name: series.name.clone(),
                len: SAMPLES as u32,
                x: x_scale.scale_to_numeric(&steps)?,
                y: y_scale.scale_to_numeric(&values)?,
                stroke: ColorOrGradient::Color(PALETTE[index % PALETTE.len()]),
                stroke_width: 1.5,
                hit: LineHit::Vertices,
                ..Default::default()
            }
            .into(),
        );
        data.insert(series.name.clone(), series.rows.clone());
    }

    let size = [plot.width, plot.height];
    let mut marks: Vec<SceneMark> = vec![SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            y_axis.grid(size).into(),
            x_axis.grid(size).into(),
            SceneGroup {
                marks: lines,
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
    let scene = SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    };
    Ok(BuiltScene { scene, data })
}

/// The layout fills the window at its new size, and the hit-testing tree finds the lines where
/// they now lie.
struct WindowResized;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for WindowResized {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::WindowResize(resize) = event else {
            return UpdateStatus::default();
        };
        // A minimized window has no size to lay the plot out in.
        if resize.size.iter().any(|&length| length < 1.0) {
            return UpdateStatus::default();
        }
        state.canvas = resize.size;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
            overlay: false,
        }
    }
}

/// Run the app in a window.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }
    let state = AppState::new();
    let engine = state.engine.clone();
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<AppState>>)> = vec![(
        EventStreamConfig {
            types: vec![SceneGraphEventType::WindowResize],
            ..Default::default()
        },
        Arc::new(WindowResized),
    )];
    // Each series' vertex nearest the pointer's step, within 20 px, and the eight of them nearest
    // the pointer's height, under the step, with their values.
    let series = TooltipTarget::new(FocusStrategy::GroupX {
        max_radius: 20.0,
        limit: Some(LISTED),
    })
    .with_content(
        TooltipContent::new()
            .with_header(TooltipField::new("step").with_label("Step"))
            .with_fields([TooltipField::new("value").with_format(".2f")]),
    );
    let app = AvengerApp::try_new(state, Arc::new(Builder), streams, engine)
        .await
        .expect("Failed to create AvengerApp")
        .with_overlay(Arc::new(Tooltips::over(vec![series])))
        .expect("Failed to build the tooltip");

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(app, SCALE);
        } else {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("the runtime builds");
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop(app, SCALE, runtime);
        }
    }
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
