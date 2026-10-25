use arrow::array::{ArrayRef, AsArray, Float32Builder, StringArray, StringBuilder};
use arrow::datatypes::Float32Type;
use arrow::record_batch::RecordBatch;
use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app_tools::navigation::PanZoom;
use avenger_app_tools::text_selection::TextSelection;
use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::data::{Datum, SceneData};
use avenger_geometry::geo_types::Geometry;
use avenger_geometry::query::Nearest;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::legend::{Legend, LegendConfig, LegendSwatches, SymbolSwatches};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Edges, Layout, Leaf, Measure, Measurement, Overflow, Rect, Side, Size, Slab, SolveOptions,
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
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use csv::Reader;
use rand_distr::Distribution;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
#[cfg(not(target_arch = "wasm32"))]
use std::io::BufReader;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[derive(Clone)]
pub struct ChartState {
    #[allow(dead_code)]
    pub hover_index: Option<usize>,
    /// Window pointer position used by the crosshair, or `None` outside the window.
    pub pointer: Option<[f32; 2]>,
    /// Plot bounds from the latest layout, used for navigation.
    pub plot: Rect,
    /// Sepal length across the plot. Navigation changes the domain, and layout sets the range.
    pub x_scale: ConfiguredScale,
    /// Sepal width up the plot. Navigation changes the domain, and layout sets the range.
    pub y_scale: ConfiguredScale,
    /// The flowers' rows, one for each point: sepal length, sepal width, and species.
    pub flowers: Arc<RecordBatch>,

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
        let flowers = RecordBatch::try_from_iter([
            ("sepal_length", sepal_length_array),
            ("sepal_width", sepal_width_array),
            ("species", species_array),
        ])
        .map(Arc::new)
        .unwrap();

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
            pointer: None,
            plot: Rect::new(0.0, 0.0, base as f32, base as f32),
            x_scale: base_x_scale.clone(),
            y_scale: base_y_scale.clone(),
            base_x_scale,
            base_y_scale,
            x,
            y,
            flowers,
            fill,
            legend: Arc::new(legend),
            text_engine,
        }
    }

    /// A position in the plot area's frame.
    fn plot_position(&self, position: [f32; 2]) -> [f32; 2] {
        [position[0] - self.plot.x, position[1] - self.plot.y]
    }
}

/// The chart's scene: the flowers, the axes, and the legend.
const CHART: &str = "chart";

/// The crosshair's scene, over the chart.
const CROSSHAIR: &str = "crosshair";

/// Selection scene above the crosshair.
const SELECTION: &str = "selection";

/// How near the pointer the crosshair finds a flower, in pixels.
const SNAP_RADIUS: f32 = 40.0;

/// The name of the flowers' mark, which names its rows too.
const FLOWERS: &str = "flowers";

/// Draw a crosshair at the nearest flower within 40 pixels, with its ring and data.
/// If no flower is nearby or the chart tree is stale, use the pointer position and
/// inverted scale values. Draw nothing outside the plot. Marks are noninteractive.
fn crosshair(state: &ChartState, scenes: &Scenes<'_>) -> Vec<SceneMark> {
    let plot = state.plot;
    match state.pointer {
        Some([x, y])
            if (plot.x..=plot.x + plot.width).contains(&x)
                && (plot.y..=plot.y + plot.height).contains(&y) =>
        {
            let tree = scenes.tree(CHART);
            let nearest = Nearest::new([x, y], SNAP_RADIUS).filter(|mark| mark.name == FLOWERS);
            let flower = if scenes.is_current(CHART) {
                tree.nearest(nearest).next()
            } else {
                None
            };
            // A circle symbol's geometry is its center.
            let flower = flower.and_then(|flower| {
                let Geometry::Point(center) = flower.instance.geometry else {
                    return None;
                };
                let datum = tree.datum(&flower.instance.mark_instance)?;
                Some(([center.x(), center.y()], flower.instance.reach(), datum))
            });
            match flower {
                Some((center, reach, datum)) => flower_marks(plot, center, reach, &datum),
                None => crosshair_marks(state, [x, y]),
            }
        }
        _ => Vec::new(),
    }
}

/// Rules across the plot area through `pointer`, and a label beside it with the sepal length and
/// width there.
fn crosshair_marks(state: &ChartState, pointer: [f32; 2]) -> Vec<SceneMark> {
    let rules = rules(state.plot, pointer);
    let [plot_x, plot_y] = state.plot_position(pointer);
    let length = state.x_scale.invert_scalar(f64::from(plot_x));
    let width = state.y_scale.invert_scalar(f64::from(plot_y));
    let (Ok(length), Ok(width)) = (length, width) else {
        return vec![rules.into()];
    };
    let text = format!("sepal length {length:.2}, sepal width {width:.2}");
    vec![rules.into(), readout(pointer, text).into()]
}

/// Rules across the plot area through the flower at `center`, a ring around it outside its
/// `reach`, and a label beside it with its row.
fn flower_marks(plot: Rect, center: [f32; 2], reach: f32, datum: &Datum) -> Vec<SceneMark> {
    let column = |name: &str| datum.rows.column_by_name(name).unwrap();
    let length = column("sepal_length")
        .as_primitive::<Float32Type>()
        .value(datum.row);
    let width = column("sepal_width")
        .as_primitive::<Float32Type>()
        .value(datum.row);
    let species = column("species").as_string::<i32>().value(datum.row);
    let diameter = 2.0 * (reach + 3.0);
    let ring = SceneSymbolMark {
        interactive: Some(false),
        len: 1,
        x: center[0].into(),
        y: center[1].into(),
        size: (diameter * diameter).into(),
        fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
        stroke: ColorOrGradient::Color([0.2, 0.2, 0.2, 1.0]).into(),
        stroke_width: Some(1.5),
        ..Default::default()
    };
    let text = format!("{species}: sepal length {length:.2}, sepal width {width:.2}");
    vec![
        rules(plot, center).into(),
        ring.into(),
        readout(center, text).into(),
    ]
}

