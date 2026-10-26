//! The movies, the search box that filters them by title, and the plot that highlights the
//! matches.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{AsArray, BooleanArray, RecordBatch};
use arrow::compute::{concat_batches, filter_record_batch, not};
use arrow::csv::ReaderBuilder;
use arrow::datatypes::{DataType, Field, Schema};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::text_selection::TextSelection;
use avenger_app_tools::tooltips::{InstanceTarget, TooltipField, Tooltips};
use avenger_color::ColorOrGradient;
use avenger_common::types::TextBaseline;
use avenger_eventstream::stream::Rebuild;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::data::SceneData;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Column, Direction, Edges, Gap, Layout, Leaf, Measure, Measurement, Overflow, Placement,
    Position, Rect, Side, Size, Slab, SolveOptions, Track,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{bundled_label_engine, LabelEngine};
use avenger_widgets::{TextInput, WidgetMeasure, Widgets};
use avenger_winit_wgpu::WinitWgpuAvengerApp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The plot, its axes and its heading.
const CHART: &str = "chart";
/// The search box, above the chart.
const CONTROLS: &str = "controls";
/// Text selection, above the controls so that the search box's label is selectable too.
const SELECTION: &str = "selection";
const TOOLTIPS: &str = "tooltips";

/// The search box's id.
const SEARCH: &str = "search";

/// The marks of the movies whose titles match the query and of the others, which name their
/// rows too.
const MATCHES: &str = "matches";
const OTHERS: &str = "others";

/// The movies' columns.
const TITLE: &str = "title";
const IMDB: &str = "imdb_rating";
const ROTTEN_TOMATOES: &str = "rotten_tomatoes_rating";
const GENRE: &str = "genre";
const YEAR: &str = "year";

const IMDB_DOMAIN: (f64, f64) = (0.0, 10.0);
const ROTTEN_TOMATOES_DOMAIN: (f64, f64) = (0.0, 100.0);

const ACCENT: [f32; 4] = [0.16, 0.42, 0.85, 1.0];

/// The movies, the query, which movies match it, and where the layout put the search box.
pub struct SearchState {
    /// Each movie's title, ratings, genre and year.
    movies: RecordBatch,
    /// Each movie's title in lowercase, to match queries against.
    titles: Vec<String>,
    query: String,
    /// Whether each movie's title contains the query, ignoring case.
    matches: BooleanArray,
    /// Each widget's rectangle from the latest layout, by id.
    controls: HashMap<String, Rect>,
    text_engine: LabelEngine,
}

impl SearchState {
    fn new() -> Self {
        let movies = movies();
        let titles = (movies
            .column_by_name(TITLE)
            .unwrap()
            .as_string::<i32>()
            .iter())
        .map(|title| title.unwrap_or_default().to_lowercase())
        .collect();
        let text_engine =
            bundled_label_engine().with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
        // The empty query matches every movie.
        let matches = BooleanArray::from(vec![true; movies.num_rows()]);
        Self {
            movies,
            titles,
            query: String::new(),
            matches,
            controls: HashMap::new(),
            text_engine,
        }
    }

    /// Store `query`, and match the movies whose titles contain it, ignoring case. An empty
    /// query matches every movie.
    fn search(&mut self, query: String) {
        let lowercase = query.to_lowercase();
        self.matches = (self.titles.iter())
            .map(|title| Some(title.contains(&lowercase)))
            .collect();
        self.query = query;
    }

    /// The heading above the plot, with the number of matches.
    fn heading(&self) -> String {
        let total = grouped(self.movies.num_rows());
        match self.matches.true_count() {
            _ if self.query.is_empty() => format!("{total} movies"),
            1 => format!("1 of {total} movies matches"),
            count => format!("{} of {total} movies match", grouped(count)),
        }
    }
}

/// The movies with both ratings, from `data/movies.csv`, which the binary embeds on every target.
fn movies() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new(TITLE, DataType::Utf8, false),
        Field::new(IMDB, DataType::Float64, false),
        Field::new(ROTTEN_TOMATOES, DataType::Float64, false),
        Field::new(GENRE, DataType::Utf8, true),
        Field::new(YEAR, DataType::Int64, false),
    ]));
    let csv = Cursor::new(include_str!("../data/movies.csv"));
    let reader = (ReaderBuilder::new(schema.clone()).with_header(true))
        .build(csv)
        .unwrap();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    concat_batches(&schema, &batches).unwrap()
}

/// `n` with a comma between each group of three digits.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// The parts of the window that the layout places.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Part {
    Plot,
    Heading,
    X(AxisPart),
    Y(AxisPart),
    /// A widget, by id.
    Widget(String),
}

