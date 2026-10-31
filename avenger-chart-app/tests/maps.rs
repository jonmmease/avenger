//! A chart app's map plots, with the `geo` feature: navigation of their projections, boxes and
//! lassos of places, the views that updates keep, and the tiles that they draw through the app's
//! image cache.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_app::app::AvengerApp;
use avenger_chart::{proto, proto::select, Chart, Encode, Selection, TupleTest, Value};
use avenger_chart_app::{
    ChartApp, ChartAppState, ChartChange, ChartFeed, ChartUpdate, ProjectionView, Views,
};
use avenger_chart_eval::{PlotCoord, ProjectionIdentity};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseInput,
    WindowMouseWheel, WindowResizeEvent,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::Projection;
use avenger_image::{FetchResponse, ImageCache, ImageCacheOptions, ImageFetcher, Validators};
use avenger_scenegraph::marks::{image::SceneImageSource, mark::SceneMark};

const SIZE: [f32; 2] = [800.0, 400.0];

/// Cities with their longitudes and latitudes.
fn cities() -> RecordBatch {
    let rows = [
        ("London", -0.13, 51.51),
        ("Lagos", 3.38, 6.52),
        ("Cairo", 31.24, 30.04),
        ("New York", -74.01, 40.71),
        ("São Paulo", -46.63, -23.55),
        ("Mumbai", 72.88, 19.08),
        ("Tokyo", 139.69, 35.69),
        ("Sydney", 151.21, -33.87),
    ];
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "name",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|row| row.0))),
        ),
        (
            "lon",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.1))),
        ),
        (
            "lat",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.2))),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The names of the cities that are in Europe or Africa.
const EUROPE_AND_AFRICA: [&str; 3] = ["London", "Lagos", "Cairo"];

/// An Equal Earth projection that fits its data and fills the space of its plot, so the size of
/// the plot does not follow its data.
fn equal_earth() -> proto::Geo {
    let surface = proto::geo::Surface::Spherical(proto::Spherical {
        projection: "equal_earth".into(),
        ..Default::default()
    });
    proto::Geo {
        surface: Some(surface),
        fit: Some(proto::geo::Fit::FitData(proto::PlotData {})),
        aspect: Some(proto::geo::Aspect::FreeAspect(proto::FreeAspect {})),
        ..Default::default()
    }
}

/// An Equal Earth projection of the whole sphere, which fills the space of its plot, so a box or
/// a lasso can reach any place.
fn world() -> proto::Geo {
    proto::Geo {
        fit: Some(proto::geo::Fit::FitSphere(proto::WholeSphere {})),
        ..equal_earth()
    }
}

/// A predicate of the rows that the selection `brush` selects, which selects none while the
/// selection is empty.
fn selected() -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: "brush".into(),
                empty_selects_none: true,
                ..Default::default()
            },
        )),
    }
}

/// A map of `table` on `geo` with the id `id`, whose cities that the selection `brush` of
/// `resolution` selects are red, and which declares `tools`.
fn map_of(
    table: RecordBatch,
    id: &str,
    geo: proto::Geo,
    tools: &[proto::Tool],
    resolution: proto::SelectionResolution,
) -> Chart {
    let mut unit = Chart::unit(table, proto::SymbolMark::default())
        .id(id)
        .longitude("lon")
        .latitude("lat")
        .color(Encode::constant("gray").when(selected(), Value::Constant("red".into())))
        .coord(geo)
        .selection(Selection::new("brush", resolution));
    for tool in tools {
        unit = unit.tool(tool.clone());
    }
    unit.build().unwrap()
}

/// A map whose selection `brush` is a union of the boxes of each map.
fn map(table: RecordBatch, id: &str, geo: proto::Geo, tools: &[proto::Tool]) -> Chart {
    map_of(table, id, geo, tools, proto::SelectionResolution::Union)
}

/// A row of a map of the first cities and a map of the others, with the ids `west` and `east`,
/// which share a projection and declare `tools`.
fn row(tools: &[proto::Tool]) -> Chart {
    row_on(equal_earth(), tools)
}

/// A row of maps on `geo`, as `row` has.
fn row_on(geo: proto::Geo, tools: &[proto::Tool]) -> Chart {
    let all = cities();
    Chart::row([
        map(all.slice(0, 5), "west", geo.clone(), tools),
        map(all.slice(5, 3), "east", geo, tools),
    ])
    .projection(proto::Sharing::Shared)
    .build()
    .unwrap()
}

fn navigate() -> proto::Tool {
    proto::Navigate::default().into()
}

type Changes = Arc<Mutex<Vec<ChartChange>>>;

