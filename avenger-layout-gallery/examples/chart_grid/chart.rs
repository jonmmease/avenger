//! A page of three plots: its layout, the measurements of its axes and legend, and its scene.
//!
//! Each plot is a leaf with its axes and legend on slabs. Each axis is prepared once as an
//! [`Axis`], and its ticks, labels and title each have a slab. The x axis's slabs are measured
//! through [`Measure::height`] at the plot's width, and the y axis's through [`Measure::width`]
//! at the plot's height, so one call to [`Layout::measure`] settles the page. Each part's answer
//! gives how far it reaches past the plot's ends as its overhang, which the gaps and the canvas
//! padding beside the plot hold.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::GuideSide;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::legend::{Legend, LegendConfig, LegendSwatches, SymbolSwatches};
use avenger_layout::{
    Edges, Gap, Grid, GridSlot, Layout, Leaf, Measure, Measured, Measurement, Rect, RegionKind,
    Side, Size, Slab, Solution, SolveOptions, Track,
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
/// The smallest plot.
const MIN_PLOT: Size = Size::new(320.0, 220.0);
/// The space between a plot's right edge and its legend.
const LEGEND_GAP: f32 = 4.0;
/// The canvas padding around the page.
const MARGIN: f32 = 14.0;
/// An axis's parts, from the plot outward.
pub const PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The page's ids: one per plot, per axis part and per legend, each with its plot's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartId {
    Plot(usize),
    YAxis(usize, AxisPart),
    XAxis(usize, AxisPart),
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

/// The page's plots, each with its axes prepared once, and the legend.
pub struct Chart {
    pub specs: Vec<PlotSpec>,
    /// Each plot's x and y axes.
    pub axes: Vec<[Axis; 2]>,
    pub legend: Legend,
    pub engine: LabelEngine,
}

impl Chart {
    pub fn new() -> Self {
        let engine = text_engine();
        let specs = specs();
        let axes = specs
            .iter()
            .map(|spec| {
                let axis = |scale: ConfiguredScale, title: &str, side| {
                    let config = AxisConfig {
                        side,
                        tick_format: ",f".into(),
                        tick_label: None,
                        style: Default::default(),
                    };
                    Axis::continuous(&scale, title, &config, &engine).expect("the axis builds")
                };
                // The y axis's range runs up the screen. Each call sets the range's length.
                [
                    axis(
                        LinearScale::configured(spec.x_domain, (0.0, 1.0)),
                        spec.x_title,
                        GuideSide::Bottom,
                    ),
                    axis(
                        LinearScale::configured(spec.y_domain, (1.0, 0.0)),
                        spec.y_title,
                        GuideSide::Left,
                    ),
                ]
            })
            .collect();
        let legend = Legend::new(
            &LegendConfig {
                title: Some("cohort".to_string()),
                text: ScalarOrArray::new_array(
                    CATEGORY_NAMES.iter().map(|s| s.to_string()).collect(),
                ),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    fill: ScalarOrArray::new_array(
                        CATEGORY_COLORS
                            .iter()
                            .map(|color| ColorOrGradient::Color(*color))
                            .collect(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &engine,
        )
        .expect("the legend builds");
        Self {
            specs,
            axes,
            legend,
            engine,
        }
    }

    /// The page: two matched columns, with the third plot spanning both.
    fn layout(&self) -> Layout<ChartId> {
        let plot = |index: usize| {
            let mut plot = Leaf::new().min_size(MIN_PLOT).id(ChartId::Plot(index));
            for part in PARTS {
                plot = plot
                    .slab(Side::Left, Slab::new().id(ChartId::YAxis(index, part)))
                    .slab(Side::Bottom, Slab::new().id(ChartId::XAxis(index, part)));
            }
            if self.specs[index].legend {
                plot = plot
                    .slab(Side::Right, Slab::new().min_width(LEGEND_GAP))
                    .slab(Side::Right, Slab::new().id(ChartId::Legend(index)));
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

    /// Lays out the page on its canvas, measuring the axes and legend.
    pub fn solve(&self, explain: bool) -> Measured<ChartId> {
        let options = SolveOptions {
            padding: Edges::all(MARGIN),
            explain,
            ..SolveOptions::sized(CANVAS.width, CANVAS.height)
        };
        self.layout()
            .measure(&options, &mut ChartMeasure { chart: self })
            .expect("the chart page lays out")
    }

    /// The page's scene: each plot's grid, points and axis parts, and the legend in its slab.
    pub fn scene(&self, solution: &Solution<ChartId>) -> SceneGraph {
        let size = solution.size();
        let mut marks: Vec<SceneMark> = vec![SceneMark::Rect(SceneRectMark {
            width: Some(size.width.into()),
            height: Some(size.height.into()),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
            ..Default::default()
        })];
        for (index, (spec, [x, y])) in self.specs.iter().zip(&self.axes).enumerate() {
            let content = plot_rect(solution, index);
            let plot = [content.width, content.height];
            // Behind the data: grid lines in the plot's frame.
            for axis in [x, y] {
                let mut grid = axis.grid(plot);
                grid.origin = [content.x, content.y];
                marks.push(grid.into());
            }
            // The data, positioned by the axes' own scales at the plot's size.
            marks.push(build_points(
                spec,
                &x.scale(content.width),
                &y.scale(content.height),
                content,
            ));
            // Each part that the layout draws, at its slab's length along the axis.
            for part in PARTS {
                for (axis, id) in [
                    (x, ChartId::XAxis(index, part)),
                    (y, ChartId::YAxis(index, part)),
                ] {
                    if let Some(group) = drawn_part(solution, axis, id, part) {
                        marks.push(group.into());
                    }
                }
            }
            if let Some(slab) = solution.region(&ChartId::Legend(index)) {
                let mut legend = self.legend.marks(slab.content.height).group;
                legend.origin = self.legend.origin(slot_of(slab.content));
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
}

impl Default for Chart {
    fn default() -> Self {
        Self::new()
    }
}

/// Measures each plot's axis parts and legend.
struct ChartMeasure<'a> {
    chart: &'a Chart,
}

/// The layout's answer for one part of `axis` at `length`.
fn answer(axis: &Axis, part: AxisPart, length: f32) -> Measurement {
    let extent = axis.extent(part, length);
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<ChartId> for ChartMeasure<'_> {
    /// An x axis's parts, at the plot's width.
    fn height(&mut self, id: &ChartId, width: f32) -> Option<Measurement> {
        match *id {
            ChartId::XAxis(index, part) => Some(answer(&self.chart.axes[index][0], part, width)),
            _ => None,
        }
    }

    /// A y axis's parts, at the plot's height, and the legend.
    fn width(&mut self, id: &ChartId, height: f32) -> Option<Measurement> {
        match *id {
            ChartId::YAxis(index, part) => Some(answer(&self.chart.axes[index][1], part, height)),
            ChartId::Legend(_) => {
                let extent = self.chart.legend.extent(height);
                Some(
                    Measurement::over(extent.thickness, extent.holds)
                        .with_overhang(extent.overhang),
                )
            }
            _ => None,
        }
    }
}

/// A rectangle as a guide takes it, `[x, y, width, height]`.
pub fn slot_of(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

/// The part of `axis` in the slab of `id` at its content's length along the axis, placed in that
/// content, when the slab is its own representative: not collapsed into or merged with another.
pub fn drawn_part(
    solution: &Solution<ChartId>,
    axis: &Axis,
    id: ChartId,
    part: AxisPart,
) -> Option<SceneGroup> {
    let region = solution.region(&id)?;
    if !matches!(
        region.kind,
        RegionKind::Slab {
            represented_by: None,
            ..
        }
    ) {
        return None;
    }
    let content = region.content;
    let length = match id {
        ChartId::XAxis(..) => content.width,
        _ => content.height,
    };
    let mut group = axis.part(part, length)?;
    group.origin = axis.origin(slot_of(content));
    Some(group)
}

pub fn plot_rect(solution: &Solution<ChartId>, index: usize) -> Rect {
    solution
        .region(&ChartId::Plot(index))
        .expect("every plot has a region")
        .content
}

/// The plot's points in its solved rectangle, positioned by the axes' scales.
fn build_points(
    spec: &PlotSpec,
    x_scale: &ConfiguredScale,
    y_scale: &ConfiguredScale,
    content: Rect,
) -> SceneMark {
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
        .map(|p| scaled(x_scale, p.0, content.x))
        .collect();
    let ys = spec
        .points
        .iter()
        .map(|p| scaled(y_scale, p.1, content.y))
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