/// Rules across `plot` through `at`.
fn rules(plot: Rect, at: [f32; 2]) -> SceneRuleMark {
    let [x, y] = at;
    SceneRuleMark {
        interactive: Some(false),
        len: 2,
        x: vec![x, plot.x].into(),
        y: vec![plot.y, y].into(),
        x2: vec![x, plot.x + plot.width].into(),
        y2: vec![plot.y + plot.height, y].into(),
        stroke: ColorOrGradient::Color([0.4, 0.4, 0.4, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

/// A label with `text` beside `at`.
fn readout(at: [f32; 2], text: String) -> SceneTextMark {
    SceneTextMark {
        interactive: Some(false),
        text: text.into(),
        x: (at[0] + 8.0).into(),
        y: (at[1] - 8.0).into(),
        font_size: 12.0.into(),
        color: [0.2, 0.2, 0.2, 1.0].into(),
        ..Default::default()
    }
}

/// Tracks pointer changes and requests crosshair updates.
/// Ignoring repeated positions avoids scheduling a redundant render before a wheel
/// event, which the host could then drop while that render is pending.
struct FollowPointer;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for FollowPointer {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let position = event.position();
        if state.pointer == position {
            return Rebuild::none();
        }
        state.pointer = position;
        Rebuild::scene(CROSSHAIR)
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
    // The y axis's top label hangs above the plot by as much as half its height, by an amount
    // that changes as a pan or zoom moves the ticks. It draws into the canvas's padding and
    // reserves no space, so the plot's top edge stays put.
    plot.slab(Side::Right, Slab::new().min_width(12.0))
        .slab(Side::Right, Slab::new().id(ChartId::Legend))
        .overflow(Side::Top, Overflow::Flush)
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

/// Draws the chart and a crosshair that follows the pointer and current chart geometry.
struct IrisChart;

#[async_trait::async_trait]
impl AppBuild<ChartState> for IrisChart {
    /// Declare chart and crosshair scenes, with pointer tracking on the crosshair.
    fn scenes(&self) -> Vec<Scene<ChartState>> {
        let follow = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        vec![
            Scene::new(CHART, 0),
            Scene::new(CROSSHAIR, 1).stream(follow, FollowPointer),
        ]
    }

    async fn build(
        &self,
        state: &mut ChartState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(CHART) {
            let scene = make_scene_graph(state, scenes.size());
            let mut data = SceneData::new();
            data.insert(FLOWERS, state.flowers.clone());
            scenes.set(CHART, BuiltScene { scene, data })?;
        }
        if scenes.requested(CROSSHAIR) || scenes.changed(CHART) {
            let marks = crosshair(state, scenes);
            if marks.is_empty() {
                return scenes.clear(CROSSHAIR);
            }
            let [width, height] = scenes.size();
            let scene = SceneGraph {
                marks,
                width,
                height,
                origin: [0.0; 2],
            };
            scenes.set(CROSSHAIR, scene.into())?;
        }
        Ok(())
    }
}

/// Lay out the chart to fill a window of `size`.
fn make_scene_graph(chart_state: &mut ChartState, size: [f32; 2]) -> SceneGraph {
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
        chart_state.x_scale.numeric_interval_domain().unwrap(),
        (0.0, 1.0),
        "Sepal Length",
        GuideSide::Bottom,
    );
    let y_axis = axis(
        chart_state.y_scale.numeric_interval_domain().unwrap(),
        (1.0, 0.0),
        "Sepal Width",
        GuideSide::Left,
    );

    // Lay the chart out in the window, measuring the axes and the legend. The domains change
    // the axis labels, so every scene measures again.
    let [width, height] = size;
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

    // Fit the scales to the plot area, and adjust the base plot's points to them
    let x_scale = (chart_state.x_scale.clone()).with_range_interval((0.0, f64::from(plot.width)));
    let y_scale = (chart_state.y_scale.clone()).with_range_interval((f64::from(plot.height), 0.0));
    let x_adjustment = chart_state.base_x_scale.adjust(&x_scale).unwrap();
    let y_adjustment = chart_state.base_y_scale.adjust(&y_scale).unwrap();
    chart_state.x_scale = x_scale;
    chart_state.y_scale = y_scale;

    // Make symbol mark
    let shape = SymbolShape::from_vega_str("circle").unwrap();

    let points = SceneSymbolMark {
        name: FLOWERS.into(),
        len: chart_state.flowers.num_rows() as u32,
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

/// Configure chart and crosshair scenes with scale navigation and text selection.
fn builder() -> AppBuilder<ChartState> {
    let state = ChartState::new();
    let text_engine = state.text_engine.clone();
    AvengerApp::builder(state, IrisChart, text_engine)
        .size([640.0, 440.0])
        .tool(PanZoom::xy(
            CHART,
            |s: &ChartState| s.plot,
            |s: &mut ChartState| &mut s.x_scale,
            |s| &mut s.y_scale,
        ))
        .tool(TextSelection::new(SELECTION, 10))
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

    let avenger_app = builder()
        .build()
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

#[cfg(test)]
mod tests;