fn engine() -> avenger_typst_label::LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// A chart app of `chart`, with a feed, that records the changes it reports.
async fn start(app: ChartApp) -> (AvengerApp<ChartAppState>, ChartFeed, Changes) {
    let feed = ChartFeed::new();
    let changes = Changes::default();
    let heard = changes.clone();
    let app = app
        .size(SIZE)
        .feed(&feed)
        .on_change(move |change| heard.lock().unwrap().push(change.clone()))
        .build()
        .await
        .unwrap();
    (app, feed, changes)
}

/// Sends events 10 ms apart, so the throttles of drags take each move.
async fn send(app: &mut AvengerApp<ChartAppState>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn drag(from: [f32; 2], to: [f32; 2]) -> [WindowEvent; 5] {
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    [
        moved(from),
        mouse(ElementState::Pressed),
        moved(middle),
        moved(to),
        mouse(ElementState::Released),
    ]
}

fn double_click(at: [f32; 2]) -> [WindowEvent; 5] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

fn wheel(lines: f32) -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, lines),
    })
}

/// The projection of the map plot at `plot` at the last evaluation.
fn projection(app: &AvengerApp<ChartAppState>, plot: usize) -> Projection {
    match &app.state().plots()[plot].coord {
        PlotCoord::Geo { projection } => projection.clone(),
        _ => panic!("a map plot"),
    }
}

/// The rotation and the view of a projection.
fn shown(projection: &Projection) -> ([f64; 3], avenger_geo::View) {
    (projection.rotate, projection.view.unwrap())
}

/// The projection of the map view `view`.
fn map_view(view: &ProjectionView) -> &Projection {
    match view {
        ProjectionView::Map(projection) => projection,
        _ => panic!("a map view"),
    }
}

/// The center of the plot at `plot`.
fn center(app: &AvengerApp<ChartAppState>, plot: usize) -> [f32; 2] {
    let [x, y, width, height] = app.state().plots()[plot].rect;
    [x + width / 2.0, y + height / 2.0]
}

/// The position in the scene of the longitude and the latitude `place` on the map plot at `plot`.
fn at(app: &AvengerApp<ChartAppState>, plot: usize, [lon, lat]: [f64; 2]) -> [f32; 2] {
    let [x, y, ..] = app.state().plots()[plot].rect;
    let (px, py) = (projection(app, plot).build().unwrap())
        .project_point(lon, lat)
        .unwrap();
    [x + px as f32, y + py as f32]
}

/// The names of the cities of the map plot at `plot` that are red, in the order of its rows.
fn reds(app: &AvengerApp<ChartAppState>, plot: usize) -> Vec<String> {
    let state = app.state();
    let name = &state.plots()[plot].name;
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
    let table = &state.chart().tables()[0];
    let names = table.column_by_name("name").unwrap();
    let names = names.as_any().downcast_ref::<StringArray>().unwrap();
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .enumerate()
        .filter(|(_, fill)| {
            matches!(fill, avenger_color::ColorOrGradient::Color(color)
                if color[0] > 0.9 && color[1] < 0.1)
        })
        .map(|(row, _)| names.value(row).to_string())
        .collect()
}

/// The contributions of the selection `brush`.
fn contributions(app: &AvengerApp<ChartAppState>) -> Vec<avenger_chart::Contribution> {
    (app.state().evaluator().selections().get("brush"))
        .cloned()
        .unwrap_or_default()
}

#[tokio::test(flavor = "current_thread")]
async fn a_drag_on_one_map_turns_every_map_of_its_projection_and_settles_once() {
    let (mut app, _, changes) = start(ChartApp::new(row(&[navigate()]), engine())).await;
    let fitted = projection(&app, 0);
    let from = center(&app, 1);
    send(&mut app, drag(from, [from[0] + 40.0, from[1]])).await;
    let [west, east] = [0, 1].map(|plot| projection(&app, plot));
    // Equal Earth pans by longitude, so the drag turns the sphere.
    assert_ne!(west.rotate, fitted.rotate);
    assert_eq!(shown(&west), shown(&east));

    let changes = changes.lock().unwrap();
    let [ChartChange {
        projections,
        settled: true,
        ..
    }] = &changes[..]
    else {
        panic!("one settled change, and not {changes:?}");
    };
    let [(identity, settled)] = &projections[..] else {
        panic!("one projection, and not {projections:?}");
    };
    let plot = app.state().plots()[0].identity.clone().unwrap();
    assert_eq!(identity, &ProjectionIdentity::Plot(plot));
    assert_eq!(shown(map_view(settled)), shown(&west));
}

