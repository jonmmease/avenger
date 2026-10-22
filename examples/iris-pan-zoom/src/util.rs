use arrow::array::{ArrayRef, AsArray, Float32Builder, StringArray, StringBuilder};
use avenger_app::app::{AvengerApp, BuildContext, Layer};
use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, EventStreamFilter, UpdateStatus};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::legend::{Legend, LegendConfig, LegendSwatches, SymbolSwatches};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Edges, Layout, Leaf, Measure, Measurement, Rect, Side, Size, Slab, SolveOptions,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::{EventStreamHandler, LayerTrees};
use csv::Reader;
use rand_distr::Distribution;
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
#[cfg(not(target_arch = "wasm32"))]
use std::io::BufReader;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[derive(Clone)]
pub struct PanAnchor {
    pub range_position: [f32; 2],
    pub x_domain: (f64, f64),
    pub y_domain: (f64, f64),
}

#[derive(Clone)]
pub struct ChartState {
    #[allow(dead_code)]
    pub hover_index: Option<usize>,
    /// The window's size, which the layout fills.
    pub canvas: [f32; 2],
    /// The plot area that the last scene laid out, which pans and zooms map positions in.
    pub plot: Rect,
    pub domain_sepal_length: (f64, f64),
    pub domain_sepal_width: (f64, f64),
    pub sepal_length: ArrayRef,
    #[allow(dead_code)]
    pub sepal_width: ArrayRef,
    #[allow(dead_code)]
    pub species: ArrayRef,

    // Scales for the base plot, which are used to scale the original x/y
    pub base_x_scale: ConfiguredScale,
    pub base_y_scale: ConfiguredScale,

    // Prescaled
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub legend: Arc<Legend>,

    // Measures and draws the scene's text
    pub text_engine: LabelEngine,

    // For panning
    pub pan_anchor: Option<PanAnchor>,
}

