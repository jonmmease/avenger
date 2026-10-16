//! A page of three plots: its layout, the measurements of its axes and legend, and its scene.
//!
//! Each plot is a leaf with its axes and legend on slabs. The x axis is measured through
//! [`Measure::height`] at the plot's width, and the y axis through [`Measure::width`] at the
//! plot's height, so one call to [`Layout::measure`] settles the page. Each axis's answer gives
//! how far its end labels reach past the plot as its overhang, which the gaps and the canvas
//! padding beside the plot hold.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::marks::TextGeometryUtils;
use avenger_guides::axis::continuous::make_continuous_axis_marks;
use avenger_guides::axis::opts::AxisOrientation::{Bottom, Left};
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_layout::{
    Edges, Gap, Grid, GridSlot, Layout, Leaf, Measure, Measured, Measurement, Rect, Side, Size,
    Slab, Solution, SolveOptions, Track,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

pub const CANVAS: Size = Size::new(900.0, 620.0);
/// The smallest plot, which also sizes the axes' other direction when they are measured.
const ESTIMATE: Size = Size::new(320.0, 220.0);
/// The space between a plot's right edge and its legend.
const LEGEND_GAP: f32 = 4.0;
/// The canvas padding around the page.
const MARGIN: f32 = 14.0;

/// The page's ids: one per plot and per measured slab, each with its plot's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartId {
    Plot(usize),
    YAxis(usize),
    XAxis(usize),
    Legend(usize),
}

/// One plot: a title per axis, the data domains, and points.
pub struct PlotSpec {
    x_title: &'static str,
    y_title: &'static str,
    x_domain: (f64, f64),
    y_domain: (f64, f64),
    /// x, y and category.
    points: Vec<(f64, f64, usize)>,
    pub legend: bool,
}

const CATEGORY_COLORS: [[f32; 4]; 3] = [
    [0.27, 0.51, 0.71, 1.0],
    [0.89, 0.47, 0.20, 1.0],
    [0.33, 0.66, 0.41, 1.0],
];
const CATEGORY_NAMES: [&str; 3] = ["alpha", "beta", "gamma"];

pub fn specs() -> Vec<PlotSpec> {
    vec![
        PlotSpec {
            x_title: "week",
            y_title: "revenue",
            x_domain: (0.0, 12.0),
            y_domain: (0.0, 100.0),
            points: vec![
                (1.0, 22.0, 0),
                (2.0, 31.0, 0),
                (4.0, 38.0, 1),
                (5.5, 45.0, 1),
                (7.0, 52.0, 2),
                (8.5, 61.0, 0),
                (10.0, 70.0, 1),
                (11.0, 84.0, 2),
            ],
            legend: false,
        },
        PlotSpec {
            x_title: "week",
            y_title: "rate",
            x_domain: (0.0, 12.0),
            y_domain: (0.0, 8.0),
            points: vec![
                (1.0, 2.2, 0),
                (2.5, 3.1, 1),
                (4.0, 2.8, 2),
                (6.0, 4.5, 0),
                (7.5, 5.2, 1),
                (9.0, 4.1, 2),
                (10.5, 6.4, 0),
                (11.5, 7.0, 1),
            ],
            legend: true,
        },
        PlotSpec {
            x_title: "hour",
            y_title: "p99 latency (µs)",
            // Labels up to 12,000 make this plot's y axis the widest, so it sets the room
            // left of the first column.
            x_domain: (0.0, 24.0),
            y_domain: (0.0, 12_000.0),
            points: vec![
                (1.0, 1_800.0, 0),
                (3.0, 2_400.0, 1),
                (6.0, 4_900.0, 2),
                (9.0, 8_200.0, 0),
                (12.0, 11_400.0, 1),
                (15.0, 9_800.0, 2),
                (18.0, 6_300.0, 0),
                (21.0, 3_500.0, 1),
                (23.0, 2_100.0, 2),
            ],
            legend: false,
        },
    ]
}