#[tokio::test(flavor = "current_thread")]
async fn a_double_click_returns_a_map_to_its_fit() {
    let (mut app, _, changes) = start(ChartApp::new(row(&[navigate()]), engine())).await;
    let fitted = projection(&app, 0);
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    assert_ne!(shown(&projection(&app, 0)), shown(&fitted));
    send(&mut app, double_click(at)).await;
    assert_eq!(shown(&projection(&app, 1)), shown(&fitted));
    assert!(app.state().evaluator().projection_view(0).is_none());
    let changes = changes.lock().unwrap();
    let last = changes.last().expect("the change of the double click");
    assert_eq!(shown(map_view(&last.projections[0].1)), shown(&fitted));
}

#[tokio::test(flavor = "current_thread")]
async fn a_press_without_a_drag_leaves_a_map_fitting_its_data() {
    let (mut app, feed, _) = start(ChartApp::new(row(&[navigate()]), engine())).await;
    let at = center(&app, 0);
    send(
        &mut app,
        [
            moved(at),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
    // The new chart moves the cities between the maps, so its projection fits them again.
    let all = cities();
    let swapped = Chart::row([
        map(all.slice(0, 3), "west", equal_earth(), &[navigate()]),
        map(all.slice(3, 5), "east", equal_earth(), &[navigate()]),
    ])
    .projection(proto::Sharing::Shared)
    .build()
    .unwrap();
    feed.send(swapped.clone()).unwrap();
    app.refresh().await;
    let (alone, _, _) = start(ChartApp::new(swapped, engine())).await;
    assert_eq!(shown(&projection(&app, 0)), shown(&projection(&alone, 0)));
    assert!(app.state().evaluator().projection_view(0).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn a_map_pans_in_the_mode_and_zooms_within_the_limits_of_its_navigation() {
    let limited = proto::Geo {
        navigation: Some(proto::GeoNavigation {
            pan_mode: proto::PanMode::Translate as i32,
            min_zoom: Some(1.0),
            max_zoom: Some(1.5),
        }),
        ..equal_earth()
    };
    let chart = row_on(limited, &[navigate()]);
    let (mut app, _, _) = start(ChartApp::new(chart, engine())).await;
    let fitted = shown(&projection(&app, 0));
    let at = center(&app, 0);
    send(&mut app, drag(at, [at[0] + 30.0, at[1] + 10.0])).await;
    let (rotate, view) = shown(&projection(&app, 0));
    // A translation moves the view and keeps the rotation.
    assert_eq!(rotate, fitted.0);
    assert!((view.translate[0] - fitted.1.translate[0] - 30.0).abs() < 1e-3);

    let scale = |app: &AvengerApp<ChartAppState>| shown(&projection(app, 0)).1.scale;
    send(&mut app, [moved(at), wheel(2.0), wheel(2.0)]).await;
    assert!((scale(&app) / fitted.1.scale - 1.5).abs() < 1e-9);
    send(&mut app, [wheel(-2.0), wheel(-2.0), wheel(-2.0)]).await;
    assert!((scale(&app) / fitted.1.scale - 1.0).abs() < 1e-9);
}

#[tokio::test(flavor = "current_thread")]
async fn a_resize_scales_the_view_of_a_map_with_its_plot() {
    let (mut app, _, _) = start(ChartApp::new(row(&[navigate()]), engine())).await;
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    let ratio = |app: &AvengerApp<ChartAppState>| {
        let home = app.state().evaluator().home_projection(0).unwrap();
        shown(&projection(app, 0)).1.scale / map_view(&home).view.unwrap().scale
    };
    let zoomed = ratio(&app);
    assert!((zoomed - 1.44).abs() < 1e-9, "{zoomed}");
    let resize = WindowEvent::WindowResize(WindowResizeEvent {
        size: [1200.0, 600.0],
    });
    send(&mut app, [resize]).await;
    app.refresh().await;
    assert!(app.state().plots()[0].rect[2] > 500.0);
    assert!((ratio(&app) - zoomed).abs() < 1e-9);
}

/// A declaration that writes the selection `brush` with `form`.
fn select(form: select::Form) -> proto::Tool {
    proto::Select {
        selection: "brush".into(),
        form: Some(form),
        scope: Vec::new(),
    }
    .into()
}

/// A map of every city that writes the selection `brush` with `form`.
fn selecting(form: select::Form) -> Chart {
    map(cities(), "world", world(), &[select(form)])
}

/// The indices of the plots whose areas hold the boxes on maps that the brush scene draws.
fn plots_with_map_boxes(app: &AvengerApp<ChartAppState>) -> Vec<usize> {
    let mut plots: Vec<usize> = (app.scenes()[1].marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Group(group)
                if group.marks.iter().any(
                    |mark| matches!(mark, SceneMark::Path(path) if path.name == "brush_box"),
                ) =>
            {
                (app.state().plots().iter()).position(|plot| {
                    (plot.rect[0] - group.origin[0]).abs() < 0.5
                        && (plot.rect[1] - group.origin[1]).abs() < 0.5
                })
            }
            _ => None,
        })
        .collect();
    plots.sort();
    plots
}

#[tokio::test(flavor = "current_thread")]
async fn a_box_on_a_map_writes_ranges_of_longitude_and_latitude() {
    let interval = select::Form::Interval(proto::IntervalSelect::default());
    let (mut app, _, changes) = start(ChartApp::new(selecting(interval), engine())).await;
    let from = at(&app, 0, [-20.0, 60.0]);
    let to = at(&app, 0, [40.0, 0.0]);
    send(&mut app, drag(from, to)).await;
    let written = contributions(&app);
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    let close = |a: f64, b: f64| (a - b).abs() < 0.1;
    let tuple = &contribution.tuples[0];
    assert!(tuple.iter().any(|test| matches!(test,
        TupleTest::CyclicRange { column, start, end, period, .. }
            if column == "lon" && close(*start, -20.0) && close(*end, 40.0) && *period == 360.0)));
    assert!(tuple.iter().any(|test| match test {
        TupleTest::Range { column, ends, .. } if column == "lat" => {
            let ends = ends.as_any().downcast_ref::<Float64Array>().unwrap();
            close(ends.value(0), 0.0) && close(ends.value(1), 60.0)
        }
        _ => false,
    }));
    assert_eq!(reds(&app, 0), EUROPE_AND_AFRICA);
    assert!(changes.lock().unwrap().last().unwrap().settled);
}

#[tokio::test(flavor = "current_thread")]
async fn a_box_on_the_longitude_of_a_map_writes_a_cyclic_range_alone() {
    let interval = select::Form::Interval(proto::IntervalSelect {
        channels: vec![proto::Channel::Longitude as i32],
    });
    let (mut app, _, _) = start(ChartApp::new(selecting(interval), engine())).await;
    let from = at(&app, 0, [-20.0, 60.0]);
    let to = at(&app, 0, [40.0, 0.0]);
    send(&mut app, drag(from, to)).await;
    let written = contributions(&app);
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert!(matches!(
        contribution.tuples[0].as_slice(),
        [TupleTest::CyclicRange { column, .. }] if column == "lon"
    ));
    assert_eq!(reds(&app, 0), EUROPE_AND_AFRICA);
}

#[tokio::test(flavor = "current_thread")]
async fn a_box_on_one_map_shows_on_each_map_that_draws_its_places() {
    let boxes = select(select::Form::Interval(proto::IntervalSelect::default()));
    let global = proto::SelectionResolution::Global;
    let chart = Chart::row([
        map_of(
            cities(),
            "left",
            world(),
            std::slice::from_ref(&boxes),
            global,
        ),
        map_of(cities(), "right", world(), &[boxes], global),
    ])
    .build()
    .unwrap();
    let (mut app, _, _) = start(ChartApp::new(chart, engine())).await;
    let from = at(&app, 0, [-20.0, 60.0]);
    let to = at(&app, 0, [40.0, 0.0]);
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_map_boxes(&app), [0, 1]);
    assert_eq!(reds(&app, 1), EUROPE_AND_AFRICA);
}

/// A lasso around `corners`, 10 pixels at a time, back to the first corner.
fn lasso_around(corners: [[f32; 2]; 4]) -> Vec<WindowEvent> {
    let mut events = vec![moved(corners[0]), mouse(ElementState::Pressed)];
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / 10.0).ceil() as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            events.push(moved([
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]));
        }
    }
    events.push(mouse(ElementState::Released));
    events
}

