//! Twelve scatter plots in a `Wrap` that takes as many columns as fit the window and reflows
//! them when the window is resized.
//!
//! The panels share their x and y domains, so the `Wrap` collapses the axes to one y axis per
//! row and one x axis per column, on the column's lowest panel, and merges each axis title into
//! one for the figure. The axes are prepared once, and every resize lays the page out again.

use std::f64::consts::TAU;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array};
use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_common::time::Instant;
use avenger_common::types::{SymbolShape, TextAlign, TextBaseline};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Edges, Gap, Layout, Leaf, Measure, Measurement, Rect, RegionKind, Side, Size, Slab,
    SolveOptions, Wrap,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use rand_distr::Distribution;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DOMAIN: (f64, f64) = (0.0, 10.0);
const POINTS: usize = 300;
/// The height of each panel's header.
const HEADER: f32 = 20.0;

/// The parts of the page that the layout places: each panel's plot area and header, and each
/// panel's axis parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PanelId {
    Plot(usize),
    Header(usize),
    X(usize, AxisPart),
    Y(usize, AxisPart),
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The tag of an axis part: the Wrap collapses the ticks and labels, and merges the titles. The
/// panels share their domains, so the tag also serves as the part's content key: equal keys
/// draw the same part, and share its measurements.
fn tag(axis: &str, part: AxisPart) -> String {
    let part = match part {
        AxisPart::Ticks => "ticks",
        AxisPart::Labels => "labels",
        AxisPart::Title => "title",
    };
    format!("{axis}-{part}")
}

#[derive(Clone)]
struct FacetState {
    x_axis: Arc<Axis>,
    y_axis: Arc<Axis>,
    /// Each panel's points, in data coordinates.
    points: Vec<(ArrayRef, ArrayRef)>,
    /// The column count of the last layout, to report when it changes.
    columns: usize,
    text_engine: LabelEngine,
}

impl FacetState {
    fn new() -> Self {
        let text_engine = avenger_typst_label::bundled_label_engine()
            .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
        // Each call sets the scale's range from its length, in the direction that the range
        // has here: up the plot for the y axis.
        let axis = |range: (f64, f64), title: &str, side: GuideSide| {
            let config = AxisConfig {
                side,
                tick_format: ",f".into(),
                tick_label: None,
                style: Default::default(),
            };
            let scale = LinearScale::configured(DOMAIN, range);
            Arc::new(Axis::continuous(&scale, title, &config, &text_engine).unwrap())
        };
        let x_axis = axis((0.0, 1.0), "Temperature", GuideSide::Bottom);
        let y_axis = axis((1.0, 0.0), "Humidity", GuideSide::Left);

        // Each month's points scatter around a center that circles the plot over the year.
        let mut rng = rand::thread_rng();
        let normal = rand_distr::Normal::new(0.0, 1.0).unwrap();
        let points = (0..MONTHS.len())
            .map(|month| {
                let angle = month as f64 / MONTHS.len() as f64 * TAU;
                let center = [5.0 + 2.5 * angle.cos(), 5.0 + 2.5 * angle.sin()];
                let mut values = |center: f64| -> ArrayRef {
                    Arc::new(Float32Array::from_iter_values(
                        (0..POINTS).map(|_| (center + normal.sample(&mut rng)) as f32),
                    ))
                };
                (values(center[0]), values(center[1]))
            })
            .collect();

        Self {
            x_axis,
            y_axis,
            points,
            columns: 0,
            text_engine,
        }
    }
}

/// The page: each month's panel in a Wrap, with a header above its plot area and its axes'
/// parts in slabs below and to its left.
fn page_layout() -> Layout<PanelId> {
    let panels = (0..MONTHS.len()).map(|index| {
        let mut panel = Leaf::new()
            .id(PanelId::Plot(index))
            .min_size(Size::new(110.0, 80.0))
            .slab(
                Side::Top,
                Slab::new().id(PanelId::Header(index)).min_height(HEADER),
            );
        for part in AXIS_PARTS {
            panel = panel
                .slab(
                    Side::Left,
                    Slab::new()
                        .id(PanelId::Y(index, part))
                        .tag(tag("y", part))
                        .content_key(tag("y", part)),
                )
                .slab(
                    Side::Bottom,
                    Slab::new()
                        .id(PanelId::X(index, part))
                        .tag(tag("x", part))
                        .content_key(tag("x", part)),
                );
        }
        panel
    });
    // The clearance keeps neighboring panels' end labels apart.
    let mut wrap = Wrap::new(panels)
        .min_column(180.0)
        .gap(Gap::new(12.0).clearance(8.0));
    for axis in ["x", "y"] {
        wrap = wrap
            .collapse_tag(tag(axis, AxisPart::Ticks))
            .collapse_tag(tag(axis, AxisPart::Labels))
            .merge_tag(tag(axis, AxisPart::Title));
    }
    wrap.into()
}