const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// Lay out the search box above the plot, which has its heading above it and its axes below and
/// to the left.
fn layout() -> Layout<Part> {
    let mut plot = Leaf::new().id(Part::Plot).min_size(Size::new(240.0, 160.0));
    for part in AXIS_PARTS {
        plot = plot
            .slab(Side::Left, Slab::new().id(Part::Y(part)))
            .slab(Side::Bottom, Slab::new().id(Part::X(part)));
    }
    // The heading stays inside the plot's slot, below the gap under the search box.
    let plot = plot
        .slab(Side::Top, Slab::new().id(Part::Heading))
        .slab(Side::Right, Slab::new().min_width(12.0))
        .overflow(Side::Top, Overflow::Contain);
    // The search box takes its preferred width, or the window's if that is narrower.
    let search = Leaf::new()
        .id(Part::Widget(SEARCH.to_string()))
        .place(Direction::Horizontal, Placement::fit(Position::Start));
    Column::new([Layout::from(search), plot.into()])
        .track(0, Track::auto())
        .track(1, Track::fr(1.0))
        .gap(Gap::new(20.0))
        .into()
}

/// Measures the axis parts, the heading and the search box for the layout.
struct Parts<'a> {
    x: &'a Axis,
    y: &'a Axis,
    /// The heading's height, with the space below it.
    heading: f32,
    controls: WidgetMeasure<'a, SearchState>,
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<Part> for Parts<'_> {
    fn height(&mut self, id: &Part, width: f32) -> Option<Measurement> {
        match id {
            Part::X(part) => Some(answer(self.x.extent(*part, width))),
            Part::Heading => Some(Measurement::always(self.heading)),
            Part::Widget(name) => self.controls.height(name, width),
            _ => None,
        }
    }

    fn width(&mut self, id: &Part, height: f32) -> Option<Measurement> {
        match id {
            Part::Y(part) => Some(answer(self.y.extent(*part, height))),
            Part::Widget(name) => self.controls.width(name, height),
            _ => None,
        }
    }

    fn max_content(&mut self, id: &Part, direction: Direction) -> Option<f32> {
        match id {
            Part::Widget(name) => self.controls.max_content(name, direction),
            _ => None,
        }
    }
}

/// Draws the plot for the search box that the `Widgets` tool draws.
struct MovieChart {
    controls: Widgets<SearchState>,
}

#[async_trait]
impl AppBuild<SearchState> for MovieChart {
    fn scenes(&self) -> Vec<Scene<SearchState>> {
        vec![Scene::new(CHART, 0)]
    }

    async fn build(
        &self,
        state: &mut SearchState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(CHART) {
            let chart = chart(state, scenes.size(), &self.controls);
            scenes.set(CHART, chart)?;
        }
        Ok(())
    }
}