#[tokio::test(flavor = "current_thread")]
async fn a_lasso_on_a_map_selects_the_places_inside_it() {
    let lasso = select::Form::Lasso(proto::LassoSelect::default());
    let (mut app, _, _) = start(ChartApp::new(selecting(lasso), engine())).await;
    let corners =
        [[-20.0, 60.0], [40.0, 60.0], [40.0, 0.0], [-20.0, 0.0]].map(|corner| at(&app, 0, corner));
    send(&mut app, lasso_around(corners)).await;
    let written = contributions(&app);
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert!(matches!(
        &contribution.tuples[0][0],
        TupleTest::Polygon { x_column, y_column, .. } if x_column == "lon" && y_column == "lat"
    ));
    assert_eq!(reds(&app, 0), EUROPE_AND_AFRICA);
}

#[tokio::test(flavor = "current_thread")]
async fn a_lasso_with_a_key_on_a_map_selects_the_keys_of_the_places_inside_it() {
    let lasso = select::Form::Lasso(proto::LassoSelect { key: "name".into() });
    let (mut app, _, _) = start(ChartApp::new(selecting(lasso), engine())).await;
    let corners =
        [[-20.0, 60.0], [40.0, 60.0], [40.0, 0.0], [-20.0, 0.0]].map(|corner| at(&app, 0, corner));
    send(&mut app, lasso_around(corners)).await;
    let written = contributions(&app);
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert!(matches!(
        contribution.tuples[0].as_slice(),
        [TupleTest::Set { column, .. }] if column == "name"
    ));
    assert_eq!(reds(&app, 0), EUROPE_AND_AFRICA);
}

