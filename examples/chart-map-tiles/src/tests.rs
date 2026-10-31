//! The map draws its tiles through the app's image cache, which a test fetcher serves here, and a
//! box on the scatter plot fades the earthquakes outside it on the map.

use std::time::Duration;

use arrow::array::{ArrayRef, Float64Array};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use avenger_app::app::AvengerApp;
use avenger_chart::TupleTest;
use avenger_chart_app::ChartAppState;
use avenger_chart_eval::PlotCoord;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_image::{FetchResponse, ImageCacheOptions, ImageFetcher, Validators};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{image::SceneImageSource, mark::SceneMark};

use super::*;

/// The indices of the plots.
const SCATTER: usize = 0;
const MAP: usize = 1;

/// A fetcher that serves a one-pixel PNG for every URL, so the tests fetch nothing.
struct PixelFetcher;

impl ImageFetcher for PixelFetcher {
    fn fetch(
        &self,
        _url: &str,
        _: &Validators,
    ) -> Result<FetchResponse, avenger_image::error::AvengerImageError> {
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        Ok(FetchResponse {
            status: 200,
            body: png.into_inner(),
            ..Default::default()
        })
    }
}

/// The app of the chart over OpenStreetMap's tiles, which the test fetcher serves.
fn app() -> AvengerApp<ChartAppState> {
    let cache = ImageCache::with_options(ImageCacheOptions {
        fetcher: Arc::new(PixelFetcher),
        ..Default::default()
    });
    let app = ChartApp::new(chart(OSM_TILES), engine())
        .size(SIZE)
        .images(cache);
    pollster::block_on(app.build()).unwrap()
}

/// The URLs of the tiles that `marks` draw, at any depth.
fn tile_urls(marks: &[SceneMark]) -> Vec<String> {
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Group(group) => tile_urls(&group.marks),
            SceneMark::MeshImage(mesh) => (mesh.instances.iter())
                .filter_map(|instance| match &instance.image {
                    SceneImageSource::Resource(resource) => Some(resource.url.clone()),
                    SceneImageSource::Inline(_) => None,
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn the_map_draws_its_tiles_and_the_earthquakes() {
    let app = app();
    let state = app.state();
    assert!(state.warnings().is_empty(), "{:?}", state.warnings());
    assert!(matches!(state.plots()[MAP].coord, PlotCoord::Geo { .. }));
    let urls = tile_urls(&app.scenes()[0].marks);
    assert!(!urls.is_empty());
    assert!(urls
        .iter()
        .all(|url| url.starts_with("https://tile.openstreetmap.org/")));
    assert_eq!(opaque(&app, MAP), earthquakes().num_rows());
}

/// The number of symbols of the plot at `plot` that draw in full.
fn opaque(app: &AvengerApp<ChartAppState>, plot: usize) -> usize {
    let name = &app.state().plots()[plot].name;
    let group = (app.scenes()[0].marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == *name => Some(group),
            _ => None,
        })
        .expect("the group of the plot");
    let symbol = (group.marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            _ => None,
        })
        .expect("the symbols of the plot");
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .filter(|fill| match fill {
            avenger_color::ColorOrGradient::Color(color) => color[3] > 0.5,
            _ => false,
        })
        .count()
}

/// The position in the window of the magnitude and the depth `values` on the scatter plot.
fn at(app: &AvengerApp<ChartAppState>, values: [f64; 2]) -> [f32; 2] {
    let plot = &app.state().plots()[SCATTER];
    let PlotCoord::Cartesian {
        x: Some(x),
        y: Some(y),
        ..
    } = &plot.coord
    else {
        panic!("a Cartesian plot with x and y");
    };
    let position = |scale: &ConfiguredScale, value: f64| {
        let value: ArrayRef = Arc::new(Float64Array::from(vec![value]));
        scale.scale_to_numeric(&value).unwrap().as_vec(1, None)[0]
    };
    [
        plot.rect[0] + position(x, values[0]),
        plot.rect[1] + position(y, values[1]),
    ]
}

/// The ends of the range of `column` in the box that the selection holds.
fn range(app: &AvengerApp<ChartAppState>, column: &str) -> [f64; 2] {
    let contributions = &app.state().evaluator().selections()[QUAKES];
    (contributions[0].tuples[0].iter())
        .find_map(|test| match test {
            TupleTest::Range {
                column: own, ends, ..
            } if own == column => {
                let ends = cast(ends, &DataType::Float64).unwrap();
                let ends = ends.as_primitive::<Float64Type>();
                Some([ends.value(0), ends.value(1)])
            }
            _ => None,
        })
        .expect("a range of the column")
}

#[test]
fn a_box_on_the_scatter_plot_fades_the_earthquakes_outside_it_on_the_map() {
    let mut app = app();
    let (from, to) = (at(&app, [8.0, 0.0]), at(&app, [9.5, 60.0]));
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    let mouse = |state| {
        WindowEvent::MouseInput(WindowMouseInput {
            state,
            button: MouseButton::Left,
        })
    };
    let moved = |position| WindowEvent::CursorMoved(WindowCursorMoved { position });
    for event in [
        moved(from),
        mouse(ElementState::Pressed),
        moved(middle),
        moved(to),
        mouse(ElementState::Released),
    ] {
        std::thread::sleep(Duration::from_millis(10));
        pollster::block_on(app.update(&event, Instant::now()));
    }

    // The map shows the earthquakes inside the box in full.
    let [low, high] = range(&app, "magnitude");
    let [shallow, deep] = range(&app, "depth_km");
    let quakes = earthquakes();
    let numbers = |name: &str| {
        let column = cast(quakes.column_by_name(name).unwrap(), &DataType::Float64).unwrap();
        column.as_primitive::<Float64Type>().values().to_vec()
    };
    let inside = (numbers("magnitude").into_iter())
        .zip(numbers("depth_km"))
        .filter(|(magnitude, depth)| {
            (low..=high).contains(magnitude) && (shallow..=deep).contains(depth)
        })
        .count();
    assert!(inside > 0 && inside < quakes.num_rows(), "{inside}");
    assert_eq!(opaque(&app, MAP), inside);
    assert_eq!(opaque(&app, SCATTER), inside);
}