impl Default for ChartState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartState {
    pub fn new() -> Self {
        // Load data
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                // Embed CSV data for WASM builds
                let csv_data = include_str!("../data/Iris.csv");
                let mut csv_reader = Reader::from_reader(csv_data.as_bytes());
            } else {
                // Load from file for native builds
                let manifest_dir = env!("CARGO_MANIFEST_DIR");
                let data_path = format!("{manifest_dir}/data/Iris.csv");
                let file = File::open(data_path).expect("Failed to open Iris.csv");
                let reader = BufReader::new(file);
                let mut csv_reader = Reader::from_reader(reader);
            }
        }

        // Initialize vectors to store the data
        let mut sepal_length: Vec<f32> = Vec::new();
        let mut sepal_width: Vec<f32> = Vec::new();
        let mut petal_length: Vec<f32> = Vec::new();
        let mut petal_width: Vec<f32> = Vec::new();
        let mut species: Vec<String> = Vec::new();

        // Read the CSV records
        for result in csv_reader.records() {
            let record = result.expect("Failed to read CSV record");

            // Skip header row by checking if the first column is numeric
            if record[1].parse::<f32>().is_ok() {
                sepal_length.push(record[1].parse::<f32>().unwrap());
                sepal_width.push(record[2].parse::<f32>().unwrap());
                petal_length.push(record[3].parse::<f32>().unwrap());
                petal_width.push(record[4].parse::<f32>().unwrap());
                species.push(record[5].to_string());
            }
        }

        // Duplicate data N times with random jitter
        let n = 100;
        let jitter = 0.1;
        // let mut large_sepal_length = Vec::with_capacity(n * species.len());
        // let mut large_sepal_width = Vec::with_capacity(n * species.len());
        // let mut large_species = Vec::with_capacity(n * species.len());

        let mut sepal_langth_builder = Float32Builder::new();
        let mut sepal_width_builder = Float32Builder::new();
        let mut species_builder = StringBuilder::new();

        let normal = rand_distr::Normal::new(0.0, jitter).unwrap();
        let mut rng = rand::thread_rng();

        for _ in 0..n {
            sepal_length.iter().for_each(|x| {
                sepal_langth_builder.append_value(x + normal.sample(&mut rng));
            });
            sepal_width.iter().for_each(|x| {
                sepal_width_builder.append_value(x + normal.sample(&mut rng));
            });
            species.iter().for_each(|x| {
                species_builder.append_value(x.clone());
            });
        }

        // Build arrays
        let sepal_length_array = Arc::new(sepal_langth_builder.finish()) as ArrayRef;
        let sepal_width_array = Arc::new(sepal_width_builder.finish()) as ArrayRef;
        let species_array = Arc::new(species_builder.finish()) as ArrayRef;

        // println!("data length: {:?}", sepal_length_array.len());

        let fill_opacity = 0.1;
        let color_scale = OrdinalScale::configured(Arc::new(StringArray::from(vec![
            "Iris-setosa",
            "Iris-versicolor",
            "Iris-virginica",
        ])))
        .with_range_colors(vec![
            [0.9, 0.0, 0.0, fill_opacity],
            [0.0, 0.9, 0.0, fill_opacity],
            [0.0, 0.0, 0.9, fill_opacity],
        ])
        .unwrap();

        // println!("color_scale {:?}", color_scale);

        // Make fill, taking the hover index into account
        let fill = color_scale.scale_to_color(&species_array).unwrap();

        let domain_sepal_length = (4.0, 8.5);
        let domain_sepal_width = (1.5, 5.0);

        // The points are scaled once to a base plot, and each scene adjusts them to the plot
        // area and domains that it shows.
        let base = 200.0;
        let base_x_scale = LinearScale::configured(domain_sepal_length, (0.0, base));
        let base_y_scale = LinearScale::configured(domain_sepal_width, (base, 0.0));

        let x = base_x_scale.scale_to_numeric(&sepal_length_array).unwrap();
        let y = base_y_scale.scale_to_numeric(&sepal_width_array).unwrap();

        // Make symbol legend, measured with the engine that formats the axes and draws the chart
        let text_engine = avenger_typst_label::bundled_label_engine()
            .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
        let legend = Legend::new(
            &LegendConfig {
                text: color_scale
                    .domain()
                    .as_string::<i32>()
                    .iter()
                    .map(|species| species.unwrap_or_default().to_string())
                    .collect::<Vec<_>>()
                    .into(),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                    stroke_width: Some(1.0),
                    fill: color_scale.scale_to_color(color_scale.domain()).unwrap(),
                    angle: 0.0.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &text_engine,
        )
        .unwrap();

        Self {
            hover_index: None,
            canvas: [640.0, 440.0],
            plot: Rect::new(0.0, 0.0, base as f32, base as f32),
            base_x_scale,
            base_y_scale,
            domain_sepal_length,
            domain_sepal_width,
            x,
            y,
            sepal_length: sepal_length_array,
            sepal_width: sepal_width_array,
            species: species_array,
            fill,
            legend: Arc::new(legend),
            text_engine,
            pan_anchor: None,
        }
    }

    /// Scale for current x domain, across the plot area
    pub fn x_scale(&self) -> ConfiguredScale {
        LinearScale::configured(self.domain_sepal_length, (0.0, f64::from(self.plot.width)))
    }

    /// Scale for current y domain, up the plot area
    pub fn y_scale(&self) -> ConfiguredScale {
        LinearScale::configured(self.domain_sepal_width, (f64::from(self.plot.height), 0.0))
    }

    /// A position in the plot area's frame.
    fn plot_position(&self, position: [f32; 2]) -> [f32; 2] {
        [position[0] - self.plot.x, position[1] - self.plot.y]
    }
}

/// Draw a crosshair and display sepal length and width at the pointer. Pointer movement rebuilds
/// only this layer. Its marks are noninteractive, so events reach the chart.
#[derive(Default)]
struct Crosshair {
    /// The pointer's position in the scene, while it lies in the window.
    pointer: Arc<Mutex<Option<[f32; 2]>>>,
}

#[async_trait::async_trait]
impl Layer<ChartState> for Crosshair {
    async fn build(
        &self,
        state: &mut ChartState,
        context: &BuildContext<'_>,
    ) -> Result<SceneGraph, AvengerAppError> {
        let chart = &context.scenes_below()[0];
        let pointer = *self.pointer.lock().unwrap_or_else(PoisonError::into_inner);
        let plot = state.plot;
        let marks = match pointer {
            Some([x, y])
                if (plot.x..=plot.x + plot.width).contains(&x)
                    && (plot.y..=plot.y + plot.height).contains(&y) =>
            {
                crosshair_marks(state, [x, y])
            }
            _ => Vec::new(),
        };
        Ok(SceneGraph {
            marks,
            width: chart.width,
            height: chart.height,
            origin: chart.origin,
        })
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<ChartState>>)> {
        let follow = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        vec![(follow, Arc::new(FollowPointer(self.pointer.clone())))]
    }
}