/// Measures the axis parts. The panels' slabs share content keys, so the layout asks once for
/// each part at each length.
struct PanelGuides<'a> {
    x: &'a Axis,
    y: &'a Axis,
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<PanelId> for PanelGuides<'_> {
    fn height(&mut self, id: &PanelId, width: f32) -> Option<Measurement> {
        match id {
            PanelId::X(_, part) => Some(answer(self.x.extent(*part, width))),
            _ => None,
        }
    }

    fn width(&mut self, id: &PanelId, height: f32) -> Option<Measurement> {
        match id {
            PanelId::Y(_, part) => Some(answer(self.y.extent(*part, height))),
            _ => None,
        }
    }
}

fn slot(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

/// A panel's plot area: its background, grid lines and points, clipped to the area.
fn plot_marks(state: &FacetState, index: usize, plot: Rect) -> SceneMark {
    let size = [plot.width, plot.height];
    let (xs, ys) = &state.points[index];
    let x = state.x_axis.scale(plot.width).scale_to_numeric(xs).unwrap();
    let y = state
        .y_axis
        .scale(plot.height)
        .scale_to_numeric(ys)
        .unwrap();
    let points = SceneSymbolMark {
        len: POINTS as u32,
        x,
        y,
        shapes: vec![SymbolShape::Circle],
        fill: ColorOrGradient::Color([0.27, 0.51, 0.71, 0.45]).into(),
        size: 16.0.into(),
        stroke_width: 0.0f32.into(),
        ..Default::default()
    };
    let background = SceneRectMark {
        width: Some(plot.width.into()),
        height: Some(plot.height.into()),
        fill: ColorOrGradient::Color([0.96, 0.97, 0.98, 1.0]).into(),
        ..Default::default()
    };
    SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            background.into(),
            state.y_axis.grid(size).into(),
            state.x_axis.grid(size).into(),
            SceneGroup {
                marks: vec![points.into()],
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
    .into()
}

/// A panel's month, centered in its header.
fn header_mark(index: usize, header: Rect) -> SceneMark {
    SceneTextMark {
        text: MONTHS[index].to_string().into(),
        x: (header.x + header.width / 2.0).into(),
        y: (header.y + header.height / 2.0).into(),
        align: TextAlign::Center.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: 12.0.into(),
        color: [0.2, 0.2, 0.2, 1.0].into(),
        ..Default::default()
    }
    .into()
}

/// Lay out the chart to fill a window of `size`.
fn make_scene_graph(state: &mut FacetState, size: [f32; 2]) -> SceneGraph {
    let start = Instant::now();
    let [width, height] = size;
    let options = SolveOptions {
        padding: Edges::all(16.0),
        ..SolveOptions::sized(width, height)
    };
    let mut guides = PanelGuides {
        x: &state.x_axis,
        y: &state.y_axis,
    };
    let solution = page_layout()
        .measure(&options, &mut guides)
        .unwrap()
        .into_solution();

    let mut marks = Vec::new();
    for region in solution.regions() {
        let Some(id) = region.id else {
            continue;
        };
        let rect = region.content;
        match (id, &region.kind) {
            (PanelId::Plot(index), _) => marks.push(plot_marks(state, index, rect)),
            (PanelId::Header(index), _) => marks.push(header_mark(index, rect)),
            // A slab that another slab draws, in a collapsed run, draws nothing.
            (
                PanelId::X(_, part) | PanelId::Y(_, part),
                RegionKind::Slab {
                    represented_by: None,
                    ..
                },
            ) => {
                let (axis, length) = match id {
                    PanelId::X(..) => (&state.x_axis, rect.width),
                    _ => (&state.y_axis, rect.height),
                };
                if let Some(mut group) = axis.part(part, length) {
                    group.origin = axis.origin(slot(rect));
                    marks.push(group.into());
                }
            }
            _ => {}
        }
    }

    if let Some(RegionKind::Wrap { columns, .. }) = solution.at_path(&[]).map(|region| &region.kind)
    {
        if *columns != state.columns {
            state.columns = *columns;
            println!(
                "{columns} columns at {width} x {height}, laid out and drawn in {:?}",
                start.elapsed()
            );
        }
    }

    SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    }
}

/// The chart's scene, which fills the window.
const CHART: &str = "chart";

/// Lays the chart out at the window's size.
struct FacetChart;

#[async_trait::async_trait]
impl AppBuild<FacetState> for FacetChart {
    fn scenes(&self) -> Vec<Scene<FacetState>> {
        vec![Scene::new(CHART, 0)]
    }

    async fn build(
        &self,
        state: &mut FacetState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        let scene = make_scene_graph(state, scenes.size());
        scenes.set(CHART, scene.into())
    }
}

fn main() {
    let state = FacetState::new();
    let text_engine = state.text_engine.clone();
    let builder = AvengerApp::builder(state, FacetChart, text_engine).size([960.0, 640.0]);
    let avenger_app = pollster::block_on(builder.build()).expect("Failed to create AvengerApp");

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