#[tokio::test(flavor = "current_thread")]
async fn an_update_keeps_the_view_of_a_projection_or_drops_it() {
    let (mut app, feed, _) = start(ChartApp::new(row(&[navigate()]), engine())).await;
    let from = center(&app, 0);
    send(&mut app, drag(from, [from[0] + 40.0, from[1]])).await;
    let moved = shown(&projection(&app, 0));

    // The new chart moves the cities between the maps, so the projection fits them again, and
    // the view keeps the places that it shows.
    let all = cities();
    let swapped = Chart::row([
        map(all.slice(0, 3), "west", equal_earth(), &[navigate()]),
        map(all.slice(3, 5), "east", equal_earth(), &[navigate()]),
    ])
    .projection(proto::Sharing::Shared)
    .build()
    .unwrap();
    feed.send(swapped.clone()).unwrap();
    app.refresh().await;
    assert_eq!(app.state().chart().tables()[0].num_rows(), 3);
    assert_eq!(shown(&projection(&app, 1)), moved);

    feed.send(ChartUpdate::new(swapped).views(Views::Drop))
        .unwrap();
    app.refresh().await;
    assert!(app.state().evaluator().projection_view(0).is_none());
    assert_ne!(shown(&projection(&app, 0)), moved);
}

/// A fetcher that serves a one-pixel PNG for every URL.
struct PixelFetcher;

impl ImageFetcher for PixelFetcher {
    fn fetch(
        &self,
        _url: &str,
        _: &Validators,
    ) -> Result<FetchResponse, avenger_image::error::AvengerImageError> {
        let mut png = std::io::Cursor::new(Vec::new());
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

/// The URLs of the tiles that the scene of `app` draws.
fn tile_urls(app: &AvengerApp<ChartAppState>) -> Vec<String> {
    fn visit(marks: &[SceneMark], urls: &mut Vec<String>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => visit(&group.marks, urls),
                SceneMark::MeshImage(mesh) => urls.extend(mesh.instances.iter().filter_map(
                    |instance| match &instance.image {
                        SceneImageSource::Resource(resource) => Some(resource.url.clone()),
                        SceneImageSource::Inline(_) => None,
                    },
                )),
                _ => {}
            }
        }
    }
    let mut urls = Vec::new();
    visit(&app.scenes()[0].marks, &mut urls);
    urls
}

#[tokio::test(flavor = "current_thread")]
async fn a_map_draws_its_tiles_through_the_image_cache_of_its_app() {
    let tiled = |table: RecordBatch| {
        let surface = proto::geo::Surface::Spherical(proto::Spherical {
            projection: "mercator".into(),
            ..Default::default()
        });
        let geo = proto::Geo {
            surface: Some(surface),
            tiles: Some(proto::TileLayer {
                url_template: "https://tiles.test/{z}/{x}/{y}.png".into(),
                ..Default::default()
            }),
            ..equal_earth()
        };
        map(table, "world", geo, &[navigate()])
    };
    let cache = ImageCache::with_options(ImageCacheOptions {
        fetcher: Arc::new(PixelFetcher),
        ..Default::default()
    });
    let all = cities();
    let app = ChartApp::new(tiled(all.slice(0, 3)), engine()).images(cache);
    let (mut app, feed, _) = start(app).await;
    let urls = tile_urls(&app);
    assert!(!urls.is_empty());
    assert!(urls
        .iter()
        .all(|url| url.starts_with("https://tiles.test/")));
    assert!(app.images().is_some());

    // The map of an installed update loads its tiles through the same cache.
    feed.send(tiled(all.slice(3, 5))).unwrap();
    app.refresh().await;
    assert_eq!(app.state().chart().tables()[0].num_rows(), 5);
    assert!(!tile_urls(&app).is_empty());
}