/// Rules across the plot area through `pointer`, and a label beside it with the sepal length and
/// width there.
fn crosshair_marks(state: &ChartState, pointer: [f32; 2]) -> Vec<SceneMark> {
    let plot = state.plot;
    let [x, y] = pointer;
    let rules = SceneRuleMark {
        interactive: Some(false),
        len: 2,
        x: vec![x, plot.x].into(),
        y: vec![plot.y, y].into(),
        x2: vec![x, plot.x + plot.width].into(),
        y2: vec![plot.y + plot.height, y].into(),
        stroke: ColorOrGradient::Color([0.4, 0.4, 0.4, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    };
    let [plot_x, plot_y] = state.plot_position(pointer);
    let length = state.x_scale().invert_scalar(f64::from(plot_x));
    let width = state.y_scale().invert_scalar(f64::from(plot_y));
    let (Ok(length), Ok(width)) = (length, width) else {
        return vec![rules.into()];
    };
    let readout = SceneTextMark {
        interactive: Some(false),
        text: format!("sepal length {length:.2}, sepal width {width:.2}").into(),
        x: (x + 8.0).into(),
        y: (y - 8.0).into(),
        font_size: 12.0.into(),
        color: [0.2, 0.2, 0.2, 1.0].into(),
        ..Default::default()
    };
    vec![rules.into(), readout.into()]
}

/// Keeps the pointer's position for the crosshair, and builds the crosshair's layer again.
struct FollowPointer(Arc<Mutex<Option<[f32; 2]>>>);

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for FollowPointer {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = event.position();
        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}

/// The parts of the chart that the layout places: the plot area, each axis part and the legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ChartId {
    Plot,
    X(AxisPart),
    Y(AxisPart),
    Legend,
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The plot area with its axes' parts in slabs below and to its left, and the legend to its
/// right after a gap.
fn chart_layout() -> Layout<ChartId> {
    let mut plot = Leaf::new()
        .id(ChartId::Plot)
        .min_size(Size::new(80.0, 80.0));
    for part in AXIS_PARTS {
        plot = plot
            .slab(Side::Left, Slab::new().id(ChartId::Y(part)))
            .slab(Side::Bottom, Slab::new().id(ChartId::X(part)));
    }
    plot.slab(Side::Right, Slab::new().min_width(12.0))
        .slab(Side::Right, Slab::new().id(ChartId::Legend))
        .into()
}

/// Measures the chart's axis parts and legend for the layout.
struct ChartGuides<'a> {
    x: &'a Axis,
    y: &'a Axis,
    legend: &'a Legend,
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<ChartId> for ChartGuides<'_> {
    fn height(&mut self, id: &ChartId, width: f32) -> Option<Measurement> {
        match id {
            ChartId::X(part) => Some(answer(self.x.extent(*part, width))),
            _ => None,
        }
    }

    fn width(&mut self, id: &ChartId, height: f32) -> Option<Measurement> {
        match id {
            ChartId::Y(part) => Some(answer(self.y.extent(*part, height))),
            ChartId::Legend => Some(answer(self.legend.extent(height))),
            _ => None,
        }
    }
}

/// The chart's layer, with the streams that pan and zoom it.
struct IrisChart(Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<ChartState>>)>);

#[async_trait::async_trait]
impl Layer<ChartState> for IrisChart {
    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<ChartState>>)> {
        self.0.clone()
    }

    async fn build(
        &self,
        state: &mut ChartState,
        _: &BuildContext<'_>,
    ) -> Result<SceneGraph, AvengerAppError> {
        Ok(make_scene_graph(state))
    }
}