/// The bundled fonts, with D3 number formatting for the axes.
pub fn text_engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// The page: two matched columns, with the third plot spanning both.
fn layout(specs: &[PlotSpec]) -> Layout<ChartId> {
    let plot = |index: usize| {
        let measured = |id| Slab::new().id(id);
        let mut plot = Leaf::new()
            .min_size(ESTIMATE)
            .id(ChartId::Plot(index))
            .slab(Side::Left, measured(ChartId::YAxis(index)))
            .slab(Side::Bottom, measured(ChartId::XAxis(index)));
        if specs[index].legend {
            plot = plot.slab(Side::Right, measured(ChartId::Legend(index)));
        }
        plot
    };
    let column = Track::fr(1.0).tag("column");
    Grid::new([column.clone(), column], [Track::fr(1.0), Track::fr(1.0)])
        .match_tag("column")
        .column_gap(Gap::new(28.0))
        .row_gap(Gap::new(28.0))
        .child(GridSlot::new(0, 0), plot(0))
        .child(GridSlot::new(1, 0), plot(1))
        .child(GridSlot::new(0, 1).span(2, 1), plot(2))
        .into()
}

/// Measures each plot's axes and legend.
struct ChartMeasure<'a> {
    specs: &'a [PlotSpec],
    engine: &'a LabelEngine,
}

impl ChartMeasure<'_> {
    /// How far one of the plot's axes reaches past a plot of `size`: across its side, which its
    /// slab holds, and past the plot's ends, which is its overhang.
    fn overflow(&self, index: usize, orientation: AxisOrientation, size: Size) -> Edges {
        let spec = &self.specs[index];
        let axis = build_axis(spec, orientation, size, [0.0, 0.0], false, self.engine);
        measure_overflow(&[axis], size, self.engine)
    }
}

impl Measure<ChartId> for ChartMeasure<'_> {
    fn height(&mut self, id: &ChartId, width: f32) -> Option<Measurement> {
        match *id {
            // The x axis's labels and title at the plot's width, and its first and last labels
            // past the plot's left and right edges.
            ChartId::XAxis(index) => {
                let size = Size::new(width, ESTIMATE.height);
                let edges = self.overflow(index, Bottom, size);
                Some(Measurement::from(edges.bottom).with_overhang([edges.left, edges.right]))
            }
            _ => None,
        }
    }

    fn width(&mut self, id: &ChartId, height: f32) -> Option<Measurement> {
        match *id {
            // The y axis's labels and title at the plot's height, and its top and bottom labels
            // past the plot's top and bottom edges.
            ChartId::YAxis(index) => {
                let size = Size::new(ESTIMATE.width, height);
                let edges = self.overflow(index, Left, size);
                Some(Measurement::from(edges.left).with_overhang([edges.top, edges.bottom]))
            }
            ChartId::Legend(_) => {
                let width = build_legend(self.engine).bounding_box(self.engine).upper()[0];
                Some(Measurement::always(width.ceil()))
            }
            _ => None,
        }
    }
}

/// Lays out the page on its canvas, measuring the axes and legend.
pub fn solve(specs: &[PlotSpec], engine: &LabelEngine, explain: bool) -> Measured<ChartId> {
    let options = SolveOptions {
        padding: Edges::all(MARGIN),
        explain,
        ..SolveOptions::sized(CANVAS.width, CANVAS.height)
    };
    layout(specs)
        .measure(&options, &mut ChartMeasure { specs, engine })
        .expect("the chart page lays out")
}

/// The page's scene: each plot's axes and points in its solved rectangle, and the legend in
/// its slab.
pub fn scene(specs: &[PlotSpec], solution: &Solution<ChartId>, engine: &LabelEngine) -> SceneGraph {
    let size = solution.size();
    let mut marks: Vec<SceneMark> = vec![SceneMark::Rect(SceneRectMark {
        width: Some(size.width.into()),
        height: Some(size.height.into()),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
        ..Default::default()
    })];
    for (index, spec) in specs.iter().enumerate() {
        let content = plot_rect(solution, index);
        let size = Size::new(content.width, content.height);
        for axis in build_axes(spec, size, [content.x, content.y], true, engine) {
            marks.push(axis.into());
        }
        marks.push(build_points(spec, content));
        if let Some(slab) = solution.region(&ChartId::Legend(index)) {
            let mut legend = build_legend(engine);
            legend.origin = [slab.slot.x, slab.slot.y];
            marks.push(legend.into());
        }
    }
    SceneGraph {
        marks: vec![
            SceneGroup {
                marks,
                ..Default::default()
            }
            .into(),
        ],
        width: size.width,
        height: size.height,
        origin: [0.0, 0.0],
    }
}

pub fn plot_rect(solution: &Solution<ChartId>, index: usize) -> Rect {
    solution
        .region(&ChartId::Plot(index))
        .expect("every plot has a region")
        .content
}

