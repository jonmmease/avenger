//! Three histograms of the same movies, by IMDB rating, Rotten Tomatoes rating, and release
//! year, that filter each other.
//!
//! A drag in a chart selects a range of its column, a drag inside the range or on its edge moves
//! or resizes it, and a click clears it. Each chart draws every movie in gray, and over them the
//! movies inside the ranges of the other charts, so a chart's own range doesn't hide the movies
//! that it brushes. The heading counts the movies inside every range.
//!
//! One `KeyedIntervalBrush` covers the three charts, keyed by column, and the app keeps one range
//! for each chart.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, Float64Array, RecordBatch};
use arrow::csv::ReaderBuilder;
use arrow::datatypes::{DataType, Field, Float64Type, Schema};
use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::brushes::{Brushes, Interval, KeyedIntervalBrush};
use avenger_color::ColorOrGradient;
use avenger_common::types::TextBaseline;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::stream::Rebuild;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart, AxisStyle};
use avenger_guides::GuideSide;
use avenger_layout::{
    Edges, Gap, Layout, Leaf, Measure, Measurement, Rect, Row, Side, Size, Slab, SolveOptions,
    Track,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

/// The charts' scene, which fills the window.
const CHART: &str = "chart";
/// The brush's scene, over the charts.
const BRUSHES: &str = "brushes";

/// The bins of each histogram.
const BINS: usize = 20;
/// The space above the charts, for the heading.
const HEADING: f32 = 56.0;

/// Each chart's column, axis title, domain, which its bins divide evenly, and the angle of its
/// tick labels, in degrees clockwise. The years turn, so their four digits don't crowd.
const COLUMNS: [(&str, &str, (f64, f64), f32); 3] = [
    ("imdb_rating", "IMDB rating", (0.0, 10.0), 0.0),
    (
        "rotten_tomatoes_rating",
        "Rotten Tomatoes rating",
        (0.0, 100.0),
        0.0,
    ),
    ("year", "Release year", (1910.0, 2010.0), 45.0),
];

/// A histogram of one column of the movies.
struct Chart {
    column: &'static str,
    axis: Axis,
    domain: (f64, f64),
    /// Each movie's value in the column.
    values: Vec<f64>,
    /// How many movies fall in each bin.
    counts: Vec<u32>,
    /// The plot area from the last layout, where the brush draws.
    plot: Rect,
}

impl Chart {
    /// The bin of `value`. The domain's top value falls in the last bin.
    fn bin(&self, value: f64) -> usize {
        let (low, high) = self.domain;
        let bin = ((value - low) / (high - low) * BINS as f64).floor();
        (bin.max(0.0) as usize).min(BINS - 1)
    }
}

struct CrossState {
    charts: Vec<Chart>,
    /// Each chart's range, by column.
    filters: HashMap<&'static str, Interval>,
    text_engine: LabelEngine,
}

impl CrossState {
    fn new() -> Self {
        let text_engine = avenger_typst_label::bundled_label_engine()
            .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
        let charts = (COLUMNS.iter().zip(columns()))
            .map(|(&(column, title, domain, label_angle), values)| {
                let config = AxisConfig {
                    side: GuideSide::Bottom,
                    tick_format: "d".into(),
                    tick_label: None,
                    style: AxisStyle {
                        label_angle,
                        ..Default::default()
                    },
                };
                // Each call sets the scale's range from the plot's width.
                let scale = LinearScale::configured(domain, (0.0, 1.0));
                let axis = Axis::continuous(&scale, title, &config, &text_engine).unwrap();
                let mut chart = Chart {
                    column,
                    axis,
                    domain,
                    values,
                    counts: vec![0; BINS],
                    plot: Rect::default(),
                };
                for &value in &chart.values {
                    let bin = chart.bin(value);
                    chart.counts[bin] += 1;
                }
                chart
            })
            .collect();
        Self {
            charts,
            filters: HashMap::new(),
            text_engine,
        }
    }

    /// The chart of `column`.
    fn chart(&self, column: &str) -> &Chart {
        (self.charts.iter())
            .find(|chart| chart.column == column)
            .expect("each area's key names a chart")
    }

    /// Whether the movie at `row` lies inside the range of every chart but `except`.
    fn passes(&self, row: usize, except: Option<usize>) -> bool {
        (self.charts.iter().enumerate()).all(|(index, chart)| {
            let range = (Some(index) != except).then(|| self.filters.get(chart.column));
            range
                .flatten()
                .is_none_or(|range| range.contains([chart.values[row], 0.0]))
        })
    }

    /// How many movies fall in each bin of chart `index` and inside the ranges of the other
    /// charts.
    fn filtered_counts(&self, index: usize) -> Vec<u32> {
        let chart = &self.charts[index];
        let mut counts = vec![0; BINS];
        for (row, &value) in chart.values.iter().enumerate() {
            if self.passes(row, Some(index)) {
                counts[chart.bin(value)] += 1;
            }
        }
        counts
    }

    /// How many movies lie inside every range.
    fn selected(&self) -> usize {
        let movies = self.charts[0].values.len();
        (0..movies).filter(|&row| self.passes(row, None)).count()
    }
}

/// Each column of `COLUMNS` for every movie, from the data of the movie-search example.
fn columns() -> Vec<Vec<f64>> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("title", DataType::Utf8, false),
        Field::new("imdb_rating", DataType::Float64, false),
        Field::new("rotten_tomatoes_rating", DataType::Float64, false),
        Field::new("genre", DataType::Utf8, true),
        Field::new("year", DataType::Float64, false),
    ]));
    let csv = Cursor::new(include_str!("../../movie-search/data/movies.csv"));
    let reader = (ReaderBuilder::new(schema).with_header(true))
        .build(csv)
        .unwrap();
    let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().unwrap();
    (COLUMNS.iter())
        .map(|(column, ..)| {
            (batches.iter())
                .flat_map(|batch| {
                    let values = batch.column_by_name(column).unwrap();
                    values.as_primitive::<Float64Type>().values().to_vec()
                })
                .collect()
        })
        .collect()
}