fn make_scene_graph(chart_state: &mut ChartState) -> SceneGraph {
    #[cfg(not(target_arch = "wasm32"))]
    let start_time = avenger_common::time::Instant::now();

    // Prepare the axes for the current domains. Each call sets the scale's range from its
    // length, in the direction that the range has here: up the plot for the y axis.
    let engine = &chart_state.text_engine;
    let axis = |domain: (f64, f64), range: (f64, f64), title: &str, side: GuideSide| {
        let config = AxisConfig {
            side,
            // Vega's default axis format
            tick_format: ",f".into(),
            tick_label: None,
            style: Default::default(),
        };
        let scale = LinearScale::configured(domain, range);
        Axis::continuous(&scale, title, &config, engine).unwrap()
    };
    let x_axis = axis(
        chart_state.domain_sepal_length,
        (0.0, 1.0),
        "Sepal Length",
        GuideSide::Bottom,
    );
    let y_axis = axis(
        chart_state.domain_sepal_width,
        (1.0, 0.0),
        "Sepal Width",
        GuideSide::Left,
    );

    // Lay the chart out in the window, measuring the axes and the legend. The domains change
    // the axis labels, so every scene measures again.
    let [width, height] = chart_state.canvas;
    let options = SolveOptions {
        padding: Edges::all(16.0),
        ..SolveOptions::sized(width, height)
    };
    let mut guides = ChartGuides {
        x: &x_axis,
        y: &y_axis,
        legend: &chart_state.legend,
    };
    let solution = chart_layout()
        .measure(&options, &mut guides)
        .unwrap()
        .into_solution();
    let region = |id: ChartId| {
        solution
            .region(&id)
            .expect("every part has a region")
            .content
    };
    let plot = region(ChartId::Plot);
    chart_state.plot = plot;

    // Build scales and compute adjustments
    let x_scale = chart_state.x_scale();
    let x_adjustment = chart_state.base_x_scale.adjust(&x_scale).unwrap();
    let y_scale = chart_state.y_scale();
    let y_adjustment = chart_state.base_y_scale.adjust(&y_scale).unwrap();

    // Make symbol mark
    let shape = SymbolShape::from_vega_str("circle").unwrap();

    let points = SceneSymbolMark {
        len: chart_state.sepal_length.len() as u32,
        x: chart_state.x.clone(),
        y: chart_state.y.clone(),
        x_adjustment: Some(x_adjustment),
        y_adjustment: Some(y_adjustment),
        shapes: vec![shape.clone()],
        shape_index: 0.into(),
        fill: chart_state.fill.clone(),
        size: 20.0.into(),
        stroke_width: 0.0f32.into(),
        ..Default::default()
    };

    // The grid lines and the points in the plot area, clipped to keep off the axes
    let size = [plot.width, plot.height];
    let plot_group = SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            y_axis.grid(size).into(),
            x_axis.grid(size).into(),
            SceneGroup {
                marks: vec![points.into()],
                clip: Clip::Rect {
                    x: 0.5,
                    y: 0.5,
                    width: plot.width - 1.0,
                    height: plot.height - 1.0,
                },
                ..Default::default()
            }
            .into(),
        ],
        ..Default::default()
    };

    // Each axis part in its slab, at the slab's length
    let mut marks: Vec<SceneMark> = vec![plot_group.into()];
    for part in AXIS_PARTS {
        for (axis, id, length) in [
            (&x_axis, ChartId::X(part), plot.width),
            (&y_axis, ChartId::Y(part), plot.height),
        ] {
            let slab = region(id);
            if let Some(mut group) = axis.part(part, length) {
                group.origin = axis.origin([slab.x, slab.y, slab.width, slab.height]);
                marks.push(group.into());
            }
        }
    }
    let slab = region(ChartId::Legend);
    let mut legend = chart_state.legend.marks(slab.height).group;
    legend.origin = chart_state
        .legend
        .origin([slab.x, slab.y, slab.width, slab.height]);
    marks.push(legend.into());

    let scene_graph = SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        let duration = start_time.elapsed(); // Calculate elapsed time
        println!("Scene construction time: {duration:?}"); // Print the duration
    }

    scene_graph
}

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

    // Predefine configs that are used in multiple handlers
    let left_mouse_down_config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            let SceneGraphEvent::MouseDown(mouse_down) = event else {
                return false;
            };
            mouse_down.button == MouseButton::Left
        }))]),
        ..Default::default()
    };
    let left_mouse_up_config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            let SceneGraphEvent::MouseUp(mouse_up) = event else {
                return false;
            };
            mouse_up.button == MouseButton::Left
        }))]),
        ..Default::default()
    };

    let state = ChartState::new();
    let text_engine = state.text_engine.clone();
    let avenger_app = AvengerApp::try_new(
        state,
        vec![
            Arc::new(IrisChart(vec![
                // Panning (record click anchor)
                (left_mouse_down_config.clone(), Arc::new(PanningClick)),
                // Panning (dragging)
                (
                    EventStreamConfig {
                        types: vec![SceneGraphEventType::CursorMoved],
                        between: Some((
                            Box::new(left_mouse_down_config.clone()),
                            Box::new(left_mouse_up_config.clone()),
                        )),
                        throttle: Some(8), // Don't update faster than 60fps
                        ..Default::default()
                    },
                    Arc::new(PanningDrag),
                ),
                // Panning (release)
                (left_mouse_up_config, Arc::new(PanningRelease)),
                // Panning (a press that the window cancels ends it, as a release would)
                (
                    EventStreamConfig {
                        types: vec![SceneGraphEventType::MouseCancel],
                        ..Default::default()
                    },
                    Arc::new(PanningRelease),
                ),
                // wheel zoom
                (
                    EventStreamConfig {
                        types: vec![SceneGraphEventType::MouseWheel],
                        throttle: Some(8), // Don't update faster than 60fps
                        ..Default::default()
                    },
                    Arc::new(WheelZoom),
                ),
                // The layout fills the window at its new size
                (
                    EventStreamConfig {
                        types: vec![SceneGraphEventType::WindowResize],
                        ..Default::default()
                    },
                    Arc::new(WindowResized),
                ),
            ])),
            Arc::new(Crosshair::default()),
        ],
        text_engine,
    )
    .await
    .expect("Failed to create AvengerApp");

    let (mut app, event_loop) = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime)
        }
        #[cfg(target_arch = "wasm32")]
        {
            WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0)
        }
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