/// Linear scales ranged to a plot of `size`, with y pointing down the screen.
fn plot_scales(spec: &PlotSpec, size: Size) -> (ConfiguredScale, ConfiguredScale) {
    let x = LinearScale::configured(spec.x_domain, (0.0, f64::from(size.width)));
    let y = LinearScale::configured(spec.y_domain, (f64::from(size.height), 0.0));
    (x, y)
}

/// The plot's axes at `size`, with their origin at `origin`.
pub fn build_axes(
    spec: &PlotSpec,
    size: Size,
    origin: [f32; 2],
    grid: bool,
    engine: &LabelEngine,
) -> Vec<SceneGroup> {
    [Bottom, Left]
        .map(|orientation| build_axis(spec, orientation, size, origin, grid, engine))
        .into()
}

/// The plot's x axis for `Bottom` or its y axis for `Left`, at `size`, with its origin at
/// `origin`.
fn build_axis(
    spec: &PlotSpec,
    orientation: AxisOrientation,
    size: Size,
    origin: [f32; 2],
    grid: bool,
    engine: &LabelEngine,
) -> SceneGroup {
    let (x_scale, y_scale) = plot_scales(spec, size);
    let (scale, title) = match orientation {
        Bottom => (x_scale, spec.x_title),
        _ => (y_scale, spec.y_title),
    };
    let config = AxisConfig {
        orientation,
        dimensions: [size.width, size.height],
        grid,
        tick_format: ",f".into(),
        tick_label: None,
        style: Default::default(),
    };
    make_continuous_axis_marks(&scale, title, origin, &config, engine).expect("the axis builds")
}

/// How far `groups` reach past a plot of `size` whose top-left corner is at their origin.
pub fn measure_overflow(groups: &[SceneGroup], size: Size, engine: &LabelEngine) -> Edges {
    let mut edges = Edges::all(0.0f32);
    for group in groups {
        let bounds = group.bounding_box(engine);
        edges.left = edges.left.max(-bounds.lower()[0]);
        edges.top = edges.top.max(-bounds.lower()[1]);
        edges.right = edges.right.max(bounds.upper()[0] - size.width);
        edges.bottom = edges.bottom.max(bounds.upper()[1] - size.height);
    }
    Edges::new(
        edges.left.max(0.0).ceil(),
        edges.top.max(0.0).ceil(),
        edges.right.max(0.0).ceil(),
        edges.bottom.max(0.0).ceil(),
    )
}

/// The legend, a gap past its group's origin.
pub fn build_legend(engine: &LabelEngine) -> SceneGroup {
    let mut legend = make_symbol_legend(
        &SymbolLegendConfig {
            title: Some("cohort".to_string()),
            text: ScalarOrArray::new_array(CATEGORY_NAMES.iter().map(|s| s.to_string()).collect()),
            fill: ScalarOrArray::new_array(
                CATEGORY_COLORS
                    .iter()
                    .map(|color| ColorOrGradient::Color(*color))
                    .collect(),
            ),
            ..Default::default()
        },
        engine,
    )
    .expect("legend")
    .group;
    legend.origin = [LEGEND_GAP, 0.0];
    SceneGroup {
        marks: vec![legend.into()],
        ..Default::default()
    }
}

/// The plot's points in its solved rectangle.
fn build_points(spec: &PlotSpec, content: Rect) -> SceneMark {
    let (x_scale, y_scale) = plot_scales(spec, Size::new(content.width, content.height));
    let scaled = |scale: &ConfiguredScale, value: f64, offset: f32| {
        offset
            + scale
                .scale_scalar(&value)
                .expect("a scaled value")
                .as_f64()
                .expect("a number") as f32
    };
    let xs = spec
        .points
        .iter()
        .map(|p| scaled(&x_scale, p.0, content.x))
        .collect();
    let ys = spec
        .points
        .iter()
        .map(|p| scaled(&y_scale, p.1, content.y))
        .collect();
    let fills = spec
        .points
        .iter()
        .map(|p| ColorOrGradient::Color(CATEGORY_COLORS[p.2]))
        .collect();
    SceneMark::Symbol(SceneSymbolMark {
        len: spec.points.len() as u32,
        x: ScalarOrArray::new_array(xs),
        y: ScalarOrArray::new_array(ys),
        fill: ScalarOrArray::new_array(fills),
        size: 48.0.into(),
        stroke_width: Some(0.0),
        shapes: vec![SymbolShape::Circle],
        shape_index: 0.into(),
        ..Default::default()
    })
}