/// The parts of the page that the layout places: each chart's plot area and its axis parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Plot(usize),
    Axis(usize, AxisPart),
}

/// The charts in a row, each with its axis below it.
fn page_layout() -> Layout<Id> {
    let charts = (0..COLUMNS.len()).map(|index| {
        let mut chart = Leaf::new()
            .id(Id::Plot(index))
            .min_size(Size::new(160.0, 120.0));
        for part in [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title] {
            chart = chart.slab(Side::Bottom, Slab::new().id(Id::Axis(index, part)));
        }
        Layout::from(chart)
    });
    Row::new(charts)
        .tracks(Track::fr(1.0))
        .gap(Gap::new(28.0))
        .into()
}

/// Measures the charts' axis parts.
struct Axes<'a>(&'a [Chart]);

impl Measure<Id> for Axes<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let Id::Axis(index, part) = id else {
            return None;
        };
        let extent = self.0[*index].axis.extent(*part, width);
        Some(Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang))
    }
}

/// Chart `index`'s bars in its plot area: every movie in gray, and over them the movies inside
/// the other charts' ranges.
fn histogram(state: &CrossState, index: usize) -> SceneMark {
    let chart = &state.charts[index];
    let plot = chart.plot;
    let (low, high) = chart.domain;
    let edges: ArrayRef = Arc::new(Float64Array::from_iter_values(
        (0..=BINS).map(|bin| low + (high - low) * bin as f64 / BINS as f64),
    ));
    let scale = chart.axis.scale(plot.width);
    let edges = scale
        .scale_to_numeric(&edges)
        .unwrap()
        .as_vec(BINS + 1, None);
    let tallest = chart.counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    // Each bar leaves a pixel on either side, so neighboring bars stay apart.
    let bars = |name: &str, counts: &[u32], color: [f32; 4]| -> SceneMark {
        let heights: Vec<f32> = (counts.iter())
            .map(|&count| plot.height * count as f32 / tallest)
            .collect();
        let widths = edges
            .windows(2)
            .map(|pair| (pair[1] - pair[0] - 2.0).max(0.0));
        SceneRectMark {
            name: name.to_string(),
            len: BINS as u32,
            x: ScalarOrArray::new_array(edges[..BINS].iter().map(|x| x + 1.0).collect()),
            y: ScalarOrArray::new_array(heights.iter().map(|h| plot.height - h).collect()),
            width: Some(ScalarOrArray::new_array(widths.collect())),
            height: Some(ScalarOrArray::new_array(heights)),
            fill: ColorOrGradient::Color(color).into(),
            ..Default::default()
        }
        .into()
    };
    let filtered = state.filtered_counts(index);
    SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            bars("all movies", &chart.counts, [0.84, 0.86, 0.89, 1.0]),
            bars("filtered movies", &filtered, [0.16, 0.42, 0.85, 1.0]),
        ],
        ..Default::default()
    }
    .into()
}

/// The heading: how many movies lie inside every range, and how to select one.
fn heading(state: &CrossState) -> SceneMark {
    let movies = state.charts[0].values.len();
    let text = format!(
        "{} of {movies} movies\nDrag in a chart to filter the others to a range, and click to clear it.",
        state.selected()
    );
    SceneTextMark {
        text: text.into(),
        x: 16.0.into(),
        y: 16.0.into(),
        baseline: TextBaseline::Top.into(),
        font_size: 13.0.into(),
        color: [0.2, 0.2, 0.2, 1.0].into(),
        ..Default::default()
    }
    .into()
}