// Panning (record click anchor)
struct PanningClick;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningClick {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let [plot_x, plot_y] = state.plot_position(event.position().unwrap());

        // Get scales
        let x_scale = state.x_scale();
        let y_scale = state.y_scale();

        // Check if cursor is over the plot area
        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let normalized_x = (f64::from(plot_x) - range_start) / (range_end - range_start);
        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let normalized_y = (f64::from(plot_y) - range_start) / (range_end - range_start);
        if !(0.0..=1.0).contains(&normalized_x) || !(0.0..=1.0).contains(&normalized_y) {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        }

        state.pan_anchor = Some(PanAnchor {
            range_position: [plot_x, plot_y],
            x_domain: state.domain_sepal_length,
            y_domain: state.domain_sepal_width,
        });

        UpdateStatus {
            rerender: false,
            rebuild_geometry: false,
        }
    }
}

// Panning (dragging)
struct PanningDrag;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningDrag {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let Some(pan_anchor) = &state.pan_anchor else {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        };

        // Get the cursor position in range space
        let [plot_x, plot_y] = state.plot_position(event.position().unwrap());

        let x_scale = state.x_scale().with_domain_interval(pan_anchor.x_domain);
        let y_scale = state.y_scale().with_domain_interval(pan_anchor.y_domain);

        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let x_delta = f64::from(plot_x - pan_anchor.range_position[0]) / (range_end - range_start);

        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let y_delta = f64::from(plot_y - pan_anchor.range_position[1]) / (range_end - range_start);

        // Update domains
        state.domain_sepal_length = x_scale
            .pan(x_delta)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();
        state.domain_sepal_width = y_scale
            .pan(y_delta)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();

        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}

// Panning (release)
struct PanningRelease;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningRelease {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        state.pan_anchor = None;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
        }
    }
}

// wheel zoom
struct WheelZoom;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for WheelZoom {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseWheel(event) = event else {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        };

        // Get scales
        let x_scale = state.x_scale();
        let y_scale = state.y_scale();

        // Get cursor position
        let [plot_x, plot_y] = state.plot_position(event.position);

        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let normalized_x = (f64::from(plot_x) - range_start) / (range_end - range_start);
        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let normalized_y = (f64::from(plot_y) - range_start) / (range_end - range_start);

        // Check if cursor is over the plot area
        if !(0.0..=1.0).contains(&normalized_x) || !(0.0..=1.0).contains(&normalized_y) {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        }

        let factor = match event.delta {
            MouseScrollDelta::LineDelta(x_line_delta, y_line_delta) => {
                -f64::from(x_line_delta + y_line_delta) * 0.005 + 1.0
            }
            MouseScrollDelta::PixelDelta(x_pixel_delta, y_pixel_delta) => {
                -(x_pixel_delta + y_pixel_delta) * 0.005 + 1.0
            }
        };

        state.domain_sepal_length = x_scale
            .zoom(normalized_x, factor)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();
        state.domain_sepal_width = y_scale
            .zoom(normalized_y, factor)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();

        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}

// The layout fills the window at its new size
struct WindowResized;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for WindowResized {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let SceneGraphEvent::WindowResize(resize) = event else {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        };
        state.canvas = resize.size;
        // Panning and zooming read the plot area from the state, not the hit-test tree, so a
        // resize skips rebuilding the tree.
        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}