/// Lay out a window of `size`, store the search box's rectangle, and draw the plot with the
/// movies that match the query above the others.
fn chart(state: &mut SearchState, size: [f32; 2], controls: &Widgets<SearchState>) -> BuiltScene {
    let engine = &state.text_engine;
    let axis = |domain: (f64, f64), range: (f64, f64), title: &str, side: GuideSide| {
        let config = AxisConfig {
            side,
            // Vega's default axis format
            tick_format: ",f".into(),
            tick_label: None,
            style: Default::default(),
        };
        Axis::continuous(
            &LinearScale::configured(domain, range),
            title,
            &config,
            engine,
        )
        .unwrap()
    };
    let x_axis = axis(IMDB_DOMAIN, (0.0, 1.0), "IMDB Rating", GuideSide::Bottom);
    let y_axis = axis(
        ROTTEN_TOMATOES_DOMAIN,
        (1.0, 0.0),
        "Rotten Tomatoes Rating",
        GuideSide::Left,
    );
    let heading = SceneTextMark {
        text: state.heading().into(),
        font_size: 15.0.into(),
        ..Default::default()
    };
    let heading_height = engine
        .bounds(&heading.labels().next().unwrap().label)
        .map_or(0.0, |bounds| bounds.height);

    let [width, height] = size;
    let options = SolveOptions {
        padding: Edges::all(16.0),
        ..SolveOptions::sized(width, height)
    };
    let mut parts = Parts {
        x: &x_axis,
        y: &y_axis,
        heading: heading_height + 10.0,
        controls: controls.measure(engine),
    };
    let solution = (layout().measure(&options, &mut parts).unwrap()).into_solution();
    let region = |id: Part| {
        solution
            .region(&id)
            .expect("every part has a region")
            .content
    };
    let plot = region(Part::Plot);
    let search = Part::Widget(SEARCH.to_string());
    state.controls = HashMap::from([(SEARCH.to_string(), region(search))]);

    let x_scale = LinearScale::configured(IMDB_DOMAIN, (0.0, f64::from(plot.width)));
    let y_scale = LinearScale::configured(ROTTEN_TOMATOES_DOMAIN, (f64::from(plot.height), 0.0));
    let points = |name: &str, rows: &RecordBatch, fill: [f32; 4], size: f32, stroke: [f32; 4]| {
        let column = |name: &str| rows.column_by_name(name).unwrap();
        SceneSymbolMark {
            name: name.to_string(),
            len: rows.num_rows() as u32,
            x: x_scale.scale_to_numeric(column(IMDB)).unwrap(),
            y: y_scale.scale_to_numeric(column(ROTTEN_TOMATOES)).unwrap(),
            fill: ColorOrGradient::Color(fill).into(),
            size: size.into(),
            stroke: ColorOrGradient::Color(stroke).into(),
            stroke_width: Some(1.0),
            ..Default::default()
        }
    };
    let matches = filter_record_batch(&state.movies, &state.matches).unwrap();
    let others = filter_record_batch(&state.movies, &not(&state.matches).unwrap()).unwrap();
    // Without a query, every movie matches, and the points stay small.
    let none = [0.0; 4];
    let (fill, size, stroke) = match state.query.is_empty() {
        true => ([ACCENT[0], ACCENT[1], ACCENT[2], 0.5], 16.0, none),
        false => (ACCENT, 50.0, [1.0; 4]),
    };
    let gray = [0.62, 0.64, 0.68, 0.45];
    let plot_size = [plot.width, plot.height];
    let plot_group = SceneGroup {
        origin: [plot.x, plot.y],
        marks: vec![
            y_axis.grid(plot_size).into(),
            x_axis.grid(plot_size).into(),
            // The ratings lie within the domains, so the points need no clip.
            points(OTHERS, &others, gray, 12.0, none).into(),
            points(MATCHES, &matches, fill, size, stroke).into(),
        ],
        ..Default::default()
    };

    let mut marks: Vec<SceneMark> = vec![plot_group.into()];
    for part in AXIS_PARTS {
        for (axis, id, length) in [
            (&x_axis, Part::X(part), plot.width),
            (&y_axis, Part::Y(part), plot.height),
        ] {
            let slab = region(id);
            if let Some(mut group) = axis.part(part, length) {
                group.origin = axis.origin([slab.x, slab.y, slab.width, slab.height]);
                marks.push(group.into());
            }
        }
    }
    let at = region(Part::Heading);
    marks.push(
        SceneTextMark {
            x: at.x.into(),
            y: at.y.into(),
            baseline: TextBaseline::Top.into(),
            ..heading
        }
        .into(),
    );

    let mut data = SceneData::new();
    data.insert(MATCHES, Arc::new(matches));
    data.insert(OTHERS, Arc::new(others));
    let scene = SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    };
    BuiltScene { scene, data }
}

/// The search box, which filters the movies by title as the user types.
fn controls() -> Widgets<SearchState> {
    let search = TextInput::new(SEARCH)
        .label("Title")
        .placeholder("Filter by title")
        .value(|s: &SearchState| s.query.clone())
        .on_change(|s, query| {
            s.search(query);
            Rebuild::tree(CHART)
        });
    Widgets::new(CONTROLS, 10, |s: &SearchState, id| {
        s.controls.get(id).copied()
    })
    .with(search)
}

/// A tooltip for the movie nearest the pointer, among the matches and the others.
fn tooltips() -> Tooltips {
    let movie = InstanceTarget::nearest(12.0)
        .with_marks([MATCHES, OTHERS])
        .with_header(TooltipField::new(TITLE))
        .with_fields([
            TooltipField::new(IMDB)
                .with_title("IMDB rating")
                .with_value_format(".1f"),
            TooltipField::new(ROTTEN_TOMATOES)
                .with_title("Rotten Tomatoes rating")
                .with_value_format("d")
                .with_value_label("#value%"),
            TooltipField::new(GENRE).with_title("Genre"),
            TooltipField::new(YEAR)
                .with_title("Year")
                .with_value_format("d"),
        ]);
    Tooltips::new(TOOLTIPS, 30).over([movie])
}

fn builder() -> AppBuilder<SearchState> {
    let state = SearchState::new();
    let text_engine = state.text_engine.clone();
    let controls = controls();
    let chart = MovieChart {
        controls: controls.clone(),
    };
    AvengerApp::builder(state, chart, text_engine)
        .size([760.0, 560.0])
        .tool(controls)
        .tool(TextSelection::new(SELECTION, 20))
        .tool(tooltips())
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