/// Lay out the charts to fill a window of `size`.
fn make_scene_graph(state: &mut CrossState, size: [f32; 2]) -> SceneGraph {
    let [width, height] = size;
    let options = SolveOptions {
        padding: Edges {
            top: HEADING,
            ..Edges::all(16.0)
        },
        ..SolveOptions::sized(width, height)
    };
    let solution = page_layout()
        .measure(&options, &mut Axes(&state.charts))
        .unwrap()
        .into_solution();
    let mut marks = vec![heading(state)];
    for region in solution.regions() {
        let rect = region.content;
        match region.id {
            Some(Id::Plot(index)) => {
                state.charts[index].plot = rect;
                marks.push(histogram(state, index));
            }
            Some(Id::Axis(index, part)) => {
                let axis = &state.charts[index].axis;
                if let Some(mut group) = axis.part(part, rect.width) {
                    group.origin = axis.origin([rect.x, rect.y, rect.width, rect.height]);
                    marks.push(group.into());
                }
            }
            None => {}
        }
    }
    SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    }
}

/// One brush over the three charts, keyed by column, with one range for each chart. A drag
/// redraws the charts without their tree, and settling builds it.
fn brushes() -> Brushes<CrossState> {
    Brushes::new(BRUSHES, 10).with(
        KeyedIntervalBrush::x("ranges")
            .areas(|s: &CrossState| (s.charts.iter()).map(|c| (c.column, c.plot)).collect())
            .x_scale(|s, column| {
                let chart = s.chart(column);
                chart.axis.scale(chart.plot.width)
            })
            .value(|s, column| s.filters.get(column).copied())
            .on_change(|s, column, range| {
                match range {
                    Some(range) => s.filters.insert(*column, range),
                    None => s.filters.remove(column),
                };
                Rebuild::scene(CHART)
            })
            .on_settle(|_, _, _| Rebuild::tree(CHART)),
    )
}

/// Lays the charts out at the window's size.
struct Charts;

#[async_trait::async_trait]
impl AppBuild<CrossState> for Charts {
    fn scenes(&self) -> Vec<Scene<CrossState>> {
        vec![Scene::new(CHART, 0)]
    }

    async fn build(
        &self,
        state: &mut CrossState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        let scene = make_scene_graph(state, scenes.size());
        scenes.set(CHART, scene.into())
    }
}

fn builder() -> AppBuilder<CrossState> {
    let state = CrossState::new();
    let text_engine = state.text_engine.clone();
    AvengerApp::builder(state, Charts, text_engine)
        .size([960.0, 420.0])
        .tool(brushes())
}

fn main() {
    let avenger_app = pollster::block_on(builder().build()).expect("Failed to create AvengerApp");
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_common::time::Instant;
    use avenger_eventstream::window::{
        ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
    };

    fn moved(position: [f32; 2]) -> WindowEvent {
        WindowEvent::CursorMoved(WindowCursorMoved { position })
    }

    fn mouse(state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput(WindowMouseInput {
            state,
            button: MouseButton::Left,
        })
    }

    fn update(app: &mut AvengerApp<CrossState>, events: impl IntoIterator<Item = WindowEvent>) {
        for event in events {
            pollster::block_on(app.update(&event, Instant::now()));
        }
    }

    /// A range in the IMDB chart filters the other charts and leaves its own chart whole, and a
    /// click clears it.
    #[test]
    fn a_range_filters_the_other_charts() {
        let mut app = pollster::block_on(builder().build()).unwrap();
        let movies = app.state().charts[0].values.len();
        let plot = app.state().charts[0].plot;
        let at = |fraction: f32| [plot.x + plot.width * fraction, plot.y + plot.height / 2.0];
        let (pressed, released) = (mouse(ElementState::Pressed), mouse(ElementState::Released));
        update(
            &mut app,
            [
                moved(at(0.7)),
                pressed.clone(),
                moved(at(0.9)),
                released.clone(),
            ],
        );
        let state = app.state();
        let [low, high] = state.filters["imdb_rating"].x.unwrap();
        assert!((low - 7.0).abs() < 1e-3 && (high - 9.0).abs() < 1e-3);
        let selected = state.selected();
        assert!(0 < selected && selected < movies);
        assert_eq!(state.filtered_counts(0), state.charts[0].counts);
        let rotten: u32 = state.filtered_counts(1).iter().sum();
        assert_eq!(rotten as usize, selected);

        update(&mut app, [moved(at(0.3)), pressed, released]);
        assert!(app.state().filters.is_empty());
        assert_eq!(app.state().selected(), movies);
    }
}
